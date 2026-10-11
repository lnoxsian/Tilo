use std::process::{Child, Command};
use tilo::config::Config;
use tilo::focus::Direction;
use tilo::keybind::{
    Action, Keysym, MOD_SHIFT, MOD_SUPER, XK_DOWN, XK_H, XK_J, XK_K, XK_L, XK_LEFT, XK_Q, XK_RIGHT,
    XK_UP, XK_1, XK_2, XK_3, XK_4, XK_5, XK_6, XK_7, XK_8, XK_9,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, EventMask, KeyButMask, KeyPressEvent, WindowClass,
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
fn test_keyboard_input_and_actions_in_xephyr() {
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

        let mut wm = WindowManager::new_with_config(wm_ctx, Config::default()).expect("Failed to initialize WM");

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

        let win1 = create_win();
        let win2 = create_win();

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());
        assert_eq!(wm.client_state.len(), 2);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 1. Action::Focus(Direction::Left) focuses master (win1)
        wm.execute_action(Action::Focus(Direction::Left)).unwrap();
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // 2. Action::Focus(Direction::Right) focuses stack (win2)
        wm.execute_action(Action::Focus(Direction::Right)).unwrap();
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 3. Action::Move(Direction::Left) swaps win2 into master position
        assert_eq!(wm.client_state.client_order, vec![win1, win2]);
        wm.execute_action(Action::Move(Direction::Left)).unwrap();
        assert_eq!(wm.client_state.client_order, vec![win2, win1]);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // 4. Action::Close closes the currently focused window (win2)
        wm.execute_action(Action::Close).unwrap();
        // Allow unmanage to run or unmanage directly
        wm.unmanage_window(win2);
        assert_eq!(wm.client_state.len(), 1);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // 5. Action::ToggleFloating toggles floating status
        assert!(!wm.client_state.get_client(win1).unwrap().floating);
        wm.execute_action(Action::ToggleFloating).unwrap();
        assert!(wm.client_state.get_client(win1).unwrap().floating);
        wm.execute_action(Action::ToggleFloating).unwrap();
        assert!(!wm.client_state.get_client(win1).unwrap().floating);

        // 6. Action::Exit opens exit launcher menu with Logout, Reboot, Poweroff
        assert!(wm.running);
        wm.execute_action(Action::Exit).unwrap();
        assert!(wm.launcher.as_ref().unwrap().visible);
        assert_eq!(wm.launcher.as_ref().unwrap().mode, tilo::launcher::LauncherMode::Exit);
        assert_eq!(wm.launcher.as_ref().unwrap().filtered_results(8), vec!["Logout", "Reboot", "Poweroff"]);
        // Executing Logout stops the WM
        wm.execute_power_action("logout");
        assert!(!wm.running);
    }
}

#[test]
fn test_keypress_event_handling_in_xephyr() {
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

        let mut wm = WindowManager::new_with_config(wm_ctx, Config::default()).expect("Failed to initialize WM");
        let mapping = wm.input_manager.mapping.as_ref().expect("Keyboard mapping missing");

        // Find keycode for 'q'
        let q_keycodes = mapping.keysym_to_keycodes(XK_Q);
        if let Some(&q_kc) = q_keycodes.first() {
            // Synthesize Mod4 + Shift + q event (Close action)
            let ev = KeyPressEvent {
                response_type: 2,
                detail: q_kc,
                sequence: 1,
                time: 0,
                root: wm.context.root_window,
                event: wm.context.root_window,
                child: 0,
                root_x: 0,
                root_y: 0,
                event_x: 0,
                event_y: 0,
                state: KeyButMask::from(MOD_SUPER | tilo::keybind::MOD_SHIFT),
                same_screen: true,
            };

            assert_eq!(wm.stats.key_presses, 0);
            wm.handle_key_press(&ev).unwrap();
            assert_eq!(wm.stats.key_presses, 1);
        }
    }
}

#[test]
fn test_mod4_shift_numbers_move_focused_window_in_xephyr() {
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

        let mut wm = WindowManager::new_with_config(wm_ctx, Config::default()).expect("Failed to initialize WM");

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

        let win1 = create_win();
        let win2 = create_win();

        wm.manage_window(win1).unwrap();
        wm.manage_window(win2).unwrap();
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        let mapping = wm.input_manager.mapping.clone().expect("Mapping missing");

        // 1. Move win2 to workspace 2 using Mod4 + Shift + 2
        let kc2 = mapping.keysym_to_keycodes(XK_2)[0];
        let ev_move2 = KeyPressEvent {
            response_type: 2,
            detail: kc2,
            sequence: 1,
            time: 0,
            root: wm.context.root_window,
            event: wm.context.root_window,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: KeyButMask::from(MOD_SUPER | MOD_SHIFT),
            same_screen: true,
        };

        wm.handle_key_press(&ev_move2).unwrap();

        // win2 is now on workspace 2 and hidden, win1 is focused on workspace 1
        assert_eq!(wm.workspace_manager.find_workspace_for(win2), Some(2));
        assert!(!wm.client_state.get_client(win2).unwrap().mapped);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // 2. Test moving win1 across workspaces 3 to 9 using Mod4 + Shift + [3..9]
        let num_syms = [
            (3, XK_3),
            (4, XK_4),
            (5, XK_5),
            (6, XK_6),
            (7, XK_7),
            (8, XK_8),
            (9, XK_9),
        ];

        for (target_ws, sym) in num_syms {
            // First switch to the workspace where win1 is currently located
            let cur_ws = wm.workspace_manager.find_workspace_for(win1).unwrap();
            let cur_sym = match cur_ws {
                1 => XK_1,
                2 => XK_2,
                3 => XK_3,
                4 => XK_4,
                5 => XK_5,
                6 => XK_6,
                7 => XK_7,
                8 => XK_8,
                _ => XK_9,
            };
            let cur_kc = mapping.keysym_to_keycodes(cur_sym)[0];
            let ev_switch = KeyPressEvent {
                response_type: 2,
                detail: cur_kc,
                sequence: 1,
                time: 0,
                root: wm.context.root_window,
                event: wm.context.root_window,
                child: 0,
                root_x: 0,
                root_y: 0,
                event_x: 0,
                event_y: 0,
                state: KeyButMask::from(MOD_SUPER),
                same_screen: true,
            };
            wm.handle_key_press(&ev_switch).unwrap();
            assert_eq!(wm.client_state.focused_window(), Some(win1));

            // Now move win1 to target_ws using Mod4 + Shift + target_ws
            let kc = mapping.keysym_to_keycodes(sym)[0];
            let ev_move = KeyPressEvent {
                response_type: 2,
                detail: kc,
                sequence: 1,
                time: 0,
                root: wm.context.root_window,
                event: wm.context.root_window,
                child: 0,
                root_x: 0,
                root_y: 0,
                event_x: 0,
                event_y: 0,
                state: KeyButMask::from(MOD_SUPER | MOD_SHIFT),
                same_screen: true,
            };

            wm.handle_key_press(&ev_move).unwrap();
            assert_eq!(wm.workspace_manager.find_workspace_for(win1), Some(target_ws));
        }

        // 3. Switch to workspace 2 using Mod4 + 2
        let ev_switch2 = KeyPressEvent {
            response_type: 2,
            detail: kc2,
            sequence: 1,
            time: 0,
            root: wm.context.root_window,
            event: wm.context.root_window,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: KeyButMask::from(MOD_SUPER),
            same_screen: true,
        };
        wm.handle_key_press(&ev_switch2).unwrap();
        assert_eq!(wm.current_workspace_id(), 2);
        assert_eq!(wm.client_state.focused_window(), Some(win2));
        assert!(wm.client_state.get_client(win2).unwrap().mapped);

        // 4. Move win2 back to workspace 1 using Mod4 + Shift + 1
        let kc1 = mapping.keysym_to_keycodes(XK_1)[0];
        let ev_move1 = KeyPressEvent {
            response_type: 2,
            detail: kc1,
            sequence: 1,
            time: 0,
            root: wm.context.root_window,
            event: wm.context.root_window,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: KeyButMask::from(MOD_SUPER | MOD_SHIFT),
            same_screen: true,
        };
        wm.handle_key_press(&ev_move1).unwrap();
        assert_eq!(wm.workspace_manager.find_workspace_for(win2), Some(1));
    }
}

#[test]
fn test_window_movement_and_focus_with_vim_and_arrow_keys_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(99);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new_with_config(wm_ctx, Config::default()).expect("Failed to initialize WM");

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

        let win1 = create_win();
        let win2 = create_win();
        let win3 = create_win();

        wm.manage_window(win1).unwrap();
        wm.manage_window(win2).unwrap();
        wm.manage_window(win3).unwrap();

        // win1 is Master (left), win2 is Stack Top (right), win3 is Stack Bottom (right)
        assert_eq!(wm.client_state.focused_window(), Some(win3));

        let send_key = |wm: &mut WindowManager, keysym: Keysym, shift: bool| {
            let mapping = wm.input_manager.mapping.clone().expect("Mapping missing");
            let kc = mapping.keysym_to_keycodes(keysym)[0];
            let mut state = MOD_SUPER;
            if shift {
                state |= MOD_SHIFT;
            }
            let ev = KeyPressEvent {
                response_type: 2,
                detail: kc,
                sequence: 1,
                time: 0,
                root: wm.context.root_window,
                event: wm.context.root_window,
                child: 0,
                root_x: 0,
                root_y: 0,
                event_x: 0,
                event_y: 0,
                state: KeyButMask::from(state),
                same_screen: true,
            };
            wm.handle_key_press(&ev).unwrap();
        };

        // 1. Test focus navigation with Vim keys (Mod + [h, j, k, l])
        // Move Up from win3 -> focuses win2
        send_key(&mut wm, XK_K, false);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Move Left from win2 -> focuses win1
        send_key(&mut wm, XK_H, false);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // Move Right from win1 -> focuses stack window (win2)
        send_key(&mut wm, XK_L, false);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Move Down from win2 -> focuses win3
        send_key(&mut wm, XK_J, false);
        assert_eq!(wm.client_state.focused_window(), Some(win3));

        // 2. Test focus navigation with Arrow keys (Mod + [Left, Down, Up, Right])
        // Move Up from win3 -> focuses win2
        send_key(&mut wm, XK_UP, false);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Move Left from win2 -> focuses win1
        send_key(&mut wm, XK_LEFT, false);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // Move Right from win1 -> focuses win2
        send_key(&mut wm, XK_RIGHT, false);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Move Down from win2 -> focuses win3
        send_key(&mut wm, XK_DOWN, false);
        assert_eq!(wm.client_state.focused_window(), Some(win3));

        // 3. Test window reordering/swapping with Vim keys (Mod + Shift + [h, j, k, l])
        // Currently win3 is focused at bottom-right stack
        assert_eq!(wm.client_state.client_order, vec![win1, win2, win3]);

        // Move Up (Mod + Shift + k): swaps win3 with win2 in stack
        send_key(&mut wm, XK_K, true);
        assert_eq!(wm.client_state.focused_window(), Some(win3));
        assert_eq!(wm.client_state.client_order, vec![win1, win3, win2]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win3, win2]);

        // Move Left (Mod + Shift + h): swaps win3 into Master position with win1
        send_key(&mut wm, XK_H, true);
        assert_eq!(wm.client_state.focused_window(), Some(win3));
        assert_eq!(wm.client_state.client_order, vec![win3, win1, win2]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win3, win1, win2]);

        // Move Right (Mod + Shift + l): swaps win3 back into Stack with win1
        send_key(&mut wm, XK_L, true);
        assert_eq!(wm.client_state.focused_window(), Some(win3));
        assert_eq!(wm.client_state.client_order, vec![win1, win3, win2]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win3, win2]);

        // Move Down (Mod + Shift + j): swaps win3 back to bottom of stack
        send_key(&mut wm, XK_J, true);
        assert_eq!(wm.client_state.focused_window(), Some(win3));
        assert_eq!(wm.client_state.client_order, vec![win1, win2, win3]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);

        // 4. Test window reordering/swapping with Arrow keys (Mod + Shift + [Left, Down, Up, Right])
        // Move focus to win1 in master pane
        send_key(&mut wm, XK_LEFT, false);
        assert_eq!(wm.client_state.focused_window(), Some(win1));

        // Move Right (Mod + Shift + Right): swaps win1 to stack
        send_key(&mut wm, XK_RIGHT, true);
        assert_eq!(wm.client_state.focused_window(), Some(win1));
        assert_eq!(wm.client_state.client_order, vec![win2, win1, win3]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win2, win1, win3]);

        // Move Left (Mod + Shift + Left): swaps win1 back to Master
        send_key(&mut wm, XK_LEFT, true);
        assert_eq!(wm.client_state.focused_window(), Some(win1));
        assert_eq!(wm.client_state.client_order, vec![win1, win2, win3]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);

        // Move focus to win2 (top stack)
        send_key(&mut wm, XK_RIGHT, false);
        assert_eq!(wm.client_state.focused_window(), Some(win2));

        // Move Down (Mod + Shift + Down): swaps win2 with win3
        send_key(&mut wm, XK_DOWN, true);
        assert_eq!(wm.client_state.focused_window(), Some(win2));
        assert_eq!(wm.client_state.client_order, vec![win1, win3, win2]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win3, win2]);

        // Move Up (Mod + Shift + Up): swaps win2 back with win3
        send_key(&mut wm, XK_UP, true);
        assert_eq!(wm.client_state.focused_window(), Some(win2));
        assert_eq!(wm.client_state.client_order, vec![win1, win2, win3]);
        assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);
    }
}

#[test]
fn test_bsp_keyboard_tile_resizing_and_preselection_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        eprintln!("Xephyr not found in PATH, skipping integration test");
        return;
    }

    let _server = match XephyrServer::start(151) {
        Some(s) => s,
        None => {
            eprintln!("Could not start Xephyr on display :151, skipping integration test");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(":151")).expect("Connect to Xephyr");
    let mut wm = WindowManager::new_with_config(ctx, Config::default()).expect("Create WindowManager");

    let root_depth = wm.context.connection.setup().roots[wm.context.screen_num].root_depth;
    let root_visual = wm.context.connection.setup().roots[wm.context.screen_num].root_visual;
    let root_win = wm.context.root_window;
    let w1 = wm.context.connection.generate_id().unwrap();
    let w2 = wm.context.connection.generate_id().unwrap();

    wm.context.connection.create_window(
        root_depth,
        w1,
        root_win,
        0, 0, 200, 200, 0,
        WindowClass::INPUT_OUTPUT,
        root_visual,
        &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
    ).unwrap();

    wm.context.connection.create_window(
        root_depth,
        w2,
        root_win,
        0, 0, 200, 200, 0,
        WindowClass::INPUT_OUTPUT,
        root_visual,
        &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
    ).unwrap();

    wm.manage_window(w1).unwrap();
    wm.manage_window(w2).unwrap();
    wm.focus_client(w1).unwrap();
    wm.retile().unwrap();

    let geom1_before = wm.context.connection.get_geometry(w1).unwrap().reply().unwrap();

    // Grow tile width using Action::ResizeTile(Direction::Right)
    wm.execute_action(Action::ResizeTile(Direction::Right)).unwrap();
    let geom1_after_grow = wm.context.connection.get_geometry(w1).unwrap().reply().unwrap();
    assert!(geom1_after_grow.width > geom1_before.width, "Window 1 should have grown after Right resize");

    // Shrink tile width using Action::ResizeTile(Direction::Left)
    wm.execute_action(Action::ResizeTile(Direction::Left)).unwrap();
    let geom1_after_shrink = wm.context.connection.get_geometry(w1).unwrap().reply().unwrap();
    assert!(geom1_after_shrink.width < geom1_after_grow.width, "Window 1 should have shrunk after Left resize");

    // Test split preselection: preselect vertical split
    wm.execute_action(Action::PreselectSplit(tilo::layout::SplitType::Vertical)).unwrap();
    let w3 = wm.context.connection.generate_id().unwrap();
    wm.context.connection.create_window(
        root_depth,
        w3,
        root_win,
        0, 0, 200, 200, 0,
        WindowClass::INPUT_OUTPUT,
        root_visual,
        &CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
    ).unwrap();

        wm.manage_window(w3).unwrap();
        wm.retile().unwrap();

        // Window 1 and 3 should now be stacked vertically on the left side
        let geom1_stacked = wm.context.connection.get_geometry(w1).unwrap().reply().unwrap();
        let geom3_stacked = wm.context.connection.get_geometry(w3).unwrap().reply().unwrap();
        assert_eq!(geom1_stacked.x, geom3_stacked.x);
        assert!(geom1_stacked.y < geom3_stacked.y);
    }



