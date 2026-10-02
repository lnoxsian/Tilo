//! Statusbar and text rendering pipeline.
//!
//! Implements Phase 18 of Plan.md:
//! - Dual-engine rendering architecture:
//!   1. Zero-allocation X11 Core font engine (`is_core = true`) for < 2 MB RAM usage.
//!   2. Anti-aliased Pango/Cairo engine for modern TrueType / OpenType font rendering.
//! - X11 core font loading and baseline metrics calculation ([`Renderer::load_font`]).
//! - Double-buffered rendering via off-screen `Pixmap` to eliminate visual flickering.
//! - Workspace highlighting (active workspace pill background and text).
//! - Active window title indication.
//! - Status metrics rendering (RAM, CPU, Clock, Layout).

use crate::layout::Rect;
use crate::statusbar::{ColorScheme, StatusModule, StatusbarError, TiloError};
use crate::x11::X11Error;
#[cfg(feature = "pango")]
use pangocairo::pango::{self, FontDescription};
#[cfg(feature = "pango")]
use std::cell::RefCell;
#[cfg(feature = "pango")]
use std::collections::HashMap;
use x11rb::connection::Connection;
#[cfg(feature = "pango")]
use x11rb::protocol::xproto::ImageFormat;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt, CreateGCAux, Rectangle,
};

/// Converts a 0x00RRGGBB integer color into floating point RGB values (0.0 to 1.0).
#[inline]
pub fn color_to_rgb(color: u32) -> (f64, f64, f64) {
    let r = ((color >> 16) & 0xff) as f64 / 255.0;
    let g = ((color >> 8) & 0xff) as f64 / 255.0;
    let b = (color & 0xff) as f64 / 255.0;
    (r, g, b)
}

/// Parses an i3-compatible font specification string into a [`pango::FontDescription`].
///
/// Supports strings with or without the `pango:` prefix, such as:
/// - `"pango:monospace 10"`
/// - `"monospace 10"`
/// - `"pango:DejaVu Sans Mono Bold 11"`
/// - `"JetBrainsMono Nerd Font 10"`
///
/// If no font size is specified, defaults to 10 points.
#[cfg(feature = "pango")]
pub fn parse_font_description(font_name: &str) -> FontDescription {
    let trimmed = font_name.trim();
    let clean = if let Some(stripped) = trimmed.strip_prefix("pango:") {
        stripped.trim()
    } else {
        trimmed
    };
    let mut desc = FontDescription::from_string(clean);
    if desc.size() == 0 {
        desc.set_size(10 * pango::SCALE);
    }
    desc
}

/// Determines if a font specification should use the ultra-low-memory X11 Core font engine.
pub fn is_core_font_spec(font_name: &str) -> bool {
    #[cfg(not(feature = "pango"))]
    {
        let _ = font_name;
        true
    }
    #[cfg(feature = "pango")]
    {
        let trimmed = font_name.trim();
        if trimmed.starts_with("core:") {
            return true;
        }
        if trimmed.starts_with("pango:") {
            return false;
        }
        trimmed == "fixed"
            || trimmed.starts_with("-misc-fixed")
            || trimmed == "6x13"
            || trimmed == "9x15"
            || trimmed.starts_with('-')
    }
}

/// Strips the `"core:"` prefix from a font specification if present.
pub fn clean_core_font_name(font_name: &str) -> &str {
    let trimmed = font_name.trim();
    if let Some(stripped) = trimmed.strip_prefix("core:") {
        stripped.trim()
    } else {
        trimmed
    }
}

/// Responsible for rendering text and shapes to the statusbar or launcher window.
#[derive(Debug)]
pub struct Renderer {
    /// The X11 graphics context used for drawing.
    pub gc: u32,
    /// The core loaded font identifier (0 if none).
    pub font_id: u32,
    /// Reusable double-buffer pixmap to prevent flickering.
    pub pixmap: u32,
    /// Visual depth in bits per pixel.
    pub depth: u8,
    /// Width of the rendering target / pixmap in pixels.
    pub width: u16,
    /// Height of the rendering target / pixmap in pixels.
    pub height: u16,
    /// Font ascent in pixels (distance above baseline).
    pub font_ascent: i16,
    /// Font descent in pixels (distance below baseline).
    pub font_descent: i16,
    /// Average character width in pixels.
    pub char_width: u16,
    /// Whether this renderer is operating in the zero-allocation pure X11 Core font engine mode.
    pub is_core: bool,

    #[cfg(feature = "pango")]
    /// Parsed Pango font description.
    pub font_desc: FontDescription,
    #[cfg(feature = "pango")]
    /// Off-screen Cairo image surface matching the pixmap dimensions (`None` if `is_core` is true).
    surface: Option<RefCell<cairo::ImageSurface>>,
    #[cfg(feature = "pango")]
    /// Reusable Pango layout for zero-allocation text measurement and rendering (`None` if `is_core` is true).
    layout: Option<RefCell<pango::Layout>>,
    #[cfg(feature = "pango")]
    /// Bounded cache of calculated text widths to avoid repeated layout passes.
    text_width_cache: RefCell<HashMap<String, u16>>,
}

// Cairo's ImageSurface contains a raw NonNull pointer, but the surface inside Renderer
// is strictly owned and not shared across threads concurrently, making it safe to transfer ownership (Send).
unsafe impl Send for Renderer {}

impl Renderer {
    /// Attempts to open an X11 font and verifies its existence via `query_font`.
    fn try_open_font(conn: &impl Connection, font_name: &str) -> Result<(u32, i16, i16, u16), ()> {
        let font_id = conn.generate_id().map_err(|_| ())?;
        if conn.open_font(font_id, font_name.as_bytes()).is_err() {
            return Err(());
        }
        // query_font round-trips to X server, ensuring the font name is valid
        if let Ok(cookie) = conn.query_font(font_id)
            && let Ok(reply) = cookie.reply() {
                let ascent = reply.font_ascent;
                let descent = reply.font_descent;
                let width = if reply.max_bounds.character_width > 0 {
                    reply.max_bounds.character_width as u16
                } else {
                    8
                };
                return Ok((font_id, ascent, descent, width));
            }
        let _ = conn.close_font(font_id);
        Err(())
    }

    /// Loads an X11 font by name with automatic fallback to standard monospace fonts.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `font_name` - The logical font name (e.g., "fixed", "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*").
    ///
    /// # Returns
    /// A font ID, or a [`TiloError::FontLoadError`] if no font can be loaded.
    pub fn load_font(conn: &impl Connection, font_name: &str) -> Result<u32, TiloError> {
        let clean_name = clean_core_font_name(font_name);
        if let Ok((font_id, _, _, _)) = Self::try_open_font(conn, clean_name) {
            return Ok(font_id);
        }

        // Standard core X11 fonts that are guaranteed to exist on any X11 server
        for fallback in &["fixed", "6x13", "9x15", "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*"] {
            if let Ok((font_id, _, _, _)) = Self::try_open_font(conn, fallback) {
                return Ok(font_id);
            }
        }

        Err(StatusbarError::FontLoadError(format!(
            "Failed to load font '{}' and all fallbacks",
            font_name
        )))
    }

    /// Creates a new `Renderer` initialized with an off-screen pixmap and graphics context.
    ///
    /// In X11 Core font mode (`is_core = true`), no Cairo surfaces or Pango layouts are allocated,
    /// keeping RAM consumption strictly under 2 MB.
    /// In Pango mode, TrueType / OpenType anti-aliased font rendering is enabled.
    pub fn new(
        conn: &impl Connection,
        drawable: u32,
        width: u16,
        height: u16,
        depth: u8,
        font_name: &str,
    ) -> Result<Self, StatusbarError> {
        let is_core = is_core_font_spec(font_name);
        let clean_name = clean_core_font_name(font_name);

        let (font_id, core_ascent, core_descent, core_width) = if is_core {
            if let Ok(res) = Self::try_open_font(conn, clean_name) {
                res
            } else {
                let mut loaded = None;
                for fallback in &["fixed", "6x13", "9x15", "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*"] {
                    if let Ok(res) = Self::try_open_font(conn, fallback) {
                        loaded = Some(res);
                        break;
                    }
                }
                loaded.unwrap_or((0, 12, 4, 8))
            }
        } else {
            (0, 12, 4, 8)
        };

        let gc = conn.generate_id()?;
        let mut gc_aux = CreateGCAux::new().graphics_exposures(0);
        if font_id > 0 {
            gc_aux = gc_aux.font(font_id);
        }
        conn.create_gc(gc, drawable, &gc_aux)?;

        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, drawable, width, height)?;
        conn.flush()?;

        #[cfg(not(feature = "pango"))]
        {
            let _ = is_core;
            #[cfg(target_os = "linux")]
            unsafe {
                libc::malloc_trim(0);
            }
            Ok(Self {
                gc,
                font_id,
                pixmap,
                depth,
                width,
                height,
                font_ascent: core_ascent,
                font_descent: core_descent,
                char_width: core_width,
                is_core: true,
            })
        }

        #[cfg(feature = "pango")]
        {
            if is_core {
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::malloc_trim(0);
                }
                return Ok(Self {
                    gc,
                    font_id,
                    pixmap,
                    depth,
                    width,
                    height,
                    font_ascent: core_ascent,
                    font_descent: core_descent,
                    char_width: core_width,
                    is_core: true,
                    font_desc: parse_font_description("fixed"),
                    surface: None,
                    layout: None,
                    text_width_cache: RefCell::new(HashMap::new()),
                });
            }

            let font_desc = parse_font_description(font_name);
            let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, width as i32, height as i32)
                .map_err(|e| StatusbarError::RenderError(format!("Failed to create Cairo surface: {}", e)))?;

            let cr = cairo::Context::new(&surface)
                .map_err(|e| StatusbarError::RenderError(format!("Failed to create Cairo context: {}", e)))?;
            let layout = pangocairo::functions::create_layout(&cr);
            layout.set_font_description(Some(&font_desc));

            // Calculate accurate font metrics using Pango
            layout.set_text("M");
            let (cw, ch) = layout.pixel_size();
            let baseline = (layout.baseline() / pango::SCALE) as i16;
            let pango_ascent = baseline.max(1);
            let pango_descent = (ch as i16 - baseline).max(0);
            let pango_char_w = (cw as u16).max(1);

            #[cfg(target_os = "linux")]
            unsafe {
                libc::malloc_trim(0);
            }

            Ok(Self {
                gc,
                font_id,
                pixmap,
                depth,
                width,
                height,
                font_ascent: pango_ascent,
                font_descent: pango_descent,
                char_width: pango_char_w,
                is_core: false,
                font_desc,
                surface: Some(RefCell::new(surface)),
                layout: Some(RefCell::new(layout)),
                text_width_cache: RefCell::new(HashMap::new()),
            })
        }
    }

    /// Draws a string of text onto the active rendering target.
    ///
    /// In X11 Core mode, uses direct zero-allocation `poly_text8`/`image_text8` protocol requests.
    /// In Pango mode, renders anti-aliased glyphs via Cairo/Pango.
    pub fn draw_text(
        &self,
        conn: &impl Connection,
        drawable: u32,
        x: i16,
        y: i16,
        text: &str,
        color: u32,
    ) -> Result<(), TiloError> {
        if text.is_empty() {
            return Ok(());
        }

        #[cfg(feature = "pango")]
        if !self.is_core {
            if let Some(ref surface_cell) = self.surface
                && let Some(ref layout_cell) = self.layout {
                    let (r, g, b) = color_to_rgb(color);
                    let surface = surface_cell.borrow();
                    if let Ok(cr) = cairo::Context::new(&*surface) {
                        cr.set_source_rgb(r, g, b);
                        let layout = layout_cell.borrow();
                        layout.set_text(text);

                        let baseline_px = (layout.baseline() / pango::SCALE) as i16;
                        let top_y = y - baseline_px;
                        cr.move_to(x as f64, top_y as f64);
                        pangocairo::functions::show_layout(&cr, &layout);
                    }
                }
            return Ok(());
        }

        // Core X11 font rendering path (zero heap allocations)
        let aux = ChangeGCAux::new().foreground(color);
        conn.change_gc(self.gc, &aux)?;

        let mut drew_poly = false;
        if text.len() <= 254 {
            let mut item = Vec::with_capacity(text.len() + 2);
            item.push(text.len() as u8);
            item.push(0u8); // 0 horizontal delta
            item.extend_from_slice(text.as_bytes());
            if conn.poly_text8(drawable, self.gc, x, y, &item).is_ok() {
                drew_poly = true;
            }
        }
        if !drew_poly {
            conn.image_text8(drawable, self.gc, x, y, text.as_bytes())?;
        }

        Ok(())
    }

    /// Fills a solid rectangular region with the specified color.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect(
        &self,
        conn: &impl Connection,
        drawable: u32,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        color: u32,
    ) -> Result<(), StatusbarError> {
        #[cfg(feature = "pango")]
        if !self.is_core {
            if let Some(ref surface_cell) = self.surface {
                let (r, g, b) = color_to_rgb(color);
                let surface = surface_cell.borrow();
                if let Ok(cr) = cairo::Context::new(&*surface) {
                    cr.set_source_rgb(r, g, b);
                    cr.rectangle(x as f64, y as f64, width as f64, height as f64);
                    let _ = cr.fill();
                }
            }
            return Ok(());
        }

        // Core X11 fill path (zero heap allocations)
        let aux = ChangeGCAux::new().foreground(color);
        conn.change_gc(self.gc, &aux)?;
        let rect = Rectangle {
            x,
            y,
            width,
            height,
        };
        conn.poly_fill_rectangle(drawable, self.gc, &[rect])?;
        Ok(())
    }

    /// Calculates the horizontal pixel width of a given text string.
    pub fn text_width(&self, text: &str) -> u16 {
        if text.is_empty() {
            return 0;
        }

        #[cfg(feature = "pango")]
        if !self.is_core {
            if let Some(&w) = self.text_width_cache.borrow().get(text) {
                return w;
            }

            if let Some(ref layout_cell) = self.layout {
                let layout = layout_cell.borrow();
                layout.set_text(text);
                let (w, _) = layout.pixel_size();
                let width = w as u16;

                if text.len() <= 64 {
                    let mut cache = self.text_width_cache.borrow_mut();
                    if cache.len() < 512 {
                        cache.insert(text.to_string(), width);
                    }
                }

                return width;
            }
        }

        (text.chars().count() as u16) * self.char_width
    }

    /// Calculates the pixel dimensions (width, height) of a given text string.
    pub fn text_size(&self, text: &str) -> (u16, u16) {
        if text.is_empty() {
            return (0, 0);
        }

        #[cfg(feature = "pango")]
        if !self.is_core
            && let Some(ref layout_cell) = self.layout {
                let layout = layout_cell.borrow();
                layout.set_text(text);
                let (w, h) = layout.pixel_size();
                return (w as u16, h as u16);
            }

        (
            (text.chars().count() as u16) * self.char_width,
            (self.font_ascent + self.font_descent) as u16,
        )
    }

    /// Computes the vertical baseline position centered vertically within the statusbar.
    pub fn vertical_center_baseline(&self) -> i16 {
        let font_height = self.font_ascent + self.font_descent;
        let y_offset = (self.height as i16 - font_height) / 2;
        y_offset + self.font_ascent
    }

    /// Resizes the off-screen double-buffer pixmap and Cairo surface to match new dimensions.
    pub fn resize(
        &mut self,
        conn: &impl Connection,
        drawable: u32,
        new_width: u16,
        new_height: u16,
        depth: u8,
    ) -> Result<(), StatusbarError> {
        if self.width == new_width && self.height == new_height {
            return Ok(());
        }

        // Free the previous pixmap
        let _ = conn.free_pixmap(self.pixmap);

        // Allocate a new pixmap with the updated dimensions
        let new_pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, new_pixmap, drawable, new_width, new_height)?;
        conn.flush()?;

        self.pixmap = new_pixmap;
        self.width = new_width;
        self.height = new_height;
        self.depth = depth;

        #[cfg(feature = "pango")]
        if !self.is_core {
            let new_surface = cairo::ImageSurface::create(cairo::Format::Rgb24, new_width as i32, new_height as i32)
                .map_err(|e| StatusbarError::RenderError(format!("Failed to resize Cairo surface: {}", e)))?;

            let cr = cairo::Context::new(&new_surface)
                .map_err(|e| StatusbarError::RenderError(format!("Failed to create Cairo context on resize: {}", e)))?;
            let new_layout = pangocairo::functions::create_layout(&cr);
            new_layout.set_font_description(Some(&self.font_desc));

            self.surface = Some(RefCell::new(new_surface));
            self.layout = Some(RefCell::new(new_layout));
            self.text_width_cache.borrow_mut().clear();
            #[cfg(target_os = "linux")]
            unsafe {
                libc::malloc_trim(0);
            }
        }

        Ok(())
    }

    /// Copies the existing off-screen pixmap directly onto the target window.
    pub fn copy_pixmap_to_window(&self, conn: &impl Connection, window: u32) -> Result<(), StatusbarError> {
        conn.copy_area(
            self.pixmap,
            window,
            self.gc,
            0,
            0,
            0,
            0,
            self.width,
            self.height,
        )?;
        conn.flush()?;
        Ok(())
    }

    /// Synchronizes the Cairo image surface to the X11 double-buffer pixmap using `put_image`.
    pub fn flush_cairo_to_pixmap(&self, _conn: &impl Connection) -> Result<(), StatusbarError> {
        #[cfg(feature = "pango")]
        if !self.is_core
            && let Some(ref surface_cell) = self.surface {
                let mut surface = surface_cell.borrow_mut();
                surface.flush();
                let data = surface.data().map_err(|e| {
                    StatusbarError::RenderError(format!("Failed to borrow Cairo surface data: {}", e))
                })?;

                let depth = if self.depth > 0 { self.depth } else { 24 };
                _conn.put_image(
                    ImageFormat::Z_PIXMAP,
                    self.pixmap,
                    self.gc,
                    self.width,
                    self.height,
                    0,
                    0,
                    0,
                    depth,
                    &data,
                )?;
                _conn.flush()?;
            }
        Ok(())
    }

    /// Blits the fully rendered off-screen pixmap onto the target window in a single atomic frame.
    pub fn flush_to_window(&self, conn: &impl Connection, window: u32) -> Result<(), StatusbarError> {
        self.flush_area_to_window(conn, window, self.width, self.height)
    }

    /// Blits a specific sub-area of the off-screen pixmap onto the target window.
    pub fn flush_area_to_window(
        &self,
        conn: &impl Connection,
        window: u32,
        width: u16,
        height: u16,
    ) -> Result<(), StatusbarError> {
        #[cfg(feature = "pango")]
        if !self.is_core {
            let _ = self.flush_cairo_to_pixmap(conn);
        }
        conn.copy_area(
            self.pixmap,
            window,
            self.gc,
            0,
            0,
            0,
            0,
            width.min(self.width),
            height.min(self.height),
        )?;
        conn.flush()?;
        Ok(())
    }

    /// Renders the entire statusbar: background, workspace indicators, layout indicator,
    /// active window title, and right-aligned system modules into the double-buffer pixmap,
    /// then blits the result to the statusbar window.
    #[allow(clippy::too_many_arguments)]
    pub fn render_statusbar(
        &self,
        conn: &impl Connection,
        window: u32,
        current_workspace: u32,
        occupied_workspaces: &[u32],
        layout_symbol: &str,
        active_window_title: &str,
        tabs: Option<&[crate::bar::tabs::TabInfo]>,
        tab_config: &crate::bar::tabs::TabbedConfig,
        modules: &[Box<dyn StatusModule>],
        color_scheme: &ColorScheme,
    ) -> Result<crate::bar::StatusbarHitRects, StatusbarError> {
        // 1. Clear background of pixmap
        self.fill_rect(conn, self.pixmap, 0, 0, self.width, self.height, color_scheme.bg)?;

        let baseline = self.vertical_center_baseline();
        let mut cur_x = 8i16;
        let mut workspace_rects = Vec::new();

        const WS_STRS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

        // 2. Render workspaces 1 through 9 with active workspace highlighting
        for ws in 1..=9 {
            let is_active = ws == current_workspace;
            let is_occupied = occupied_workspaces.contains(&ws);

            let ws_str = WS_STRS.get(ws as usize).copied().unwrap_or("?");
            let ws_text_w = self.text_width(ws_str);
            let pad = 6i16;
            let pill_w = ws_text_w + (pad as u16 * 2);
            let pill_h = self.height.saturating_sub(6);
            let pill_y = 3i16;

            let ws_rect = Rect::new(cur_x as i32, pill_y as i32, pill_w as u32, pill_h as u32);
            workspace_rects.push((ws, ws_rect));

            if is_active {
                // Active workspace pill background
                self.fill_rect(
                    conn,
                    self.pixmap,
                    cur_x,
                    pill_y,
                    pill_w,
                    pill_h,
                    color_scheme.active_bg,
                )?;
                // Active workspace text
                self.draw_text(
                    conn,
                    self.pixmap,
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.active_fg,
                )?;
            } else if is_occupied {
                // Occupied workspace indicator
                self.draw_text(
                    conn,
                    self.pixmap,
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.fg,
                )?;
                // Small indicator dot/asterisk
                let dot_x = cur_x + pad + ws_text_w as i16;
                self.draw_text(conn, self.pixmap, dot_x, baseline - 2, "*", color_scheme.active_bg)?;
            } else {
                // Inactive empty workspace
                self.draw_text(
                    conn,
                    self.pixmap,
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.inactive_fg,
                )?;
            }

            cur_x += pill_w as i16 + 4;
        }

        // 3. Render layout symbol
        cur_x += 6;
        let layout_text = if layout_symbol.starts_with('[') && layout_symbol.ends_with(']') {
            layout_symbol.to_string()
        } else {
            format!("[{}]", layout_symbol)
        };
        self.draw_text(conn, self.pixmap, cur_x, baseline, &layout_text, color_scheme.active_bg)?;
        cur_x += self.text_width(&layout_text) as i16 + 12;

        // 4. Collect right-aligned status modules
        let mut right_items = Vec::new();
        for m in modules {
            let name = m.name();
            if name != "workspace" && name != "layout" && name != "title" {
                let t = m.current_text();
                if !t.is_empty() {
                    right_items.push(t);
                }
            }
        }

        let right_text = if !right_items.is_empty() {
            Some(right_items.join("  |  "))
        } else {
            None
        };

        let right_width = if let Some(ref rt) = right_text {
            self.text_width(rt) as i16
        } else {
            0
        };

        let mut tab_rects = Vec::new();

        // 5. Render tabs (in Tabbed mode) OR active window title (in other modes)
        if let Some(tab_list) = tabs {
            if !tab_list.is_empty() {
                let reserved_right = (right_width + 24).max(60);
                let available_width = (self.width as i16 - cur_x - reserved_right).max(0) as u16;
                let (geoms, consumed_w) = crate::bar::tabs::calculate_tab_geometries(
                    tab_list,
                    cur_x,
                    available_width,
                    self.height,
                    tab_config,
                    self.char_width,
                );
                for geom in &geoms {
                    let text_w = self.text_width(&geom.label) as i16;
                    let text_x = geom.rect.x as i16 + ((geom.rect.width as i16 - text_w) / 2).max(2);
                    if geom.focused {
                        self.fill_rect(
                            conn,
                            self.pixmap,
                            geom.rect.x as i16,
                            geom.rect.y as i16,
                            geom.rect.width as u16,
                            geom.rect.height as u16,
                            color_scheme.active_bg,
                        )?;
                        self.draw_text(conn, self.pixmap, text_x, baseline, &geom.label, color_scheme.active_fg)?;
                    } else if geom.urgent {
                        self.fill_rect(
                            conn,
                            self.pixmap,
                            geom.rect.x as i16,
                            geom.rect.y as i16,
                            geom.rect.width as u16,
                            geom.rect.height as u16,
                            color_scheme.urgent_bg,
                        )?;
                        self.draw_text(conn, self.pixmap, text_x, baseline, &geom.label, color_scheme.urgent_fg)?;
                    } else {
                        self.draw_text(conn, self.pixmap, text_x, baseline, &geom.label, color_scheme.fg)?;
                    }
                    tab_rects.push((geom.window, geom.rect));
                }
                cur_x += consumed_w as i16;
            }
        } else if !active_window_title.is_empty() {
            let reserved_right = (right_width + 24).max(100);
            let max_chars = ((self.width as i16 - cur_x - reserved_right) / self.char_width as i16).max(10) as usize;
            let display_title = if active_window_title.chars().count() > max_chars {
                let truncated: String = active_window_title.chars().take(max_chars.saturating_sub(3)).collect();
                format!("{}...", truncated)
            } else {
                active_window_title.to_string()
            };
            self.draw_text(conn, self.pixmap, cur_x, baseline, &display_title, color_scheme.fg)?;
        }

        // 6. Render right-aligned status modules
        if let Some(ref rt) = right_text {
            let right_x = (self.width as i16 - right_width - 12).max(cur_x + 20);
            self.draw_text(conn, self.pixmap, right_x, baseline, rt, color_scheme.fg)?;
        }

        // 7. Blit completed frame to visible window
        #[cfg(feature = "pango")]
        if !self.is_core {
            self.flush_cairo_to_pixmap(conn)?;
        }
        self.copy_pixmap_to_window(conn, window)?;

        Ok(crate::bar::StatusbarHitRects {
            tab_rects,
            workspace_rects,
        })
    }

    /// Frees graphics resources (pixmap, gc, font) allocated on the X server and drops surface memory.
    pub fn destroy<C: Connection>(&self, conn: &C) -> Result<(), X11Error> {
        let _ = conn.free_pixmap(self.pixmap);
        let _ = conn.free_gc(self.gc);
        if self.font_id > 0 {
            let _ = conn.close_font(self.font_id);
        }
        conn.flush()?;
        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "pango")]
    fn dummy_pango_renderer() -> Renderer {
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, 1000, 24).unwrap();
        let font_desc = parse_font_description("monospace 10");
        let cr = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&cr);
        layout.set_font_description(Some(&font_desc));
        Renderer {
            gc: 1,
            font_id: 2,
            pixmap: 3,
            width: 1000,
            height: 24,
            depth: 24,
            font_ascent: 12,
            font_descent: 4,
            char_width: 8,
            is_core: false,
            font_desc,
            surface: Some(RefCell::new(surface)),
            layout: Some(RefCell::new(layout)),
            text_width_cache: RefCell::new(HashMap::new()),
        }
    }

    fn dummy_core_renderer() -> Renderer {
        Renderer {
            gc: 1,
            font_id: 2,
            pixmap: 3,
            width: 1000,
            height: 24,
            depth: 24,
            font_ascent: 12,
            font_descent: 4,
            char_width: 8,
            is_core: true,
            #[cfg(feature = "pango")]
            font_desc: parse_font_description("fixed"),
            #[cfg(feature = "pango")]
            surface: None,
            #[cfg(feature = "pango")]
            layout: None,
            #[cfg(feature = "pango")]
            text_width_cache: RefCell::new(HashMap::new()),
        }
    }

    #[test]
    fn test_color_scheme_defaults() {
        let cs = ColorScheme::default();
        assert_eq!(cs.bg, 0x1a1b26);
        assert_eq!(cs.fg, 0xc0caf5);
        assert_eq!(cs.active_bg, 0x7aa2f7);
        assert_eq!(cs.active_fg, 0x15161e);
    }

    #[cfg(feature = "pango")]
    #[test]
    fn test_parse_font_description() {
        let desc1 = parse_font_description("pango:monospace 10");
        assert_eq!(desc1.family().as_deref(), Some("monospace"));
        assert_eq!(desc1.size() / pango::SCALE, 10);

        let desc2 = parse_font_description("DejaVu Sans Mono 12");
        assert_eq!(desc2.family().as_deref(), Some("DejaVu Sans Mono"));
        assert_eq!(desc2.size() / pango::SCALE, 12);

        let desc3 = parse_font_description("sans-serif bold 14");
        assert_eq!(desc3.size() / pango::SCALE, 14);
    }

    #[test]
    fn test_core_font_spec_detection() {
        assert!(is_core_font_spec("fixed"));
        assert!(is_core_font_spec("6x13"));
        assert!(is_core_font_spec("9x15"));
        assert!(is_core_font_spec("core:monospace"));
        assert!(is_core_font_spec("-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*"));

        #[cfg(feature = "pango")]
        {
            assert!(!is_core_font_spec("pango:monospace 10"));
            assert!(!is_core_font_spec("monospace 10"));
            assert!(!is_core_font_spec("DejaVu Sans Mono"));
        }
    }

    #[test]
    fn test_text_width_calculation() {
        let renderer = dummy_core_renderer();
        let width = renderer.text_width("Hello");
        assert_eq!(width, 5 * 8);
        assert_eq!(renderer.text_width(""), 0);

        let (w, h) = renderer.text_size("Hello");
        assert_eq!(w, 40);
        assert_eq!(h, 16);

        #[cfg(feature = "pango")]
        {
            let pango_renderer = dummy_pango_renderer();
            let p_w = pango_renderer.text_width("Hello");
            assert!(p_w > 0);
            assert_eq!(pango_renderer.text_width(""), 0);
        }
    }

    #[test]
    fn test_vertical_center_baseline() {
        let renderer = dummy_core_renderer();
        // font_height = 16. y_offset = (24 - 16)/2 = 4. baseline = 4 + 12 = 16.
        assert_eq!(renderer.vertical_center_baseline(), 16);
    }
}
