//! X11 Cursor Management for tilo.
//!
//! Provides proper pointer cursors for the root window, desktop, statusbar,
//! and launcher, eliminating the default X11 'X' cursor (`XC_X_cursor`).
//!
//! Supports modern Xcursor themes (e.g. Adwaita, Breeze, Bibata) with automatic
//! fallback to standard X11 core font glyph cursors (`XC_left_ptr`, `XC_fleur`, `XC_sizing`).

use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::cursor::Handle;
use x11rb::protocol::xproto::{self, ChangeWindowAttributesAux, ConnectionExt, Window};
use x11rb::resource_manager::Database;

/// Standard X11 font cursor glyph IDs from `cursorfont.h`.
pub const XC_FLEUR: u16 = 52;
pub const XC_LEFT_PTR: u16 = 68;
pub const XC_SIZING: u16 = 120;

/// Creates an X11 core font cursor using glyphs from the standard `cursor` font.
///
/// In standard X11 font cursor protocol:
/// - `shape`: The source character index (e.g. 68 for `XC_left_ptr`).
/// - `shape + 1`: The mask character index (e.g. 69 for `XC_left_ptr` mask).
/// - Foreground is black (0, 0, 0) and background is white (65535, 65535, 65535).
pub fn create_font_cursor<C: Connection>(conn: &C, shape: u16) -> Result<xproto::Cursor, X11Error> {
    let font = conn.generate_id()?;
    conn.open_font(font, b"cursor")?;
    let cursor = conn.generate_id()?;
    let res = conn.create_glyph_cursor(
        cursor,
        font,
        font,
        shape,
        shape + 1,
        0,
        0,
        0,
        u16::MAX,
        u16::MAX,
        u16::MAX,
    );
    let _ = conn.close_font(font);
    res?;
    Ok(cursor)
}

/// Manages X11 cursors used by the window manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CursorManager {
    /// Normal arrow pointer cursor (`left_ptr`).
    pub normal: xproto::Cursor,
    /// 4-way move cursor (`fleur`) used when moving floating windows.
    pub move_cursor: xproto::Cursor,
    /// Resize cursor (`sizing`) used when resizing windows.
    pub resize_cursor: xproto::Cursor,
}

impl CursorManager {
    /// Initializes cursors for the given X11 connection and screen.
    ///
    /// Attempts to load cursors from the user's active Xcursor theme first.
    /// If theme loading fails or returns None, it falls back to standard core font cursors.
    pub fn new<C: Connection>(conn: &C, screen_num: usize) -> Self {
        let handle = {
            let db = x11rb::resource_manager::new_from_default(conn)
                .unwrap_or_else(|_| Database::new_from_data(&[]));
            Handle::new(conn, screen_num, &db).ok().and_then(|c| c.reply().ok())
        };

        let load = |h: &Handle, names: &[&str]| -> Option<xproto::Cursor> {
            for name in names {
                if let Ok(c) = h.load_cursor(conn, name)
                    && c != 0 {
                        return Some(c);
                    }
            }
            None
        };

        let normal = handle
            .as_ref()
            .and_then(|h| load(h, &["left_ptr", "default", "arrow"]))
            .or_else(|| create_font_cursor(conn, XC_LEFT_PTR).ok())
            .unwrap_or(0);

        let move_cursor = handle
            .as_ref()
            .and_then(|h| load(h, &["fleur", "move", "all-scroll"]))
            .or_else(|| create_font_cursor(conn, XC_FLEUR).ok())
            .unwrap_or(normal);

        let resize_cursor = handle
            .as_ref()
            .and_then(|h| load(h, &["sizing", "se-resize", "nwse-resize", "crosshair"]))
            .or_else(|| create_font_cursor(conn, XC_SIZING).ok())
            .unwrap_or(normal);

        Self {
            normal,
            move_cursor,
            resize_cursor,
        }
    }

    /// Sets the normal arrow cursor on the root window.
    ///
    /// This immediately replaces the default black 'X' cursor (`XC_X_cursor`)
    /// with the standard arrow pointer across the desktop.
    pub fn apply_root<C: Connection>(&self, conn: &C, root: Window) -> Result<(), X11Error> {
        if self.normal != 0 && root != 0 {
            let aux = ChangeWindowAttributesAux::new().cursor(self.normal);
            conn.change_window_attributes(root, &aux)?;
            conn.flush()?;
        }
        Ok(())
    }

    /// Sets the normal arrow cursor on an individual window (such as statusbar or launcher).
    pub fn apply_window<C: Connection>(&self, conn: &C, window: Window) -> Result<(), X11Error> {
        if self.normal != 0 && window != 0 {
            let aux = ChangeWindowAttributesAux::new().cursor(self.normal);
            conn.change_window_attributes(window, &aux)?;
            conn.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_constants() {
        assert_eq!(XC_FLEUR, 52);
        assert_eq!(XC_LEFT_PTR, 68);
        assert_eq!(XC_SIZING, 120);
    }

    #[test]
    fn test_cursor_manager_default() {
        let mgr = CursorManager::default();
        assert_eq!(mgr.normal, 0);
        assert_eq!(mgr.move_cursor, 0);
        assert_eq!(mgr.resize_cursor, 0);
    }
}

