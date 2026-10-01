use std::process::{Child, Command};
use tilo::keybind::Action;
use tilo::layout::{
    apply_layout, calculate_master_stack, LayoutConfig, LayoutType, Rect,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, WindowClass,
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

#[test]
fn test_master_stack_layout_math() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let config = LayoutConfig {
        master_ratio: 0.55,
        inner_gap: 10,
        outer_gap: 15,
        border_width: 2,
    };

    // 0 windows
    assert!(calculate_master_stack(&[], &screen, &config).is_empty());

    // 1 window
    let r1 = calculate_master_stack(&[1], &screen, &config);
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].x, 15);
    assert_eq!(r1[0].y, 15);
    assert_eq!(r1[0].width, 1920 - 30 - 4);
    assert_eq!(r1[0].height, 1080 - 30 - 4);

    // 2 windows
    let r2 = calculate_master_stack(&[1, 2], &screen, &config);
    assert_eq!(r2.len(), 2);
    assert!(!r2[0].overlaps(&r2[1]));

    // 4 windows
    let r4 = calculate_master_stack(&[1, 2, 3, 4], &screen, &config);
    assert_eq!(r4.len(), 4);
    for i in 0..r4.len() {
        for j in (i + 1)..r4.len() {
            assert!(!r4[i].overlaps(&r4[j]), "Rect {} and {} overlap", i, j);
        }
    }
}

#[test]
fn test_tiling_live_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(90);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init failed");
        wm.layout_config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Helper to spawn a client window
        let create_win = |conn: &x11rb::rust_connection::RustConnection| -> u32 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0,
                0,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .unwrap();
            conn.flush().unwrap();
            conn.sync().unwrap();
            win
        };

        // Window 1: Becomes single full tiled window
        let win1 = create_win(&client_conn);
        assert!(wm.manage_window(win1).unwrap());

        let g1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        // Screen is 1000x800. Outer gap 10, border 2.
        // Expected width: 1000 - 2*10 - 2*2 = 976
        // Expected height: 800 - 2*10 - 2*2 = 776
        assert_eq!(g1.x, 10);
        assert_eq!(g1.y, 10);
        assert_eq!(g1.width, 976);
        assert_eq!(g1.height, 776);

        // Window 2: Window 1 becomes Master, Window 2 becomes Stack
        let win2 = create_win(&client_conn);
        assert!(wm.manage_window(win2).unwrap());

        let g1_two = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_two = client_conn.get_geometry(win2).unwrap().reply().unwrap();

        // Win1 (master) should be on left
        assert_eq!(g1_two.x, 10);
        // Win2 (stack) should be on right
        assert!(g2_two.x > g1_two.x + g1_two.width as i16);
        // Neither overlaps
        assert!(g1_two.x + g1_two.width as i16 <= g2_two.x);

        // Window 3: Window 1 stays Master, Window 2 and 3 share Stack vertically
        let win3 = create_win(&client_conn);
        assert!(wm.manage_window(win3).unwrap());

        let g2_three = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_three = client_conn.get_geometry(win3).unwrap().reply().unwrap();

        // Win2 and Win3 are in stack column (same x)
        assert_eq!(g2_three.x, g3_three.x);
        // Win3 is below Win2 (g3.y > g2.y)
        assert!(g3_three.y > g2_three.y);
        assert!(g2_three.y + g2_three.height as i16 <= g3_three.y);

        // Close Window 3: Win2 re-expands to full stack height
        wm.unmanage_window(win3);
        let g2_after_unmanage = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(g2_after_unmanage.height > g2_three.height);

        // Close Window 2: Win1 re-expands to single window full screen
        wm.unmanage_window(win2);
        let g1_after_unmanage = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(g1_after_unmanage.width, 976);
        assert_eq!(g1_after_unmanage.height, 776);

        wm.stop();
    }
}

#[test]
fn test_layout_switching_and_cycling_live_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(91);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init failed");
        wm.layout_config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = |conn: &x11rb::rust_connection::RustConnection| -> u32 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0,
                0,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .unwrap();
            conn.flush().unwrap();
            conn.sync().unwrap();
            win
        };

        let win1 = create_win(&client_conn);
        let win2 = create_win(&client_conn);
        let win3 = create_win(&client_conn);

        assert!(wm.manage_window(win1).unwrap());
        assert!(wm.manage_window(win2).unwrap());
        assert!(wm.manage_window(win3).unwrap());

        // 1. Test Vertical Layout: all 3 windows placed side-by-side
        wm.execute_action(Action::SetLayout(LayoutType::Vertical)).unwrap();
        let g1_v = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_v = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_v = client_conn.get_geometry(win3).unwrap().reply().unwrap();

        assert_eq!(g1_v.y, 10);
        assert_eq!(g2_v.y, 10);
        assert_eq!(g3_v.y, 10);
        assert_eq!(g1_v.height, 776);
        assert_eq!(g2_v.height, 776);
        assert_eq!(g3_v.height, 776);
        assert!(g1_v.x < g2_v.x);
        assert!(g2_v.x < g3_v.x);
        assert!(g1_v.x + g1_v.width as i16 <= g2_v.x);
        assert!(g2_v.x + g2_v.width as i16 <= g3_v.x);

        // 2. Test Horizontal Layout: all 3 windows stacked top-to-bottom
        wm.execute_action(Action::SetLayout(LayoutType::Horizontal)).unwrap();
        let g1_h = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_h = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_h = client_conn.get_geometry(win3).unwrap().reply().unwrap();

        assert_eq!(g1_h.x, 10);
        assert_eq!(g2_h.x, 10);
        assert_eq!(g3_h.x, 10);
        assert_eq!(g1_h.width, 976);
        assert_eq!(g2_h.width, 976);
        assert_eq!(g3_h.width, 976);
        assert!(g1_h.y < g2_h.y);
        assert!(g2_h.y < g3_h.y);
        assert!(g1_h.y + g1_h.height as i16 <= g2_h.y);
        assert!(g2_h.y + g2_h.height as i16 <= g3_h.y);

        // 3. Test Monocle Layout: all windows take full screen
        wm.execute_action(Action::SetLayout(LayoutType::Monocle)).unwrap();
        let g1_m = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_m = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_m = client_conn.get_geometry(win3).unwrap().reply().unwrap();

        assert_eq!(g1_m.x, 10);
        assert_eq!(g1_m.y, 10);
        assert_eq!(g1_m.width, 976);
        assert_eq!(g1_m.height, 776);
        assert_eq!(g2_m.x, 10);
        assert_eq!(g2_m.y, 10);
        assert_eq!(g2_m.width, 976);
        assert_eq!(g2_m.height, 776);
        assert_eq!(g3_m.x, 10);
        assert_eq!(g3_m.y, 10);
        assert_eq!(g3_m.width, 976);
        assert_eq!(g3_m.height, 776);

        // 4. Test cycling layout: Monocle -> Bsp -> Grid -> MasterStack -> Vertical
        wm.execute_action(Action::CycleLayout).unwrap();
        let current_ws = wm.workspace_manager.current_id();
        assert_eq!(wm.workspace_manager.get(current_ws).unwrap().layout, LayoutType::Bsp);

        wm.execute_action(Action::CycleLayout).unwrap();
        assert_eq!(wm.workspace_manager.get(current_ws).unwrap().layout, LayoutType::Grid);

        // Windows should tile in a grid
        let g1_grid = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_grid = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_grid = client_conn.get_geometry(win3).unwrap().reply().unwrap();
        // Row 0 has 2 windows, Row 1 has 1 window
        assert_eq!(g1_grid.y, g2_grid.y);
        assert!(g1_grid.x < g2_grid.x);
        assert!(g3_grid.y > g1_grid.y);

        wm.execute_action(Action::CycleLayout).unwrap();
        assert_eq!(wm.workspace_manager.get(current_ws).unwrap().layout, LayoutType::MasterStack);

        wm.execute_action(Action::CycleLayout).unwrap();
        assert_eq!(wm.workspace_manager.get(current_ws).unwrap().layout, LayoutType::Vertical);

        // 5. Test apply_layout standalone function from Phase 27
        let area = Rect::new(0, 0, 1000, 800);
        assert!(apply_layout(LayoutType::Monocle, &[win1, win2], &area, 0.5, &client_conn).is_ok());

        wm.stop();
    }
}

#[test]
fn test_bsp_four_windows_diagram() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let config = LayoutConfig {
        master_ratio: 0.50,
        inner_gap: 10,
        outer_gap: 10,
        border_width: 2,
    };

    println!("--- Case 1: Sequential tree.insert (1 -> 2 -> 3 -> 4) ---");
    let mut tree1 = tilo::layout::BspTree::new();
    tree1.insert(None, 1, &screen, config.inner_gap);
    tree1.insert(Some(1), 2, &screen, config.inner_gap);
    tree1.insert(Some(2), 3, &screen, config.inner_gap);
    tree1.insert(Some(3), 4, &screen, config.inner_gap);
    for (w, r) in tree1.calculate_rects(&screen, &config) {
        println!("Window {}: x={}, y={}, w={}, h={}", w, r.x, r.y, r.width, r.height);
    }

    println!("--- Case 2: calculate_layout(LayoutType::Bsp, &[1, 2, 3, 4]) ---");
    let rects2 = tilo::layout::calculate_layout(LayoutType::Bsp, &[1, 2, 3, 4], &screen, &config);
    for (i, r) in rects2.iter().enumerate() {
        println!("Window {}: x={}, y={}, w={}, h={}", i + 1, r.x, r.y, r.width, r.height);
    }

    println!("--- Case 3: sync_clients on empty tree with focused = Some(4) ---");
    let mut tree3 = tilo::layout::BspTree::new();
    tree3.sync_clients(&[1, 2, 3, 4], Some(4), &screen, config.inner_gap);
    for (w, r) in tree3.calculate_rects(&screen, &config) {
        println!("Window {}: x={}, y={}, w={}, h={}", w, r.x, r.y, r.width, r.height);
    }

    println!("--- Case 4: sync_clients on empty tree with focused = Some(1) ---");
    let mut tree4 = tilo::layout::BspTree::new();
    tree4.sync_clients(&[1, 2, 3, 4], Some(1), &screen, config.inner_gap);
    for (w, r) in tree4.calculate_rects(&screen, &config) {
        println!("Window {}: x={}, y={}, w={}, h={}", w, r.x, r.y, r.width, r.height);
    }
}

#[test]
fn test_bsp_live_four_windows() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }
    let xephyr = XephyrServer::start(152);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());
        let mut wm = WindowManager::new(wm_ctx).expect("WM init");
        wm.layout_config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };
        wm.execute_action(Action::SetLayout(LayoutType::Bsp)).unwrap();

        let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).expect("Client connect");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = |conn: &x11rb::rust_connection::RustConnection| -> u32 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0, 0, 100, 100, 0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            ).unwrap();
            conn.flush().unwrap();
            conn.sync().unwrap();
            win
        };

        let w1 = create_win(&client_conn);
        wm.manage_window(w1).unwrap();
        let w2 = create_win(&client_conn);
        wm.manage_window(w2).unwrap();
        let w3 = create_win(&client_conn);
        wm.manage_window(w3).unwrap();
        let w4 = create_win(&client_conn);
        wm.manage_window(w4).unwrap();

        let g1 = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2 = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3 = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        let g4 = client_conn.get_geometry(w4).unwrap().reply().unwrap();

        println!("--- Live WindowManager manage_window (w1, w2, w3, w4) ---");
        println!("w1 (A): x={}, y={}, w={}, h={}", g1.x, g1.y, g1.width, g1.height);
        println!("w2 (B): x={}, y={}, w={}, h={}", g2.x, g2.y, g2.width, g2.height);
        println!("w3 (C): x={}, y={}, w={}, h={}", g3.x, g3.y, g3.width, g3.height);
        println!("w4 (D): x={}, y={}, w={}, h={}", g4.x, g4.y, g4.width, g4.height);

        wm.stop();
    }
}

#[test]
fn test_all_layouts_and_floating_4_directional_resizing_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }
    let xephyr = XephyrServer::start(155);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display)).expect("WM connect");
        assert!(wm_ctx.become_wm().is_ok());
        let mut wm = WindowManager::new(wm_ctx).expect("WM init");
        wm.layout_config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        let (client_conn, screen_num) = x11rb::connect(Some(&server.display)).expect("Client connect");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = |conn: &x11rb::rust_connection::RustConnection| -> u32 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0, 0, 100, 100, 0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            ).unwrap();
            conn.flush().unwrap();
            conn.sync().unwrap();
            win
        };

        let w1 = create_win(&client_conn);
        let w2 = create_win(&client_conn);
        let w3 = create_win(&client_conn);

        wm.manage_window(w1).unwrap();
        wm.manage_window(w2).unwrap();
        wm.manage_window(w3).unwrap();

        // --- 1. Master/Stack layout testing ---
        wm.execute_action(Action::SetLayout(LayoutType::MasterStack)).unwrap();
        let g1_init = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_init = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_init = client_conn.get_geometry(w3).unwrap().reply().unwrap();

        // In Master/Stack: w2 is top stack, w3 is bottom stack
        assert_eq!(g2_init.x, g3_init.x);
        assert!(g3_init.y > g2_init.y);

        // Resize Up/Down on stack window w2: expands w2 height downward!
        wm.focus_client(w2).unwrap();
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Down)).unwrap();
        let g1_down = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_down = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_down = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        assert!(g2_down.height > g2_init.height, "w2 height should increase on Down resize");
        assert!(g3_down.height < g3_init.height, "w3 height should decrease on Down resize");
        assert_eq!(g1_down.width, g1_init.width, "Master w1 width must not change when stack window w2 is resized");
        assert_eq!(g1_down.height, g1_init.height, "Master w1 height must not change when stack window w2 is resized");

        // Resize Left/Right on master window w1: expands master width!
        wm.focus_client(w1).unwrap();
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Right)).unwrap();
        let g1_right = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        assert!(g1_right.width > g1_init.width, "w1 width should increase on Right resize");

        // --- 2. Vertical layout testing ---
        wm.execute_action(Action::SetLayout(LayoutType::Vertical)).unwrap();
        let g1_v_init = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_v_init = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_v_init = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        assert_eq!(g1_v_init.y, g2_v_init.y);

        // Resize Right on column w1: expands w1 column width!
        wm.focus_client(w1).unwrap();
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Right)).unwrap();
        let g1_v_right = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_v_right = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_v_right = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        assert!(g1_v_right.width > g1_v_init.width, "w1 column width should increase on Right resize in Vertical layout");
        assert!(g2_v_right.width < g2_v_init.width, "Adjacent w2 column width should decrease on Right resize in Vertical layout");
        assert!((g3_v_right.width as i32 - g3_v_init.width as i32).abs() <= 1, "Non-adjacent column w3 width must remain unchanged");

        // --- 3. Horizontal layout testing ---
        wm.execute_action(Action::SetLayout(LayoutType::Horizontal)).unwrap();
        let g1_h_init = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_h_init = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_h_init = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        assert_eq!(g1_h_init.x, g2_h_init.x);

        // Resize Down on row w1: expands w1 row height!
        wm.focus_client(w1).unwrap();
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Down)).unwrap();
        let g1_h_down = client_conn.get_geometry(w1).unwrap().reply().unwrap();
        let g2_h_down = client_conn.get_geometry(w2).unwrap().reply().unwrap();
        let g3_h_down = client_conn.get_geometry(w3).unwrap().reply().unwrap();
        assert!(g1_h_down.height > g1_h_init.height, "w1 row height should increase on Down resize in Horizontal layout");
        assert!(g2_h_down.height < g2_h_init.height, "Adjacent w2 row height should decrease on Down resize in Horizontal layout");
        assert!((g3_h_down.height as i32 - g3_h_init.height as i32).abs() <= 1, "Non-adjacent row w3 height must remain unchanged");

        // --- 4. Floating window 4-directional keyboard resizing ---
        let w_float = create_win(&client_conn);
        wm.manage_window(w_float).unwrap();
        wm.toggle_floating(w_float).unwrap();
        wm.focus_client(w_float).unwrap();

        let g_float_init = client_conn.get_geometry(w_float).unwrap().reply().unwrap();

        // Resize Right expands width
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Right)).unwrap();
        let g_float_r = client_conn.get_geometry(w_float).unwrap().reply().unwrap();
        assert!(g_float_r.width > g_float_init.width, "Floating window width should grow on Right");

        // Resize Down expands height
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Down)).unwrap();
        let g_float_d = client_conn.get_geometry(w_float).unwrap().reply().unwrap();
        assert!(g_float_d.height > g_float_init.height, "Floating window height should grow on Down");

        // Resize Left shrinks width
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Left)).unwrap();
        let g_float_l = client_conn.get_geometry(w_float).unwrap().reply().unwrap();
        assert!(g_float_l.width < g_float_r.width, "Floating window width should shrink on Left");

        // Resize Up shrinks height
        wm.execute_action(Action::ResizeTile(tilo::focus::Direction::Up)).unwrap();
        let g_float_u = client_conn.get_geometry(w_float).unwrap().reply().unwrap();
        assert!(g_float_u.height < g_float_d.height, "Floating window height should shrink on Up");

        wm.stop();
    }
}


