use std::process::{Child, Command};
use tilo::focus::Direction;
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, WindowClass};
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
fn test_focus_and_spatial_navigation_in_xephyr() {
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

        // Client connection to create windows
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Helper to create test window
        let create_win = |w: u16, h: u16| {
            let win = client_conn.generate_id().expect("Failed XID");
            client_conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    win,
                    root,
                    0,
                    0,
                    w,
                    h,
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

        let win1 = create_win(200, 200);
        let win2 = create_win(200, 200);
        let win3 = create_win(200, 200);

        // 1. Manage Window 1
        assert!(wm.manage_window(win1).unwrap());
        assert_eq!(wm.client_state.focused_window(), Some(win1));
        assert_eq!(wm.workspace_focus.focused_window_on(1), Some(win1));

        // 2. Manage Window 2
        assert!(wm.manage_window(win2).unwrap());
        assert_eq!(wm.client_state.focused_window(), Some(win2));
        assert_eq!(wm.workspace_focus.focused_window_on(1), Some(win2));

        // 3. Manage Window 3 (Master is win1 on left, win2 & win3 stacked on right)
        assert!(wm.manage_window(win3).unwrap());
        assert_eq!(wm.client_state.focused_window(), Some(win3));

        // 4. Directional spatial navigation
        // win3 is bottom-right stack window. Moving Up should focus win2 (top-right stack)
        let target = wm.focus_in_direction(Direction::Up, false).unwrap();
        assert_eq!(target, Some(win2));
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // From win2 (top-right stack), moving Left should focus win1 (master pane on left)
        let target = wm.focus_in_direction(Direction::Left, false).unwrap();
        assert_eq!(target, Some(win1));
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // From win1 (master pane), moving Left without wrap yields None
        let target = wm.focus_in_direction(Direction::Left, false).unwrap();
        assert_eq!(target, None);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // From win1 (master pane), moving Left WITH wrap wraps around to right stack window
        let target = wm.focus_in_direction(Direction::Left, true).unwrap();
        assert!(target == Some(win2) || target == Some(win3));

        // 5. Per-workspace focus tracking
        // Put win1 on workspace 1 and win2 on workspace 2
        wm.client_state.get_client_mut(win1).unwrap().workspace = 1;
        wm.client_state.get_client_mut(win2).unwrap().workspace = 2;
        wm.workspace_focus.focus(1, win1);
        wm.workspace_focus.focus(2, win2);

        assert_eq!(wm.workspace_focus.focused_window_on(1), Some(win1));
        assert_eq!(wm.workspace_focus.focused_window_on(2), Some(win2));

        // Switch workspace to 2 restores win2 focus
        wm.set_current_workspace(2).unwrap();
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Switch workspace back to 1 restores win1 focus
        wm.set_current_workspace(1).unwrap();
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // 6. Focus restoration upon window unmanage
        // Unmanaging win1 restores focus to next available client
        let unmanaged = wm.unmanage_window(win1);
        assert!(unmanaged.is_some());
        assert_ne!(wm.client_state.focused_window(), Some(win1));

        // Clean up remaining windows
        wm.unmanage_window(win2);
        wm.unmanage_window(win3);
        assert_eq!(wm.client_state.focused_window(), None);
    }
}
