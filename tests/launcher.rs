use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use tilo::launcher::{filter_executables, scan_desktop_dirs, scan_path_from, spawn_app, Launcher};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::protocol::xproto::{ConnectionExt, KeyPressEvent};

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
fn test_path_scanning_and_permissions() {
    let temp_dir = std::env::temp_dir().join(format!("tilo_launcher_scan_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

    // Create an executable file
    let exe1 = temp_dir.join("my_app");
    std::fs::write(&exe1, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&exe1).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&exe1, perms).unwrap();

    // Create a non-executable file
    let non_exe = temp_dir.join("readme.txt");
    std::fs::write(&non_exe, "Hello\n").unwrap();
    let mut perms_non = std::fs::metadata(&non_exe).unwrap().permissions();
    perms_non.set_mode(0o644);
    std::fs::set_permissions(&non_exe, perms_non).unwrap();

    // Create a second executable
    let exe2 = temp_dir.join("editor");
    std::fs::write(&exe2, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms2 = std::fs::metadata(&exe2).unwrap().permissions();
    perms2.set_mode(0o755);
    std::fs::set_permissions(&exe2, perms2).unwrap();

    let scanned = scan_path_from(temp_dir.to_str().unwrap()).unwrap();
    assert_eq!(scanned, vec!["editor".to_string(), "my_app".to_string()]);
    assert!(!scanned.contains(&"readme.txt".to_string()));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_spawn_app_with_arguments_and_error_handling() {
    // 1. Spawning executable with arguments
    let test_file = std::env::temp_dir().join(format!("tilo_spawn_test_{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&test_file);

    let cmd = format!("touch {}", test_file.to_str().unwrap());
    let pid = spawn_app(&cmd).expect("spawn_app with touch should succeed");
    assert!(pid > 0);

    // Wait briefly for touch to create the file
    for _ in 0..20 {
        if test_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(test_file.exists(), "Command with argument should have touched the file");
    let _ = std::fs::remove_file(&test_file);

    // 2. Spawning empty command line should error
    let empty_res = spawn_app("   ");
    assert!(empty_res.is_err());

    // 3. Spawning non-existent binary should return SpawnError
    let bad_res = spawn_app("non_existent_binary_xyz_12345 --flag");
    assert!(bad_res.is_err());
}

#[test]
fn test_fuzzy_match_and_arguments_filtering() {
    let executables = vec![
        "firefox".to_string(),
        "firefox-esr".to_string(),
        "firewall-config".to_string(),
        "foot".to_string(),
        "xterm".to_string(),
    ];

    // Direct match
    let res = filter_executables(&executables, "firefox", 5);
    assert_eq!(res[0], "firefox");

    // Match with arguments (e.g. firefox --private-window)
    let res_args = filter_executables(&executables, "firefox --private-window", 5);
    assert_eq!(res_args[0], "firefox");

    // Partial prefix with arguments
    let res_prefix = filter_executables(&executables, "fire --private", 5);
    assert!(res_prefix.contains(&"firefox".to_string()));

    // Fuzzy matching
    let res_fuzzy = filter_executables(&executables, "fx", 5);
    assert!(res_fuzzy.contains(&"firefox".to_string()));
}

#[test]
fn test_launcher_in_xephyr_lifecycle_and_interaction() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(102);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WM");

        // 1. Initial state: launcher is closed
        assert!(!wm.launcher.as_ref().unwrap().visible);

        // 2. Open launcher
        wm.open_launcher().expect("open_launcher should succeed");
        let launcher = wm.launcher.as_ref().unwrap();
        assert!(launcher.visible);
        assert!(launcher.window_id > 0);
        assert_eq!(launcher.rect.height, 256, "Launcher height must be 256px to fit top 8 suggestions with no empty space");
        let launcher_win = launcher.window_id;

        // Verify window attributes: override_redirect must be true
        let attrs = wm
            .context
            .connection
            .get_window_attributes(launcher_win)
            .unwrap()
            .reply()
            .unwrap();
        assert!(attrs.override_redirect);

        // 3. Render launcher UI
        wm.render_launcher().expect("render_launcher should succeed");

        // 4. Simulate typing characters: 'f', 'o', 'o', 't'
        // Using handle_launcher_keypress and update_launcher_state directly
        let mapping = wm.input_manager.mapping.as_ref().unwrap().clone();
        let keycodes_f = mapping.keysym_to_keycodes('f' as u32);
        if let Some(&kc) = keycodes_f.first() {
            let ev = KeyPressEvent {
                response_type: 2,
                detail: kc,
                sequence: 0,
                time: 0,
                root: wm.context.root_window,
                event: launcher_win,
                child: 0,
                root_x: 0,
                root_y: 0,
                event_x: 0,
                event_y: 0,
                state: 0u16.into(),
                same_screen: true,
            };
            wm.handle_launcher_key(&ev).unwrap();
        }

        assert_eq!(wm.launcher.as_ref().unwrap().query, "f");
        assert_eq!(wm.launcher.as_ref().unwrap().rect.height, 256, "Launcher height must stay fixed at 256px");

        // 5. Test close launcher
        wm.close_launcher().expect("close_launcher should succeed");
        assert!(!wm.launcher.as_ref().unwrap().visible);
        assert_eq!(wm.launcher.as_ref().unwrap().query, "");

        // 6. Test toggle launcher
        wm.toggle_launcher().expect("toggle_launcher (open) should succeed");
        assert!(wm.launcher.as_ref().unwrap().visible);

        wm.toggle_launcher().expect("toggle_launcher (close) should succeed");
        assert!(!wm.launcher.as_ref().unwrap().visible);

        // 7. Test triggering launcher via KeyPress event (Mod4+d)
        let d_kc = mapping.keysym_to_keycodes(tilo::keybind::XK_D)[0];

        let ev_mod4_d = KeyPressEvent {
            response_type: 2,
            detail: d_kc,
            sequence: 0,
            time: 0,
            root: wm.context.root_window,
            event: wm.context.root_window,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: tilo::keybind::MOD_SUPER.into(),
            same_screen: true,
        };
        wm.handle_event(x11rb::protocol::Event::KeyPress(ev_mod4_d)).unwrap();
        assert!(wm.launcher.as_ref().unwrap().visible, "Mod4+d should open launcher");
        wm.close_launcher().unwrap();

        // 8. Test SIGUSR1 signal toggle
        tilo::wm::LAUNCHER_SIGNAL.store(true, std::sync::atomic::Ordering::SeqCst);
        if tilo::wm::LAUNCHER_SIGNAL.swap(false, std::sync::atomic::Ordering::SeqCst) {
            wm.toggle_launcher().unwrap();
        }
        assert!(wm.launcher.as_ref().unwrap().visible, "SIGUSR1 should toggle launcher open");
        wm.close_launcher().unwrap();
    }
}

#[test]
fn test_dynamic_refresh_on_open_launcher() {
    let temp_bin_dir = std::env::temp_dir().join(format!("tilo_test_dyn_bin_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_bin_dir);
    let _ = std::fs::create_dir_all(&temp_bin_dir);

    // 1. Initial binary
    let app1 = temp_bin_dir.join("init_app");
    std::fs::write(&app1, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&app1).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&app1, perms).unwrap();

    let path_var = temp_bin_dir.to_str().unwrap();

    let mut launcher = Launcher::new();
    launcher.refresh_from(path_var, &[]);
    assert!(launcher.executable_cache.contains(&"init_app".to_string()));
    assert!(!launcher.executable_cache.contains(&"newly_installed_pkg".to_string()));

    // Verify refresh_if_needed_from returns false when no files or directories changed
    assert!(!launcher.refresh_if_needed_from(path_var, &[]));

    // 2. Simulate installing a new package/binary while system is running
    std::thread::sleep(std::time::Duration::from_millis(50));
    let app2 = temp_bin_dir.join("newly_installed_pkg");
    std::fs::write(&app2, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms2 = std::fs::metadata(&app2).unwrap().permissions();
    perms2.set_mode(0o755);
    std::fs::set_permissions(&app2, perms2).unwrap();

    // 3. Opening launcher triggers refresh_if_needed_from
    assert!(launcher.refresh_if_needed_from(path_var, &[]));
    assert!(launcher.executable_cache.contains(&"newly_installed_pkg".to_string()));

    launcher.query = "newly".to_string();
    let filtered = launcher.filtered_results(5);
    assert!(filtered.contains(&"newly_installed_pkg".to_string()));

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_bin_dir);
}

#[test]
fn test_desktop_applications_indexing_and_command_resolution() {
    let temp_desktop_dir = std::env::temp_dir().join(format!("tilo_test_desktop_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_desktop_dir);
    let _ = std::fs::create_dir_all(&temp_desktop_dir);

    let desktop_file = temp_desktop_dir.join("vlc.desktop");
    let content = r#"
[Desktop Entry]
Version=1.0
Type=Application
Name=VLC media player
Exec=vlc --started-from-file %U
NoDisplay=false
"#;
    std::fs::write(&desktop_file, content).unwrap();

    let (parsed_names, commands) = scan_desktop_dirs(std::slice::from_ref(&temp_desktop_dir));
    assert!(parsed_names.contains(&"VLC media player".to_string()));
    assert_eq!(commands.get("VLC media player").unwrap(), "vlc --started-from-file");
    assert_eq!(commands.get("vlc").unwrap(), "vlc --started-from-file");

    let _ = std::fs::remove_dir_all(&temp_desktop_dir);
}
