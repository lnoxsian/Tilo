use std::process::{Child, Command};
use tilo::error::{is_client_disconnect_error, is_connection_lost_error, WmError};
use tilo::wm::WindowManager;
use tilo::x11::{X11Context, X11Error};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, EventMask, Window, WindowClass,
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

fn create_test_window(conn: &impl Connection, screen_num: usize, x: i16, y: i16, w: u16, h: u16) -> Window {
    let win = conn.generate_id().unwrap();
    let screen = &conn.setup().roots[screen_num];
    let values = CreateWindowAux::new()
        .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        screen.root,
        x,
        y,
        w,
        h,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &values,
    )
    .unwrap();
    conn.sync().unwrap();
    win
}

#[test]
fn test_client_unexpected_destruction_and_focus_recovery() {
    let server = match XephyrServer::start(180) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test: Xephyr not available");
            return;
        }
    };

    let wm_ctx = X11Context::connect_to(Some(&server.display)).unwrap();
    wm_ctx.become_wm().unwrap();
    let mut wm = WindowManager::new(wm_ctx).unwrap();

    let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).unwrap();
    let w1 = create_test_window(&client_conn, screen_num, 50, 50, 200, 200);
    let w2 = create_test_window(&client_conn, screen_num, 300, 50, 200, 200);

    // Map windows
    client_conn.map_window(w1).unwrap();
    client_conn.map_window(w2).unwrap();
    client_conn.sync().unwrap();

    wm.process_pending_events().unwrap();

    assert!(wm.client_state.contains(w1));
    assert!(wm.client_state.contains(w2));
    assert_eq!(wm.focused_window(), Some(w2));

    // Client w2 unexpectedly terminates / destroys window
    client_conn.destroy_window(w2).unwrap();
    client_conn.sync().unwrap();

    // Process events: WM must handle DestroyNotify and restore focus to w1
    wm.process_pending_events().unwrap();

    assert!(!wm.client_state.contains(w2));
    assert!(wm.client_state.contains(w1));
    assert_eq!(wm.focused_window(), Some(w1));
}

#[test]
fn test_client_unmap_and_retile_recovery() {
    let server = match XephyrServer::start(181) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test: Xephyr not available");
            return;
        }
    };

    let wm_ctx = X11Context::connect_to(Some(&server.display)).unwrap();
    wm_ctx.become_wm().unwrap();
    let mut wm = WindowManager::new(wm_ctx).unwrap();

    let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).unwrap();
    let w1 = create_test_window(&client_conn, screen_num, 50, 50, 200, 200);
    let w2 = create_test_window(&client_conn, screen_num, 300, 50, 200, 200);

    client_conn.map_window(w1).unwrap();
    client_conn.map_window(w2).unwrap();
    client_conn.sync().unwrap();

    wm.process_pending_events().unwrap();
    assert_eq!(wm.focused_window(), Some(w2));

    // w2 unmaps itself
    client_conn.unmap_window(w2).unwrap();
    client_conn.sync().unwrap();

    wm.process_pending_events().unwrap();

    // Remaining mapped window w1 receives focus
    assert_eq!(wm.focused_window(), Some(w1));
}

#[test]
fn test_workspace_changes_during_client_destruction() {
    let server = match XephyrServer::start(182) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test: Xephyr not available");
            return;
        }
    };

    let wm_ctx = X11Context::connect_to(Some(&server.display)).unwrap();
    wm_ctx.become_wm().unwrap();
    let mut wm = WindowManager::new(wm_ctx).unwrap();

    let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).unwrap();
    let w1 = create_test_window(&client_conn, screen_num, 50, 50, 200, 200);
    let w2 = create_test_window(&client_conn, screen_num, 300, 50, 200, 200);

    client_conn.map_window(w1).unwrap();
    client_conn.sync().unwrap();
    wm.process_pending_events().unwrap();

    // Move w2 to workspace 2
    wm.switch_workspace(2).unwrap();
    client_conn.map_window(w2).unwrap();
    client_conn.sync().unwrap();
    wm.process_pending_events().unwrap();

    // Switch back to workspace 1: w1 is active and focused
    wm.switch_workspace(1).unwrap();
    assert_eq!(wm.current_workspace_id(), 1);
    assert_eq!(wm.focused_window(), Some(w1));

    // Client w2 on workspace 2 is destroyed while workspace 1 is active
    client_conn.destroy_window(w2).unwrap();
    client_conn.sync().unwrap();

    wm.process_pending_events().unwrap();

    // Focus on workspace 1 must NOT be stolen or cleared!
    assert_eq!(wm.current_workspace_id(), 1);
    assert_eq!(wm.focused_window(), Some(w1));
    assert!(!wm.client_state.contains(w2));

    // Switching to workspace 2 is safe and leaves workspace 2 clean
    wm.switch_workspace(2).unwrap();
    assert_eq!(wm.current_workspace_id(), 2);
    assert_eq!(wm.focused_window(), None);
}

#[test]
fn test_launcher_unexpected_destruction_recovery() {
    let server = match XephyrServer::start(183) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test: Xephyr not available");
            return;
        }
    };

    let wm_ctx = X11Context::connect_to(Some(&server.display)).unwrap();
    wm_ctx.become_wm().unwrap();
    let mut wm = WindowManager::new(wm_ctx).unwrap();

    let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).unwrap();
    let w1 = create_test_window(&client_conn, screen_num, 50, 50, 200, 200);
    client_conn.map_window(w1).unwrap();
    client_conn.sync().unwrap();
    wm.process_pending_events().unwrap();

    // Open launcher
    wm.open_launcher().unwrap();
    let launcher_win = wm.launcher.as_ref().unwrap().window_id;
    assert_ne!(launcher_win, 0);
    assert!(wm.launcher.as_ref().unwrap().visible);

    // Destroy launcher window unexpectedly via client connection
    client_conn.destroy_window(launcher_win).unwrap();
    client_conn.sync().unwrap();

    // Process destroy event: WM must reset launcher state cleanly and restore client focus
    wm.process_pending_events().unwrap();

    let launcher = wm.launcher.as_ref().unwrap();
    assert!(!launcher.visible);
    assert_eq!(launcher.window_id, 0);
    assert_eq!(wm.focused_window(), Some(w1));
}

#[test]
fn test_invalid_config_fallback_without_panic() {
    let temp_dir = std::env::temp_dir().join(format!("tilo_test_cfg_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let cfg_file = temp_dir.join("config");

    // Write completely invalid/corrupted config
    std::fs::write(&cfg_file, "this_is_an_invalid_directive @@@ 9999\nborder_width not_a_number\n").unwrap();

    // load_config_from_path should fail with ConfigError
    let load_res = tilo::config::load_config_from_path(&cfg_file);
    assert!(load_res.is_err());

    // Point XDG_CONFIG_HOME to temp_dir so load_config_or_default attempts to load the corrupted config
    let tilo_dir = temp_dir.join("tilo");
    let _ = std::fs::create_dir_all(&tilo_dir);
    let xdg_cfg_file = tilo_dir.join("config");
    std::fs::write(&xdg_cfg_file, "this_is_an_invalid_directive @@@ 9999\nborder_width not_a_number\n").unwrap();

    let prev_xdg = std::env::var("XDG_CONFIG_HOME").ok();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &temp_dir);
    }

    // Fallback load_config_or_default returns Default without panicking when config file is corrupted
    let default_cfg = tilo::config::load_config_or_default();
    assert_eq!(default_cfg.mod_key, "Mod4");
    assert_eq!(default_cfg.border_width, 2);

    unsafe {
        if let Some(prev) = prev_xdg {
            std::env::set_var("XDG_CONFIG_HOME", prev);
        } else {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_missing_display_and_wm_error_classification() {
    // Attempting to connect to invalid display
    let err = X11Context::connect_to(Some(":99999")).unwrap_err();
    assert!(is_connection_lost_error(&err));

    let wm_err = WmError::from(err);
    assert!(wm_err.is_fatal());
    assert!(!wm_err.is_recoverable());

    let cfg_err = WmError::ConfigError("Bad config".to_string());
    assert!(!cfg_err.is_fatal());
    assert!(cfg_err.is_recoverable());

    assert!(!is_client_disconnect_error(&X11Error::AnotherWmRunning));
}
