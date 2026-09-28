use std::process::{Child, Command};
use tilo::keybind::MOD_SUPER;
use tilo::mouse::{
    default_mouse_bindings, handle_button_press, MouseAction, MouseActionType,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ButtonPressEvent, ButtonReleaseEvent, ConnectionExt as _, CreateWindowAux, EventMask,
    KeyButMask, Motion, MotionNotifyEvent, WindowClass,
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
fn test_mouse_bindings_and_actions() {
    let bindings = default_mouse_bindings(MOD_SUPER);
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].button, 1);
    assert_eq!(bindings[0].action_type, MouseActionType::Move);
    assert_eq!(bindings[1].button, 3);
    assert_eq!(bindings[1].action_type, MouseActionType::Resize);

    let mut action = MouseAction::None;
    assert!(!action.is_active());
    assert_eq!(action.window(), None);

    let ev = ButtonPressEvent {
        response_type: 4,
        detail: 1,
        sequence: 1,
        time: 0,
        root: 1,
        event: 100,
        child: 200,
        root_x: 350,
        root_y: 250,
        event_x: 50,
        event_y: 50,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    handle_button_press(&ev, &mut action).unwrap();
    assert!(action.is_active());
    assert_eq!(action.window(), Some(200));
    assert_eq!(
        action,
        MouseAction::Moving {
            window: 200,
            start_x: 350,
            start_y: 250,
            start_win_x: 300,
            start_win_y: 200,
        }
    );
}

#[test]
fn test_mouse_interaction_in_xephyr() {
    let xephyr = match XephyrServer::start(234) {
        Some(s) => s,
        None => {
            eprintln!("Skipping test_mouse_interaction_in_xephyr: Xephyr not available");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(&xephyr.display)).expect("Failed to connect to Xephyr");
    ctx.become_wm().expect("Failed to become WM in Xephyr");

    let mut wm = WindowManager::new(ctx).expect("Failed to initialize WindowManager");

    // 1. Create and manage a client window
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

    wm.manage_window(client_win).unwrap();
    assert!(wm.client_state.contains(client_win));

    // 2. Simulate Mod + Left Click (Button 1) on window to initiate Move
    let press_move = ButtonPressEvent {
        response_type: 4,
        detail: 1, // Left click
        sequence: 1,
        time: 0,
        root: wm.context.root_window,
        event: client_win,
        child: 0,
        root_x: 200,
        root_y: 200,
        event_x: 50,
        event_y: 50,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    wm.handle_button_press(&press_move).unwrap();
    assert!(wm.mouse_action.is_active());
    match wm.mouse_action {
        MouseAction::Moving { window, .. } => assert_eq!(window, client_win),
        _ => panic!("Expected MouseAction::Moving"),
    }

    let client = wm.client_state.get_client(client_win).unwrap();
    assert!(client.floating, "Window must become floating when dragged");

    // 3. Simulate pointer motion while moving (drag by +100, +50)
    let motion_move = MotionNotifyEvent {
        response_type: 6,
        detail: Motion::NORMAL,
        sequence: 2,
        time: 10,
        root: wm.context.root_window,
        event: wm.context.root_window,
        child: client_win,
        root_x: 300,
        root_y: 250,
        event_x: 300,
        event_y: 250,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    wm.handle_motion_notify(&motion_move).unwrap();
    let geom = wm.client_state.get_client(client_win).unwrap().floating_geometry.unwrap();
    // Verify coordinates updated
    assert!(geom.x > 0);
    assert!(geom.y > 0);

    // 4. Release mouse button to end Move
    let release = ButtonReleaseEvent {
        response_type: 5,
        detail: 1,
        sequence: 3,
        time: 20,
        root: wm.context.root_window,
        event: wm.context.root_window,
        child: 0,
        root_x: 300,
        root_y: 250,
        event_x: 300,
        event_y: 250,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    wm.handle_button_release(&release).unwrap();
    assert!(!wm.mouse_action.is_active(), "MouseAction should reset to None on ButtonRelease");

    // 5. Simulate Mod + Right Click (Button 3) on window to initiate Resize
    let press_resize = ButtonPressEvent {
        response_type: 4,
        detail: 3, // Right click
        sequence: 4,
        time: 30,
        root: wm.context.root_window,
        event: client_win,
        child: 0,
        root_x: 300,
        root_y: 250,
        event_x: 100,
        event_y: 100,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    wm.handle_button_press(&press_resize).unwrap();
    assert!(wm.mouse_action.is_active());
    match wm.mouse_action {
        MouseAction::Resizing { window, .. } => assert_eq!(window, client_win),
        _ => panic!("Expected MouseAction::Resizing"),
    }

    // 6. Simulate pointer motion while resizing (expand by +80, +60)
    let motion_resize = MotionNotifyEvent {
        response_type: 6,
        detail: Motion::NORMAL,
        sequence: 5,
        time: 40,
        root: wm.context.root_window,
        event: wm.context.root_window,
        child: client_win,
        root_x: 380,
        root_y: 310,
        event_x: 380,
        event_y: 310,
        state: KeyButMask::from(MOD_SUPER),
        same_screen: true,
    };

    wm.handle_motion_notify(&motion_resize).unwrap();
    let geom_after = wm.client_state.get_client(client_win).unwrap().floating_geometry.unwrap();
    assert!(geom_after.width >= 400);
    assert!(geom_after.height >= 300);

    // 7. Release mouse button to end Resize
    wm.handle_button_release(&release).unwrap();
    assert!(!wm.mouse_action.is_active());
}
