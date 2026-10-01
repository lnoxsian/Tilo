use std::process::{Child, Command};
use tilo::keybind::Action;
use tilo::layout::LayoutType;
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ConnectionExt, CreateWindowAux, EventMask, MapState, PropMode,
    PropertyNotifyEvent, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

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

fn create_client_window(conn: &impl Connection, root: Window, title: &str) -> Window {
    let win = conn.generate_id().unwrap();
    let aux = CreateWindowAux::new()
        .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE)
        .background_pixel(0x002244);
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        400,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .unwrap();

    // Set WM_NAME
    conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        title.as_bytes(),
    )
    .unwrap();

    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    conn.sync().unwrap();
    win
}

#[test]
fn test_tabbed_layout_single_window_visible_and_status_bar_integration() {
    let server = match XephyrServer::start(171) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :171, skipping integration test");
            return;
        }
    };

    let client_ctx = X11Context::connect_to(Some(&server.display)).expect("Client connect failed");
    let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect failed");
    assert!(wm_ctx.become_wm().is_ok());

    let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WindowManager");
    let bar_win = wm.enable_statusbar(24).expect("enable_statusbar failed");

    // Switch workspace 1 to Tabbed layout
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().layout, LayoutType::Tabbed);

    // Create 3 client windows
    let root = client_ctx.root_window;
    let win1 = create_client_window(&client_ctx.connection, root, "Terminal");
    let win2 = create_client_window(&client_ctx.connection, root, "nvim");
    let win3 = create_client_window(&client_ctx.connection, root, "Firefox");

    // Manage all 3 windows
    assert!(wm.manage_window(win1).unwrap());
    assert!(wm.manage_window(win2).unwrap());
    assert!(wm.manage_window(win3).unwrap());

    // In Tabbed layout:
    // Only the currently active window (win3, last managed) should be mapped
    // win1 and win2 must be unmapped (hidden)
    let c1 = wm.client_state.get_client(win1).unwrap();
    let c2 = wm.client_state.get_client(win2).unwrap();
    let c3 = wm.client_state.get_client(win3).unwrap();

    assert!(!c1.mapped, "win1 must be unmapped in tabbed layout");
    assert!(!c2.mapped, "win2 must be unmapped in tabbed layout");
    assert!(c3.mapped, "win3 (active tab) must be mapped");
    assert_eq!(wm.focused_window(), Some(win3));

    // Verify X11 MapState on server
    let attr1 = wm.context.connection.get_window_attributes(win1).unwrap().reply().unwrap();
    let attr2 = wm.context.connection.get_window_attributes(win2).unwrap().reply().unwrap();
    let attr3 = wm.context.connection.get_window_attributes(win3).unwrap().reply().unwrap();

    assert_eq!(attr1.map_state, MapState::UNMAPPED);
    assert_eq!(attr2.map_state, MapState::UNMAPPED);
    assert_eq!(attr3.map_state, MapState::VIEWABLE);

    // Sized to usable rect (occupies the entire layout area)
    let geom3 = wm.context.connection.get_geometry(win3).unwrap().reply().unwrap();
    assert_eq!(geom3.y, 24, "Should be placed directly below the 24px status bar");
    assert_eq!(geom3.width, 1000 - 2 * wm.layout_config.border_width as u16);

    // Check status bar tab metadata
    let tabs = wm.get_workspace_tabs(1);
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[0].window, win1);
    assert_eq!(tabs[0].title, "Terminal");
    assert_eq!(tabs[0].index, 1);
    assert!(!tabs[0].focused);

    assert_eq!(tabs[1].window, win2);
    assert_eq!(tabs[1].title, "nvim");
    assert_eq!(tabs[1].index, 2);
    assert!(!tabs[1].focused);

    assert_eq!(tabs[2].window, win3);
    assert_eq!(tabs[2].title, "Firefox");
    assert_eq!(tabs[2].index, 3);
    assert!(tabs[2].focused);

    // Status bar tab rects hit test vector populated
    let bar = wm.statusbar.as_ref().unwrap();
    assert_eq!(bar.tab_rects.len(), 3);
    assert_eq!(bar.tab_rects[0].0, win1);
    assert_eq!(bar.tab_rects[1].0, win2);
    assert_eq!(bar.tab_rects[2].0, win3);

    // Verify status bar pixels were written
    let img = wm.context.connection.get_image(
        x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
        bar_win,
        0,
        0,
        400,
        24,
        !0,
    ).unwrap().reply().unwrap();
    let has_colored_pixels = img.data.as_chunks::<4>().0.iter().any(|p| {
        let color = (p[2] as u32) << 16 | (p[1] as u32) << 8 | (p[0] as u32);
        color == 0x7aa2f7 || color == 0xc0caf5 || color == 0x15161e
    });
    assert!(has_colored_pixels, "Statusbar must contain rendered tab pills");
}

#[test]
fn test_tabbed_keyboard_navigation_and_reordering() {
    let server = match XephyrServer::start(172) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :172, skipping integration test");
            return;
        }
    };

    let client_ctx = X11Context::connect_to(Some(&server.display)).expect("Client connect failed");
    let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect failed");
    assert!(wm_ctx.become_wm().is_ok());

    let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WindowManager");
    wm.enable_statusbar(24).expect("enable_statusbar failed");
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();

    let root = client_ctx.root_window;
    let win1 = create_client_window(&client_ctx.connection, root, "term");
    let win2 = create_client_window(&client_ctx.connection, root, "nvim");
    let win3 = create_client_window(&client_ctx.connection, root, "firefox");

    wm.manage_window(win1).unwrap();
    wm.manage_window(win2).unwrap();
    wm.manage_window(win3).unwrap();

    // Currently focused is win3 (index 2)
    assert_eq!(wm.focused_window(), Some(win3));

    // 1. FocusNextTab wraps around to win1
    wm.execute_action(Action::FocusNextTab).unwrap();
    assert_eq!(wm.focused_window(), Some(win1));
    assert!(wm.client_state.get_client(win1).unwrap().mapped);
    assert!(!wm.client_state.get_client(win3).unwrap().mapped);

    // 2. FocusNextTab moves to win2
    wm.execute_action(Action::FocusNextTab).unwrap();
    assert_eq!(wm.focused_window(), Some(win2));
    assert!(wm.client_state.get_client(win2).unwrap().mapped);
    assert!(!wm.client_state.get_client(win1).unwrap().mapped);

    // 3. FocusPrevTab moves back to win1
    wm.execute_action(Action::FocusPrevTab).unwrap();
    assert_eq!(wm.focused_window(), Some(win1));

    // 4. FocusPrevTab wraps around to win3
    wm.execute_action(Action::FocusPrevTab).unwrap();
    assert_eq!(wm.focused_window(), Some(win3));

    // 5. SelectTab direct selection
    wm.execute_action(Action::SelectTab(2)).unwrap();
    assert_eq!(wm.focused_window(), Some(win2));

    // 6. Tab reordering: move win2 left
    // ws.clients order before: [win1, win2, win3]
    assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);
    wm.execute_action(Action::MoveTabLeft).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win2, win1, win3]);

    // Move win2 right twice -> [win1, win2, win3] -> [win1, win3, win2]
    wm.execute_action(Action::MoveTabRight).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);
    wm.execute_action(Action::MoveTabRight).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win3, win2]);
}

#[test]
fn test_tabbed_layout_switching_preserves_windows() {
    let server = match XephyrServer::start(173) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :173, skipping integration test");
            return;
        }
    };

    let client_ctx = X11Context::connect_to(Some(&server.display)).expect("Client connect failed");
    let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect failed");
    assert!(wm_ctx.become_wm().is_ok());

    let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WindowManager");
    wm.enable_statusbar(24).expect("enable_statusbar failed");

    // Start in MasterStack
    wm.execute_action(Action::SetLayout(LayoutType::MasterStack)).unwrap();

    let root = client_ctx.root_window;
    let win1 = create_client_window(&client_ctx.connection, root, "A");
    let win2 = create_client_window(&client_ctx.connection, root, "B");
    let win3 = create_client_window(&client_ctx.connection, root, "C");

    wm.manage_window(win1).unwrap();
    wm.manage_window(win2).unwrap();
    wm.manage_window(win3).unwrap();

    // In MasterStack: all 3 windows are mapped and visible
    assert!(wm.client_state.get_client(win1).unwrap().mapped);
    assert!(wm.client_state.get_client(win2).unwrap().mapped);
    assert!(wm.client_state.get_client(win3).unwrap().mapped);

    // Switch to Tabbed
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().layout, LayoutType::Tabbed);

    // In Tabbed: only focused window (win3) is mapped
    assert!(!wm.client_state.get_client(win1).unwrap().mapped);
    assert!(!wm.client_state.get_client(win2).unwrap().mapped);
    assert!(wm.client_state.get_client(win3).unwrap().mapped);

    // Switch to Bsp
    wm.execute_action(Action::SetLayout(LayoutType::Bsp)).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().layout, LayoutType::Bsp);

    // In Bsp: all 3 windows are mapped and visible again
    assert!(wm.client_state.get_client(win1).unwrap().mapped);
    assert!(wm.client_state.get_client(win2).unwrap().mapped);
    assert!(wm.client_state.get_client(win3).unwrap().mapped);

    // Switch back to Tabbed: original order preserved
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();
    assert_eq!(wm.workspace_manager.get(1).unwrap().clients, vec![win1, win2, win3]);
    let tabs = wm.get_workspace_tabs(1);
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[0].window, win1);
    assert_eq!(tabs[1].window, win2);
    assert_eq!(tabs[2].window, win3);
}

#[test]
fn test_tabbed_mouse_interaction_clicks_and_wheel() {
    let server = match XephyrServer::start(174) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :174, skipping integration test");
            return;
        }
    };

    let client_ctx = X11Context::connect_to(Some(&server.display)).expect("Client connect failed");
    let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect failed");
    assert!(wm_ctx.become_wm().is_ok());

    let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WindowManager");
    let bar_win = wm.enable_statusbar(24).expect("enable_statusbar failed");
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();

    let root = client_ctx.root_window;
    let win1 = create_client_window(&client_ctx.connection, root, "first");
    let win2 = create_client_window(&client_ctx.connection, root, "second");
    let win3 = create_client_window(&client_ctx.connection, root, "third");

    wm.manage_window(win1).unwrap();
    wm.manage_window(win2).unwrap();
    wm.manage_window(win3).unwrap();

    // Initially win3 is focused
    assert_eq!(wm.focused_window(), Some(win3));

    // Get tab rect for win1
    let tab1_rect = wm.statusbar.as_ref().unwrap().tab_rects[0].1;
    let click_x = tab1_rect.x as i16 + (tab1_rect.width as i16 / 2);
    let click_y = tab1_rect.y as i16 + (tab1_rect.height as i16 / 2);

    // 1. Left Click (detail = 1) on Tab 1 -> Focuses win1
    let click_ev = ButtonPressEvent {
        response_type: 4,
        detail: 1,
        sequence: 0,
        time: 100,
        root,
        event: bar_win,
        child: 0,
        root_x: click_x,
        root_y: click_y,
        event_x: click_x,
        event_y: click_y,
        state: 0u16.into(),
        same_screen: true,
    };
    wm.handle_button_press(&click_ev).unwrap();
    assert_eq!(wm.focused_window(), Some(win1));
    assert!(wm.client_state.get_client(win1).unwrap().mapped);
    assert!(!wm.client_state.get_client(win3).unwrap().mapped);

    // 2. Mouse wheel scroll down (detail = 5) over tab region -> Focus next tab (win2)
    let wheel_down_ev = ButtonPressEvent {
        response_type: 4,
        detail: 5,
        sequence: 0,
        time: 200,
        root,
        event: bar_win,
        child: 0,
        root_x: click_x,
        root_y: click_y,
        event_x: click_x,
        event_y: click_y,
        state: 0u16.into(),
        same_screen: true,
    };
    wm.handle_button_press(&wheel_down_ev).unwrap();
    assert_eq!(wm.focused_window(), Some(win2));

    // 3. Mouse wheel scroll up (detail = 4) over tab region -> Focus prev tab (win1)
    let wheel_up_ev = ButtonPressEvent {
        response_type: 4,
        detail: 4,
        sequence: 0,
        time: 300,
        root,
        event: bar_win,
        child: 0,
        root_x: click_x,
        root_y: click_y,
        event_x: click_x,
        event_y: click_y,
        state: 0u16.into(),
        same_screen: true,
    };
    wm.handle_button_press(&wheel_up_ev).unwrap();
    assert_eq!(wm.focused_window(), Some(win1));

    // 4. Middle Click (detail = 2) on Tab 3 -> Closes/kills win3
    let tab3_rect = wm.statusbar.as_ref().unwrap().tab_rects[2].1;
    let click3_x = tab3_rect.x as i16 + (tab3_rect.width as i16 / 2);
    let click3_y = tab3_rect.y as i16 + (tab3_rect.height as i16 / 2);

    let middle_click_ev = ButtonPressEvent {
        response_type: 4,
        detail: 2,
        sequence: 0,
        time: 400,
        root,
        event: bar_win,
        child: 0,
        root_x: click3_x,
        root_y: click3_y,
        event_x: click3_x,
        event_y: click3_y,
        state: 0u16.into(),
        same_screen: true,
    };
    wm.handle_button_press(&middle_click_ev).unwrap();
    // Simulate DestroyNotify from X server when client window closes
    wm.handle_destroy_notify(win3).unwrap();

    assert!(!wm.client_state.contains(win3));
    let tabs = wm.get_workspace_tabs(1);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0].window, win1);
    assert_eq!(tabs[1].window, win2);
}

#[test]
fn test_tabbed_dynamic_title_update() {
    let server = match XephyrServer::start(175) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :175, skipping integration test");
            return;
        }
    };

    let client_ctx = X11Context::connect_to(Some(&server.display)).expect("Client connect failed");
    let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect failed");
    assert!(wm_ctx.become_wm().is_ok());

    let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WindowManager");
    wm.enable_statusbar(24).expect("enable_statusbar failed");
    wm.execute_action(Action::SetLayout(LayoutType::Tabbed)).unwrap();

    let root = client_ctx.root_window;
    let win1 = create_client_window(&client_ctx.connection, root, "Initial Title");
    wm.manage_window(win1).unwrap();

    let tabs = wm.get_workspace_tabs(1);
    assert_eq!(tabs[0].title, "Initial Title");

    // Client dynamically changes title via WM_NAME
    client_ctx.connection.change_property8(
        PropMode::REPLACE,
        win1,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"New Dynamic Title",
    ).unwrap();
    client_ctx.flush().unwrap();

    let prop_ev = PropertyNotifyEvent {
        response_type: 28,
        sequence: 0,
        window: win1,
        atom: AtomEnum::WM_NAME.into(),
        time: 500,
        state: x11rb::protocol::xproto::Property::NEW_VALUE,
    };
    wm.handle_property_notify(&prop_ev).unwrap();

    let updated_tabs = wm.get_workspace_tabs(1);
    assert_eq!(updated_tabs[0].title, "New Dynamic Title");
}
