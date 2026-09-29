use std::fs;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use tilo::autostart::{run_autostart_commands, AutostartCmd};
use tilo::config::{parse, tokenize, Config};
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

#[test]
fn test_autostart_configuration_and_process_execution() {
    let witness_path = std::env::temp_dir().join(format!("tilo_autostart_test_{}", std::process::id()));
    if witness_path.exists() {
        let _ = fs::remove_file(&witness_path);
    }

    let config_content = format!(
        r#"
autostart "touch {}"
autostart "__non_existent_binary_xyz123__"
autostart "true"
"#,
        witness_path.display()
    );

    let tokens = tokenize(&config_content).expect("Tokenization failed");
    let config = parse(&tokens).expect("Parsing failed");

    assert_eq!(config.autostart.len(), 3);
    assert_eq!(config.autostart[0].program, "touch");
    assert_eq!(config.autostart[0].args, vec![witness_path.display().to_string()]);
    assert_eq!(config.autostart[1].program, "__non_existent_binary_xyz123__");
    assert_eq!(config.autostart[2].program, "true");

    // Execute autostart commands
    let result = run_autostart_commands(&config.autostart);
    assert!(result.is_ok(), "run_autostart_commands must not fail even if some commands are missing");

    // Poll for the witness file created by `touch`
    let start = Instant::now();
    let mut created = false;
    while start.elapsed() < Duration::from_secs(3) {
        if witness_path.exists() {
            created = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if witness_path.exists() {
        let _ = fs::remove_file(&witness_path);
    }

    assert!(created, "Autostart command failed to execute and touch witness file");
}

#[test]
fn test_wm_autostart_integration_in_xephyr() {
    let xephyr = match XephyrServer::start(168) {
        Some(x) => x,
        None => {
            eprintln!("Xephyr not available on :168, skipping integration test");
            return;
        }
    };

    let ctx = match X11Context::connect_to(Some(&xephyr.display)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Xephyr: {}", e);
            return;
        }
    };

    let witness_path = std::env::temp_dir().join(format!("tilo_wm_autostart_test_{}", std::process::id()));
    if witness_path.exists() {
        let _ = fs::remove_file(&witness_path);
    }

    let config = Config {
        autostart: vec![
            AutostartCmd::new("touch", vec![witness_path.display().to_string()]),
            AutostartCmd::new("__non_existent_app__", vec![]),
        ],
        ..Default::default()
    };

    let wm = WindowManager::new_with_config(ctx, config).expect("Failed to initialize WindowManager");
    let autostart_res = wm.run_autostart();
    assert!(autostart_res.is_ok());

    let start = Instant::now();
    let mut created = false;
    while start.elapsed() < Duration::from_secs(3) {
        if witness_path.exists() {
            created = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if witness_path.exists() {
        let _ = fs::remove_file(&witness_path);
    }

    assert!(created, "WindowManager::run_autostart failed to execute commands");
}
