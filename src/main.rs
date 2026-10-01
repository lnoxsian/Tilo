use tilo::wm::WindowManager;
use tilo::x11::{X11Context, X11Error};

fn main() {
    let ctx = match X11Context::new() {
        Ok(ctx) => {
            println!("[INFO  tilo::x11] Connected to X11 server.");
            println!("[INFO  tilo::x11] Managing root window: 0x{:x}", ctx.root_window);
            println!(
                "[INFO  tilo::x11] Screen dimensions: {}x{}",
                ctx.screen_width, ctx.screen_height
            );
            ctx
        }
        Err(err) => {
            eprintln!("[ERROR tilo::x11] {}", err);
            std::process::exit(1);
        }
    };

    if let Err(err) = ctx.become_wm() {
        match err {
            X11Error::AnotherWmRunning => {
                eprintln!("[ERROR tilo::wm] Another window manager is already running on this display.");
            }
            _ => {
                eprintln!("[ERROR tilo::wm] Failed to initialize window manager: {}", err);
            }
        }
        std::process::exit(1);
    }

    println!("[INFO  tilo::wm] Successfully became the window manager.");

    let mut wm = match WindowManager::new(ctx) {
        Ok(wm) => wm,
        Err(err) => {
            eprintln!(
                "[ERROR tilo::wm] Failed to initialize window manager state: {}",
                err
            );
            std::process::exit(1);
        }
    };
    if let Err(err) = wm.enable_statusbar(24) {
        eprintln!("[WARN  tilo::wm] Failed to enable statusbar: {}", err);
    }
    if let Err(err) = wm.manage_existing_windows() {
        eprintln!("[WARN  tilo::wm] Failed to manage pre-existing windows: {}", err);
    }
    println!("[INFO  tilo::wm] Entering X11 event loop...");
    if let Err(err) = wm.run() {
        eprintln!("[ERROR tilo::wm] Event loop error: {}", err);
        let _ = wm.perform_graceful_shutdown();
        std::process::exit(1);
    }
    std::process::exit(0);
}
