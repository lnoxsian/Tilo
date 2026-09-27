use std::process::{Child, Command};
use tilo::floating::{ClientLayoutState, FloatingGeometry};
use tilo::keybind::Action;
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, WindowClass,
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
fn test_floating_window_lifecycle_and_actions_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(97);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        // Client connection to create windows
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

        // 1. Create two windows on Workspace 1
        let win1 = create_win();
        let win2 = create_win();

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());

        // Both are initially tiled
        assert!(!wm.client_state.get_client(win1).unwrap().floating);
        assert!(!wm.client_state.get_client(win2).unwrap().floating);
        assert_eq!(wm.client_state.visible_tiled_clients_on_workspace(1).len(), 2);

        let initial_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let initial_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        // win1 is master (left half), win2 is stack (right half)
        assert!(initial_geom1.width < 1000);
        assert!(initial_geom2.width < 1000);

        // 2. Toggle win2 to floating
        wm.toggle_floating(win2).expect("Toggle floating win2 failed");

        let client2 = wm.client_state.get_client(win2).unwrap();
        assert!(client2.floating);
        assert!(client2.is_floating());
        assert!(matches!(client2.layout_state, ClientLayoutState::Floating(_)));
        assert!(client2.floating_geometry.is_some());

        // Now only win1 is tiled on Workspace 1
        assert_eq!(wm.client_state.visible_tiled_clients_on_workspace(1), vec![win1]);

        // win1 expands to take the whole workspace screen area
        let expanded_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert!(expanded_geom1.width > initial_geom1.width);

        // win2 has default centered floating geometry
        let float_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(float_geom2.width, 600); // 1000 * 60% = 600
        assert_eq!(float_geom2.height, 480); // 800 * 60% = 480
        assert_eq!(float_geom2.x, 200); // (1000 - 600) / 2 = 200
        assert_eq!(float_geom2.y, 160); // (800 - 480) / 2 = 160

        // 3. Move floating window win2 to (120, 140)
        wm.move_floating(win2, 120, 140).expect("Move floating failed");
        let moved_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(moved_geom2.x, 120);
        assert_eq!(moved_geom2.y, 140);
        assert_eq!(
            wm.client_state.get_client(win2).unwrap().floating_geometry,
            Some(FloatingGeometry::new(120, 140, 600, 480))
        );

        // 4. Resize floating window win2 to 450x350
        wm.resize_floating(win2, 450, 350).expect("Resize floating failed");
        let resized_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(resized_geom2.width, 450);
        assert_eq!(resized_geom2.height, 350);
        assert_eq!(
            wm.client_state.get_client(win2).unwrap().floating_geometry,
            Some(FloatingGeometry::new(120, 140, 450, 350))
        );

        // 5. Toggle win2 back to tiled using Action::ToggleFloating
        wm.focus_client(win2).unwrap();
        wm.execute_action(Action::ToggleFloating).unwrap();

        assert!(!wm.client_state.get_client(win2).unwrap().floating);
        assert_eq!(
            wm.client_state.get_client(win2).unwrap().layout_state,
            ClientLayoutState::Tiled
        );
        // Preserves saved floating geometry
        assert_eq!(
            wm.client_state.get_client(win2).unwrap().floating_geometry,
            Some(FloatingGeometry::new(120, 140, 450, 350))
        );

        // Both windows retiled
        assert_eq!(wm.client_state.visible_tiled_clients_on_workspace(1).len(), 2);
        let retiled_geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(retiled_geom1.width, initial_geom1.width);

        // 6. Toggle win2 to floating again -> restores saved geometry (120, 140, 450, 350)
        wm.execute_action(Action::ToggleFloating).unwrap();
        assert!(wm.client_state.get_client(win2).unwrap().floating);
        let restored_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(restored_geom2.x, 120);
        assert_eq!(restored_geom2.y, 140);
        assert_eq!(restored_geom2.width, 450);
        assert_eq!(restored_geom2.height, 350);

        // 7. Workspace switching with floating window
        wm.switch_workspace(2).expect("Switch to ws 2 failed");
        // win1 and win2 are hidden on workspace 2
        assert!(!wm.client_state.get_client(win2).unwrap().mapped);

        // Switch back to workspace 1
        wm.switch_workspace(1).expect("Switch back to ws 1 failed");
        assert!(wm.client_state.get_client(win2).unwrap().mapped);
        let ws_restored_geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(ws_restored_geom2.x, 120);
        assert_eq!(ws_restored_geom2.y, 140);
        assert_eq!(ws_restored_geom2.width, 450);
        assert_eq!(ws_restored_geom2.height, 350);
    }
}
