use std::process::{Child, Command};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, MapState, WindowClass,
};

struct XephyrServer {
    child: Child,
    display: String,
}

impl XephyrServer {
    fn start(display_num: u32) -> Option<Self> {
        let display = format!(":{}", display_num);
        let child = Command::new("Xephyr")
            .arg(&display)
            .arg("-screen")
            .arg("640x480")
            .arg("-ac")
            .arg("-br")
            .arg("-noreset")
            .spawn()
            .ok()?;

        // Wait up to 3 seconds for Xephyr to accept connections
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if X11Context::connect_to(Some(&display)).is_ok() {
                return Some(Self { child, display });
            }
        }

        let mut s = Self { child, display };
        s.kill();
        None
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for XephyrServer {
    fn drop(&mut self) {
        self.kill();
    }
}

#[test]
fn test_event_loop_map_and_destroy_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(92);
    if let Some(server) = xephyr {
        // 1. Initialize WM on Xephyr
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok(), "Failed to become WM on Xephyr");

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WindowManager");

        // 2. Connect client and create a window
        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let test_window = client_conn.generate_id().expect("Failed to generate XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                test_window,
                root,
                10,
                10,
                200,
                150,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Failed to create window");

        // 3. Request mapping the window
        client_conn.map_window(test_window).expect("Map window failed");
        client_conn.flush().expect("Client flush failed");

        // Give server a moment to deliver the MapRequest event
        std::thread::sleep(std::time::Duration::from_millis(100));

        // 4. Process pending events on WM side
        let processed = wm.process_pending_events().expect("Failed processing events");
        assert!(processed >= 1, "Expected at least 1 event, got {}", processed);
        assert!(
            wm.stats.map_requests >= 1,
            "Expected map request to be recorded in stats"
        );

        // 5. Verify the window is now mapped
        let attrs = client_conn
            .get_window_attributes(test_window)
            .expect("Failed getting attrs")
            .reply()
            .expect("Failed to get reply");
        assert_eq!(
            attrs.map_state,
            MapState::VIEWABLE,
            "Window should be mapped and viewable by WM"
        );

        // 6. Destroy window and verify DestroyNotify
        client_conn
            .destroy_window(test_window)
            .expect("Failed to destroy window");
        client_conn.flush().expect("Client flush failed");

        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = wm.process_pending_events().expect("Failed processing destroy events");
        assert!(
            wm.stats.destroy_notifications >= 1,
            "Expected destroy notification to be recorded in stats"
        );

        wm.stop();
        assert!(!wm.running);
    }
}

#[test]
fn test_event_driven_run_loop_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(93);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok(), "Failed to become WM on Xephyr");

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WindowManager");

        // Spawn a thread to run the event-driven run loop
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let _ = tx.send(());
            let res = wm.run();
            (res, start.elapsed())
        });

        // Wait until run() has started
        let _ = rx.recv();
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Connect client and send a MapRequest
        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let test_window = client_conn.generate_id().expect("Failed to generate XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                test_window,
                root,
                10,
                10,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Failed to create window");
        client_conn.map_window(test_window).expect("Map window failed");
        client_conn.flush().expect("Client flush failed");

        // The event loop should process it almost immediately
        std::thread::sleep(std::time::Duration::from_millis(100));

        let attrs = client_conn
            .get_window_attributes(test_window)
            .expect("Failed getting attrs")
            .reply()
            .expect("Failed to get reply");
        assert_eq!(
            attrs.map_state,
            MapState::VIEWABLE,
            "Window should be mapped and viewable by event-driven WM"
        );

        // Send SIGTERM to self to test signal-driven wakeup and termination
        unsafe {
            libc::raise(libc::SIGTERM);
        }

        let (res, elapsed) = handle.join().expect("WM thread panicked");
        assert!(res.is_ok(), "run() exited with error: {:?}", res);
        assert!(elapsed < std::time::Duration::from_secs(2), "WM took too long to terminate: {:?}", elapsed);
    }
}
