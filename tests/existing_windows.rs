use std::process::{Child, Command};
use std::time::Duration;
use tilo::manage_existing_windows;
use tilo::rules::{RuleAction, RuleCondition, WindowRule};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use tilo::WmState;
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
fn test_startup_existing_windows_adoption() {
    if !has_xephyr() {
        return;
    }

    let xephyr = XephyrServer::start(190);
    if let Some(server) = xephyr {
        // Step 1: Create client connection *before* window manager starts
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Window 1: Mapped normal top-level window
        let win1 = client_conn.generate_id().expect("XID 1");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win1,
                root,
                10,
                10,
                200,
                150,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win1");
        client_conn.map_window(win1).expect("Map win1");

        // Window 2: Mapped normal top-level window (stacked on top of win1)
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
        client_conn.map_window(win2).expect("Map win2");

        // Window 3: Override-redirect window (should be IGNORED by manage_existing_windows)
        let win3_override = client_conn.generate_id().expect("XID 3");
        let aux_override = CreateWindowAux::new().override_redirect(1);
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win3_override,
                root,
                0,
                0,
                800,
                24,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux_override,
            )
            .expect("Create win3_override");
        client_conn.map_window(win3_override).expect("Map win3_override");

        // Window 4: Unmapped window (should be IGNORED by manage_existing_windows)
        let win4_unmapped = client_conn.generate_id().expect("XID 4");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win4_unmapped,
                root,
                100,
                100,
                200,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win4_unmapped");
        // win4 is intentionally NOT mapped

        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        // Step 2: Initialize WindowManager on Xephyr display
        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init");
        assert_eq!(wm.client_state.len(), 0);

        // Step 3: Run manage_existing_windows()
        let result = wm.manage_existing_windows();
        assert!(result.is_ok(), "manage_existing_windows failed: {:?}", result);

        // Step 4: Verify adopted clients
        assert_eq!(wm.client_state.len(), 2, "Only mapped non-override windows should be managed");
        assert!(wm.client_state.contains(win1), "win1 must be managed");
        assert!(wm.client_state.contains(win2), "win2 must be managed");
        assert!(!wm.client_state.contains(win3_override), "override-redirect must be ignored");
        assert!(!wm.client_state.contains(win4_unmapped), "unmapped window must be ignored");

        // Workspace allocation: both on workspace 1
        let ws1_clients = &wm.workspace_manager.get(1).expect("Workspace 1").clients;
        assert_eq!(ws1_clients.len(), 2);
        assert!(ws1_clients.contains(&win1));
        assert!(ws1_clients.contains(&win2));

        // Topmost window (win2, created after win1) must have focus
        assert_eq!(wm.focused_window(), Some(win2), "Topmost window must be focused");

        // Both windows must be marked as mapped
        assert!(wm.client_state.get_client(win1).unwrap().mapped);
        assert!(wm.client_state.get_client(win2).unwrap().mapped);
    }
}

#[test]
fn test_startup_existing_windows_with_rules() {
    if !has_xephyr() {
        return;
    }

    let xephyr = XephyrServer::start(191);
    if let Some(server) = xephyr {
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Window 1: Terminal window (stays on active workspace 1)
        let win_term = client_conn.generate_id().expect("XID term");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win_term,
                root,
                10,
                10,
                300,
                300,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create term");
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_term,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"xterm\0XTerm\0",
            )
            .unwrap();
        client_conn.map_window(win_term).expect("Map term");

        // Window 2: Firefox browser window (targeted to workspace 2 via rule)
        let win_browser = client_conn.generate_id().expect("XID browser");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win_browser,
                root,
                50,
                50,
                400,
                400,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create browser");
        client_conn
            .change_property8(
                PropMode::REPLACE,
                win_browser,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"firefox\0Firefox\0",
            )
            .unwrap();
        client_conn.map_window(win_browser).expect("Map browser");

        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        // Start WM
        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init");

        // Configure rule: Firefox -> Workspace 2
        wm.add_rule(WindowRule::new(
            RuleCondition::Class("Firefox".to_string()),
            RuleAction::new().with_workspace(2),
        ));

        // Adopt pre-existing windows
        wm.manage_existing_windows().expect("manage_existing_windows");

        assert_eq!(wm.client_state.len(), 2);

        // Check workspace 1 has win_term
        let ws1 = wm.workspace_manager.get(1).expect("Workspace 1");
        assert_eq!(ws1.clients, vec![win_term]);
        assert!(wm.client_state.get_client(win_term).unwrap().mapped);

        // Check workspace 2 has win_browser, and is unmapped on background workspace
        let ws2 = wm.workspace_manager.get(2).expect("Workspace 2");
        assert_eq!(ws2.clients, vec![win_browser]);
        assert!(!wm.client_state.get_client(win_browser).unwrap().mapped);

        // Focus must be on the active workspace's client (win_term)
        assert_eq!(wm.focused_window(), Some(win_term));

        // Switch to workspace 2 -> browser window is mapped and focused
        wm.switch_workspace(2).expect("Switch to ws 2");
        assert_eq!(wm.current_workspace, 2);
        assert_eq!(wm.focused_window(), Some(win_browser));
        assert!(wm.client_state.get_client(win_browser).unwrap().mapped);
    }
}

#[test]
fn test_startup_existing_windows_free_function_and_idempotency() {
    if !has_xephyr() {
        return;
    }

    let xephyr = XephyrServer::start(192);
    if let Some(server) = xephyr {
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let win = client_conn.generate_id().expect("XID");
        client_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                20,
                20,
                300,
                300,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .expect("Create win");
        client_conn.map_window(win).expect("Map win");
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm: WmState = WindowManager::new(wm_ctx).expect("WM init");

        // Test free function: manage_existing_windows(conn, root, &mut state)
        let res = manage_existing_windows(&client_conn, root, &mut wm);
        assert!(res.is_ok(), "Free function manage_existing_windows failed");
        assert_eq!(wm.client_state.len(), 1);
        assert_eq!(wm.focused_window(), Some(win));

        // Test idempotency: calling wm.manage_existing_windows() again changes nothing
        let res2 = wm.manage_existing_windows();
        assert!(res2.is_ok(), "Second manage_existing_windows call failed");
        assert_eq!(wm.client_state.len(), 1);
        assert_eq!(wm.focused_window(), Some(win));
    }
}
