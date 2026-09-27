use tilo::x11::{X11Context, X11Error};

#[test]
fn test_integration_connect_invalid_display() {
    let result = X11Context::connect_to(Some(":98"));
    assert!(result.is_err());
    match result.unwrap_err() {
        X11Error::Connect(_) => {
            // Succeeded in failing cleanly with Connect error
        }
        other => panic!("Expected X11Error::Connect, got {:?}", other),
    }
}

#[test]
fn test_integration_connect_active_display() {
    if std::env::var("DISPLAY").is_ok() {
        let ctx = X11Context::new().expect("Failed to connect to active X11 display");

        // Verify root window ID is non-zero
        assert_ne!(ctx.root_window, 0);

        // Verify screen dimensions
        assert!(ctx.screen_width > 0);
        assert!(ctx.screen_height > 0);

        // Verify screen info consistency
        let screen = ctx.screen();
        assert_eq!(screen.root, ctx.root_window);
        assert_eq!(screen.width_in_pixels, ctx.screen_width);
        assert_eq!(screen.height_in_pixels, ctx.screen_height);

        // Verify flush
        assert!(ctx.flush().is_ok());
    }
}
