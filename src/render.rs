//! Statusbar and text rendering pipeline.
//!
//! Implements Phase 18 of Plan.md:
//! - Lightweight text and shape rendering without external GUI frameworks.
//! - X11 core font loading and baseline metrics calculation ([`Renderer::load_font`]).
//! - Double-buffered rendering via off-screen [`Pixmap`] to eliminate visual flickering.
//! - Workspace highlighting (active workspace pill background and text).
//! - Active window title indication.
//! - Status metrics rendering (RAM, CPU, Clock, Layout).

use crate::statusbar::{ColorScheme, StatusModule, StatusbarError, TiloError};
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt, CreateGCAux, Rectangle,
};

/// Responsible for rendering text and shapes to the statusbar or launcher window.
#[derive(Debug)]
pub struct Renderer {
    /// The X11 graphics context used for drawing.
    pub gc: u32,
    /// The loaded font identifier.
    pub font_id: u32,
    /// Reusable double-buffer pixmap to prevent flickering.
    pub pixmap: u32,
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
}

impl Renderer {
    /// Loads an X11 font by name with automatic fallback to standard monospace fonts.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `font_name` - The logical font name (e.g., "fixed", "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*").
    ///
    /// # Returns
    /// A font ID, or a [`TiloError::FontLoadError`] if no font can be loaded.
    pub fn load_font(conn: &impl Connection, font_name: &str) -> Result<u32, TiloError> {
        let font_id = conn.generate_id()?;

        // 1. Try requested font name
        if conn.open_font(font_id, font_name.as_bytes()).is_ok() && conn.flush().is_ok() {
            return Ok(font_id);
        }

        // 2. Try standard X11 "fixed" font
        let fallback_fixed = conn.generate_id()?;
        if conn.open_font(fallback_fixed, b"fixed").is_ok() && conn.flush().is_ok() {
            return Ok(fallback_fixed);
        }

        // 3. Try standard X11 "9x15" font
        let fallback_9x15 = conn.generate_id()?;
        if conn.open_font(fallback_9x15, b"9x15").is_ok() && conn.flush().is_ok() {
            return Ok(fallback_9x15);
        }

        // 4. Try standard X11 "6x13" font
        let fallback_6x13 = conn.generate_id()?;
        if conn.open_font(fallback_6x13, b"6x13").is_ok() && conn.flush().is_ok() {
            return Ok(fallback_6x13);
        }

        Err(StatusbarError::FontLoadError(format!(
            "Failed to load font '{}' and all fallbacks",
            font_name
        )))
    }

    /// Creates a new `Renderer` initialized with an off-screen pixmap and graphics context.
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
        let font_id = Self::load_font(conn, font_name)?;

        let (font_ascent, font_descent, char_width) = if let Ok(reply) = conn.query_font(font_id)?.reply() {
            let ascent = reply.font_ascent;
            let descent = reply.font_descent;
            let width = if reply.max_bounds.character_width > 0 {
                reply.max_bounds.character_width as u16
            } else {
                8
            };
            (ascent, descent, width)
        } else {
            (11, 3, 7)
        };

        let gc = conn.generate_id()?;
        let gc_aux = CreateGCAux::new()
            .font(font_id)
            .graphics_exposures(0);
        conn.create_gc(gc, drawable, &gc_aux)?;

        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, drawable, width, height)?;
        conn.flush()?;

        Ok(Self {
            gc,
            font_id,
            pixmap,
            width,
            height,
            font_ascent,
            font_descent,
            char_width,
        })
    }

    /// Draws a string of text at the specified baseline coordinates.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `drawable` - The target window or pixmap ID.
    /// * `x` - The starting X coordinate.
    /// * `y` - The starting Y (baseline) coordinate.
    /// * `text` - The UTF-8 string to draw.
    /// * `color` - The text color in 0xRRGGBB format.
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
        let aux = ChangeGCAux::new().foreground(color);
        conn.change_gc(self.gc, &aux)?;
        conn.image_text8(drawable, self.gc, x, y, text.as_bytes())?;
        Ok(())
    }

    /// Fills a solid rectangular region with the specified color.
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
        let aux = ChangeGCAux::new().foreground(color);
        conn.change_gc(self.gc, &aux)?;
        conn.poly_fill_rectangle(
            drawable,
            self.gc,
            &[Rectangle {
                x,
                y,
                width,
                height,
            }],
        )?;
        Ok(())
    }

    /// Calculates the horizontal pixel width of a given text string.
    pub fn text_width(&self, text: &str) -> u16 {
        text.chars().count() as u16 * self.char_width
    }

    /// Computes the vertical baseline position centered vertically within the statusbar.
    pub fn vertical_center_baseline(&self) -> i16 {
        let font_height = self.font_ascent + self.font_descent;
        let y_offset = (self.height as i16 - font_height) / 2;
        y_offset + self.font_ascent
    }

    /// Blits the fully rendered off-screen pixmap onto the target window in a single atomic frame.
    pub fn flush_to_window(&self, conn: &impl Connection, window: u32) -> Result<(), StatusbarError> {
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

    /// Renders the entire statusbar: background, workspace indicators, layout indicator,
    /// active window title, and right-aligned system modules into the double-buffer pixmap,
    /// then blits the result to the statusbar window.
    pub fn render_statusbar(
        &self,
        conn: &impl Connection,
        window: u32,
        current_workspace: u32,
        occupied_workspaces: &[u32],
        layout_symbol: &str,
        active_window_title: &str,
        modules: &[Box<dyn StatusModule>],
        color_scheme: &ColorScheme,
    ) -> Result<(), StatusbarError> {
        // 1. Clear background of pixmap
        self.fill_rect(conn, self.pixmap, 0, 0, self.width, self.height, color_scheme.bg)?;

        let baseline = self.vertical_center_baseline();
        let mut cur_x = 8i16;

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
                    0x555566, // Dimmed text for empty workspaces
                )?;
            }

            cur_x += pill_w as i16 + 4;
        }

        // 3. Render layout symbol
        cur_x += 6;
        let layout_text = format!("[{}]", layout_symbol);
        self.draw_text(conn, self.pixmap, cur_x, baseline, &layout_text, color_scheme.active_bg)?;
        cur_x += self.text_width(&layout_text) as i16 + 12;

        // 4. Render active window title
        if !active_window_title.is_empty() {
            let max_chars = ((self.width as i16 - cur_x - 300) / self.char_width as i16).max(10) as usize;
            let display_title = if active_window_title.chars().count() > max_chars {
                let truncated: String = active_window_title.chars().take(max_chars.saturating_sub(3)).collect();
                format!("{}...", truncated)
            } else {
                active_window_title.to_string()
            };
            self.draw_text(conn, self.pixmap, cur_x, baseline, &display_title, color_scheme.fg)?;
        }

        // 5. Render right-aligned status modules (RAM, CPU, Clock)
        let mut right_items = Vec::new();
        for m in modules {
            let name = m.name();
            if name == "ram" || name == "cpu" || name == "clock" {
                let t = m.current_text();
                if !t.is_empty() {
                    right_items.push(t);
                }
            }
        }

        if !right_items.is_empty() {
            let right_text = right_items.join("  |  ");
            let right_width = self.text_width(&right_text);
            let right_x = (self.width as i16 - right_width as i16 - 12).max(cur_x + 20);
            self.draw_text(conn, self.pixmap, right_x, baseline, &right_text, color_scheme.fg)?;
        }

        // 6. Blit completed frame to visible window
        self.flush_to_window(conn, window)?;

        Ok(())
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

impl Drop for Renderer {
    fn drop(&mut self) {
        // Drop cleans up local Rust memory; X server handles cleanup if connection drops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_scheme_defaults() {
        let cs = ColorScheme::default();
        assert_eq!(cs.bg, 0x1a1b26);
        assert_eq!(cs.fg, 0xc0caf5);
        assert_eq!(cs.active_bg, 0x7aa2f7);
        assert_eq!(cs.active_fg, 0x15161e);
    }

    #[test]
    fn test_text_width_calculation() {
        let renderer = Renderer {
            gc: 1,
            font_id: 2,
            pixmap: 3,
            width: 1000,
            height: 24,
            font_ascent: 12,
            font_descent: 4,
            char_width: 8,
        };
        assert_eq!(renderer.text_width("Hello"), 40);
        assert_eq!(renderer.text_width(""), 0);
    }

    #[test]
    fn test_vertical_center_baseline() {
        let renderer = Renderer {
            gc: 1,
            font_id: 2,
            pixmap: 3,
            width: 1000,
            height: 24,
            font_ascent: 12,
            font_descent: 4,
            char_width: 8,
        };
        // font_height = 16. y_offset = (24 - 16)/2 = 4. baseline = 4 + 12 = 16.
        assert_eq!(renderer.vertical_center_baseline(), 16);
    }
}
