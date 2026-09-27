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

    match ctx.become_wm() {
        Ok(()) => {
            println!("[INFO  tilo::wm] Successfully became the window manager.");
        }
        Err(X11Error::AnotherWmRunning) => {
            eprintln!("[ERROR tilo::wm] Another window manager is already running on this display.");
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("[ERROR tilo::wm] Failed to initialize window manager: {}", err);
            std::process::exit(1);
        }
    }
}
