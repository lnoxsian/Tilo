use std::process::{Child, Command};
use tilo::rules::{RuleAction, RuleCondition, WindowRule};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass,
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
fn test_window_rules_in_xephyr() {
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

        // Register window rules
        wm.add_rule(WindowRule::new(
            RuleCondition::Class("Gimp".to_string()),
            RuleAction::new().with_floating(true),
        ));
        wm.add_rule(WindowRule::new(
            RuleCondition::Class("Firefox".to_string()),
            RuleAction::new().with_workspace(2),
        ));
        wm.add_rule(WindowRule::new(
            RuleCondition::Title("Special App".to_string()),
            RuleAction::new().with_workspace(3).with_floating(true),
        ));

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

        // 1. Regular window (no rules match)
        let win_regular = create_win();
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_regular,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"regular\0Regular\0",
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        assert!(wm.manage_window(win_regular).unwrap());
        let client_regular = wm.client_state.get_client(win_regular).unwrap();
        assert_eq!(client_regular.workspace, 1);
        assert!(!client_regular.floating);
        assert!(client_regular.mapped);

        // 2. Window matching Rule 1 (Class="Gimp" -> floating=true)
        let win_gimp = create_win();
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_gimp,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"gimp\0Gimp\0",
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        assert!(wm.manage_window(win_gimp).unwrap());
        let client_gimp = wm.client_state.get_client(win_gimp).unwrap();
        assert_eq!(client_gimp.workspace, 1);
        assert!(client_gimp.floating);
        assert!(client_gimp.is_floating());
        assert!(client_gimp.mapped);
        assert!(client_gimp.floating_geometry.is_some());

        // 3. Window matching Rule 2 (Class="Firefox" -> workspace=2)
        let win_firefox = create_win();
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_firefox,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"navigator\0Firefox\0",
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        assert!(wm.manage_window(win_firefox).unwrap());
        let client_firefox = wm.client_state.get_client(win_firefox).unwrap();
        assert_eq!(client_firefox.workspace, 2);
        assert!(!client_firefox.floating);
        assert!(!client_firefox.mapped, "Firefox should spawn unmapped on background workspace");
        assert!(wm.workspace_manager.get(2).unwrap().clients.contains(&win_firefox));

        // 4. Window matching Rule 3 (Title="Special App" -> workspace=3, floating=true)
        let win_special = create_win();
        let net_wm_name = wm.atoms.net_wm_name;
        let utf8_string = wm.atoms.utf8_string;
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_special,
                net_wm_name,
                utf8_string,
                b"Special App",
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        assert!(wm.manage_window(win_special).unwrap());
        let client_special = wm.client_state.get_client(win_special).unwrap();
        assert_eq!(client_special.workspace, 3);
        assert!(client_special.floating);
        assert!(!client_special.mapped, "Special App should spawn unmapped on background workspace");
        assert!(wm.workspace_manager.get(3).unwrap().clients.contains(&win_special));

        // 5. Switch to Workspace 2 -> Firefox becomes visible and active
        wm.switch_workspace(2).expect("Switch to ws 2 failed");
        assert!(wm.client_state.get_client(win_firefox).unwrap().mapped);
        assert_eq!(wm.client_state.focused_window(), Some(win_firefox));

        // 6. Switch to Workspace 3 -> Special App becomes visible and floating
        wm.switch_workspace(3).expect("Switch to ws 3 failed");
        let client_special_mapped = wm.client_state.get_client(win_special).unwrap();
        assert!(client_special_mapped.mapped);
        assert!(client_special_mapped.floating);
        assert_eq!(wm.client_state.focused_window(), Some(win_special));
    }
}
