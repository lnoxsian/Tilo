use std::process::{Child, Command};
use tilo::ewmh::get_net_wm_state;
use tilo::keybind::Action;
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ClientMessageData, ClientMessageEvent, ConnectionExt, CreateWindowAux, WindowClass,
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
fn test_fullscreen_mode_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(95);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");
        let root = wm.context.root_window;
        let ewmh = wm.ewmh_atoms;
        let screen_width = wm.context.screen_width as u16;
        let screen_height = wm.context.screen_height as u16;

        assert_eq!(screen_width, 1000);
        assert_eq!(screen_height, 800);

        // Client connection to create windows and query properties
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        assert_eq!(screen.root, root);

        let create_win = || {
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
            client_conn.flush().unwrap();
            client_conn.sync().unwrap();
            win
        };

        let win1 = create_win();
        let win2 = create_win();
        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());

        // 1. Initial State: Both windows are tiled side-by-side / master-stack
        let states1 = get_net_wm_state(&client_conn, win1, &ewmh).unwrap();
        assert!(!states1.contains(&ewmh.net_wm_state_fullscreen));
        assert!(!wm.client_state.get_client(win1).unwrap().is_fullscreen());

        let geom1_init = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_init.border_width, 2);
        assert!(geom1_init.width < screen_width);

        // 2. Toggle win1 Fullscreen via wm.toggle_fullscreen()
        wm.toggle_fullscreen(win1).unwrap();
        client_conn.sync().unwrap();

        assert!(wm.client_state.get_client(win1).unwrap().is_fullscreen());
        let states1_fs = get_net_wm_state(&client_conn, win1, &ewmh).unwrap();
        assert!(states1_fs.contains(&ewmh.net_wm_state_fullscreen));

        let geom1_fs = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_fs.x, 0);
        assert_eq!(geom1_fs.y, 0);
        assert_eq!(geom1_fs.width, screen_width);
        assert_eq!(geom1_fs.height, screen_height);
        assert_eq!(geom1_fs.border_width, 0, "Fullscreen window must have 0 border width");

        // The remaining tiled client (win2) expands to fill the entire workspace while win1 is fullscreen
        let geom2_while_fs = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(geom2_while_fs.width > 900, "win2 should expand to fill workspace when win1 is fullscreen");

        // 3. Toggle win1 Off Fullscreen via Action::ToggleFullscreen
        wm.focus_client(win1).unwrap();
        wm.execute_action(Action::ToggleFullscreen).unwrap();
        client_conn.sync().unwrap();

        assert!(!wm.client_state.get_client(win1).unwrap().is_fullscreen());
        let states1_restored = get_net_wm_state(&client_conn, win1, &ewmh).unwrap();
        assert!(!states1_restored.contains(&ewmh.net_wm_state_fullscreen));

        let geom1_restored = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_restored.border_width, 2);
        assert!(geom1_restored.width < screen_width);

        // 4. Test Fullscreen on Floating Window
        wm.toggle_floating(win2).unwrap();
        assert!(wm.client_state.get_client(win2).unwrap().floating);

        let geom2_floating = client_conn.get_geometry(win2).unwrap().reply().unwrap();

        // Make floating win2 fullscreen
        wm.toggle_fullscreen(win2).unwrap();
        client_conn.sync().unwrap();

        assert!(wm.client_state.get_client(win2).unwrap().is_fullscreen());
        let geom2_fs = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(geom2_fs.x, 0);
        assert_eq!(geom2_fs.y, 0);
        assert_eq!(geom2_fs.width, screen_width);
        assert_eq!(geom2_fs.height, screen_height);
        assert_eq!(geom2_fs.border_width, 0);

        // Toggle floating win2 off fullscreen
        wm.toggle_fullscreen(win2).unwrap();
        client_conn.sync().unwrap();

        assert!(!wm.client_state.get_client(win2).unwrap().is_fullscreen());
        assert!(wm.client_state.get_client(win2).unwrap().floating);

        let geom2_restored_floating = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(geom2_restored_floating.x, geom2_floating.x);
        assert_eq!(geom2_restored_floating.y, geom2_floating.y);
        assert_eq!(geom2_restored_floating.width, geom2_floating.width);
        assert_eq!(geom2_restored_floating.height, geom2_floating.height);
        assert_eq!(geom2_restored_floating.border_width, 2);

        // 5. Test EWMH ClientMessage _NET_WM_STATE (Add, Remove, Toggle)
        // Add (action = 1)
        let event_add = ClientMessageEvent::new(
            32,
            win1,
            ewmh.net_wm_state,
            ClientMessageData::from([1u32, ewmh.net_wm_state_fullscreen, 0, 0, 0]),
        );
        wm.handle_client_message(&event_add).unwrap();
        client_conn.sync().unwrap();
        assert!(wm.client_state.get_client(win1).unwrap().is_fullscreen());
        assert!(get_net_wm_state(&client_conn, win1, &ewmh).unwrap().contains(&ewmh.net_wm_state_fullscreen));

        // Remove (action = 0)
        let event_remove = ClientMessageEvent::new(
            32,
            win1,
            ewmh.net_wm_state,
            ClientMessageData::from([0u32, ewmh.net_wm_state_fullscreen, 0, 0, 0]),
        );
        wm.handle_client_message(&event_remove).unwrap();
        client_conn.sync().unwrap();
        assert!(!wm.client_state.get_client(win1).unwrap().is_fullscreen());
        assert!(!get_net_wm_state(&client_conn, win1, &ewmh).unwrap().contains(&ewmh.net_wm_state_fullscreen));

        // Toggle (action = 2)
        let event_toggle = ClientMessageEvent::new(
            32,
            win1,
            ewmh.net_wm_state,
            ClientMessageData::from([2u32, ewmh.net_wm_state_fullscreen, 0, 0, 0]),
        );
        wm.handle_client_message(&event_toggle).unwrap();
        client_conn.sync().unwrap();
        assert!(wm.client_state.get_client(win1).unwrap().is_fullscreen());
        assert!(get_net_wm_state(&client_conn, win1, &ewmh).unwrap().contains(&ewmh.net_wm_state_fullscreen));

        // 6. Test destroying/closing a fullscreen window
        wm.unmanage_window(win1);
        assert!(!wm.client_state.contains(win1));
    }
}
