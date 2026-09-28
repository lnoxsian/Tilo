use std::io::Write;
use std::process::{Child, Command};
use tilo::config::{load_config_from_path, parse, tokenize, Config, ConfigError, Token};
use tilo::focus::Direction;
use tilo::keybind::{Action, MOD_SHIFT, MOD_SUPER, XK_1, XK_COMMA, XK_H, XK_PERIOD, XK_Q, XK_RETURN, XK_SPACE};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;

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
fn test_tokenize_comments_and_whitespace() {
    let input = "# This is a comment\n\n   # Indented comment\nmod Mod4 # Inline comment\n   border_width   4   \n\n";
    let tokens = tokenize(input).unwrap();

    assert_eq!(
        tokens,
        vec![
            Token::Newline,
            Token::Newline,
            Token::Newline,
            Token::Word("mod".to_string()),
            Token::Word("Mod4".to_string()),
            Token::Newline,
            Token::Word("border_width".to_string()),
            Token::Number(4),
            Token::Newline,
            Token::Newline,
            Token::Eof,
        ]
    );
}

#[test]
fn test_parse_plan_md_example() {
    let input = r#"
mod Mod4

font JetBrainsMono Nerd Font 10

border_width 2
inner_gap 8
outer_gap 8
master_ratio 0.60

terminal foot

bind Mod4+Return spawn foot
bind Mod4+Space launcher
bind Mod4+q close

bind Mod4+h focus left
bind Mod4+j focus down
bind Mod4+k focus up
bind Mod4+l focus right

bind Mod4+Shift+h move left
bind Mod4+Shift+j move down
bind Mod4+Shift+k move up
bind Mod4+Shift+l move right

bind Mod4+1 workspace 1
bind Mod4+2 workspace 2

bind Mod4+Shift+1 move_workspace 1
bind Mod4+Shift+2 move_workspace 2
"#;
    let tokens = tokenize(input).unwrap();
    let config = parse(&tokens).unwrap();

    assert_eq!(config.mod_key, "Mod4");
    assert_eq!(config.font, "JetBrainsMono Nerd Font 10");
    assert_eq!(config.border_width, 2);
    assert_eq!(config.inner_gap, 8);
    assert_eq!(config.outer_gap, 8);
    assert!((config.master_ratio - 0.60).abs() < f32::EPSILON);
    assert_eq!(config.terminal, "foot");

    assert_eq!(config.bindings.len(), 15);

    // Verify keybindings
    assert_eq!(config.bindings[0].modifiers, MOD_SUPER);
    assert_eq!(config.bindings[0].keysym, XK_RETURN);
    assert_eq!(config.bindings[0].action, Action::Spawn("foot".to_string()));

    assert_eq!(config.bindings[1].modifiers, MOD_SUPER);
    assert_eq!(config.bindings[1].keysym, XK_SPACE);
    assert_eq!(config.bindings[1].action, Action::Launcher);

    assert_eq!(config.bindings[2].modifiers, MOD_SUPER);
    assert_eq!(config.bindings[2].keysym, XK_Q);
    assert_eq!(config.bindings[2].action, Action::Close);

    assert_eq!(config.bindings[3].modifiers, MOD_SUPER);
    assert_eq!(config.bindings[3].keysym, XK_H);
    assert_eq!(config.bindings[3].action, Action::Focus(Direction::Left));

    assert_eq!(config.bindings[7].modifiers, MOD_SUPER | MOD_SHIFT);
    assert_eq!(config.bindings[7].keysym, XK_H);
    assert_eq!(config.bindings[7].action, Action::Move(Direction::Left));

    assert_eq!(config.bindings[11].modifiers, MOD_SUPER);
    assert_eq!(config.bindings[11].keysym, XK_1);
    assert_eq!(config.bindings[11].action, Action::Workspace(1));

    assert_eq!(config.bindings[13].modifiers, MOD_SUPER | MOD_SHIFT);
    assert_eq!(config.bindings[13].keysym, XK_1);
    assert_eq!(config.bindings[13].action, Action::MoveToWorkspace(1));
}

#[test]
fn test_parse_monitor_and_window_state_bindings() {
    let input = r#"
mod Mod4
bind Mod4+f toggle_fullscreen
bind Mod4+Shift+space toggle_floating
bind Mod4+comma focus_monitor left
bind Mod4+period focus_monitor right
bind Mod4+Shift+comma move_monitor 0
bind Mod4+Shift+r reload
bind Mod4+Shift+e exit
"#;
    let tokens = tokenize(input).unwrap();
    let config = parse(&tokens).unwrap();

    assert_eq!(config.bindings.len(), 7);
    assert_eq!(config.bindings[0].action, Action::ToggleFullscreen);
    assert_eq!(config.bindings[1].action, Action::ToggleFloating);
    assert_eq!(config.bindings[2].action, Action::FocusMonitorLeft);
    assert_eq!(config.bindings[2].keysym, XK_COMMA);
    assert_eq!(config.bindings[3].action, Action::FocusMonitorRight);
    assert_eq!(config.bindings[3].keysym, XK_PERIOD);
    assert_eq!(config.bindings[4].action, Action::MoveToMonitor(0));
    assert_eq!(config.bindings[5].action, Action::Reload);
    assert_eq!(config.bindings[6].action, Action::Exit);
}

#[test]
fn test_error_handling_invalid_syntax() {
    // 1. Unknown directive
    let input1 = "unknown_directive 42\n";
    let tokens1 = tokenize(input1).unwrap();
    assert_eq!(
        parse(&tokens1).unwrap_err(),
        ConfigError::InvalidValue {
            line: 1,
            value: "unknown_directive".to_string()
        }
    );

    // 2. Invalid master ratio > 1.0
    let input2 = "\nmaster_ratio 1.25\n";
    let tokens2 = tokenize(input2).unwrap();
    assert_eq!(
        parse(&tokens2).unwrap_err(),
        ConfigError::InvalidValue {
            line: 2,
            value: "1.25".to_string()
        }
    );

    // 3. Invalid border width non-numeric
    let input3 = "border_width invalid\n";
    let tokens3 = tokenize(input3).unwrap();
    assert_eq!(
        parse(&tokens3).unwrap_err(),
        ConfigError::InvalidValue {
            line: 1,
            value: "invalid".to_string()
        }
    );
}

#[test]
fn test_load_config_from_disk_and_fallback() {
    let temp_dir = std::env::temp_dir().join("tilo_test_config");
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("test_config");

    let content = "border_width 4\ninner_gap 12\nouter_gap 16\nmaster_ratio 0.55\n";
    let mut file = std::fs::File::create(&config_path).unwrap();
    file.write_all(content.as_bytes()).unwrap();

    let loaded = load_config_from_path(&config_path).unwrap();
    assert_eq!(loaded.border_width, 4);
    assert_eq!(loaded.inner_gap, 12);
    assert_eq!(loaded.outer_gap, 16);
    assert!((loaded.master_ratio - 0.55).abs() < f32::EPSILON);

    let nonexistent = temp_dir.join("does_not_exist");
    assert!(load_config_from_path(&nonexistent).is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_wm_apply_config_in_xephyr() {
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

        let mut custom_config = Config::default();
        custom_config.border_width = 6;
        custom_config.inner_gap = 14;
        custom_config.outer_gap = 18;
        custom_config.master_ratio = 0.70;

        let mut wm = WindowManager::new_with_config(wm_ctx, custom_config.clone())
            .expect("Failed to initialize WM with config");

        assert_eq!(wm.border_config.width, 6);
        assert_eq!(wm.layout_config.border_width, 6);
        assert_eq!(wm.layout_config.inner_gap, 14);
        assert_eq!(wm.layout_config.outer_gap, 18);
        assert!((wm.layout_config.master_ratio - 0.70).abs() < f32::EPSILON);

        // Test dynamic apply_config (e.g. reload)
        let mut reloaded_config = custom_config;
        reloaded_config.border_width = 3;
        reloaded_config.inner_gap = 10;
        reloaded_config.master_ratio = 0.50;

        assert!(wm.apply_config(reloaded_config).is_ok());
        assert_eq!(wm.border_config.width, 3);
        assert_eq!(wm.layout_config.border_width, 3);
        assert_eq!(wm.layout_config.inner_gap, 10);
        assert!((wm.layout_config.master_ratio - 0.50).abs() < f32::EPSILON);
    }
}
