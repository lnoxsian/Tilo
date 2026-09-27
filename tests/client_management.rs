use std::process::{Child, Command};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, WindowClass,
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
            .arg("800x600")
            .arg("-ac")
            .arg("-br")
            .arg("-noreset")
            .spawn()
            .ok()?;

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
fn test_client_lifecycle_and_management_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(91);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        // Client connection to create windows
        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // 1. Create Window 1
        let win1 = client_conn.generate_id().expect("Failed XID 1");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win1,
                root,
                0,
                0,
                200,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win1 failed");
        client_conn.flush().unwrap();

        // Manage win1
        let managed = wm.manage_window(win1).expect("Manage win1 failed");
        assert!(managed, "win1 should be newly managed");
        assert_eq!(wm.client_state.len(), 1);
        assert!(wm.client_state.contains(win1));
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // 2. Move and resize win1
        wm.move_resize_client(win1, 50, 75, 400, 300)
            .expect("move_resize_client failed");
        let geom = client_conn
            .get_geometry(win1)
            .unwrap()
            .reply()
            .expect("get_geometry failed");
        assert_eq!(geom.x, 50);
        assert_eq!(geom.y, 75);
        assert_eq!(geom.width, 400);
        assert_eq!(geom.height, 300);

        // 3. Create and manage Window 2
        let win2 = client_conn.generate_id().expect("Failed XID 2");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win2,
                root,
                100,
                100,
                300,
                250,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win2 failed");
        client_conn.flush().unwrap();

        let managed2 = wm.manage_window(win2).expect("Manage win2 failed");
        assert!(managed2);
        assert_eq!(wm.client_state.len(), 2);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 4. Focus switching
        wm.focus_client(win1).expect("Focus win1 failed");
        assert_eq!(wm.client_state.focused_window(), Some(win1));
        assert!(wm.client_state.get_client(win1).unwrap().focused);
        assert!(!wm.client_state.get_client(win2).unwrap().focused);

        // 5. Override redirect window should NOT be managed
        let override_win = client_conn.generate_id().expect("Failed XID override");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                override_win,
                root,
                0,
                0,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default().override_redirect(1),
            )
            .expect("Create override win failed");
        client_conn.flush().unwrap();

        let managed_override = wm.manage_window(override_win).expect("Manage override failed");
        assert!(!managed_override, "Override-redirect window must be ignored");
        assert_eq!(wm.client_state.len(), 2);

        // 6. Unmanage win1 and check focus restoration to win2
        let unmanaged = wm.unmanage_window(win1);
        assert!(unmanaged.is_some());
        assert_eq!(unmanaged.unwrap().window, win1);
        assert_eq!(wm.client_state.len(), 1);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 7. Kill remaining win2
        wm.kill_client(win2).expect("Kill win2 failed");
        wm.unmanage_window(win2);
        assert_eq!(wm.client_state.len(), 0);
        assert_eq!(wm.client_state.focused_window(), None);

        wm.stop();
    }
}
