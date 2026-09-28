use std::process::{Child, Command};
use tilo::scratchpad::{
    calculate_scratchpad_geometry, set_scratchpad_client, toggle_scratchpad, Scratchpad,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, EventMask, WindowClass,
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
            .arg("1024x768")
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
fn test_scratchpad_standalone_logic() {
    let mut sp = Scratchpad::new();
    assert_eq!(sp.client, None);
    assert!(!sp.is_visible);

    set_scratchpad_client(&mut sp, 42);
    assert_eq!(sp.client, Some(42));
    assert!(sp.is_scratchpad(42));
    assert!(!sp.is_scratchpad(43));

    let geom = calculate_scratchpad_geometry(0, 0, 1000, 800);
    assert_eq!(geom.width, 700);
    assert_eq!(geom.height, 560);
    assert_eq!(geom.x, 150);
    assert_eq!(geom.y, 120);

    sp.clear_client();
    assert_eq!(sp.client, None);
    assert!(!sp.is_visible);
}

#[test]
fn test_scratchpad_in_xephyr() {
    let xephyr = match XephyrServer::start(232) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test_scratchpad_in_xephyr: Xephyr not available");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(&xephyr.display)).expect("Failed to connect to Xephyr");
    ctx.become_wm().expect("Failed to become WM in Xephyr");

    let mut wm = WindowManager::new(ctx).expect("Failed to initialize WindowManager");

    // 1. Create a client window
    let client_win = wm.context.connection.generate_id().unwrap();
    let screen = wm.context.screen();
    let aux = CreateWindowAux::new()
        .background_pixel(screen.white_pixel)
        .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE);

    wm.context
        .connection
        .create_window(
            screen.root_depth,
            client_win,
            wm.context.root_window,
            100,
            100,
            400,
            300,
            1,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )
        .unwrap();

    // Map and manage the window
    wm.manage_window(client_win).unwrap();
    assert!(wm.client_state.contains(client_win));

    // 2. Assign window as scratchpad
    wm.set_scratchpad_client(client_win);
    assert_eq!(wm.scratchpad.client, Some(client_win));
    assert!(wm.scratchpad.is_visible);

    let client = wm.client_state.get_client(client_win).unwrap();
    assert!(client.floating, "Scratchpad client must be marked as floating");

    // 3. Toggle scratchpad: visible -> hidden
    wm.toggle_scratchpad().unwrap();
    assert!(!wm.scratchpad.is_visible);
    let client = wm.client_state.get_client(client_win).unwrap();
    assert!(!client.mapped, "Scratchpad client must be unmapped when hidden");

    // 4. Toggle scratchpad: hidden -> visible
    wm.toggle_scratchpad().unwrap();
    assert!(wm.scratchpad.is_visible);
    let client = wm.client_state.get_client(client_win).unwrap();
    assert!(client.mapped, "Scratchpad client must be mapped when visible");
    assert!(client.floating);

    // Verify centered geometry
    let geom = client.floating_geometry.expect("Must have floating geometry");
    let mon = wm.monitor_manager.active_monitor();
    let expected_geom = calculate_scratchpad_geometry(mon.x, mon.y, mon.width, mon.height);
    assert_eq!(geom.x, expected_geom.x);
    assert_eq!(geom.y, expected_geom.y);
    assert_eq!(geom.width, expected_geom.width);
    assert_eq!(geom.height, expected_geom.height);

    // 5. Switch to workspace 2 while scratchpad is visible
    wm.switch_workspace(2).unwrap();
    assert_eq!(wm.current_workspace, 2);
    assert!(wm.scratchpad.is_visible);
    let client = wm.client_state.get_client(client_win).unwrap();
    assert_eq!(client.workspace, 2, "Scratchpad should move with user to new workspace");
    assert!(client.mapped);

    // 6. Destroy scratchpad window and verify cleanup
    wm.handle_destroy_notify(client_win).unwrap();
    assert_eq!(wm.scratchpad.client, None, "Scratchpad client must be cleared on DestroyNotify");
    assert!(!wm.scratchpad.is_visible);
}

#[test]
fn test_scratchpad_function_toggle() {
    let xephyr = match XephyrServer::start(233) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test_scratchpad_function_toggle: Xephyr not available");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(&xephyr.display)).expect("Failed to connect to Xephyr");
    let screen = ctx.screen();
    let client_win = ctx.connection.generate_id().unwrap();
    let aux = CreateWindowAux::new()
        .background_pixel(screen.white_pixel)
        .event_mask(EventMask::STRUCTURE_NOTIFY);

    ctx.connection
        .create_window(
            screen.root_depth,
            client_win,
            ctx.root_window,
            50,
            50,
            300,
            200,
            1,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )
        .unwrap();

    let mut sp = Scratchpad::with_client(client_win);
    assert_eq!(sp.client, Some(client_win));
    assert!(!sp.is_visible);

    // Call toggle_scratchpad directly
    toggle_scratchpad(&mut sp, &ctx.connection, 1).unwrap();
    assert!(sp.is_visible);

    // Call toggle_scratchpad again to hide
    toggle_scratchpad(&mut sp, &ctx.connection, 1).unwrap();
    assert!(!sp.is_visible);
}
