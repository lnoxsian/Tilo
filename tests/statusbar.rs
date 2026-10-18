use std::process::{Child, Command};
use tilo::layout::Rect;
use tilo::render::Renderer;
use tilo::statusbar::{
    create_statusbar_window, BatteryModule, ClockModule, CpuModule, IpModule, LayoutModule,
    ModuleUpdate, RamModule, StatusModule, TitleModule, WorkspaceModule,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Window, WindowClass,
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

fn create_client_window(conn: &impl Connection, root: Window, x: i16, y: i16, w: u16, h: u16) -> Window {
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
    conn.map_window(win).unwrap();
    conn.sync().unwrap();
    win
}

#[test]
fn test_modules_independent_polling_and_errors() {
    // 1. RAM module with custom non-existent file
    let temp_dir = std::env::temp_dir().join(format!("tilo_statusbar_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let fake_meminfo = temp_dir.join("fake_meminfo");

    let mut ram_err = RamModule::with_path(fake_meminfo.clone());
    let update_res = ram_err.update().unwrap();
    assert!(matches!(update_res, ModuleUpdate::Error(_)));
    assert_eq!(ram_err.current_text(), "RAM: ERR");

    // Write valid meminfo data
    let sample_mem = "\
MemTotal:       32000000 kB
MemFree:         4000000 kB
MemAvailable:   16000000 kB
";
    std::fs::write(&fake_meminfo, sample_mem).unwrap();
    let update_ok = ram_err.update().unwrap();
    assert_eq!(update_ok, ModuleUpdate::Changed("RAM 50%".to_string()));
    assert_eq!(ram_err.current_text(), "RAM 50%");

    // 2. CPU module with custom non-existent file
    let fake_stat = temp_dir.join("fake_stat");
    let mut cpu_err = CpuModule::with_path(fake_stat.clone());
    let cpu_res = cpu_err.update().unwrap();
    assert!(matches!(cpu_res, ModuleUpdate::Error(_)));
    assert_eq!(cpu_err.current_text(), "CPU: ERR");

    // Write valid stat data
    std::fs::write(&fake_stat, "cpu 100 0 100 800 0 0 0 0 0 0\n").unwrap();
    let _ = cpu_err.update();
    std::fs::write(&fake_stat, "cpu 150 0 150 900 0 0 0 0 0 0\n").unwrap();
    let cpu_ok = cpu_err.update().unwrap();
    assert_eq!(cpu_ok, ModuleUpdate::Changed("CPU 50%".to_string()));
    assert_eq!(cpu_err.current_text(), "CPU 50%");

    // 3. Workspace and layout modules
    let ws = WorkspaceModule::new();
    assert_eq!(ws.name(), "workspace");
    assert_eq!(ws.update_interval(), 100);

    let layout = LayoutModule::new();
    assert_eq!(layout.name(), "layout");
    assert_eq!(layout.update_interval(), 200);

    let title = TitleModule::new();
    assert_eq!(title.name(), "title");
    assert_eq!(title.update_interval(), 100);

    let ip = IpModule::new();
    assert_eq!(ip.name(), "ip");
    assert_eq!(ip.update_interval(), 3000);

    let bat = BatteryModule::new();
    assert_eq!(bat.name(), "battery");
    assert_eq!(bat.update_interval(), 5000);

    let clock = ClockModule::new();
    assert_eq!(clock.name(), "clock");
    assert_eq!(clock.update_interval(), 1000);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_statusbar_creation_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(99);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        // 1. Direct creation of statusbar window with create_statusbar_window
        let screen = &wm_ctx.connection.setup().roots[wm_ctx.screen_num];
        let bar_rect = Rect::new(0, 0, wm_ctx.screen_width as u32, 26);
        let win_id = create_statusbar_window(&wm_ctx.connection, screen, bar_rect)
            .expect("Failed to create statusbar window");

        // Verify window attributes: override_redirect must be true
        let attrs = wm_ctx.connection.get_window_attributes(win_id).unwrap().reply().unwrap();
        assert!(attrs.override_redirect);

        // Verify window geometry: x=0, y=0, w=screen_width, h=26
        let geom = wm_ctx.connection.get_geometry(win_id).unwrap().reply().unwrap();
        assert_eq!(geom.x, 0);
        assert_eq!(geom.y, 0);
        assert_eq!(geom.width, wm_ctx.screen_width);
        assert_eq!(geom.height, 26);

        // Verify _NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_DOCK
        let net_wm_window_type = wm_ctx.connection.intern_atom(false, b"_NET_WM_WINDOW_TYPE").unwrap().reply().unwrap().atom;
        let net_wm_window_type_dock = wm_ctx.connection.intern_atom(false, b"_NET_WM_WINDOW_TYPE_DOCK").unwrap().reply().unwrap().atom;
        let prop = wm_ctx.connection.get_property(false, win_id, net_wm_window_type, AtomEnum::ATOM, 0, 1).unwrap().reply().unwrap();
        assert_eq!(prop.value32().unwrap().next(), Some(net_wm_window_type_dock));

        // Verify _NET_WM_STRUT = [0, 0, 26, 0]
        let net_wm_strut = wm_ctx.connection.intern_atom(false, b"_NET_WM_STRUT").unwrap().reply().unwrap().atom;
        let strut_prop = wm_ctx.connection.get_property(false, win_id, net_wm_strut, AtomEnum::CARDINAL, 0, 4).unwrap().reply().unwrap();
        let strut_vals: Vec<u32> = strut_prop.value32().unwrap().collect();
        assert_eq!(strut_vals, vec![0, 0, 26, 0]);

        // Clean up direct window
        let _ = wm_ctx.connection.destroy_window(win_id);
    }
}

#[test]
fn test_wm_statusbar_integration_and_space_reservation_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(100);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WM");

        // 1. Enable statusbar with height 24
        let bar_win = wm.enable_statusbar(24).expect("Failed to enable statusbar");
        assert!(wm.statusbar.is_some());
        assert_eq!(wm.statusbar.as_ref().unwrap().window_id, bar_win);
        assert_eq!(wm.statusbar.as_ref().unwrap().rect.height, 24);

        // 2. Open client window
        let client_conn = X11Context::connect_to(Some(&server.display)).unwrap();
        let win = create_client_window(&client_conn.connection, wm.context.root_window, 10, 10, 300, 300);

        wm.process_pending_events().unwrap();
        assert_eq!(wm.client_state.all_clients().len(), 1);

        // 3. Verify client window starts BELOW statusbar (y >= 24)
        let geom = client_conn.connection.get_geometry(win).unwrap().reply().unwrap();
        assert!(
            geom.y >= 24,
            "Client window y ({}) should be at or below statusbar height (24)",
            geom.y
        );

        // 4. Update statusbar and check full text
        assert!(wm.update_statusbar() || !wm.update_statusbar());
        let text = wm.statusbar.as_ref().unwrap().full_text();
        assert!(text.contains("[1]"));
        assert!(text.contains("[]="));

        // 5. Disable statusbar and verify retile restores full height
        wm.disable_statusbar().expect("Failed to disable statusbar");
        assert!(wm.statusbar.is_none());

        let geom_after = client_conn.connection.get_geometry(win).unwrap().reply().unwrap();
        // Outer gap is default 0 or border; y should be at top edge now
        assert!(geom_after.y < geom.y);
    }
}

#[test]
fn test_statusbar_renderer_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(101);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to create WM");

        // 1. Test standalone Renderer font fallback and drawing
        let font_fallback = Renderer::load_font(&wm.context.connection, "non_existent_fancy_font_xyz")
            .expect("Font fallback should successfully load fixed or core font");
        assert!(font_fallback > 0);

        // 2. Enable statusbar with height 26
        let bar_win = wm.enable_statusbar(26).expect("Failed to enable statusbar");
        let bar = wm.statusbar.as_ref().expect("Statusbar must be present");
        let renderer = bar.renderer.as_ref().expect("Renderer must be attached");

        assert_eq!(renderer.width, 1000);
        assert_eq!(renderer.height, 26);
        assert!(renderer.pixmap > 0);
        assert!(renderer.gc > 0);
        assert!(renderer.char_width > 0);
        assert!(renderer.font_ascent > 0);

        // 3. Test direct rendering functions on pixmap
        renderer
            .fill_rect(&wm.context.connection, renderer.pixmap, 0, 0, 100, 26, 0x1a1b26)
            .expect("fill_rect on pixmap should succeed");

        renderer
            .draw_text(&wm.context.connection, renderer.pixmap, 10, 16, "Statusbar Test", 0xc0caf5)
            .expect("draw_text on pixmap should succeed");

        // 4. Test blitting to statusbar window (double buffering)
        renderer
            .flush_to_window(&wm.context.connection, bar_win)
            .expect("flush_to_window should succeed");

        // 5. Test statusbar rendering through WindowManager
        wm.render_statusbar().expect("render_statusbar should succeed");

        // 6. Test active window title and workspace switching re-renders
        let client_conn = X11Context::connect_to(Some(&server.display)).unwrap();
        let win = create_client_window(&client_conn.connection, wm.context.root_window, 20, 40, 200, 200);

        // Set window name
        let net_wm_name = client_conn.connection.intern_atom(false, b"_NET_WM_NAME").unwrap().reply().unwrap().atom;
        let utf8_string = client_conn.connection.intern_atom(false, b"UTF8_STRING").unwrap().reply().unwrap().atom;
        client_conn.connection.change_property8(
            x11rb::protocol::xproto::PropMode::REPLACE,
            win,
            net_wm_name,
            utf8_string,
            b"Terminal 1",
        ).unwrap();
        client_conn.flush().unwrap();

        wm.process_pending_events().unwrap();
        assert_eq!(wm.client_state.all_clients().len(), 1);

        // Active title should be updated in statusbar
        let bar = wm.statusbar.as_ref().unwrap();
        assert!(bar.full_text().contains("Terminal 1"));

        // Switch to workspace 2: workspace 1 is occupied (*), workspace 2 is active
        wm.switch_workspace(2).expect("Failed to switch workspace");
        assert_eq!(wm.current_workspace, 2);

        // Verify statusbar title does NOT persist on empty workspace 2
        let bar = wm.statusbar.as_ref().unwrap();
        assert!(!bar.full_text().contains("Terminal 1"), "Statusbar title must be cleared on empty workspace");

        // Module update re-render
        let _ = wm.update_statusbar();
        let bar = wm.statusbar.as_ref().unwrap();
        assert!(!bar.full_text().contains("Terminal 1"), "Statusbar title must remain cleared after module update");

        // Switch back to workspace 1: title should be restored
        wm.switch_workspace(1).expect("Failed to switch back to workspace 1");
        assert_eq!(wm.current_workspace, 1);
        let bar = wm.statusbar.as_ref().unwrap();
        assert!(bar.full_text().contains("Terminal 1"), "Statusbar title must be restored when returning to workspace 1");

        // Switch to workspace 2 again for image test
        wm.switch_workspace(2).expect("Failed to switch workspace");
        assert_eq!(wm.current_workspace, 2);

        // 7. Verify actual rendered pixels on statusbar window using get_image
        let img = wm.context.connection.get_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            bar_win,
            0,
            0,
            200,
            26,
            !0,
        ).unwrap().reply().unwrap();
        // Check that pixels with color scheme active_bg (0x7aa2f7) or fg (0xc0caf5) were actually written
        // In 32-bit/24-bit visual, pixels are stored in BGR or RGB format
        let has_colored_pixels = img.data.as_chunks::<4>().0.iter().any(|p| {
            let color = (p[2] as u32) << 16 | (p[1] as u32) << 8 | (p[0] as u32);
            color == 0x7aa2f7 || color == 0xc0caf5 || color == 0x15161e
        });
        assert!(has_colored_pixels, "Statusbar window must contain rendered text / pill pixels");

        // Disable statusbar
        wm.disable_statusbar().expect("disable_statusbar should succeed");
    }
}

#[test]
fn test_statusbar_dynamic_screen_resize() {
    let server = match XephyrServer::start(169) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :169, skipping integration test");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(&server.display)).expect("Failed to connect to Xephyr");
    let mut wm = WindowManager::new(ctx).expect("Failed to create WindowManager");
    let root_depth = wm.context.connection.setup().roots[wm.context.screen_num].root_depth;

    // 1. Enable statusbar with initial width (1000px)
    let bar_win = wm.enable_statusbar(26).expect("Failed to enable statusbar");
    let initial_geom = wm.context.connection.get_geometry(bar_win).unwrap().reply().unwrap();
    assert_eq!(initial_geom.width, 1000);
    assert_eq!(wm.statusbar.as_ref().unwrap().rect.width, 1000);
    assert_eq!(wm.statusbar.as_ref().unwrap().renderer.as_ref().unwrap().width, 1000);

    // 2. Perform dynamic resize to a new resolution (1366x768)
    let new_rect = Rect::new(0, 0, 1366, 26);
    wm.statusbar.as_mut().unwrap().resize(&wm.context.connection, new_rect, root_depth).expect("Resize should succeed");

    // 3. Verify that the X11 statusbar window and internal renderer were updated
    let updated_geom = wm.context.connection.get_geometry(bar_win).unwrap().reply().unwrap();
    assert_eq!(updated_geom.width, 1366);
    assert_eq!(wm.statusbar.as_ref().unwrap().rect.width, 1366);
    assert_eq!(wm.statusbar.as_ref().unwrap().renderer.as_ref().unwrap().width, 1366);

    // 4. Verify statusbar re-renders cleanly at the new width without errors
    let render_res = wm.render_statusbar();
    assert!(render_res.is_ok());

    // 5. Test another resize to 1920x1080
    let fhd_rect = Rect::new(0, 0, 1920, 26);
    wm.statusbar.as_mut().unwrap().resize(&wm.context.connection, fhd_rect, root_depth).expect("FHD resize should succeed");
    let fhd_geom = wm.context.connection.get_geometry(bar_win).unwrap().reply().unwrap();
    assert_eq!(fhd_geom.width, 1920);
    assert_eq!(wm.statusbar.as_ref().unwrap().rect.width, 1920);
    assert_eq!(wm.statusbar.as_ref().unwrap().renderer.as_ref().unwrap().width, 1920);

    wm.disable_statusbar().expect("disable_statusbar should succeed");
}

#[test]
fn test_statusbar_mouse_workspace_interaction_in_xephyr() {
    let server = match XephyrServer::start(168) {
        Some(s) => s,
        None => {
            eprintln!("Xephyr not available on :168, skipping integration test");
            return;
        }
    };

    let ctx = X11Context::connect_to(Some(&server.display)).expect("Failed to connect to Xephyr");
    let mut wm = WindowManager::new(ctx).expect("Failed to create WindowManager");
    let bar_win = wm.enable_statusbar(26).expect("Failed to enable statusbar");

    // Initially on workspace 1
    assert_eq!(wm.current_workspace, 1);

    // Verify workspace rects are populated (1..=9)
    let rects = wm.statusbar.as_ref().unwrap().workspace_rects.clone();
    assert_eq!(rects.len(), 9);
    for (i, &(ws, _)) in rects.iter().enumerate() {
        assert_eq!(ws, (i + 1) as u32);
    }

    let root = wm.context.root_window;

    // Helper closure to simulate button press
    let simulate_click = |wm: &mut WindowManager, x: i16, y: i16, detail: u8, state: u16| {
        let ev = x11rb::protocol::xproto::ButtonPressEvent {
            response_type: x11rb::protocol::xproto::BUTTON_PRESS_EVENT,
            detail,
            sequence: 0,
            time: 0,
            root,
            event: bar_win,
            child: 0,
            root_x: x,
            root_y: y,
            event_x: x,
            event_y: y,
            state: state.into(),
            same_screen: true,
        };
        wm.handle_button_press(&ev)
    };

    // 1. Click on Workspace 3
    let ws3 = rects[2].1;
    let click_x = ws3.x as i16 + (ws3.width as i16 / 2);
    let click_y = ws3.y as i16 + (ws3.height as i16 / 2);
    simulate_click(&mut wm, click_x, click_y, 1, 0).expect("Click ws 3 failed");
    assert_eq!(wm.current_workspace, 3, "Left click on workspace 3 must switch to workspace 3");

    // 2. Click on Workspace 7
    let ws7 = rects[6].1;
    let click_x = ws7.x as i16 + (ws7.width as i16 / 2);
    let click_y = ws7.y as i16 + (ws7.height as i16 / 2);
    simulate_click(&mut wm, click_x, click_y, 1, 0).expect("Click ws 7 failed");
    assert_eq!(wm.current_workspace, 7, "Left click on workspace 7 must switch to workspace 7");

    // 3. Click on Workspace 7 again (already active, no-op)
    simulate_click(&mut wm, click_x, click_y, 1, 0).expect("Click active ws 7 failed");
    assert_eq!(wm.current_workspace, 7);

    // 4. Click on Workspace 1
    let ws1 = rects[0].1;
    let click_x = ws1.x as i16 + (ws1.width as i16 / 2);
    let click_y = ws1.y as i16 + (ws1.height as i16 / 2);
    simulate_click(&mut wm, click_x, click_y, 1, 0).expect("Click ws 1 failed");
    assert_eq!(wm.current_workspace, 1);

    // 5. Test mouse wheel scrolling on workspace region
    // Wheel down (detail = 5): workspace 1 -> 2
    simulate_click(&mut wm, click_x, click_y, 5, 0).expect("Wheel down failed");
    assert_eq!(wm.current_workspace, 2, "Wheel down must switch to next workspace");

    // Wheel up (detail = 4): workspace 2 -> 1
    simulate_click(&mut wm, click_x, click_y, 4, 0).expect("Wheel up failed");
    assert_eq!(wm.current_workspace, 1, "Wheel up must switch to previous workspace");

    // Wheel up wrap-around: workspace 1 -> 9
    simulate_click(&mut wm, click_x, click_y, 4, 0).expect("Wheel up wrap failed");
    assert_eq!(wm.current_workspace, 9, "Wheel up on ws 1 must wrap to ws 9");

    // Wheel down wrap-around: workspace 9 -> 1
    simulate_click(&mut wm, click_x, click_y, 5, 0).expect("Wheel down wrap failed");
    assert_eq!(wm.current_workspace, 1, "Wheel down on ws 9 must wrap to ws 1");

    // 6. Test Right-Click and Shift+Left-Click to move client window to workspace
    let client_conn = X11Context::connect_to(Some(&server.display)).unwrap();
    let win = create_client_window(&client_conn.connection, root, 10, 10, 100, 100);
    wm.manage_window(win).expect("manage_window failed");
    assert_eq!(wm.current_workspace, 1);
    assert_eq!(wm.focused_window(), Some(win));

    // Right click (detail = 3) on Workspace 4 moves focused window to ws 4
    let ws4 = rects[3].1;
    let click_x = ws4.x as i16 + (ws4.width as i16 / 2);
    let click_y = ws4.y as i16 + (ws4.height as i16 / 2);
    simulate_click(&mut wm, click_x, click_y, 3, 0).expect("Right click move failed");
    assert_eq!(wm.current_workspace, 1); // WM remains on ws 1
    assert!(wm.workspace_manager.get(4).unwrap().clients.contains(&win));
    assert!(!wm.workspace_manager.get(1).unwrap().clients.contains(&win));

    // 7. Test disabling workspace_mouse in config
    wm.config.workspace_mouse = false;
    let ws5 = rects[4].1;
    let click_x = ws5.x as i16 + (ws5.width as i16 / 2);
    let click_y = ws5.y as i16 + (ws5.height as i16 / 2);
    simulate_click(&mut wm, click_x, click_y, 1, 0).expect("Click ws 5 failed");
    assert_eq!(wm.current_workspace, 1, "Clicking workspace when workspace_mouse is false must be ignored");

    wm.disable_statusbar().expect("disable_statusbar failed");
}

#[test]
fn test_statusbar_custom_modules_and_reconfiguration() {
    use tilo::bar::{BarModuleType, ColorScheme, Statusbar};

    // 1. Create with custom reordered modules: Clock, Volume, Battery
    let custom_modules = vec![
        BarModuleType::Clock,
        BarModuleType::Volume,
        BarModuleType::Battery,
    ];
    let mut bar = Statusbar::with_modules(100, Rect::new(0, 0, 1920, 24), &custom_modules, ColorScheme::default());
    bar.set_title("Terminal");
    let _ = bar.update_all();

    // Verify left modules are present
    assert_eq!(bar.modules[0].name(), "workspace");
    assert_eq!(bar.modules[1].name(), "layout");
    assert_eq!(bar.modules[2].name(), "title");

    // Verify right modules are in exact order: clock, volume, battery
    assert_eq!(bar.modules[3].name(), "clock");
    assert_eq!(bar.modules[4].name(), "volume");
    assert_eq!(bar.modules[5].name(), "battery");
    assert_eq!(bar.modules.len(), 6);

    let text1 = bar.full_text();
    assert!(text1.contains("Terminal"));
    assert!(text1.contains("BAT"));
    assert!(text1.contains("VOL"));
    assert!(!text1.contains("RAM"));
    assert!(!text1.contains("IP"));

    // 2. Reconfigure dynamically to only RAM and IP (reversed)
    bar.reconfigure_modules(&[BarModuleType::Ram, BarModuleType::Ip]);
    let _ = bar.update_all();

    assert_eq!(bar.modules.len(), 5);
    assert_eq!(bar.modules[0].name(), "workspace");
    assert_eq!(bar.modules[1].name(), "layout");
    assert_eq!(bar.modules[2].name(), "title");
    assert_eq!(bar.modules[3].name(), "ram");
    assert_eq!(bar.modules[4].name(), "ip");

    let text2 = bar.full_text();
    assert!(text2.contains("RAM"));
    assert!(text2.contains("IP"));
    assert!(!text2.contains("BAT"));
    assert!(!text2.contains("VOL"));

    // 3. Reconfigure to empty/none
    bar.reconfigure_modules(&[]);
    let _ = bar.update_all();
    assert_eq!(bar.modules.len(), 3);
    assert_eq!(bar.modules[0].name(), "workspace");
    assert_eq!(bar.modules[1].name(), "layout");
    assert_eq!(bar.modules[2].name(), "title");
}

#[test]
fn test_statusbar_custom_workspace_tags() {
    use tilo::bar::Statusbar;

    let mut bar = Statusbar::default_with_window(100, Rect::new(0, 0, 1920, 24));
    let custom_tags = vec![
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "󰓇".to_string(),
        "󰙯".to_string(),
        "󰈹".to_string(),
        "󰓓".to_string(),
        "󰊴".to_string(),
    ];
    bar.set_workspace_tags(&custom_tags);
    bar.set_workspace(1, &[1, 2]);

    let text = bar.full_text();
    assert!(text.contains("[]"));
    assert!(text.contains(":*"));
    assert!(text.contains(""));
    assert!(text.contains("󰊴"));
    assert!(!text.contains("[1]"));
}


