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
//! - Graceful error handling (modules return `ModuleUpdate::Error` without crashing the WM).

use crate::layout::Rect;
use crate::x11::X11Error;
use std::net::UdpSocket;
use std::path::PathBuf;
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

    /// Parsing failure for system metrics.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Failed to load font.
    #[error("Failed to load font: {0}")]
    FontLoadError(String),

    /// PATH scanning failure.
    #[error("Path scan error: {0}")]
    PathScanError(String),

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

/// Represents the data returned by a module after an update tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleUpdate {
    /// The module's text has changed; UI needs to be re-rendered.
    Changed(String),
    /// No change in module text since last tick.
    Unchanged,
    /// The module encountered an error (e.g., failed to read `/proc`).
    Error(String),
}

/// Trait defining the behavior of an independent statusbar module.
pub trait StatusModule: std::fmt::Debug + Send {
    /// Returns the name of the module for debugging and identification.
    fn name(&self) -> &'static str;

    /// The interval in milliseconds at which this module should be polled.
    fn update_interval(&self) -> u64;

    /// Called periodically to fetch new data.
    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError>;

    /// Returns the current cached text to be drawn.
    fn current_text(&self) -> &str;

    /// Returns the timestamp when this module was last updated, if available.
    fn last_updated(&self) -> Option<Instant> {
        None
    }

    /// Returns a mutable reference to Any for downcasting if supported.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
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
    pub fn render(
        &self,
        conn: &impl Connection,
        current_ws: u32,
        occupied_ws: &[u32],
        layout_symbol: &str,
        active_title: &str,
    ) -> Result<(), StatusbarError> {
        if let Some(ref r) = self.renderer {
            r.render_statusbar(
                conn,
                self.window_id,
                current_ws,
                occupied_ws,
                layout_symbol,
                active_title,
                &self.modules,
                &self.color_scheme,
            )?;
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

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

/// Workspace status module displaying active and occupied workspace tags.
#[derive(Debug, Clone)]
pub struct WorkspaceModule {
    pub current_workspace: u32,
    pub occupied_workspaces: Vec<u32>,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl WorkspaceModule {
    pub fn new() -> Self {
        let mut s = Self {
            current_workspace: 1,
            occupied_workspaces: vec![1],
            cached_text: String::new(),
            last_updated: None,
        };
        s.rebuild_text();
        s
    }

    pub fn set_workspace(&mut self, current: u32, occupied: &[u32]) {
        self.current_workspace = current;
        self.occupied_workspaces = occupied.to_vec();
        self.rebuild_text();
    }

    fn rebuild_text(&mut self) {
        let mut parts = Vec::new();
        for ws in 1..=9 {
            if ws == self.current_workspace {
                parts.push(format!("[{}]", ws));
            } else if self.occupied_workspaces.contains(&ws) {
                parts.push(format!("{}:*", ws));
            } else {
                parts.push(format!("{}", ws));
            }
        }
        self.cached_text = parts.join(" ");
    }
}

impl Default for WorkspaceModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for WorkspaceModule {
    fn name(&self) -> &'static str {
        "workspace"
    }

    fn update_interval(&self) -> u64 {
        100
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        let prev = self.cached_text.clone();
        self.rebuild_text();
        self.last_updated = Some(Instant::now());
        if self.cached_text != prev {
            Ok(ModuleUpdate::Changed(self.cached_text.clone()))
        } else {
            Ok(ModuleUpdate::Unchanged)
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Layout status module displaying the active window tiling layout symbol.
#[derive(Debug, Clone)]
pub struct LayoutModule {
    pub layout_name: String,
    pub layout_symbol: String,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl LayoutModule {
    pub fn new() -> Self {
        Self {
            layout_name: "tiling".to_string(),
            layout_symbol: "[]=".to_string(),
            cached_text: "[]=".to_string(),
            last_updated: None,
        }
    }

    pub fn set_layout(&mut self, layout: &str) {
        self.layout_name = layout.to_string();
        self.layout_symbol = match layout {
            "floating" | "><>" => "><>".to_string(),
            "tabbed" | "[T]" => "[T]".to_string(),
            "stacking" | "stacked" | "[S]" => "[S]".to_string(),
            "vertical" | "|||" => "|||".to_string(),
            "horizontal" | "===" => "===".to_string(),
            "monocle" | "[M]" => "[M]".to_string(),
            "bsp" | "[B]" => "[B]".to_string(),
            "master" | "[]=" => "[]=".to_string(),
            _ => {
                if layout.starts_with('[') && layout.ends_with(']') {
                    layout.to_string()
                } else {
                    "[]=".to_string()
                }
            }
        };
        self.cached_text = self.layout_symbol.clone();
    }
}

impl Default for LayoutModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for LayoutModule {
    fn name(&self) -> &'static str {
        "layout"
    }

    fn update_interval(&self) -> u64 {
        200
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        Ok(ModuleUpdate::Unchanged)
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Window Title status module displaying the currently focused client title.
#[derive(Debug, Clone)]
pub struct TitleModule {
    pub title: String,
    pub max_length: usize,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl TitleModule {
    pub fn new() -> Self {
        Self {
            title: String::new(),
            max_length: 50,
            cached_text: String::new(),
            last_updated: None,
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        if self.title.chars().count() > self.max_length {
            let truncated: String = self.title.chars().take(self.max_length - 3).collect();
            self.cached_text = format!("{}...", truncated);
        } else {
            self.cached_text = self.title.clone();
        }
    }
}

impl Default for TitleModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for TitleModule {
    fn name(&self) -> &'static str {
        "title"
    }

    fn update_interval(&self) -> u64 {
        100
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        Ok(ModuleUpdate::Unchanged)
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// IP status module displaying the active local IP address.
#[derive(Debug, Clone)]
pub struct IpModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub custom_ip: Option<String>,
}

impl IpModule {
    pub fn new() -> Self {
        Self::with_ip(None)
    }

    pub fn with_ip(ip: Option<String>) -> Self {
        let mut s = Self {
            cached_text: "IP ...".to_string(),
            last_updated: None,
            custom_ip: ip,
        };
        let _ = s.update();
        s
    }

    pub fn set_ip(&mut self, ip: Option<String>) {
        self.custom_ip = ip;
    }

    fn query_local_ip() -> Option<String> {
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("1.1.1.1:80").ok()?;
        let addr = socket.local_addr().ok()?;
        let ip = addr.ip();
        if ip.is_unspecified() || ip.is_loopback() {
            None
        } else {
            Some(ip.to_string())
        }
    }
}

impl Default for IpModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for IpModule {
    fn name(&self) -> &'static str {
        "ip"
    }

    fn update_interval(&self) -> u64 {
        3000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let ip_opt = if let Some(ref custom) = self.custom_ip {
            Some(custom.clone())
        } else {
            Self::query_local_ip()
        };

        let new_text = match ip_opt {
            Some(ip) if !ip.is_empty() => format!("IP {}", ip),
            _ => "IP Offline".to_string(),
        };

        if new_text != self.cached_text {
            self.cached_text = new_text.clone();
            Ok(ModuleUpdate::Changed(new_text))
        } else {
            Ok(ModuleUpdate::Unchanged)
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// RAM status module reading `/proc/meminfo` directly without external child processes.
#[derive(Debug, Clone)]
pub struct RamModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub meminfo_path: PathBuf,
}

impl RamModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/meminfo"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            cached_text: "RAM --%".to_string(),
            last_updated: None,
            meminfo_path: path,
        };
        let _ = s.update();
        s
    }

    fn parse_meminfo(content: &str) -> Option<u32> {
        let mut mem_total: Option<u64> = None;
        let mut mem_available: Option<u64> = None;

        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let val = parts.next()?.parse::<u64>().ok()?;

            if key == "MemTotal:" {
                mem_total = Some(val);
            } else if key == "MemAvailable:" {
                mem_available = Some(val);
            }

            if mem_total.is_some() && mem_available.is_some() {
                break;
            }
        }

        let total = mem_total?;
        let available = mem_available?;
        if total == 0 {
            return None;
        }

        let used = total.saturating_sub(available);
        let percent = ((used as f64 / total as f64) * 100.0).round() as u32;
        Some(percent.min(100))
    }
}

impl Default for RamModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for RamModule {
    fn name(&self) -> &'static str {
        "ram"
    }

    fn update_interval(&self) -> u64 {
        2000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let content = match std::fs::read_to_string(&self.meminfo_path) {
            Ok(c) => c,
            Err(e) => {
                self.cached_text = "RAM: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.meminfo_path.display(),
                    e
                )));
            }
        };

        if let Some(percent) = Self::parse_meminfo(&content) {
            let new_text = format!("RAM {}%", percent);
            if new_text != self.cached_text {
                self.cached_text = new_text.clone();
                Ok(ModuleUpdate::Changed(new_text))
            } else {
                Ok(ModuleUpdate::Unchanged)
            }
        } else {
            self.cached_text = "RAM: ERR".to_string();
            Ok(ModuleUpdate::Error("Malformed /proc/meminfo content".to_string()))
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}

/// CPU status module reading `/proc/stat` delta metrics.
#[derive(Debug, Clone)]
pub struct CpuModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub stat_path: PathBuf,
    pub prev_total: u64,
    pub prev_idle: u64,
}

impl CpuModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/stat"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            cached_text: "CPU --%".to_string(),
            last_updated: None,
            stat_path: path,
            prev_total: 0,
            prev_idle: 0,
        };
        let _ = s.update();
        s
    }

    fn parse_cpu_times(content: &str) -> Option<(u64, u64)> {
        for line in content.lines() {
            if line.starts_with("cpu ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    let user = parts[1].parse::<u64>().ok()?;
                    let nice = parts[2].parse::<u64>().ok()?;
                    let system = parts[3].parse::<u64>().ok()?;
                    let idle = parts[4].parse::<u64>().ok()?;
                    let iowait = parts.get(5).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let irq = parts.get(6).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let softirq = parts.get(7).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let steal = parts.get(8).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

                    let idle_time = idle + iowait;
                    let total_time = user + nice + system + idle + iowait + irq + softirq + steal;
                    return Some((total_time, idle_time));
                }
            }
        }
        None
    }
}

impl Default for CpuModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for CpuModule {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn update_interval(&self) -> u64 {
        1000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let content = match std::fs::read_to_string(&self.stat_path) {
            Ok(c) => c,
            Err(e) => {
                self.cached_text = "CPU: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.stat_path.display(),
                    e
                )));
            }
        };

        if let Some((total, idle)) = Self::parse_cpu_times(&content) {
            if self.prev_total == 0 {
                self.prev_total = total;
                self.prev_idle = idle;
                self.cached_text = "CPU 0%".to_string();
                return Ok(ModuleUpdate::Changed(self.cached_text.clone()));
            }

            let total_delta = total.saturating_sub(self.prev_total);
            let idle_delta = idle.saturating_sub(self.prev_idle);

            self.prev_total = total;
            self.prev_idle = idle;

            if total_delta > 0 {
                let usage = 100.0 * (total_delta.saturating_sub(idle_delta)) as f64 / total_delta as f64;
                let percent = (usage.round() as u32).min(100);
                let new_text = format!("CPU {}%", percent);
                if new_text != self.cached_text {
                    self.cached_text = new_text.clone();
                    Ok(ModuleUpdate::Changed(new_text))
                } else {
                    Ok(ModuleUpdate::Unchanged)
                }
            } else {
                Ok(ModuleUpdate::Unchanged)
            }
        } else {
            self.cached_text = "CPU: ERR".to_string();
            Ok(ModuleUpdate::Error("Malformed /proc/stat content".to_string()))
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}

/// Battery status module reading `/sys/class/power_supply/BAT*` capacity and charging status.
#[derive(Debug, Clone)]
pub struct BatteryModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub sysfs_path: Option<PathBuf>,
}

impl BatteryModule {
    pub fn new() -> Self {
        Self::with_path(Self::find_battery_path())
    }

    pub fn with_path(path: Option<PathBuf>) -> Self {
        let mut s = Self {
            cached_text: "BAT --%".to_string(),
            last_updated: None,
            sysfs_path: path,
        };
        let _ = s.update();
        s
    }

    pub fn find_battery_path() -> Option<PathBuf> {
        let base = std::path::Path::new("/sys/class/power_supply");
        if let Ok(entries) = std::fs::read_dir(base) {
            let mut bat_paths = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("BAT") || name_str.starts_with("bat") {
                    bat_paths.push(entry.path());
                }
            }
            bat_paths.sort();
            if let Some(first) = bat_paths.into_iter().next() {
                return Some(first);
            }
        }
        None
    }

    pub fn parse_battery_info(bat_dir: &std::path::Path) -> Result<(u32, bool), String> {
        let capacity_path = bat_dir.join("capacity");
        let status_path = bat_dir.join("status");

        let cap_content = std::fs::read_to_string(&capacity_path)
            .map_err(|e| format!("Failed to read capacity: {}", e))?;
        let cap: u32 = cap_content
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("Failed to parse capacity: {}", e))?;

        let is_charging = if let Ok(status_content) = std::fs::read_to_string(&status_path) {
            status_content.trim().eq_ignore_ascii_case("charging")
        } else {
            false
        };

        Ok((cap.min(100), is_charging))
    }
}

impl Default for BatteryModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for BatteryModule {
    fn name(&self) -> &'static str {
        "battery"
    }

    fn update_interval(&self) -> u64 {
        5000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let path = match self.sysfs_path {
            Some(ref p) => p,
            None => {
                let na = "BAT N/A".to_string();
                if self.cached_text != na {
                    self.cached_text = na.clone();
                    return Ok(ModuleUpdate::Changed(na));
                }
                return Ok(ModuleUpdate::Unchanged);
            }
        };

        match Self::parse_battery_info(path) {
            Ok((cap, is_charging)) => {
                let new_text = if is_charging {
                    format!("BAT {}% (+)", cap)
                } else {
                    format!("BAT {}%", cap)
                };

                if new_text != self.cached_text {
                    self.cached_text = new_text.clone();
                    Ok(ModuleUpdate::Changed(new_text))
                } else {
                    Ok(ModuleUpdate::Unchanged)
                }
            }
            Err(e) => {
                self.cached_text = "BAT: ERR".to_string();
                Ok(ModuleUpdate::Error(e))
            }
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Clock status module displaying the system time in `DAY:DATE:TIME` format (e.g. `Tue:2026-09-29:04:18`).
#[derive(Debug, Clone)]
pub struct ClockModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl ClockModule {
    pub fn new() -> Self {
        let mut s = Self {
            cached_text: String::new(),
            last_updated: None,
        };
        let _ = s.update();
        s
    }

    pub fn format_now() -> String {
        let mut now = 0 as libc::time_t;
        unsafe {
            libc::time(&mut now);
            let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
            if !libc::localtime_r(&now, tm.as_mut_ptr()).is_null() {
                let tm = tm.assume_init();
                const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
                let day_name = DAYS.get(tm.tm_wday.clamp(0, 6) as usize).unwrap_or(&"---");
                let year = 1900 + tm.tm_year;
                let month = tm.tm_mon + 1;
                let mday = tm.tm_mday;
                let hour = tm.tm_hour;
                let min = tm.tm_min;
                format!("{}:{:04}-{:02}-{:02}:{:02}:{:02}", day_name, year, month, mday, hour, min)
            } else {
                "---:0000-00-00:00:00".to_string()
            }
        }
    }
}

impl Default for ClockModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for ClockModule {
    fn name(&self) -> &'static str {
        "clock"
    }

    fn update_interval(&self) -> u64 {
        1000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        let new_text = Self::format_now();
        if new_text != self.cached_text {
            self.cached_text = new_text.clone();
            Ok(ModuleUpdate::Changed(new_text))
        } else {
            Ok(ModuleUpdate::Unchanged)
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
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
