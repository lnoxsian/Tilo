use std::process::{Child, Command};
use tilo::x11::{X11Context, X11Error};

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
            .arg("640x480")
            .arg("-ac")
            .arg("-br")
            .arg("-noreset")
            .spawn()
            .ok()?;

        // Wait up to 3 seconds for Xephyr to accept connections
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
fn test_become_wm_conflict_on_active_wm() {
    if std::env::var("DISPLAY").is_ok() {
        let ctx = X11Context::new().expect("Failed to connect to active DISPLAY");
        // On active desktop session, another WM is running:
        match ctx.become_wm() {
            Ok(()) => {}
            Err(X11Error::AnotherWmRunning) => {
                // Expected conflict detection
            }
            Err(other) => panic!("Unexpected error: {:?}", other),
        }
    }
}

#[test]
fn test_become_wm_success_and_contention_in_xephyr() {
    // Only run if Xephyr is available
    if Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        let xephyr = XephyrServer::start(94);
        if let Some(server) = xephyr {
            // First WM instance connects and becomes WM
            let wm1 = X11Context::connect_to(Some(&server.display))
                .expect("WM1 failed to connect to Xephyr");
            assert!(
                wm1.become_wm().is_ok(),
                "First WM instance must successfully become WM on clean X server"
            );

            // Second WM instance connects and attempts to become WM on same display
            let wm2 = X11Context::connect_to(Some(&server.display))
                .expect("WM2 failed to connect to Xephyr");
            match wm2.become_wm() {
                Err(X11Error::AnotherWmRunning) => {
                    // Correctly rejected second WM!
                }
                other => panic!("Expected AnotherWmRunning for second instance, got: {:?}", other),
            }
        }
    }
}
