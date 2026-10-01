//! Scratchpad floating window management.
//!
//! Implements Phase 23 of Plan.md:
//! - Dedicated single-window scratchpad.
//! - Toggle visibility between hidden and visible floating state (`Mod4 + grave`).
//! - Automatically centers the scratchpad on the active monitor when shown.
//! - Excluded from standard tiling layouts (treated as floating).
//! - Clears scratchpad tracking when the client window is destroyed.

use crate::layout::Rect;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, InputFocus, StackMode, Window,
};

/// State of the scratchpad window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scratchpad {
    /// The X11 window ID of the current scratchpad client, if any.
    pub client: Option<Window>,
    /// Whether the scratchpad is currently visible.
    pub is_visible: bool,
}

impl Scratchpad {
    /// Creates a new default (empty and hidden) `Scratchpad`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `Scratchpad` tracking the specified window.
    pub fn with_client(client: Window) -> Self {
        Self {
            client: Some(client),
            is_visible: false,
        }
    }

    /// Assigns a client window to this scratchpad.
    pub fn set_client(&mut self, window_id: Window) {
        self.client = Some(window_id);
    }

    /// Clears the assigned client window, resetting visibility.
    pub fn clear_client(&mut self) {
        self.client = None;
        self.is_visible = false;
    }

    /// Returns `true` if the specified window is the scratchpad client.
    pub fn is_scratchpad(&self, window_id: Window) -> bool {
        self.client == Some(window_id)
    }
}

/// Calculates the centered geometry for the scratchpad on a given monitor or screen area.
///
/// Uses 70% of the monitor dimensions by default, clamped to reasonable minimums.
pub fn calculate_scratchpad_geometry(
    mon_x: i32,
    mon_y: i32,
    mon_w: u32,
    mon_h: u32,
) -> Rect {
    let width = (mon_w * 70 / 100).max(400).min(mon_w);
    let height = (mon_h * 70 / 100).max(300).min(mon_h);
    let x = mon_x + (mon_w as i32 - width as i32) / 2;
    let y = mon_y + (mon_h as i32 - height as i32) / 2;
    Rect::new(x, y, width, height)
}

/// Assigns a specific window to be the scratchpad.
///
/// # Arguments
/// * `scratchpad` - Mutable reference to the scratchpad state.
/// * `window_id` - The X11 window ID to track.
pub fn set_scratchpad_client(scratchpad: &mut Scratchpad, window_id: Window) {
    scratchpad.set_client(window_id);
}

/// Toggles the visibility of the scratchpad window.
///
/// If the scratchpad is hidden, it is mapped and raised. If visible, it is unmapped and hidden.
///
/// # Arguments
/// * `scratchpad` - Mutable reference to the scratchpad state.
/// * `conn` - The X11 connection.
/// * `_current_workspace` - The ID of the currently active workspace.
///
/// # Returns
/// `Result<(), X11Error>` indicating success or failure of X11 calls.
pub fn toggle_scratchpad(
    scratchpad: &mut Scratchpad,
    conn: &impl Connection,
    _current_workspace: u32,
) -> Result<(), X11Error> {
    let window_id = match scratchpad.client {
        Some(w) => w,
        None => return Ok(()),
    };

    if scratchpad.is_visible {
        // Hide: unmap window
        conn.unmap_window(window_id)?;
        scratchpad.is_visible = false;
    } else {
        // Show: query root geometry to center if needed, map, raise, and focus
        let setup = conn.setup();
        let (screen_w, screen_h) = if let Some(screen) = setup.roots.first() {
            (screen.width_in_pixels as u32, screen.height_in_pixels as u32)
        } else {
            (1280, 720)
        };
        let geom = calculate_scratchpad_geometry(0, 0, screen_w, screen_h);

        let aux = ConfigureWindowAux::new()
            .x(geom.x)
            .y(geom.y)
            .width(geom.width)
            .height(geom.height)
            .stack_mode(StackMode::ABOVE);
        conn.configure_window(window_id, &aux)?;
        conn.map_window(window_id)?;
        let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, window_id, x11rb::CURRENT_TIME);
        scratchpad.is_visible = true;
    }
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scratchpad_state_defaults() {
        let sp = Scratchpad::new();
        assert_eq!(sp.client, None);
        assert!(!sp.is_visible);
        assert!(!sp.is_scratchpad(100));
    }

    #[test]
    fn test_scratchpad_set_and_clear_client() {
        let mut sp = Scratchpad::new();
        set_scratchpad_client(&mut sp, 0x1234);
        assert_eq!(sp.client, Some(0x1234));
        assert!(sp.is_scratchpad(0x1234));
        assert!(!sp.is_scratchpad(0x5678));

        sp.is_visible = true;
        sp.clear_client();
        assert_eq!(sp.client, None);
        assert!(!sp.is_visible);
    }

    #[test]
    fn test_calculate_scratchpad_geometry() {
        let geom = calculate_scratchpad_geometry(0, 0, 1920, 1080);
        assert_eq!(geom.width, 1344); // 70% of 1920
        assert_eq!(geom.height, 756);  // 70% of 1080
        assert_eq!(geom.x, (1920 - 1344) / 2);
        assert_eq!(geom.y, (1080 - 756) / 2);

        // Clamped minimums on small screen
        let small_geom = calculate_scratchpad_geometry(100, 50, 400, 300);
        assert_eq!(small_geom.width, 400);
        assert_eq!(small_geom.height, 300);
        assert_eq!(small_geom.x, 100);
        assert_eq!(small_geom.y, 50);
    }
}
