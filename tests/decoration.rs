use std::process::{Child, Command};
use tilo::config::{parse, tokenize, Config};
use tilo::decoration::{
    parse_color, set_window_border_color, set_window_border_width, update_window_border,
    BorderConfig,
};
use tilo::wm::WindowManager;
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

fn create_client_window(
    conn: &impl Connection,
    root: Window,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
) -> Window {
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
    win
}

#[test]
fn test_border_config_and_color_parsing() {
    let default_config = BorderConfig::default();
    assert_eq!(default_config.width, 2);
    assert_eq!(default_config.focused_color, 0x00ffffff);
    assert_eq!(default_config.normal_color, 0x00444444);
    assert_eq!(default_config.urgent_color, 0x00ff0000);

    assert_eq!(parse_color("#ffffff"), Some(0x00ffffff));
    assert_eq!(parse_color("#000000"), Some(0x00000000));
    assert_eq!(parse_color("#444444"), Some(0x00444444));
    assert_eq!(parse_color("#ff0000"), Some(0x00ff0000));
    assert_eq!(parse_color("#123456"), Some(0x00123456));
    assert_eq!(parse_color("#fff"), Some(0x00ffffff));
    assert_eq!(parse_color("#abc"), Some(0x00aabbcc));
    assert_eq!(parse_color("0x112233"), Some(0x00112233));
    assert_eq!(parse_color("0Xaabbcc"), Some(0x00aabbcc));
    assert_eq!(parse_color("16777215"), Some(0x00ffffff));
    assert_eq!(parse_color("white"), Some(0x00ffffff));
    assert_eq!(parse_color("red"), Some(0x00ff0000));
    assert_eq!(parse_color("not_a_color"), None);
}

#[test]
fn test_config_border_directives() {
    let input = r#"
border_width 5
focused_color #00ff00
normal_color #111111
urgent_color #ff8800
"#;
    let tokens = tokenize(input).expect("tokenization should succeed");
    let config = parse(&tokens).expect("parsing should succeed");

    assert_eq!(config.border_width, 5);
    assert_eq!(config.focused_color, 0x0000ff00);
    assert_eq!(config.normal_color, 0x00111111);
    assert_eq!(config.urgent_color, 0x00ff8800);
}

#[test]
fn test_window_decoration_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = match XephyrServer::start(143) {
        Some(x) => x,
        None => return,
    };

    let context = X11Context::connect_to(Some(&xephyr.display)).unwrap();
    let root = context.root_window;

    let mut config = Config::default();
    config.border_width = 4;
    config.focused_color = 0x0000ff00;
    config.normal_color = 0x00222222;
    config.urgent_color = 0x00ff5500;

    let mut wm = WindowManager::new_with_config(context, config).expect("WindowManager should initialize");

    let win1 = create_client_window(&wm.context.connection, root, 50, 50, 200, 150);
    let win2 = create_client_window(&wm.context.connection, root, 300, 50, 200, 150);
    wm.context.connection.map_window(win1).unwrap();
    wm.context.connection.map_window(win2).unwrap();
    wm.context.connection.flush().unwrap();

    wm.manage_window(win1).unwrap();
    wm.manage_window(win2).unwrap();

    // Verify both windows received configured initial border width
    let geom1 = wm.context.connection.get_geometry(win1).unwrap().reply().unwrap();
    let geom2 = wm.context.connection.get_geometry(win2).unwrap().reply().unwrap();
    assert_eq!(geom1.border_width, 4);
    assert_eq!(geom2.border_width, 4);

    // Test direct border setters
    set_window_border_width(&wm.context.connection, win1, 6).unwrap();
    let geom1_updated = wm.context.connection.get_geometry(win1).unwrap().reply().unwrap();
    assert_eq!(geom1_updated.border_width, 6);

    set_window_border_color(&wm.context.connection, win1, 0x00ff0000).unwrap();

    // Test update_window_border helper
    let border_cfg = BorderConfig {
        width: 3,
        focused_color: 0x00ffffff,
        normal_color: 0x00333333,
        urgent_color: 0x00ff0000,
    };
    update_window_border(&wm.context.connection, win1, &border_cfg, true, false).unwrap();
    let geom1_helper = wm.context.connection.get_geometry(win1).unwrap().reply().unwrap();
    assert_eq!(geom1_helper.border_width, 3);

    // Test focus change updates borders
    wm.focus_client(win1).unwrap();
    assert_eq!(wm.client_state.focused_window(), Some(win1));

    wm.focus_client(win2).unwrap();
    assert_eq!(wm.client_state.focused_window(), Some(win2));

    // Test applying new config updates existing client window border widths and colors
    let mut new_config = Config::default();
    new_config.border_width = 7;
    new_config.focused_color = 0x0000ffff;
    new_config.normal_color = 0x00555555;
    new_config.urgent_color = 0x00ff00ff;

    wm.apply_config(new_config).unwrap();

    assert_eq!(wm.border_config.width, 7);
    assert_eq!(wm.border_config.focused_color, 0x0000ffff);
    assert_eq!(wm.border_config.normal_color, 0x00555555);
    assert_eq!(wm.border_config.urgent_color, 0x00ff00ff);

    let geom1_after_apply = wm.context.connection.get_geometry(win1).unwrap().reply().unwrap();
    let geom2_after_apply = wm.context.connection.get_geometry(win2).unwrap().reply().unwrap();
    assert_eq!(geom1_after_apply.border_width, 7);
    assert_eq!(geom2_after_apply.border_width, 7);
}

#[test]
fn test_newly_spawned_window_focus_and_border_highlight_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = match XephyrServer::start(144) {
        Some(x) => x,
        None => return,
    };

    let context = X11Context::connect_to(Some(&xephyr.display)).unwrap();
    let root = context.root_window;

    let config = Config {
        border_width: 3,
        focused_color: 0x00bd93f9,
        normal_color: 0x0044475a,
        ..Default::default()
    };

    let mut wm = WindowManager::new_with_config(context, config).expect("WM should init");

    // 1. Create and manage window 1
    let win1 = create_client_window(&wm.context.connection, root, 50, 50, 200, 150);
    wm.context.connection.map_window(win1).unwrap();
    wm.context.connection.flush().unwrap();
    wm.manage_window(win1).unwrap();

    // Verify window 1 has full focus and focused border color
    assert_eq!(wm.client_state.focused_window(), Some(win1));
    assert!(wm.client_state.get_client(win1).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win1).unwrap().border_color,
        Some(0x00bd93f9)
    );
    let focus1 = wm.context.connection.get_input_focus().unwrap().reply().unwrap().focus;
    assert_eq!(focus1, win1);

    // 2. Spawn and manage window 2
    let win2 = create_client_window(&wm.context.connection, root, 300, 50, 200, 150);
    wm.context.connection.map_window(win2).unwrap();
    wm.context.connection.flush().unwrap();
    wm.manage_window(win2).unwrap();

    // Verify window 2 has full on focus and focused border highlight
    assert_eq!(wm.client_state.focused_window(), Some(win2));
    assert!(wm.client_state.get_client(win2).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win2).unwrap().border_color,
        Some(0x00bd93f9)
    );
    let focus2 = wm.context.connection.get_input_focus().unwrap().reply().unwrap().focus;
    assert_eq!(focus2, win2);

    // Crucially verify window 1's highlight did NOT persist: it is normal color and unfocused!
    assert!(!wm.client_state.get_client(win1).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win1).unwrap().border_color,
        Some(0x0044475a)
    );

    // 3. Spawn and manage window 3
    let win3 = create_client_window(&wm.context.connection, root, 500, 50, 200, 150);
    wm.context.connection.map_window(win3).unwrap();
    wm.context.connection.flush().unwrap();
    wm.manage_window(win3).unwrap();

    // Window 3 has full focus
    assert_eq!(wm.client_state.focused_window(), Some(win3));
    assert!(wm.client_state.get_client(win3).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win3).unwrap().border_color,
        Some(0x00bd93f9)
    );
    let focus3 = wm.context.connection.get_input_focus().unwrap().reply().unwrap().focus;
    assert_eq!(focus3, win3);

    // Both window 1 and window 2 are unhighlighted
    assert!(!wm.client_state.get_client(win1).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win1).unwrap().border_color,
        Some(0x0044475a)
    );
    assert!(!wm.client_state.get_client(win2).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win2).unwrap().border_color,
        Some(0x0044475a)
    );

    // 4. Test click-to-focus on window 1 transfers full focus back to window 1
    let ev_click = x11rb::protocol::xproto::ButtonPressEvent {
        response_type: 4,
        detail: 1, // Button 1
        sequence: 1,
        time: 0,
        root,
        event: win1,
        child: win1,
        root_x: 60,
        root_y: 60,
        event_x: 10,
        event_y: 10,
        state: x11rb::protocol::xproto::KeyButMask::from(0u16),
        same_screen: true,
    };
    wm.handle_button_press(&ev_click).unwrap();

    assert_eq!(wm.client_state.focused_window(), Some(win1));
    assert!(wm.client_state.get_client(win1).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win1).unwrap().border_color,
        Some(0x00bd93f9)
    );
    assert!(!wm.client_state.get_client(win3).unwrap().focused);
    assert_eq!(
        wm.client_state.get_client(win3).unwrap().border_color,
        Some(0x0044475a)
    );
    let focus_back = wm.context.connection.get_input_focus().unwrap().reply().unwrap().focus;
    assert_eq!(focus_back, win1);
}

