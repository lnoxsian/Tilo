//! Statusbar and text rendering pipeline.
//!
//! Implements Phase 18 of Plan.md:
//! - Lightweight text and shape rendering without external GUI frameworks.
//! - X11 core font loading and baseline metrics calculation ([`Renderer::load_font`]).
//! - Double-buffered rendering via off-screen `Pixmap` to eliminate visual flickering.
//! - Workspace highlighting (active workspace pill background and text).
//! - Active window title indication.
//! - Status metrics rendering (RAM, CPU, Clock, Layout).

use crate::layout::Rect;
use crate::statusbar::{ColorScheme, StatusModule, StatusbarError, TiloError};
use crate::x11::X11Error;
use pangocairo::pango::{self, FontDescription};
use std::cell::RefCell;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt, CreateGCAux, ImageFormat, Rectangle,
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

/// Responsible for rendering text and shapes to the statusbar or launcher window.
#[derive(Debug)]
pub struct Renderer {
    /// The X11 graphics context used for drawing.
    pub gc: u32,
    /// The legacy loaded font identifier.
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
    /// Parsed Pango font description.
    pub font_desc: FontDescription,
    /// Off-screen Cairo image surface matching the pixmap dimensions.
    surface: RefCell<cairo::ImageSurface>,
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
        if let Ok((font_id, _, _, _)) = Self::try_open_font(conn, font_name) {
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

    /// Creates a new `Renderer` initialized with an off-screen pixmap, Cairo surface, and graphics context.
    ///
    /// # Arguments
    /// * `conn` - The active X11 connection.
    /// * `drawable` - The reference window or root window.
    /// * `width` - The width of the statusbar in pixels.
    /// * `height` - The height of the statusbar in pixels.
    /// * `depth` - The visual depth (typically 24).
    /// * `font_name` - The desired font name.
    pub fn new(
        conn: &impl Connection,
        drawable: u32,
        width: u16,
        height: u16,
        depth: u8,
        font_name: &str,
    ) -> Result<Self, StatusbarError> {
        let (font_id, font_ascent, font_descent, char_width) =
            if let Ok(res) = Self::try_open_font(conn, font_name) {
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

        let font_desc = parse_font_description(font_name);
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, width as i32, height as i32)
            .map_err(|e| StatusbarError::RenderError(format!("Failed to create Cairo surface: {}", e)))?;

        // Calculate accurate font metrics using Pango
        let (pango_ascent, pango_descent, pango_char_w) = {
            if let Ok(cr) = cairo::Context::new(&surface) {
                let layout = pangocairo::functions::create_layout(&cr);
                layout.set_font_description(Some(&font_desc));
                layout.set_text("M");
                let (cw, ch) = layout.pixel_size();
                let baseline = (layout.baseline() / pango::SCALE) as i16;
                let ascent = baseline.max(1);
                let descent = (ch as i16 - baseline).max(0);
                (ascent, descent, (cw as u16).max(1))
            } else {
                (font_ascent, font_descent, char_width)
            }
        };

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
            font_desc,
            surface: RefCell::new(surface),
        })
    }

    /// Draws a string of text with Pango anti-aliased font rendering onto the Cairo surface.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `drawable` - The target window or pixmap ID.
    /// * `x` - The starting X coordinate.
    /// * `y` - The baseline Y coordinate.
    /// * `text` - The UTF-8 string to draw.
    /// * `color` - The text color in 0xRRGGBB format.
    pub fn draw_text(
        &self,
        conn: &impl Connection,
        _drawable: u32,
        x: i16,
        y: i16,
        text: &str,
        color: u32,
    ) -> Result<(), TiloError> {
        if text.is_empty() {
            return Ok(());
        }

        let (r, g, b) = color_to_rgb(color);
        let surface = self.surface.borrow();
        if let Ok(cr) = cairo::Context::new(&*surface) {
            cr.set_source_rgb(r, g, b);
            let layout = pangocairo::functions::create_layout(&cr);
            layout.set_font_description(Some(&self.font_desc));
            layout.set_text(text);

            let baseline_px = (layout.baseline() / pango::SCALE) as i16;
            let top_y = y - baseline_px;
            cr.move_to(x as f64, top_y as f64);
            pangocairo::functions::show_layout(&cr, &layout);
        }

        let aux = ChangeGCAux::new().foreground(color);
        let _ = conn.change_gc(self.gc, &aux);

        Ok(())
    }

    /// Fills a solid rectangular region on the Cairo surface and updates the underlying X11 drawable.
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
        let (r, g, b) = color_to_rgb(color);
        let surface = self.surface.borrow();
        if let Ok(cr) = cairo::Context::new(&*surface) {
            cr.set_source_rgb(r, g, b);
            cr.rectangle(x as f64, y as f64, width as f64, height as f64);
            let _ = cr.fill();
        }

        let aux = ChangeGCAux::new().foreground(color);
        let _ = conn.change_gc(self.gc, &aux);
        let _ = conn.poly_fill_rectangle(
            drawable,
            self.gc,
            &[Rectangle {
                x,
                y,
                width,
                height,
            }],
        );
        Ok(())
    }

    /// Calculates the horizontal pixel width of a given text string using Pango.
    pub fn text_width(&self, text: &str) -> u16 {
        if text.is_empty() {
            return 0;
        }
        let surface = self.surface.borrow();
        if let Ok(cr) = cairo::Context::new(&*surface) {
            let layout = pangocairo::functions::create_layout(&cr);
            layout.set_font_description(Some(&self.font_desc));
            layout.set_text(text);
            let (w, _) = layout.pixel_size();
            w as u16
        } else {
            (text.chars().count() as u16) * self.char_width
        }
    }

    /// Calculates the pixel dimensions (width, height) of a given text string using Pango.
    pub fn text_size(&self, text: &str) -> (u16, u16) {
        if text.is_empty() {
            return (0, 0);
        }
        let surface = self.surface.borrow();
        if let Ok(cr) = cairo::Context::new(&*surface) {
            let layout = pangocairo::functions::create_layout(&cr);
            layout.set_font_description(Some(&self.font_desc));
            layout.set_text(text);
            let (w, h) = layout.pixel_size();
            (w as u16, h as u16)
        } else {
            ((text.chars().count() as u16) * self.char_width, (self.font_ascent + self.font_descent) as u16)
        }
    }

    /// Computes the vertical baseline position centered vertically within the statusbar.
    pub fn vertical_center_baseline(&self) -> i16 {
        let font_height = self.font_ascent + self.font_descent;
        let y_offset = (self.height as i16 - font_height) / 2;
        y_offset + self.font_ascent
    }

    /// Resizes the off-screen double-buffer pixmap and Cairo surface to match the new dimensions.
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

        let new_surface = cairo::ImageSurface::create(cairo::Format::Rgb24, new_width as i32, new_height as i32)
            .map_err(|e| StatusbarError::RenderError(format!("Failed to resize Cairo surface: {}", e)))?;

        self.pixmap = new_pixmap;
        self.width = new_width;
        self.height = new_height;
        self.depth = depth;
        *self.surface.borrow_mut() = new_surface;
        Ok(())
    }

    /// Synchronizes the Cairo image surface to the X11 double-buffer pixmap using `put_image`.
    pub fn flush_cairo_to_pixmap(&self, conn: &impl Connection) -> Result<(), StatusbarError> {
        let mut surface = self.surface.borrow_mut();
        surface.flush();
        let data = surface.data().map_err(|e| {
            StatusbarError::RenderError(format!("Failed to borrow Cairo surface data: {}", e))
        })?;

        let depth = if self.depth > 0 { self.depth } else { 24 };
        conn.put_image(
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
        conn.flush()?;
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
        let _ = self.flush_cairo_to_pixmap(conn);
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

        // 2. Render workspaces 1 through 9 with active workspace highlighting
        for ws in 1..=9 {
            let is_active = ws == current_workspace;
            let is_occupied = occupied_workspaces.contains(&ws);

            let ws_str = format!("{}", ws);
            let ws_text_w = self.text_width(&ws_str);
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
                    &ws_str,
                    color_scheme.active_fg,
                )?;
            } else if is_occupied {
                // Occupied workspace indicator
                self.draw_text(
                    conn,
                    self.pixmap,
                    cur_x + pad,
                    baseline,
                    &ws_str,
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
                    &ws_str,
                    color_scheme.inactive_fg, // Configurable dimmed text for empty workspaces
                )?;
            }

            cur_x += pill_w as i16 + 4;
        }

        // 3. Render layout symbol
        cur_x += 6;
        let layout_text = format!("[{}]", layout_symbol);
        self.draw_text(conn, self.pixmap, cur_x, baseline, &layout_text, color_scheme.active_bg)?;
        cur_x += self.text_width(&layout_text) as i16 + 12;

        // 4. Collect right-aligned status modules (all modules except workspace, layout, and title)
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
        self.flush_to_window(conn, window)?;

        Ok(crate::bar::StatusbarHitRects {
            tab_rects,
            workspace_rects,
        })
    }

    /// Frees graphics resources (pixmap, gc, font) allocated on the X server.
    pub fn destroy<C: Connection>(&self, conn: &C) -> Result<(), X11Error> {
        let _ = conn.free_pixmap(self.pixmap);
        let _ = conn.free_gc(self.gc);
        let _ = conn.close_font(self.font_id);
        conn.flush()?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_renderer() -> Renderer {
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, 1000, 24).unwrap();
        let font_desc = parse_font_description("monospace 10");
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
            font_desc,
            surface: RefCell::new(surface),
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
    fn test_text_width_calculation() {
        let renderer = dummy_renderer();
        let width = renderer.text_width("Hello");
        assert!(width > 0, "text_width for non-empty string should be positive");
        assert_eq!(renderer.text_width(""), 0);

        let (w, h) = renderer.text_size("Hello");
        assert_eq!(w, width);
        assert!(h > 0, "text_height should be positive");
    }

    #[test]
    fn test_vertical_center_baseline() {
        let renderer = dummy_renderer();
        // font_height = 16. y_offset = (24 - 16)/2 = 4. baseline = 4 + 12 = 16.
        assert_eq!(renderer.vertical_center_baseline(), 16);
    }
}
