use std::process::{Child, Command};
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
fn test_workspace_lifecycle_and_switching_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(98);
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
                .expect("Create window failed");
            client_conn.flush().unwrap();
            client_conn.sync().unwrap();
            win
        };

        // 1. Initial State: active workspace is 1
        assert_eq!(wm.current_workspace_id(), 1);
        assert!(wm.workspace_exists(1));
        assert!(wm.workspace_exists(9));

        // Create and manage two windows on workspace 1
        let win1 = create_win();
        let win2 = create_win();

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());

        assert_eq!(wm.workspace_manager.get(1).unwrap().len(), 2);
        assert!(wm.client_state.get_client(win1).unwrap().mapped);
        assert!(wm.client_state.get_client(win2).unwrap().mapped);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 2. Switch to workspace 2
        wm.switch_workspace(2).unwrap();
        assert_eq!(wm.current_workspace_id(), 2);

        // Windows on workspace 1 should now be unmapped (hidden)
        assert!(!wm.client_state.get_client(win1).unwrap().mapped);
        assert!(!wm.client_state.get_client(win2).unwrap().mapped);
        assert!(wm.workspace_manager.get(2).unwrap().is_empty());

        // Create and manage window 3 on workspace 2
        let win3 = create_win();
        assert!(wm.manage_window(win3).unwrap());

        assert_eq!(wm.workspace_manager.get(2).unwrap().len(), 1);
        assert!(wm.client_state.get_client(win3).unwrap().mapped);
        assert_eq!(wm.client_state.focused_window(), Some(win3));

        // 3. Switch back to workspace 1
        wm.switch_workspace(1).unwrap();
        assert_eq!(wm.current_workspace_id(), 1);

        // Window 3 should now be unmapped, windows 1 and 2 mapped again
        assert!(!wm.client_state.get_client(win3).unwrap().mapped);
        assert!(wm.client_state.get_client(win1).unwrap().mapped);
        assert!(wm.client_state.get_client(win2).unwrap().mapped);
        // Focus should be restored to previously focused window on workspace 1 (win2)
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 4. Move window 1 from workspace 1 to workspace 2
        wm.move_to_workspace(win1, 2).unwrap();

        // Window 1 should now belong to workspace 2 and be hidden
        assert_eq!(wm.workspace_manager.find_workspace_for(win1), Some(2));
        assert!(!wm.client_state.get_client(win1).unwrap().mapped);
        assert_eq!(wm.workspace_manager.get(1).unwrap().len(), 1);
        assert_eq!(wm.workspace_manager.get(2).unwrap().len(), 2);

        // Focus on workspace 1 should remain on win2
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 5. Switch to workspace 2 to verify both win3 and win1 are mapped and visible
        wm.switch_workspace(2).unwrap();
        assert_eq!(wm.current_workspace_id(), 2);
        assert!(wm.client_state.get_client(win3).unwrap().mapped);
        assert!(wm.client_state.get_client(win1).unwrap().mapped);
        assert!(!wm.client_state.get_client(win2).unwrap().mapped);

        // 6. Clean up windows
        wm.unmanage_window(win1);
        wm.unmanage_window(win2);
        wm.unmanage_window(win3);

        assert!(wm.workspace_manager.get(1).unwrap().is_empty());
        assert!(wm.workspace_manager.get(2).unwrap().is_empty());
        assert_eq!(wm.client_state.len(), 0);
    }
}
