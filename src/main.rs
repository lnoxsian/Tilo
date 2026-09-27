use tilo::x11::X11Context;

fn main() {
    match X11Context::new() {
        Ok(ctx) => {
            println!("[INFO  tilo::x11] Connected to X11 server.");
            println!("[INFO  tilo::x11] Managing root window: 0x{:x}", ctx.root_window);
            println!(
                "[INFO  tilo::x11] Screen dimensions: {}x{}",
                ctx.screen_width, ctx.screen_height
            );
        }
        Err(err) => {
            eprintln!("[ERROR tilo::x11] {}", err);
            std::process::exit(1);
        }
    }
}
