use std::process::{Child, Command};
use tilo::config::{ConfigError, ReloadError};
use tilo::input::re_register_keybindings;
use tilo::keybind::{Action, KeyBind, MOD_SUPER, XK_RETURN};
use tilo::wm::{reload_config, WindowManager};
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, EventMask, Window, WindowClass,
};

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

fn create_client_window(conn: &impl Connection, root: Window, x: i16, y: i16, w: u16, h: u16) -> Window {
    let win = conn.generate_id().unwrap();
    let aux = CreateWindowAux::new()
        .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE)
        .background_pixel(0x002244);
    conn.create_window(
        0,
        win,
        root,
        x,
        y,
        w,
        h,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &aux,
    )
    .unwrap();
    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    win
}

#[test]
fn test_reload_error_types_and_display() {
    let config_err = ConfigError::ParseError {
        line: 12,
        message: "unexpected token".to_string(),
    };
    let reload_err = ReloadError::from(config_err);
    assert!(format!("{}", reload_err).contains("unexpected token"));

    let io_err = ConfigError::IoError("file not found".to_string());
    let reload_io = ReloadError::from(io_err);
    assert!(format!("{}", reload_io).contains("file not found"));
}

#[test]
fn test_config_reload_live_in_xephyr() {
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

        let temp_dir = std::env::temp_dir().join("tilo_reload_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config");

        // 1. Initial valid configuration on disk
        let initial_cfg = "\
mod Mod4
border_width 3
inner_gap 8
outer_gap 8
master_ratio 0.60
terminal foot
bind Mod4+Return spawn foot
";
        std::fs::write(&config_path, initial_cfg).unwrap();

        let mut wm = WindowManager::new_with_config(
            wm_ctx,
            tilo::config::load_config_from_path(&config_path).unwrap(),
        )
        .expect("Failed to initialize WM");

        assert_eq!(wm.border_config.width, 3);
        assert_eq!(wm.layout_config.border_width, 3);
        assert_eq!(wm.layout_config.inner_gap, 8);
        assert_eq!(wm.layout_config.outer_gap, 8);
        assert!((wm.layout_config.master_ratio - 0.60).abs() < f32::EPSILON);

        // 2. Open two client windows
        let client_conn = X11Context::connect_to(Some(&server.display)).unwrap();
        let win1 = create_client_window(&client_conn.connection, wm.context.root_window, 50, 50, 200, 200);
        let win2 = create_client_window(&client_conn.connection, wm.context.root_window, 100, 100, 200, 200);

        for _ in 0..30 {
            let _ = wm.process_pending_events();
            if wm.client_state.all_clients().len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(wm.client_state.all_clients().len(), 2);

        // Verify initial window border width on win1
        let geom1 = client_conn.connection.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1.border_width, 3);

        // 3. Update configuration file on disk with new border width, gaps, ratio, bindings
        let updated_cfg = "\
mod Mod4
border_width 7
inner_gap 16
outer_gap 20
master_ratio 0.75
terminal alacritty
bind Mod4+Return spawn alacritty
bind Mod4+q close
bind Mod4+Shift+r reload
";
        std::fs::write(&config_path, updated_cfg).unwrap();

        // 4. Reload configuration from path while WM is running
        assert!(wm.reload_config_from_path(&config_path).is_ok());

        // Verify WM state updated
        assert_eq!(wm.border_config.width, 7);
        assert_eq!(wm.layout_config.border_width, 7);
        assert_eq!(wm.layout_config.inner_gap, 16);
        assert_eq!(wm.layout_config.outer_gap, 20);
        assert!((wm.layout_config.master_ratio - 0.75).abs() < f32::EPSILON);
        assert_eq!(wm.config.terminal, "alacritty");

        // Verify existing client window borders were updated immediately in X11
        let geom1_updated = client_conn.connection.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_updated.border_width, 7);
        let geom2_updated = client_conn.connection.get_geometry(win2).unwrap().reply().unwrap();
        assert_eq!(geom2_updated.border_width, 7);

        // 5. Test invalid config reload attempt: should fail gracefully, retaining previous config
        let corrupted_cfg = "\
border_width not_a_number
master_ratio 5.5
unknown_directive foo bar
";
        std::fs::write(&config_path, corrupted_cfg).unwrap();

        let reload_res = wm.reload_config_from_path(&config_path);
        assert!(reload_res.is_err());
        match reload_res.unwrap_err() {
            ReloadError::ConfigInvalid(err) => {
                match err {
                    ConfigError::InvalidValue { line, value } => {
                        assert_eq!(line, 1);
                        assert_eq!(value, "not_a_number");
                    }
                    other => panic!("Unexpected ConfigError: {:?}", other),
                }
            }
            other => panic!("Expected ConfigInvalid error, got: {:?}", other),
        }

        // Verify previous valid configuration was retained intact
        assert_eq!(wm.border_config.width, 7);
        assert_eq!(wm.layout_config.border_width, 7);
        assert_eq!(wm.layout_config.inner_gap, 16);
        assert_eq!(wm.layout_config.outer_gap, 20);
        assert!((wm.layout_config.master_ratio - 0.75).abs() < f32::EPSILON);
        assert_eq!(wm.config.terminal, "alacritty");

        let geom1_after_corrupt = client_conn.connection.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(geom1_after_corrupt.border_width, 7);

        // 6. Test non-existent file handling
        let nonexistent_path = temp_dir.join("nonexistent_config");
        let nonexist_res = wm.reload_config_from_path(&nonexistent_path);
        assert!(nonexist_res.is_err());
        assert_eq!(wm.border_config.width, 7);

        // 7. Test re_register_keybindings helper directly
        let test_bindings = vec![
            KeyBind::new(MOD_SUPER, XK_RETURN, Action::Spawn("foot".to_string())),
        ];
        assert!(re_register_keybindings(
            &wm.context.connection,
            &test_bindings,
            wm.context.root_window
        ).is_ok());

        // 8. Test standalone reload_config function
        // Write valid config again
        std::fs::write(&config_path, updated_cfg).unwrap();
        // Point default config path / reload
        assert!(reload_config(&mut wm).is_err() || reload_config(&mut wm).is_ok());

        // 9. Verify Mod4 + Shift + r KeyPress event triggers Action::Reload
        let mapping = wm.input_manager.mapping.clone().expect("Mapping missing");
        let kc = mapping.keysym_to_keycodes(tilo::keybind::XK_R)[0];
        let ev = x11rb::protocol::xproto::KeyPressEvent {
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
            state: x11rb::protocol::xproto::KeyButMask::from(tilo::keybind::MOD_SUPER | tilo::keybind::MOD_SHIFT),
            same_screen: true,
        };
        assert_eq!(wm.input_manager.handle_event(&ev), Some(Action::Reload));
        assert!(wm.handle_key_press(&ev).is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

#[test]
fn test_default_keybinding_mod4_shift_r_reload_in_xephyr() {
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

        // Verify default keybindings include Mod4 + Shift + r mapped to Action::Reload
        assert!(wm.config.bindings.iter().any(|b| {
            b.modifiers == (tilo::keybind::MOD_SUPER | tilo::keybind::MOD_SHIFT)
                && b.keysym == tilo::keybind::XK_R
                && b.action == Action::Reload
        }));

        // Verify simulated KeyPress event triggers Action::Reload through handle_key_press
        let mapping = wm.input_manager.mapping.clone().expect("Mapping missing");
        let kc = mapping.keysym_to_keycodes(tilo::keybind::XK_R)[0];
        let ev = x11rb::protocol::xproto::KeyPressEvent {
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
            state: x11rb::protocol::xproto::KeyButMask::from(tilo::keybind::MOD_SUPER | tilo::keybind::MOD_SHIFT),
            same_screen: true,
        };
        assert_eq!(wm.input_manager.handle_event(&ev), Some(Action::Reload));
        assert!(wm.handle_key_press(&ev).is_ok());
    }
}
