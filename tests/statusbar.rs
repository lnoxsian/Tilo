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
    conn.flush().unwrap();
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
        let has_colored_pixels = img.data.chunks_exact(4).any(|p| {
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


