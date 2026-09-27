use std::process::{Child, Command};
use tilo::focus::Direction;
use tilo::keybind::{Action, MOD_SUPER, XK_Q};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, KeyButMask, KeyPressEvent, WindowClass,
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

        // 6. Action::Exit stops the WM
        assert!(wm.running);
        wm.execute_action(Action::Exit).unwrap();
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

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");
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

