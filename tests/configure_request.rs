use std::process::{Child, Command};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, CreateWindowAux,
    EventMask, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

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
            .arg("1000x800")
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
fn test_tiled_window_configure_request_holds_geometry() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(116);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = || {
            let win = client_conn.generate_id().expect("Failed XID");
            client_conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    win,
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
                .expect("Failed to create client window");
            client_conn.flush().unwrap();
            client_conn.sync().unwrap();
            win
        };

        let win1 = create_win();
        let win2 = create_win();

        // Subscribe client_conn to StructureNotify on win1 so it receives synthetic ConfigureNotify
        let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::STRUCTURE_NOTIFY);
        client_conn.change_window_attributes(win1, &aux).unwrap();
        client_conn.flush().unwrap();

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());

        // Drain any initial events on client connection
        while client_conn.poll_for_event().unwrap().is_some() {}

        let initial_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let initial_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(initial_geom1.width < 1000);

        // Tiled windows should partition screen width (< 1000)
        assert!(initial_geom1.width < 1000);
        assert!(initial_geom2.width < 1000);

        // Client 1 attempts to reconfigure its size and position outside its tile
        let req_aux = ConfigureWindowAux::default()
            .x(50)
            .y(50)
            .width(850)
            .height(700);
        client_conn.configure_window(win1, &req_aux).unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = wm.process_pending_events();

        // Check geometry: win1 MUST hold its authoritative tile geometry!
        let current_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let current_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();

        assert_eq!(current_geom1.x, initial_geom1.x, "X position should remain unchanged");
        assert_eq!(current_geom1.y, initial_geom1.y, "Y position should remain unchanged");
        assert_eq!(current_geom1.width, initial_geom1.width, "Width should remain authoritative");
        assert_eq!(current_geom1.height, initial_geom1.height, "Height should remain authoritative");

        // win2 must not be overlapped or affected
        assert_eq!(current_geom2.x, initial_geom2.x);
        assert_eq!(current_geom2.width, initial_geom2.width);

        // Check that client received synthetic ConfigureNotify
        let mut got_synth_notify = false;
        while let Some(ev) = client_conn.poll_for_event().unwrap() {
            if let Event::ConfigureNotify(conf) = ev {
                if conf.window == win1 {
                    assert_eq!(conf.x, initial_geom1.x);
                    assert_eq!(conf.y, initial_geom1.y);
                    assert_eq!(conf.width, initial_geom1.width);
                    assert_eq!(conf.height, initial_geom1.height);
                    got_synth_notify = true;
                    break;
                }
            }
        }
        assert!(got_synth_notify, "Client should receive synthetic ConfigureNotify");
    }
}

#[test]
fn test_floating_window_configure_request_applies_size_hints() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(117);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let win = client_conn.generate_id().expect("Failed XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0,
                0,
                300,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Failed to create client window");

        // Set size hints: base 100x100, width_inc 20, height_inc 15
        let flags = (1 << 6) | (1 << 8); // PResizeInc | PBaseSize
        let mut normal_hints_data = vec![0u32; 18];
        normal_hints_data[0] = flags;
        normal_hints_data[9] = 20; // width_inc
        normal_hints_data[10] = 15; // height_inc
        normal_hints_data[15] = 100; // base_width
        normal_hints_data[16] = 100; // base_height

        client_conn
            .change_property32(
                PropMode::REPLACE,
                win,
                AtomEnum::WM_NORMAL_HINTS,
                AtomEnum::WM_SIZE_HINTS,
                &normal_hints_data,
            )
            .unwrap();

        let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::STRUCTURE_NOTIFY);
        client_conn.change_window_attributes(win, &aux).unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        assert!(wm.manage_window(win).unwrap());
        // Toggle to floating
        wm.toggle_floating(win).unwrap();
        assert!(wm.client_state.get_client(win).unwrap().floating);

        // Issue ConfigureRequest with non-increment values: width 413, height 319
        // 413 -> 100 + (313 / 20)*20 = 400
        // 319 -> 100 + (219 / 15)*15 = 310
        let req_aux = ConfigureWindowAux::default()
            .x(80)
            .y(80)
            .width(413)
            .height(319);
        client_conn.configure_window(win, &req_aux).unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = wm.process_pending_events();

        let geom = client_conn.get_geometry(win).unwrap().reply().unwrap();
        assert_eq!(geom.width, 400, "Width should be snapped to increment");
        assert_eq!(geom.height, 310, "Height should be snapped to increment");
    }
}

#[test]
fn test_unmanaged_window_configure_request_passthrough() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(118);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let unmanaged_win = client_conn.generate_id().expect("Failed XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                unmanaged_win,
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
            .expect("Failed to create client window");
        client_conn.flush().unwrap();

        // Do NOT manage the window
        assert!(!wm.client_state.contains(unmanaged_win));

        // Send ConfigureRequest
        let req_aux = ConfigureWindowAux::default()
            .x(123)
            .y(234)
            .width(345)
            .height(456);
        client_conn.configure_window(unmanaged_win, &req_aux).unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = wm.process_pending_events();

        let geom = client_conn.get_geometry(unmanaged_win).unwrap().reply().unwrap();
        assert_eq!(geom.x, 123);
        assert_eq!(geom.y, 234);
        assert_eq!(geom.width, 345);
        assert_eq!(geom.height, 456);
    }
}

#[test]
fn test_tiled_window_constraint_fallback_to_floating() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(119);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = || {
            let win = client_conn.generate_id().expect("Failed XID");
            client_conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    win,
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
                .expect("Failed to create client window");
            client_conn.flush().unwrap();
            client_conn.sync().unwrap();
            win
        };

        let win1 = create_win();
        let win2 = create_win();

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());

        assert!(!wm.client_state.get_client(win1).unwrap().floating);
        assert!(!wm.client_state.get_client(win2).unwrap().floating);
        assert_eq!(
            wm.client_state.visible_tiled_clients_on_workspace(1).len(),
            2
        );

        let initial_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let initial_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(initial_geom1.width < 1000);

        // Now set WM_NORMAL_HINTS on win1 with min_width exceeding tile width (e.g. 800px)
        let flags = 1 << 4; // PMinSize
        let mut normal_hints_data = vec![0u32; 18];
        normal_hints_data[0] = flags;
        normal_hints_data[5] = 800; // min_width > initial tile width (which is < 500)
        normal_hints_data[6] = 400; // min_height

        client_conn
            .change_property32(
                PropMode::REPLACE,
                win1,
                AtomEnum::WM_NORMAL_HINTS,
                AtomEnum::WM_SIZE_HINTS,
                &normal_hints_data,
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = wm.process_pending_events();

        // win1 should now have fallen back to floating!
        assert!(
            wm.client_state.get_client(win1).unwrap().floating,
            "Window with min_width exceeding tile should fall back to floating"
        );

        // Remaining tiled windows on workspace 1 should now only be win2, which expands
        assert_eq!(
            wm.client_state.visible_tiled_clients_on_workspace(1).len(),
            1
        );
        let retiled_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(
            retiled_geom2.width > initial_geom2.width,
            "Remaining tiled window should expand to fill workspace"
        );
    }
}

#[test]
fn test_fixed_size_window_manages_as_floating() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(120);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let win = client_conn.generate_id().expect("Failed XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0,
                0,
                350,
                250,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Failed to create client window");

        // Set fixed-size hints (min == max)
        let flags = (1 << 4) | (1 << 5); // PMinSize | PMaxSize
        let mut normal_hints_data = vec![0u32; 18];
        normal_hints_data[0] = flags;
        normal_hints_data[5] = 350; // min_width
        normal_hints_data[6] = 250; // min_height
        normal_hints_data[7] = 350; // max_width
        normal_hints_data[8] = 250; // max_height

        client_conn
            .change_property32(
                PropMode::REPLACE,
                win,
                AtomEnum::WM_NORMAL_HINTS,
                AtomEnum::WM_SIZE_HINTS,
                &normal_hints_data,
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        // Manage window
        assert!(wm.manage_window(win).unwrap());

        // Should automatically be floating
        assert!(
            wm.client_state.get_client(win).unwrap().floating,
            "Fixed-size window should default to floating"
        );
        let geom = client_conn.get_geometry(win).unwrap().reply().unwrap();
        assert_eq!(geom.width, 350);
        assert_eq!(geom.height, 250);
    }
}
