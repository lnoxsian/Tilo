use std::process::{Child, Command};
use std::time::Duration;
use tilo::perform_graceful_shutdown;
use tilo::rules::{RuleAction, RuleCondition, WindowRule};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use tilo::keybind::{default_keybindings, Action, MOD_SHIFT, MOD_SUPER, XK_Q};
use tilo::WmState;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, InputFocus, MapState, PropMode, WindowClass,
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
            .arg("800x600")
            .arg("-ac")
            .arg("-br")
            .arg("-noreset")
            .spawn()
            .ok()?;

        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
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

fn has_xephyr() -> bool {
    Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn test_default_keybindings_include_mod4_shift_q_exit() {
    let binds = default_keybindings();
    let exit_bind = binds
        .iter()
        .find(|b| b.modifiers == (MOD_SUPER | MOD_SHIFT) && b.keysym == XK_Q);
    assert!(
        exit_bind.is_some(),
        "Mod4 + Shift + Q must be configured in default keybindings"
    );
    assert_eq!(exit_bind.unwrap().action, Action::Quit);
}

#[test]
fn test_graceful_shutdown_restores_borders_and_maps_all_windows() {
    if !has_xephyr() {
        return;
    }

    let xephyr = XephyrServer::start(193);
    if let Some(server) = xephyr {
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Window 1: On active workspace (workspace 1)
        let win1 = client_conn.generate_id().expect("XID 1");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win1,
                root,
                10,
                10,
                300,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win1");
        client_conn.map_window(win1).expect("Map win1");

        // Window 2: Will be assigned to workspace 2 via rule
        let win2 = client_conn.generate_id().expect("XID 2");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win2,
                root,
                50,
                50,
                300,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win2");
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win2,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"bgapp\0BgApp\0",
            )
            .unwrap();
        client_conn.map_window(win2).expect("Map win2");

        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        // Start WM
        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init");
        wm.add_rule(WindowRule::new(
            RuleCondition::Class("BgApp".to_string()),
            RuleAction::new().with_workspace(2),
        ));

        // Manage pre-existing windows
        wm.manage_existing_windows().expect("manage existing");
        assert_eq!(wm.client_state.len(), 2);

        // Verify win1 has borders > 0 while managed
        let geom1_before = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(u32::from(geom1_before.border_width), wm.border_config.width);

        // Verify win2 was unmapped on workspace 2
        client_conn.sync().unwrap();
        let attrs2_before = client_conn.get_window_attributes(win2).unwrap().reply().unwrap();
        assert_eq!(attrs2_before.map_state, MapState::UNMAPPED);

        // Execute graceful shutdown
        wm.perform_graceful_shutdown().expect("perform_graceful_shutdown");

        // 1. WM event loop stopped and state cleared
        assert!(!wm.running);
        assert_eq!(wm.client_state.len(), 0);

        // 2. Borders restored to 0 on win1
        let geom1_after = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_after.border_width, 0, "Borders must be reset to 0 upon shutdown");

        // 3. Inactive workspace window (win2) is mapped so it's not hidden after WM exits
        client_conn.sync().unwrap();
        let attrs2_after = client_conn.get_window_attributes(win2).unwrap().reply().unwrap();
        assert_ne!(
            attrs2_after.map_state,
            MapState::UNMAPPED,
            "Inactive workspace windows must be remapped upon graceful shutdown"
        );
    }
}

#[test]
fn test_graceful_shutdown_releases_substructure_and_allows_reacquisition() {
    if !has_xephyr() {
        return;
    }

    let xephyr = XephyrServer::start(194);
    if let Some(server) = xephyr {
        // Step 1: WM 1 starts and enables statusbar
        let wm_ctx1 = X11Context::connect_to(Some(&server.display)).expect("WM1 connect");
        assert!(wm_ctx1.become_wm().is_ok());

        let mut wm1: WmState = WindowManager::new(wm_ctx1).expect("WM1 init");
        let _ = wm1.enable_statusbar(24);
        assert!(wm1.statusbar.is_some());

        // Step 2: WM 1 performs graceful shutdown using free function API
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let res = perform_graceful_shutdown(&client_conn, &mut wm1);
        assert!(res.is_ok(), "Free function perform_graceful_shutdown failed");
        assert!(!wm1.running);
        assert!(wm1.statusbar.is_none());

        // Step 3: Verify input focus reverted to PointerRoot (or root)
        let focus_reply = client_conn.get_input_focus().unwrap().reply().unwrap();
        assert!(
            focus_reply.focus == root || focus_reply.focus == x11rb::NONE || focus_reply.revert_to == InputFocus::POINTER_ROOT,
            "Focus should revert cleanly to PointerRoot"
        );

        // Step 4: Another WM can now acquire ownership on root window without error
        let wm_ctx2 = X11Context::connect_to(Some(&server.display)).expect("WM2 connect");
        let become_res = wm_ctx2.become_wm();
        assert!(
            become_res.is_ok(),
            "Another WM must be able to claim ownership after graceful shutdown"
        );
    }
}
