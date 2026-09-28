use std::process::{Child, Command};
use tilo::keybind::Action;
use tilo::monitor::{focus_monitor_left, focus_monitor_right, move_to_monitor, Monitor, MonitorManager};
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
fn test_multi_monitor_unit_logic() {
    let mut mm = MonitorManager {
        monitors: vec![
            Monitor::new(0, 0, 0, 1920, 1080, 1, true),
            Monitor::new(1, 1920, 0, 1920, 1080, 2, false),
        ],
        active_monitor_id: 0,
    };

    assert_eq!(mm.active_monitor().id, 0);
    assert_eq!(mm.monitor_for_workspace(1).unwrap().id, 0);
    assert_eq!(mm.monitor_for_workspace(2).unwrap().id, 1);
    assert!(mm.monitor_for_point(500, 500).unwrap().id == 0);
    assert!(mm.monitor_for_point(2000, 500).unwrap().id == 1);

    // Navigation
    assert_eq!(mm.focus_monitor_right(), Some(1));
    assert_eq!(mm.active_monitor_id, 1);
    assert_eq!(mm.focus_monitor_right(), None);

    assert_eq!(mm.focus_monitor_left(), Some(0));
    assert_eq!(mm.active_monitor_id, 0);
    assert_eq!(mm.focus_monitor_left(), None);
}

#[test]
fn test_multi_monitor_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(96);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");
        let root = wm.context.root_window;

        // Configure 2 monitors across the 1000x800 Xephyr screen:
        // Monitor 0: Left half (0, 0, 500, 800) hosting Workspace 1
        // Monitor 1: Right half (500, 0, 500, 800) hosting Workspace 2
        wm.monitor_manager.monitors = vec![
            Monitor::new(0, 0, 0, 500, 800, 1, true),
            Monitor::new(1, 500, 0, 500, 800, 2, false),
        ];
        wm.monitor_manager.active_monitor_id = 0;

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

        // 1. Create win1 on Monitor 0 (Workspace 1)
        let win1 = create_win();
        assert!(wm.manage_window(win1).unwrap());
        assert_eq!(wm.client_state.get_client(win1).unwrap().monitor, 0);

        // 2. Switch to Monitor 1 (Workspace 2) and create win2
        focus_monitor_right(&mut wm).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 2);

        let win2 = create_win();
        assert!(wm.manage_window(win2).unwrap());
        assert_eq!(wm.client_state.get_client(win2).unwrap().workspace, 2);
        assert_eq!(wm.client_state.get_client(win2).unwrap().monitor, 1);

        // 3. Verify simultaneous display: Both windows are mapped and placed on their respective monitors!
        client_conn.sync().unwrap();
        let geom1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let geom2 = client_conn.get_geometry(win2).unwrap().reply().unwrap();

        // win1 is within Monitor 0: x in [0, 500)
        assert!(geom1.x >= 0 && geom1.x < 500);
        assert!(u32::from(geom1.width) < 500);

        // win2 is within Monitor 1: x in [500, 1000)
        assert!(geom2.x >= 500 && geom2.x < 1000);
        assert!(u32::from(geom2.width) < 500);

        // 4. Test directional monitor switching functions
        focus_monitor_left(&mut wm).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
        assert_eq!(wm.current_workspace, 1);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        focus_monitor_right(&mut wm).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 2);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 5. Test move_to_monitor
        // Move win2 from Monitor 1 to Monitor 0
        move_to_monitor(&mut wm, 0).unwrap();
        assert_eq!(wm.client_state.get_client(win2).unwrap().workspace, 1);
        assert_eq!(wm.client_state.get_client(win2).unwrap().monitor, 0);

        client_conn.sync().unwrap();
        let geom2_moved = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        // win2 is now tiled on Monitor 0 (x < 500)
        assert!(geom2_moved.x >= 0 && geom2_moved.x < 500);

        // 6. Test keybind Action::FocusMonitorLeft and Action::FocusMonitorRight
        wm.execute_action(Action::FocusMonitorLeft).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);

        wm.execute_action(Action::FocusMonitorRight).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);

        // 7. Test hotplug disconnect migration
        // Disconnect Monitor 1: only Monitor 0 remains
        let single_mon = vec![Monitor::new(0, 0, 0, 1000, 800, 1, true)];
        let disconnected = wm.monitor_manager.reconcile_monitors(single_mon);
        assert_eq!(disconnected, vec![(1, 2)]);
        assert_eq!(wm.monitor_manager.monitors.len(), 1);

        // Retile after disconnect
        wm.retile().unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
    }
}
