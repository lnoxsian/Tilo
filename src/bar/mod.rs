//! Built-in X11 Statusbar and module system.
//!
//! Implements Phase 17 of Plan.md:
//! - Creation of an `override_redirect` statusbar window positioned at the screen edge.
//! - Setting EWMH `_NET_WM_WINDOW_TYPE` to `_NET_WM_WINDOW_TYPE_DOCK`.
//! - Setting `_NET_WM_STRUT` and `_NET_WM_STRUT_PARTIAL` to reserve workarea space so tiled windows do not overlap.
//! - Modular statusbar architecture with independent polling intervals:
//!   - Workspace module (active and occupied workspaces)
//!   - Layout module (active layout indicator e.g. `[]=`, `><>`)
//!   - Window title module (active client title)
//!   - RAM module (reads `/proc/meminfo`)
//!   - CPU module (reads `/proc/stat`)
//!   - Clock module (system time)
pub mod modules;
pub mod tags;
pub mod tabs;

pub use modules::*;
pub use tags::*;
pub use tabs::*;

use crate::layout::Rect;
use crate::x11::X11Error;
use std::time::Instant;
use thiserror::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, Screen, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

/// Errors that can occur within the statusbar system.
#[derive(Debug, Error)]
pub enum StatusbarError {
    /// X11 protocol or connection error.
    #[error("X11 error in statusbar: {0}")]
    X11(#[from] X11Error),

    /// System I/O error (e.g. reading /proc files).
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to load font.
    #[error("Failed to load font: {0}")]
    FontLoadError(String),

    /// Process spawn error.
    #[error("Spawn error: {0}")]
    SpawnError(String),
}

impl From<x11rb::errors::ConnectionError> for StatusbarError {
    fn from(e: x11rb::errors::ConnectionError) -> Self {
        StatusbarError::X11(X11Error::Connection(e))
    }
}

impl From<x11rb::errors::ReplyError> for StatusbarError {
    fn from(e: x11rb::errors::ReplyError) -> Self {
        StatusbarError::X11(X11Error::Reply(e))
    }
}

impl From<x11rb::errors::ReplyOrIdError> for StatusbarError {
    fn from(e: x11rb::errors::ReplyOrIdError) -> Self {
        StatusbarError::X11(X11Error::ReplyOrId(e))
    }
}

/// Type alias for statusbar errors matching Plan.md specifications.
pub type TiloError = StatusbarError;

/// Active foreground and background colors for the statusbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorScheme {
    /// Statusbar background color (0xRRGGBB).
    pub bg: u32,
    /// Default text foreground color (0xRRGGBB).
    pub fg: u32,
    /// Background color for the currently active workspace.
    pub active_bg: u32,
    /// Foreground text color for the currently active workspace.
    pub active_fg: u32,
    /// Urgent workspace background.
    pub urgent_bg: u32,
    /// Urgent workspace foreground.
    pub urgent_fg: u32,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            bg: 0x1a1b26,
            fg: 0xc0caf5,
            active_bg: 0x7aa2f7,
            active_fg: 0x15161e,
            urgent_bg: 0xf7768e,
            urgent_fg: 0x15161e,
        }
    }
}

/// Core struct representing the built-in statusbar.
#[derive(Debug)]
pub struct Statusbar {
    /// The X11 window ID for the statusbar window.
    pub window_id: u32,
    /// The geometric bounds of the statusbar.
    pub rect: Rect,
    /// The collection of active modules to display.
    pub modules: Vec<Box<dyn StatusModule>>,
    /// Active foreground and background colors.
    pub color_scheme: ColorScheme,
    /// Optional graphical text and shape renderer.
    pub renderer: Option<crate::render::Renderer>,
    /// Hit-test geometries for clickable tabs in tabbed layout: (client_window, tab_rect).
    pub tab_rects: Vec<(u32, Rect)>,
}

impl Statusbar {
    /// Creates a new `Statusbar` instance with custom modules and color scheme.
    pub fn new(
        window_id: u32,
        rect: Rect,
        modules: Vec<Box<dyn StatusModule>>,
        color_scheme: ColorScheme,
    ) -> Self {
        Self {
            window_id,
            rect,
            modules,
            color_scheme,
            renderer: None,
            tab_rects: Vec::new(),
        }
    }

    /// Creates a default `Statusbar` equipped with standard modules:
    /// Workspace, Layout, Window Title, IP, RAM, CPU, Battery, and Clock.
    pub fn default_with_window(window_id: u32, rect: Rect) -> Self {
        let modules: Vec<Box<dyn StatusModule>> = vec![
            Box::new(WorkspaceModule::new()),
            Box::new(LayoutModule::new()),
            Box::new(TitleModule::new()),
            Box::new(IpModule::new()),
            Box::new(RamModule::new()),
            Box::new(CpuModule::new()),
            Box::new(BatteryModule::new()),
            Box::new(ClockModule::new()),
        ];
        Self::new(window_id, rect, modules, ColorScheme::default())
    }

    /// Polls all statusbar modules whose update intervals have elapsed.
    ///
    /// # Returns
    /// `true` if any module updated its text or entered an error state.
    pub fn update_all(&mut self) -> bool {
        let now = Instant::now();
        let mut any_changed = false;
        for module in &mut self.modules {
            if let Some(last) = module.last_updated()
                && now.duration_since(last).as_millis() < module.update_interval() as u128 {
                    continue;
                }
            match module.update() {
                Ok(ModuleUpdate::Changed(_)) | Ok(ModuleUpdate::Error(_)) => {
                    any_changed = true;
                }
                _ => {}
            }
        }
        any_changed
    }

    /// Returns the duration until the next scheduled periodic module update is due.
    pub fn time_until_next_update(&self) -> std::time::Duration {
        let now = Instant::now();
        let mut min_remaining = std::time::Duration::from_millis(1000);
        for module in &self.modules {
            let interval_ms = module.update_interval();
            if interval_ms >= 1000 {
                let interval = std::time::Duration::from_millis(interval_ms);
                if let Some(last) = module.last_updated() {
                    let elapsed = now.duration_since(last);
                    let remaining = interval.saturating_sub(elapsed);
                    if remaining < min_remaining {
                        min_remaining = remaining;
                    }
                } else {
                    return std::time::Duration::ZERO;
                }
            }
        }
        min_remaining
    }

    /// Returns the combined formatted status string across all modules.
    pub fn full_text(&self) -> String {
        self.modules
            .iter()
            .map(|m| m.current_text())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("  |  ")
    }

    /// Updates the active and occupied workspaces in the workspace module if present.
    pub fn set_workspace(&mut self, current: u32, occupied: &[u32]) {
        for m in &mut self.modules {
            if let Some(ws_mod) = m.as_any_mut().and_then(|a| a.downcast_mut::<WorkspaceModule>()) {
                ws_mod.set_workspace(current, occupied);
            }
        }
    }

    /// Updates the active layout indicator in the layout module if present.
    pub fn set_layout(&mut self, layout_name: &str) {
        for m in &mut self.modules {
            if let Some(layout_mod) = m.as_any_mut().and_then(|a| a.downcast_mut::<LayoutModule>()) {
                layout_mod.set_layout(layout_name);
            }
        }
    }

    /// Updates the active window title in the title module if present.
    pub fn set_title(&mut self, title: &str) {
        for m in &mut self.modules {
            if let Some(title_mod) = m.as_any_mut().and_then(|a| a.downcast_mut::<TitleModule>()) {
                title_mod.set_title(title);
            }
        }
    }

    /// Attaches an initialized [`crate::render::Renderer`] to this statusbar.
    pub fn attach_renderer(&mut self, renderer: crate::render::Renderer) {
        self.renderer = Some(renderer);
    }

    /// Renders the complete statusbar UI using the attached [`crate::render::Renderer`].
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        conn: &impl Connection,
        current_ws: u32,
        occupied_ws: &[u32],
        layout_symbol: &str,
        active_title: &str,
        tabs: Option<&[crate::bar::tabs::TabInfo]>,
        tab_config: &crate::bar::tabs::TabbedConfig,
    ) -> Result<(), StatusbarError> {
        if let Some(ref r) = self.renderer {
            let rects = r.render_statusbar(
                conn,
                self.window_id,
                current_ws,
                occupied_ws,
                layout_symbol,
                active_title,
                tabs,
                tab_config,
                &self.modules,
                &self.color_scheme,
            )?;
            self.tab_rects = rects;
        }
        Ok(())
    }

    /// Resizes the statusbar window, updates _NET_WM_STRUT properties, and resizes the off-screen pixmap.
    pub fn resize(
        &mut self,
        conn: &impl Connection,
        new_rect: Rect,
        root_depth: u8,
    ) -> Result<(), StatusbarError> {
        self.rect = new_rect;

        // 1. Configure the X11 statusbar window geometry
        let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
            .x(new_rect.x)
            .y(new_rect.y)
            .width(new_rect.width)
            .height(new_rect.height);
        conn.configure_window(self.window_id, &aux)?;

        // 2. Update EWMH strut reservation for the new geometry
        let _ = set_statusbar_struts(conn, self.window_id, new_rect);

        // 3. Resize the off-screen pixmap in the renderer
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(conn, self.window_id, new_rect.width as u16, new_rect.height as u16, root_depth)?;
        }

        conn.flush()?;
        Ok(())
    }

    /// Unmaps and destroys the statusbar window on the X server.
    pub fn destroy<C: Connection>(&self, conn: &C) -> Result<(), X11Error> {
        conn.unmap_window(self.window_id)?;
        conn.destroy_window(self.window_id)?;
        conn.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Window Creation with override_redirect, _NET_WM_WINDOW_TYPE, and struts
// ---------------------------------------------------------------------------

/// Sets the EWMH strut properties on the statusbar window to reserve screen space.
pub fn set_statusbar_struts(
    conn: &impl Connection,
    window_id: u32,
    rect: Rect,
) -> Result<(), StatusbarError> {
    let top_strut = if rect.y == 0 { rect.height } else { 0 };
    let bottom_strut = if rect.y > 0 { rect.height } else { 0 };
    let net_wm_strut = conn.intern_atom(false, b"_NET_WM_STRUT")?.reply()?.atom;
    conn.change_property32(
        PropMode::REPLACE,
        window_id,
        net_wm_strut,
        AtomEnum::CARDINAL,
        &[0, 0, top_strut, bottom_strut],
    )?;

    let net_wm_strut_partial = conn.intern_atom(false, b"_NET_WM_STRUT_PARTIAL")?.reply()?.atom;
    let strut_partial: [u32; 12] = [
        0,                                                  // left
        0,                                                  // right
        top_strut,                                          // top
        bottom_strut,                                       // bottom
        0, 0,                                               // left_start_y, left_end_y
        0, 0,                                               // right_start_y, right_end_y
        rect.x as u32, (rect.x + rect.width as i32) as u32, // top_start_x, top_end_x
        rect.x as u32, (rect.x + rect.width as i32) as u32, // bottom_start_x, bottom_end_x
    ];
    conn.change_property32(
        PropMode::REPLACE,
        window_id,
        net_wm_strut_partial,
        AtomEnum::CARDINAL,
        &strut_partial,
    )?;
    Ok(())
}

/// Creates and maps the statusbar window using the X11 `override_redirect` attribute.
///
/// Sets `_NET_WM_WINDOW_TYPE` to `_NET_WM_WINDOW_TYPE_DOCK` and reserves screen space
/// using `_NET_WM_STRUT` and `_NET_WM_STRUT_PARTIAL`.
///
/// # Arguments
/// * `conn` - The X11 connection handle.
/// * `screen` - The root screen information.
/// * `rect` - The target geometry of the statusbar.
///
/// # Returns
/// The newly created window ID, or a [`StatusbarError`].
pub fn create_statusbar_window(
    conn: &impl Connection,
    screen: &Screen,
    rect: Rect,
) -> Result<u32, StatusbarError> {
    create_statusbar_window_with_color(conn, screen, rect, ColorScheme::default().bg)
}

/// Creates and maps the statusbar window with a specific background color.
pub fn create_statusbar_window_with_color(
    conn: &impl Connection,
    screen: &Screen,
    rect: Rect,
    background_color: u32,
) -> Result<u32, StatusbarError> {
    let window_id = conn.generate_id()?;

    let aux = CreateWindowAux::new()
        .override_redirect(1) // Avoid WM window management interception
        .background_pixel(background_color)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::STRUCTURE_NOTIFY
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE,
        );

    conn.create_window(
        screen.root_depth,
        window_id,
        screen.root,
        rect.x as i16,
        rect.y as i16,
        rect.width as u16,
        rect.height as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &aux,
    )?;

    // 1. Set _NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_DOCK
    let net_wm_window_type = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE")?.reply()?.atom;
    let net_wm_window_type_dock = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE_DOCK")?.reply()?.atom;
    conn.change_property32(
        PropMode::REPLACE,
        window_id,
        net_wm_window_type,
        AtomEnum::ATOM,
        &[net_wm_window_type_dock],
    )?;

    // 2. Set _NET_WM_STRUT and _NET_WM_STRUT_PARTIAL
    set_statusbar_struts(conn, window_id, rect)?;

    // 3. Map window
    conn.map_window(window_id)?;
    conn.flush()?;

    Ok(window_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ram_module_parsing() {
        let sample = "\
MemTotal:       16384000 kB
MemFree:         4096000 kB
MemAvailable:    8192000 kB
Buffers:          500000 kB
Cached:          2000000 kB
";
        let percent = RamModule::parse_meminfo(sample).unwrap();
        // (16384000 - 8192000) / 16384000 = 0.50 -> 50%
        assert_eq!(percent, 50);

        let invalid = "Invalid meminfo line\n";
        assert_eq!(RamModule::parse_meminfo(invalid), None);
    }

    #[test]
    fn test_cpu_module_parsing_and_delta() {
        let stat1 = "cpu  100 0 100 800 0 0 0 0 0 0\n";
        let (total1, idle1) = CpuModule::parse_cpu_times(stat1).unwrap();
        assert_eq!(total1, 1000);
        assert_eq!(idle1, 800);

        let stat2 = "cpu  150 0 150 900 0 0 0 0 0 0\n";
        let (total2, idle2) = CpuModule::parse_cpu_times(stat2).unwrap();
        assert_eq!(total2, 1200);
        assert_eq!(idle2, 900);

        let delta_total = total2 - total1; // 200
        let delta_idle = idle2 - idle1;   // 100
        let usage = 100.0 * (delta_total - delta_idle) as f64 / delta_total as f64;
        assert_eq!(usage.round() as u32, 50);
    }

    #[test]
    fn test_workspace_and_layout_module_updates() {
        let mut ws = WorkspaceModule::new();
        assert!(ws.current_text().contains("[1]"));

        ws.set_workspace(3, &[1, 3]);
        assert!(ws.current_text().contains("[3]"));
        assert!(ws.current_text().contains("1:*"));

        let mut layout = LayoutModule::new();
        assert_eq!(layout.current_text(), "[]=");
        layout.set_layout("floating");
        assert_eq!(layout.current_text(), "><>");
        layout.set_layout("tabbed");
        assert_eq!(layout.current_text(), "[T]");
        layout.set_layout("grid");
        assert_eq!(layout.current_text(), "[G]");
    }

    #[test]
    fn test_title_module_truncation() {
        let mut title = TitleModule::new();
        title.set_title("Short");
        assert_eq!(title.current_text(), "Short");

        let long = "A very long window title that exceeds the maximum statusbar length allowed in configuration";
        title.set_title(long);
        assert!(title.current_text().ends_with("..."));
        assert!(title.current_text().len() <= 50);
    }

    #[test]
    fn test_clock_module_format() {
        let mut clock = ClockModule::new();
        let _ = clock.update();
        let text = clock.current_text();
        // Format is DAY:DATE:TIME, e.g. "Tue:2026-09-29:04:18"
        let parts: Vec<&str> = text.split(':').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].len(), 3); // 3-letter day
        assert_eq!(parts[1].len(), 10); // YYYY-MM-DD
    }

    #[test]
    fn test_ip_module() {
        let mut ip = IpModule::with_ip(Some("192.168.1.10".to_string()));
        assert_eq!(ip.name(), "ip");
        assert_eq!(ip.update_interval(), 3000);
        assert_eq!(ip.current_text(), "IP 192.168.1.10");

        ip.set_ip(None);
        // When custom_ip is None, update queries system or falls back
        let _ = ip.update();
        assert!(ip.current_text().starts_with("IP "));

        let ip_offline = IpModule::with_ip(Some("".to_string()));
        assert_eq!(ip_offline.current_text(), "IP Offline");
    }

    #[test]
    fn test_battery_module() {
        let temp_dir = std::env::temp_dir().join(format!("tilo_bat_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        // When no path is provided
        let bat_na = BatteryModule::with_path(None);
        assert_eq!(bat_na.name(), "battery");
        assert_eq!(bat_na.update_interval(), 5000);
        assert_eq!(bat_na.current_text(), "BAT N/A");

        // Write fake capacity and status (Discharging)
        std::fs::write(temp_dir.join("capacity"), "75\n").unwrap();
        std::fs::write(temp_dir.join("status"), "Discharging\n").unwrap();
        let mut bat = BatteryModule::with_path(Some(temp_dir.clone()));
        assert_eq!(bat.current_text(), "BAT 75%");

        // Write fake capacity and status (Charging)
        std::fs::write(temp_dir.join("capacity"), "76\n").unwrap();
        std::fs::write(temp_dir.join("status"), "Charging\n").unwrap();
        let update_res = bat.update().unwrap();
        assert_eq!(update_res, ModuleUpdate::Changed("BAT 76% (+)".to_string()));
        assert_eq!(bat.current_text(), "BAT 76% (+)");

        // Invalid capacity
        std::fs::write(temp_dir.join("capacity"), "invalid\n").unwrap();
        let update_err = bat.update().unwrap();
        assert!(matches!(update_err, ModuleUpdate::Error(_)));
        assert_eq!(bat.current_text(), "BAT: ERR");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_statusbar_full_text() {
        let mut bar = Statusbar::default_with_window(12345, Rect::new(0, 0, 1920, 24));
        bar.set_title("Terminal");
        let full = bar.full_text();
        println!("Full statusbar text: {}", full);
        assert!(full.contains("[1]"));
        assert!(full.contains("[]="));
        assert!(full.contains("Terminal"));
        assert!(full.contains("IP"));
        assert!(full.contains("RAM"));
        assert!(full.contains("CPU"));
        assert!(full.contains("BAT"));
    }
}
