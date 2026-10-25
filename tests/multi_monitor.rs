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
            Monitor::new(1, 1920, 0, 1920, 1080, 11, false),
        ],
        active_monitor_id: 0,
    };

    assert_eq!(mm.active_monitor().id, 0);
    assert_eq!(mm.monitor_for_workspace(1).unwrap().id, 0);
    assert_eq!(mm.monitor_for_workspace(11).unwrap().id, 1);
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
        // Monitor 1: Right half (500, 0, 500, 800) hosting Workspace 11 (Monitor 1's Workspace 1)
        wm.monitor_manager.monitors = vec![
            Monitor::new(0, 0, 0, 500, 800, 1, true),
            Monitor::new(1, 500, 0, 500, 800, 11, false),
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
        assert_eq!(wm.client_state.get_client(win1).unwrap().workspace, 1);

        // 2. Switch to Monitor 1 (Workspace 11, display tag 1) and create win2
        focus_monitor_right(&mut wm).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 11);

        let win2 = create_win();
        assert!(wm.manage_window(win2).unwrap());
        assert_eq!(wm.client_state.get_client(win2).unwrap().workspace, 11);
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
        assert_eq!(wm.current_workspace, 11);
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

        // Move win2 back to Monitor 1 (Workspace 11)
        wm.focus_client(win2).unwrap();
        move_to_monitor(&mut wm, 1).unwrap();
        assert_eq!(wm.client_state.get_client(win2).unwrap().workspace, 11);
        assert_eq!(wm.client_state.get_client(win2).unwrap().monitor, 1);

        // Focus win2 on Monitor 1 to make Monitor 1 the active monitor
        wm.focus_client(win2).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 11);

        // 6. Test Option 1 independent workspace switching on Monitor 1:
        // When active monitor is 1, Action::Workspace(2) switches Monitor 1 to its workspace 2 (12)!
        wm.execute_action(Action::Workspace(2)).unwrap();
        assert_eq!(wm.current_workspace, 12);
        assert_eq!(wm.monitor_manager.monitors[0].workspace, 1, "Monitor 0 must remain on workspace 1");
        assert_eq!(wm.monitor_manager.monitors[1].workspace, 12, "Monitor 1 must be on workspace 12");
        assert_eq!(wm.client_state.get_client(win2).unwrap().mapped, false, "win2 on ws 11 should be unmapped");
        assert_eq!(wm.client_state.get_client(win1).unwrap().mapped, true, "win1 on ws 1 should remain mapped");

        // Switch Monitor 1 back to workspace 1
        wm.execute_action(Action::Workspace(1)).unwrap();
        assert_eq!(wm.current_workspace, 11);
        assert_eq!(wm.client_state.get_client(win2).unwrap().mapped, true, "win2 on ws 11 should be remapped");

        // 7. Test keybind Action::FocusMonitorLeft and Action::FocusMonitorRight (Mod + < and Mod + >)
        wm.execute_action(Action::FocusMonitorLeft).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
        assert_eq!(wm.current_workspace, 1);

        wm.execute_action(Action::FocusMonitorRight).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 11);

        // 7. Test pointer-aware launcher positioning
        // Warp mouse cursor to Monitor 1 (x: 750, y: 400)
        client_conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 750, 400).unwrap();
        client_conn.sync().unwrap();
        wm.open_launcher().unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        let launcher_rect_mon1 = wm.launcher.as_ref().unwrap().rect;
        assert!(launcher_rect_mon1.x >= 500, "Launcher x ({}) should be >= 500 on Monitor 1", launcher_rect_mon1.x);
        wm.close_launcher().unwrap();

        // Warp mouse cursor to Monitor 0 (x: 250, y: 400)
        client_conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 250, 400).unwrap();
        client_conn.sync().unwrap();
        wm.open_launcher().unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
        let launcher_rect_mon0 = wm.launcher.as_ref().unwrap().rect;
        assert!(launcher_rect_mon0.x < 500, "Launcher x ({}) should be < 500 on Monitor 0", launcher_rect_mon0.x);
        wm.close_launcher().unwrap();

        // 8. Test focus_client synchronization across monitors
        // Focus win2 and move it back to Monitor 1 (workspace 11)
        wm.focus_client(win2).unwrap();
        move_to_monitor(&mut wm, 1).unwrap();
        assert_eq!(wm.client_state.get_client(win2).unwrap().workspace, 11);
        assert_eq!(wm.client_state.get_client(win2).unwrap().monitor, 1);

        // Focus win1 on Monitor 0: active monitor and workspace should synchronize to Monitor 0
        wm.focus_client(win1).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
        assert_eq!(wm.current_workspace, 1);
        assert_eq!(wm.focused_window(), Some(win1));

        // Focus win2 on Monitor 1: active monitor and workspace should synchronize to Monitor 1
        wm.focus_client(win2).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 11);
        assert_eq!(wm.focused_window(), Some(win2));

        // 9. Test background desktop click activating Monitor 0
        let click_mon0 = x11rb::protocol::xproto::ButtonPressEvent {
            response_type: x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
            detail: 1,
            sequence: 0,
            time: x11rb::CURRENT_TIME,
            root,
            event: root,
            child: x11rb::NONE,
            root_x: 250,
            root_y: 400,
            event_x: 250,
            event_y: 400,
            state: 0u16.into(),
            same_screen: true,
        };
        wm.handle_button_press(&click_mon0).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);
        assert_eq!(wm.current_workspace, 1);

        // 10. Test secondary statusbar on Monitor 1
        wm.enable_statusbar(24).unwrap();
        assert!(wm.statusbar.is_some(), "Primary statusbar must be present");
        let prim_bar = wm.statusbar.as_ref().unwrap();
        assert_eq!(prim_bar.rect.x, 0);
        assert_eq!(prim_bar.rect.width, 500);

        assert_eq!(wm.secondary_statusbars.len(), 1, "Secondary statusbar must exist on Monitor 1");
        let sec_bar_win = wm.secondary_statusbars[0].window_id;
        {
            let sec_bar = &wm.secondary_statusbars[0];
            assert_eq!(sec_bar.rect.x, 500);
            assert_eq!(sec_bar.rect.width, 500);

            // Verify secondary statusbar does NOT have ip, ram, cpu, battery, volume modules
            let sec_mod_names: Vec<&str> = sec_bar.modules.iter().map(|m| m.name()).collect();
            assert!(!sec_mod_names.contains(&"ip"));
            assert!(!sec_mod_names.contains(&"ram"));
            assert!(!sec_mod_names.contains(&"cpu"));
            assert!(!sec_mod_names.contains(&"battery"));
            assert!(!sec_mod_names.contains(&"volume"));
            // Core features (workspace, layout, title) are present
            assert!(sec_mod_names.contains(&"workspace"));
            assert!(sec_mod_names.contains(&"layout"));
            assert!(sec_mod_names.contains(&"title"));
        }

        // Verify statusbar deduction on both monitors during retile (24px bar + outer_gap)
        wm.retile().unwrap();
        client_conn.sync().unwrap();
        let geom1_bar = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let geom2_bar = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let expected_y = 24 + wm.config.outer_gap as i16;
        assert_eq!(geom1_bar.y, expected_y, "Monitor 0 window y offset should reserve 24px for primary statusbar + outer gap");
        assert_eq!(geom2_bar.y, expected_y, "Monitor 1 window y offset should reserve 24px for secondary statusbar + outer gap");

        // Verify clicking the secondary statusbar activates Monitor 1
        let click_sec_bar = x11rb::protocol::xproto::ButtonPressEvent {
            response_type: x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
            detail: 1,
            sequence: 0,
            time: x11rb::CURRENT_TIME,
            root,
            event: sec_bar_win,
            child: x11rb::NONE,
            root_x: 800,
            root_y: 10,
            event_x: 300,
            event_y: 10,
            state: 0u16.into(),
            same_screen: true,
        };
        // Switch to Monitor 0 first
        focus_monitor_left(&mut wm).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);

        // Click secondary bar on Monitor 1
        wm.handle_button_press(&click_sec_bar).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1, "Clicking secondary statusbar should activate Monitor 1");

        // Click tag 3 on secondary statusbar (x ~ 60)
        let click_tag3_sec = x11rb::protocol::xproto::ButtonPressEvent {
            response_type: x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
            detail: 1,
            sequence: 0,
            time: x11rb::CURRENT_TIME,
            root,
            event: sec_bar_win,
            child: x11rb::NONE,
            root_x: 560,
            root_y: 10,
            event_x: 60,
            event_y: 10,
            state: 0u16.into(),
            same_screen: true,
        };
        wm.handle_button_press(&click_tag3_sec).unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 1);
        assert_eq!(wm.current_workspace, 13, "Clicking tag 3 on secondary status bar should switch Monitor 1 to workspace 13");
        assert_eq!(wm.monitor_manager.monitors[0].workspace, 1, "Monitor 0 must remain on workspace 1");

        // Disable statusbar
        wm.disable_statusbar().unwrap();
        assert!(wm.statusbar.is_none());
        assert!(wm.secondary_statusbars.is_empty());

        // 11. Test hotplug disconnect migration
        // Disconnect Monitor 1: only Monitor 0 remains
        let single_mon = vec![Monitor::new(0, 0, 0, 1000, 800, 1, true)];
        let disconnected = wm.monitor_manager.reconcile_monitors(single_mon);
        assert_eq!(disconnected, vec![(1, 13)]);
        assert_eq!(wm.monitor_manager.monitors.len(), 1);

        // Retile after disconnect
        wm.retile().unwrap();
        assert_eq!(wm.monitor_manager.active_monitor_id, 0);

        // 12. Test keysym parsing for Mod + < and Mod + >
        use tilo::keybind::{parse_keysym, XK_LESS, XK_GREATER};
        assert_eq!(parse_keysym("<"), Some(XK_LESS));
        assert_eq!(parse_keysym(">"), Some(XK_GREATER));
        assert_eq!(parse_keysym("less"), Some(XK_LESS));
        assert_eq!(parse_keysym("greater"), Some(XK_GREATER));
    }
}
