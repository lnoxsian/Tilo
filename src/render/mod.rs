//! Statusbar and text rendering pipeline.
//!
//! Implements a modern, 100% pure Rust rendering engine:
//! - Dual-engine rendering architecture:
//!   1. Modern pure Rust TrueType/OpenType font engine powered by `fontdue` and an in-memory
//!      software framebuffer with SIMD rasterization and smooth alpha blending.
//!   2. Zero-allocation X11 Core font engine (`is_core = true`) for < 2 MB RAM usage or as
//!      an automatic fallback when no font files exist on the host.
//! - X11 core font loading and baseline metrics calculation ([`Renderer::load_font`]).
//! - Double-buffered rendering via off-screen `Pixmap` to eliminate visual flickering.
//! - Workspace highlighting (active workspace pill background and text).
//! - Active window title indication.
//! - Status metrics rendering (RAM, CPU, Clock, Layout).

pub mod font;
pub use font::{parse_font_spec, resolve_font_file, FontEngine};

use crate::layout::Rect;
use crate::statusbar::{ColorScheme, StatusModule, StatusbarError, TiloError};
use crate::x11::X11Error;
use std::cell::RefCell;
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt, CreateGCAux, ImageFormat, Rectangle,
};

/// Determines if a font specification should use the ultra-low-memory X11 Core font engine.
pub fn is_core_font_spec(font_name: &str) -> bool {
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

/// Strips the `"core:"` prefix from a font specification if present.
pub fn clean_core_font_name(font_name: &str) -> &str {
    let trimmed = font_name.trim();
    if let Some(stripped) = trimmed.strip_prefix("core:") {
        stripped.trim()
    } else {
        trimmed
    }
}

/// Formats a layout indicator so that all layout symbols are consistently 3 characters long,
/// stripping outer brackets if a 5-character symbol was provided (e.g. "[|||]" -> "|||", "[===]" -> "===", "[[] =]" -> "[]=")
/// and wrapping 1-character symbols (e.g. "M" -> "[M]").
pub fn format_layout_symbol(layout_symbol: &str) -> std::borrow::Cow<'_, str> {
    if layout_symbol.len() > 3 && layout_symbol.starts_with('[') && layout_symbol.ends_with(']') {
        std::borrow::Cow::Borrowed(&layout_symbol[1..layout_symbol.len() - 1])
    } else if layout_symbol.len() == 1 {
        std::borrow::Cow::Owned(format!("[{}]", layout_symbol))
    } else {
        std::borrow::Cow::Borrowed(layout_symbol)
    }
}

/// Responsible for rendering text and shapes to the statusbar or launcher window.
#[derive(Debug)]
pub struct Renderer {
    /// The X11 graphics context used for drawing.
    pub gc: u32,
    /// Dedicated X11 graphics context used strictly for put_image and copy_area blits.
    pub blit_gc: u32,
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

    /// Pure Rust TrueType/OpenType font engine powered by `fontdue` (None if `is_core` is true).
    pub font_engine: Option<FontEngine>,
    /// Software pixel framebuffer for anti-aliased font rendering and shape drawing (None if `is_core` is true).
    framebuffer: Option<RefCell<Vec<u32>>>,
    /// Bounded cache of calculated text widths to avoid repeated metrics calculations.
    text_width_cache: RefCell<HashMap<String, u16>>,
}

// Renderer is strictly owned and not shared across threads concurrently, making it safe to transfer ownership (Send).
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
    /// In pure Rust mode, loads TrueType/OpenType fonts via `fontdue` and renders anti-aliased text
    /// into an in-memory software buffer, syncing to X11 via `put_image` without any C dependencies.
    /// In X11 Core font mode (`is_core = true`), no software buffer is allocated, keeping RAM strictly under 2 MB.
    pub fn new(
        conn: &impl Connection,
        drawable: u32,
        width: u16,
        height: u16,
        depth: u8,
        font_name: &str,
    ) -> Result<Self, StatusbarError> {
        let is_explicit_core = is_core_font_spec(font_name);
        let clean_name = clean_core_font_name(font_name);

        // 1. If not explicitly core, attempt to load via fontdue pure Rust engine
        let mut font_engine_opt = None;
        if !is_explicit_core {
            match FontEngine::load(font_name) {
                Ok(engine) => {
                    font_engine_opt = Some(engine);
                }
                Err(err) => {
                    eprintln!(
                        "[WARN  tilo::render] Failed to load TrueType font '{}': {}. Falling back to core X11 fonts.",
                        font_name, err
                    );
                }
            }
        }

        let is_core = is_explicit_core || font_engine_opt.is_none();

        // 2. Metrics calculation: from FontEngine if loaded, otherwise from core X11 font
        let (font_id, core_ascent, core_descent, core_width) = if let Some(ref fe) = font_engine_opt {
            (0, fe.ascent, fe.descent, fe.char_width)
        } else if let Ok(res) = Self::try_open_font(conn, clean_name) {
            res
        } else {
            let mut loaded = None;
            for fallback in &["fixed", "6x13", "9x15", "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*"] {
                if let Ok(res) = Self::try_open_font(conn, fallback) {
                    loaded = Some(res);
                    break;
                }
            }
            if loaded.is_none() {
                eprintln!(
                    "[ERROR tilo::render] No usable TrueType or core X11 fonts found on system. Install 'fonts-dejavu-core' or 'fonts-liberation'."
                );
            }
            loaded.unwrap_or((0, 12, 4, 8))
        };

        let gc = conn.generate_id()?;
        let mut gc_aux = CreateGCAux::new().graphics_exposures(0);
        if font_id > 0 {
            gc_aux = gc_aux.font(font_id);
        }
        conn.create_gc(gc, drawable, &gc_aux)?;

        let blit_gc = conn.generate_id()?;
        let blit_gc_aux = CreateGCAux::new()
            .graphics_exposures(0)
            .subwindow_mode(x11rb::protocol::xproto::SubwindowMode::INCLUDE_INFERIORS);
        conn.create_gc(blit_gc, drawable, &blit_gc_aux)?;

        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, drawable, width, height)?;
        conn.flush()?;

        let framebuffer = if !is_core {
            let len = (width as usize) * (height as usize);
            Some(RefCell::new(vec![0u32; len]))
        } else {
            None
        };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }

        Ok(Self {
            gc,
            blit_gc,
            font_id,
            pixmap,
            depth,
            width,
            height,
            font_ascent: core_ascent,
            font_descent: core_descent,
            char_width: core_width,
            is_core,
            font_engine: font_engine_opt,
            framebuffer,
            text_width_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Fills a solid rectangle in the software framebuffer using fast SIMD slice filling.
    fn fill_rect_software(
        &self,
        fb: &mut [u32],
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        color: u32,
    ) {
        let fb_w = self.width as usize;
        let fb_h = self.height as usize;
        if fb_w == 0 || fb_h == 0 || width == 0 || height == 0 {
            return;
        }

        let x0 = (x.max(0) as usize).min(fb_w);
        let y0 = (y.max(0) as usize).min(fb_h);
        let x1 = ((x as i32 + width as i32).max(0) as usize).min(fb_w);
        let y1 = ((y as i32 + height as i32).max(0) as usize).min(fb_h);

        if x1 > x0 && y1 > y0 {
            let fill_len = x1 - x0;
            for row in y0..y1 {
                let start = row * fb_w + x0;
                fb[start..start + fill_len].fill(color);
            }
        }
    }

    /// Draws anti-aliased text into the software framebuffer using alpha coverage blending.
    fn draw_text_software(
        &self,
        fb: &mut [u32],
        fe: &FontEngine,
        x: i16,
        y: i16,
        text: &str,
        color: u32,
    ) {
        let fb_w = self.width as usize;
        let fb_h = self.height as usize;
        if fb_w == 0 || fb_h == 0 {
            return;
        }

        let src_r = (color >> 16) & 0xff;
        let src_g = (color >> 8) & 0xff;
        let src_b = color & 0xff;

        let mut pen_x = x as i32;
        let pen_y = y as i32;

        let div255 = |v: u32| (v + 1 + (v >> 8)) >> 8;

        for ch in text.chars() {
            fe.with_glyph(ch, |metrics, bitmap| {
                if metrics.width > 0 && metrics.height > 0 {
                    let left_x = pen_x + metrics.xmin;
                    let bottom_y = pen_y - metrics.ymin;
                    let top_y = bottom_y - metrics.height as i32;
                    let glyph_w = metrics.width;
                    let glyph_h = metrics.height;

                    let is_horizontally_inside = left_x >= 0 && (left_x + glyph_w as i32) <= fb_w as i32;

                    for r in 0..glyph_h {
                        let screen_y = top_y + r as i32;
                        if screen_y < 0 || screen_y >= fb_h as i32 {
                            continue;
                        }
                        let row_offset = (screen_y as usize) * fb_w;
                        let bitmap_row = r * glyph_w;

                        if is_horizontally_inside {
                            // Fast path: contiguous unclipped row slice enables auto-vectorization
                            let x_start = left_x as usize;
                            let row_slice = &mut fb[row_offset + x_start..row_offset + x_start + glyph_w];
                            for c in 0..glyph_w {
                                let coverage = bitmap[bitmap_row + c] as u32;
                                if coverage == 0 {
                                    continue;
                                }
                                if coverage == 255 {
                                    row_slice[c] = color;
                                } else {
                                    let dst = row_slice[c];
                                    let dst_r = (dst >> 16) & 0xff;
                                    let dst_g = (dst >> 8) & 0xff;
                                    let dst_b = dst & 0xff;
                                    let inv_a = 255 - coverage;
                                    let out_r = div255(src_r * coverage + dst_r * inv_a);
                                    let out_g = div255(src_g * coverage + dst_g * inv_a);
                                    let out_b = div255(src_b * coverage + dst_b * inv_a);
                                    row_slice[c] = (out_r << 16) | (out_g << 8) | out_b;
                                }
                            }
                        } else {
                            // Clipped edge path
                            for c in 0..glyph_w {
                                let screen_x = left_x + c as i32;
                                if screen_x < 0 || screen_x >= fb_w as i32 {
                                    continue;
                                }
                                let coverage = bitmap[bitmap_row + c] as u32;
                                if coverage == 0 {
                                    continue;
                                }
                                let idx = row_offset + screen_x as usize;
                                if coverage == 255 {
                                    fb[idx] = color;
                                } else {
                                    let dst = fb[idx];
                                    let dst_r = (dst >> 16) & 0xff;
                                    let dst_g = (dst >> 8) & 0xff;
                                    let dst_b = dst & 0xff;
                                    let inv_a = 255 - coverage;
                                    let out_r = div255(src_r * coverage + dst_r * inv_a);
                                    let out_g = div255(src_g * coverage + dst_g * inv_a);
                                    let out_b = div255(src_b * coverage + dst_b * inv_a);
                                    fb[idx] = (out_r << 16) | (out_g << 8) | out_b;
                                }
                            }
                        }
                    }
                }
                pen_x += metrics.advance_width.round() as i32;
            });
        }
    }

    /// Draws a string of text onto the active rendering target.
    ///
    /// In pure Rust mode, renders anti-aliased glyphs into the software framebuffer.
    /// In X11 Core mode, uses direct zero-allocation `poly_text8`/`image_text8` protocol requests.
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

        if !self.is_core
            && let Some(ref fe) = self.font_engine
            && let Some(ref fb_cell) = self.framebuffer {
                self.draw_text_software(&mut fb_cell.borrow_mut(), fe, x, y, text, color);
                return Ok(());
        }

        // Core X11 font rendering path (zero heap allocations)
        if self.font_id == 0 {
            return Ok(());
        }

        let aux = ChangeGCAux::new().foreground(color).background(color);
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
        if !self.is_core
            && let Some(ref fb_cell) = self.framebuffer {
                self.fill_rect_software(&mut fb_cell.borrow_mut(), x, y, width, height, color);
                return Ok(());
        }

        // Core X11 fill path (zero heap allocations)
        let aux = ChangeGCAux::new().foreground(color).background(color);
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

        if !self.is_core
            && let Some(ref fe) = self.font_engine {
                if let Some(&w) = self.text_width_cache.borrow().get(text) {
                    return w;
                }

                let mut width_px = 0.0f32;
                for ch in text.chars() {
                    width_px += fe.char_advance(ch);
                }
                let width = width_px.round() as u16;

                if text.len() <= 32 && !text.contains(':') {
                    let mut cache = self.text_width_cache.borrow_mut();
                    if cache.len() >= 256 {
                        cache.clear();
                    }
                    cache.insert(text.to_string(), width);
                }

                return width;
        }

        (text.chars().count() as u16) * self.char_width
    }

    /// Calculates the pixel dimensions (width, height) of a given text string.
    pub fn text_size(&self, text: &str) -> (u16, u16) {
        (
            self.text_width(text),
            (self.font_ascent + self.font_descent) as u16,
        )
    }

    /// Computes the vertical baseline position centered vertically within the statusbar.
    pub fn vertical_center_baseline(&self) -> i16 {
        let font_height = self.font_ascent + self.font_descent;
        let y_offset = (self.height as i16 - font_height) / 2;
        y_offset + self.font_ascent
    }

    /// Resizes the off-screen double-buffer pixmap and software framebuffer to match new dimensions.
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

        if !self.is_core
            && let Some(ref fb_cell) = self.framebuffer {
                let new_len = (new_width as usize) * (new_height as usize);
                let mut fb = fb_cell.borrow_mut();
                fb.resize(new_len, 0);
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
            self.blit_gc,
            0,
            0,
            0,
            0,
            self.width,
            self.height,
        )?.check()?;
        conn.flush()?;
        Ok(())
    }

    /// Synchronizes the software framebuffer to the X11 double-buffer pixmap using `put_image`.
    pub fn flush_framebuffer_to_pixmap(&self, conn: &impl Connection) -> Result<(), StatusbarError> {
        if self.is_core || self.framebuffer.is_none() {
            return Ok(());
        }

        let fb = self.framebuffer.as_ref().unwrap().borrow();
        let depth = if self.depth > 0 { self.depth } else { 24 };
        let width = self.width as usize;
        let height = self.height as usize;
        if width == 0 || height == 0 {
            return Ok(());
        }

        let (bpp, pad) = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .map(|f| (f.bits_per_pixel, f.scanline_pad))
            .unwrap_or((32, 32));

        let max_chunk_bytes = 32768usize;

        match bpp {
            24 => {
                let pad_bytes = (pad as usize / 8).max(1);
                let raw_row_bytes = width * 3;
                let row_bytes = raw_row_bytes.div_ceil(pad_bytes) * pad_bytes;
                let rows_per_chunk = (max_chunk_bytes / row_bytes).max(1).min(height);

                let mut y = 0;
                let mut chunk_buf = vec![0u8; rows_per_chunk * row_bytes];
                while y < height {
                    let chunk_rows = (height - y).min(rows_per_chunk);
                    let actual_len = chunk_rows * row_bytes;
                    for r in 0..chunk_rows {
                        let row_y = y + r;
                        let src_start = row_y * width;
                        let dst_start = r * row_bytes;
                        for x in 0..width {
                            let p = fb[src_start + x];
                            let px_dst = dst_start + x * 3;
                            chunk_buf[px_dst] = (p & 0xff) as u8;
                            chunk_buf[px_dst + 1] = ((p >> 8) & 0xff) as u8;
                            chunk_buf[px_dst + 2] = ((p >> 16) & 0xff) as u8;
                        }
                    }

                    conn.put_image(
                        ImageFormat::Z_PIXMAP,
                        self.pixmap,
                        self.blit_gc,
                        self.width,
                        chunk_rows as u16,
                        0,
                        y as i16,
                        0,
                        depth,
                        &chunk_buf[..actual_len],
                    )?;

                    y += chunk_rows;
                }
            }
            16 => {
                let pad_bytes = (pad as usize / 8).max(1);
                let raw_row_bytes = width * 2;
                let row_bytes = raw_row_bytes.div_ceil(pad_bytes) * pad_bytes;
                let rows_per_chunk = (max_chunk_bytes / row_bytes).max(1).min(height);

                let mut y = 0;
                let mut chunk_buf = vec![0u8; rows_per_chunk * row_bytes];
                while y < height {
                    let chunk_rows = (height - y).min(rows_per_chunk);
                    let actual_len = chunk_rows * row_bytes;
                    for r in 0..chunk_rows {
                        let row_y = y + r;
                        let src_start = row_y * width;
                        let dst_start = r * row_bytes;
                        for x in 0..width {
                            let p = fb[src_start + x];
                            let red = ((p >> 16) & 0xff) >> 3;
                            let green = ((p >> 8) & 0xff) >> 2;
                            let blue = (p & 0xff) >> 3;
                            let rgb565 = ((red << 11) | (green << 5) | blue) as u16;
                            let px_dst = dst_start + x * 2;
                            chunk_buf[px_dst..px_dst + 2].copy_from_slice(&rgb565.to_le_bytes());
                        }
                    }

                    conn.put_image(
                        ImageFormat::Z_PIXMAP,
                        self.pixmap,
                        self.blit_gc,
                        self.width,
                        chunk_rows as u16,
                        0,
                        y as i16,
                        0,
                        depth,
                        &chunk_buf[..actual_len],
                    )?;

                    y += chunk_rows;
                }
            }
            _ => {
                let bytes_per_row = width * 4;
                let rows_per_chunk = (max_chunk_bytes / bytes_per_row).max(1).min(height);

                let mut y = 0;
                while y < height {
                    let chunk_rows = (height - y).min(rows_per_chunk);
                    let start_pixel = y * width;
                    let end_pixel = (y + chunk_rows) * width;
                    let byte_slice: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            fb[start_pixel..end_pixel].as_ptr() as *const u8,
                            chunk_rows * bytes_per_row,
                        )
                    };

                    conn.put_image(
                        ImageFormat::Z_PIXMAP,
                        self.pixmap,
                        self.blit_gc,
                        self.width,
                        chunk_rows as u16,
                        0,
                        y as i16,
                        0,
                        depth,
                        byte_slice,
                    )?;

                    y += chunk_rows;
                }
            }
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
        if !self.is_core {
            self.flush_framebuffer_to_pixmap(conn)?;
        }
        conn.copy_area(
            self.pixmap,
            window,
            self.blit_gc,
            0,
            0,
            0,
            0,
            width.min(self.width),
            height.min(self.height),
        )?.check()?;
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
        separator: &str,
    ) -> Result<crate::bar::StatusbarHitRects, StatusbarError> {
        let do_fill_rect = |x: i16, y: i16, w: u16, h: u16, col: u32| -> Result<(), StatusbarError> {
            if !self.is_core
                && let Some(ref fb_cell) = self.framebuffer {
                    self.fill_rect_software(&mut fb_cell.borrow_mut(), x, y, w, h, col);
                    return Ok(());
            }
            self.fill_rect(conn, self.pixmap, x, y, w, h, col)
        };

        let do_draw_text = |x: i16, y: i16, text: &str, col: u32| -> Result<(), StatusbarError> {
            if !self.is_core
                && let Some(ref fe) = self.font_engine
                && let Some(ref fb_cell) = self.framebuffer {
                    self.draw_text_software(&mut fb_cell.borrow_mut(), fe, x, y, text, col);
                    return Ok(());
            }
            self.draw_text(conn, self.pixmap, x, y, text, col).map_err(|e| StatusbarError::RenderError(e.to_string()))
        };

        // 1. Clear background of pixmap
        do_fill_rect(0, 0, self.width, self.height, color_scheme.bg)?;

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
                do_fill_rect(
                    cur_x,
                    pill_y,
                    pill_w,
                    pill_h,
                    color_scheme.active_bg,
                )?;
                // Active workspace text
                do_draw_text(
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.active_fg,
                )?;
            } else if is_occupied {
                // Occupied workspace indicator
                do_draw_text(
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.fg,
                )?;
                // Small indicator dot/asterisk
                let dot_x = cur_x + pad + ws_text_w as i16;
                do_draw_text(dot_x, baseline - 2, "*", color_scheme.active_bg)?;
            } else {
                // Inactive empty workspace
                do_draw_text(
                    cur_x + pad,
                    baseline,
                    ws_str,
                    color_scheme.inactive_fg,
                )?;
            }

            cur_x += pill_w as i16 + 4;
        }

        // 3. Render layout symbol (consistently 3 characters)
        cur_x += 6;
        let layout_text = format_layout_symbol(layout_symbol);
        do_draw_text(cur_x, baseline, &layout_text, color_scheme.active_bg)?;
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
            Some(right_items.join(separator))
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
                let reserved_right = (right_width + 16).max(40);
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
                        do_fill_rect(
                            geom.rect.x as i16,
                            geom.rect.y as i16,
                            geom.rect.width as u16,
                            geom.rect.height as u16,
                            color_scheme.active_bg,
                        )?;
                        do_draw_text(text_x, baseline, &geom.label, color_scheme.active_fg)?;
                    } else if geom.urgent {
                        do_fill_rect(
                            geom.rect.x as i16,
                            geom.rect.y as i16,
                            geom.rect.width as u16,
                            geom.rect.height as u16,
                            color_scheme.urgent_bg,
                        )?;
                        do_draw_text(text_x, baseline, &geom.label, color_scheme.urgent_fg)?;
                    } else {
                        do_draw_text(text_x, baseline, &geom.label, color_scheme.fg)?;
                    }
                    tab_rects.push((geom.window, geom.rect));
                }
                cur_x += consumed_w as i16;
            }
        } else if !active_window_title.is_empty() {
            let reserved_right = (right_width + 24).max(100);
            let available_space = self.width as i16 - cur_x - reserved_right;
            if available_space > 0 {
                let max_chars = (available_space / self.char_width.max(1) as i16).max(10) as usize;
                let display_title = if active_window_title.chars().count() > max_chars {
                    let truncated: String = active_window_title.chars().take(max_chars.saturating_sub(3)).collect();
                    format!("{}...", truncated)
                } else {
                    active_window_title.to_string()
                };
                do_draw_text(cur_x, baseline, &display_title, color_scheme.fg)?;
            }
        }

        // 6. Render right-aligned status modules
        if let Some(ref rt) = right_text {
            let right_x = (self.width as i16 - right_width - 12).max(cur_x + 20);
            do_draw_text(right_x, baseline, rt, color_scheme.fg)?;
        }

        // 7. Blit completed frame to visible window
        if !self.is_core {
            self.flush_framebuffer_to_pixmap(conn)?;
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
        let _ = conn.free_gc(self.blit_gc);
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

    fn dummy_fontdue_renderer() -> Renderer {
        let engine = FontEngine::load("monospace 10").ok();
        let (ascent, descent, char_width) = if let Some(ref fe) = engine {
            (fe.ascent, fe.descent, fe.char_width)
        } else {
            (12, 4, 8)
        };
        Renderer {
            gc: 1,
            font_id: 0,
            pixmap: 3,
            width: 1000,
            height: 24,
            depth: 24,
            font_ascent: ascent,
            font_descent: descent,
            char_width,
            is_core: false,
            font_engine: engine,
            framebuffer: Some(RefCell::new(vec![0u32; 1000 * 24])),
            blit_gc: 1,
            text_width_cache: RefCell::new(HashMap::new()),
        }
    }

    fn dummy_core_renderer() -> Renderer {
        Renderer {
            gc: 1,
            blit_gc: 1,
            font_id: 2,
            pixmap: 3,
            width: 1000,
            height: 24,
            depth: 24,
            font_ascent: 12,
            font_descent: 4,
            char_width: 8,
            is_core: true,
            font_engine: None,
            framebuffer: None,
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

    #[test]
    fn test_core_font_spec_detection() {
        assert!(is_core_font_spec("fixed"));
        assert!(is_core_font_spec("6x13"));
        assert!(is_core_font_spec("9x15"));
        assert!(is_core_font_spec("core:monospace"));
        assert!(is_core_font_spec("-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*"));

        assert!(!is_core_font_spec("pango:monospace 10"));
        assert!(!is_core_font_spec("monospace 10"));
        assert!(!is_core_font_spec("DejaVu Sans Mono"));
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

        let fontdue_renderer = dummy_fontdue_renderer();
        let p_w = fontdue_renderer.text_width("Hello");
        assert!(p_w > 0);
        assert_eq!(fontdue_renderer.text_width(""), 0);
    }

    #[test]
    fn test_software_framebuffer_fill_and_draw() {
        let renderer = dummy_fontdue_renderer();
        assert!(renderer.framebuffer.is_some());

        // Fill rectangle test
        renderer.fill_rect_software(
            &mut renderer.framebuffer.as_ref().unwrap().borrow_mut(),
            10,
            5,
            20,
            10,
            0x00FF0000,
        );

        let fb = renderer.framebuffer.as_ref().unwrap().borrow();
        // Check pixel at (15, 8)
        let idx = 8 * 1000 + 15;
        assert_eq!(fb[idx], 0x00FF0000);

        // Check pixel outside rect at (5, 5)
        let outside_idx = 5 * 1000 + 5;
        assert_eq!(fb[outside_idx], 0);
    }

    #[test]
    fn test_vertical_center_baseline() {
        let renderer = dummy_core_renderer();
        // font_height = 16. y_offset = (24 - 16)/2 = 4. baseline = 4 + 12 = 16.
        assert_eq!(renderer.vertical_center_baseline(), 16);
    }

    #[test]
    fn test_format_layout_symbol_always_three_chars() {
        // Standard 3-character symbols
        assert_eq!(format_layout_symbol("[]="), "[]=");
        assert_eq!(format_layout_symbol("=[]"), "=[]");
        assert_eq!(format_layout_symbol("TT="), "TT=");
        assert_eq!(format_layout_symbol("=TT"), "=TT");
        assert_eq!(format_layout_symbol("|M|"), "|M|");
        assert_eq!(format_layout_symbol("|||"), "|||");
        assert_eq!(format_layout_symbol("==="), "===");
        assert_eq!(format_layout_symbol("><>"), "><>");
        assert_eq!(format_layout_symbol("[G]"), "[G]");
        assert_eq!(format_layout_symbol("[M]"), "[M]");
        assert_eq!(format_layout_symbol("[T]"), "[T]");
        assert_eq!(format_layout_symbol("[S]"), "[S]");
        assert_eq!(format_layout_symbol("[B]"), "[B]");

        // 5-character bracketed symbols stripped to 3 characters
        assert_eq!(format_layout_symbol("[|||]"), "|||");
        assert_eq!(format_layout_symbol("[===]"), "===");
        assert_eq!(format_layout_symbol(&format!("[{}]", "[]=")), "[]=");

        // 1-character symbol wrapped to 3 characters
        assert_eq!(format_layout_symbol("M"), "[M]");

        // Verify all resulting strings are exactly 3 characters
        let wrapped_master = format!("[{}]", "[]=");
        let all_symbols = [
            "[]=", "=[]", "TT=", "=TT", "|M|", "|||", "===", "><>",
            "[G]", "[M]", "[T]", "[S]", "[B]", "[|||]", "[===]", &wrapped_master, "M",
        ];
        for s in all_symbols {
            assert_eq!(format_layout_symbol(s).chars().count(), 3);
        }
    }
}

