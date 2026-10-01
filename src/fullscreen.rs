//! Fullscreen window management.
//!
//! Implements fullscreen mode for client windows according to Phase 13 of Plan.md:
//! - Window geometry expansion to cover the entire monitor/screen area (`0, 0, width, height`).
//! - Removal of window borders (`border_width = 0`).
//! - Raising the fullscreen window to the top (`StackMode::ABOVE`).
//! - Saving previous geometry and border width in [`FullscreenState`].
//! - Restoring previous geometry, border width, and tiling layout when toggled off.
//! - Updating EWMH `_NET_WM_STATE_FULLSCREEN`.

use crate::floating::FloatingGeometry;
use crate::layout::Rect;
use x11rb::protocol::xproto::{ConfigureWindowAux, StackMode, Window};

/// Represents the 2D bounding geometry of a window or monitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Geometry {
    /// The X coordinate of the top-left corner.
    pub x: i32,
    /// The Y coordinate of the top-left corner.
    pub y: i32,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
}

impl Geometry {
    /// Creates a new `Geometry`.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }
}

impl From<Rect> for Geometry {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

impl From<Geometry> for Rect {
    fn from(g: Geometry) -> Self {
        Self {
            x: g.x,
            y: g.y,
            width: g.width,
            height: g.height,
        }
    }
}

impl From<FloatingGeometry> for Geometry {
    fn from(f: FloatingGeometry) -> Self {
        Self {
            x: f.x,
            y: f.y,
            width: f.width,
            height: f.height,
        }
    }
}

impl From<Geometry> for FloatingGeometry {
    fn from(g: Geometry) -> Self {
        Self {
            x: g.x,
            y: g.y,
            width: g.width,
            height: g.height,
        }
    }
}

pub use crate::monitor::{Monitor, MonitorId};

/// The fullscreen state of a client window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FullscreenState {
    /// The window is not in fullscreen mode.
    #[default]
    None,
    /// The window is fullscreen. Stores the previous geometry and border width to restore upon exit.
    Fullscreen {
        previous_geometry: Geometry,
        previous_border_width: u32,
    },
}

impl FullscreenState {
    /// Returns true if this state represents an active fullscreen window.
    pub fn is_fullscreen(&self) -> bool {
        matches!(self, FullscreenState::Fullscreen { .. })
    }
}


/// Constructs a `ConfigureWindowAux` to apply fullscreen geometry to a window.
pub fn fullscreen_configure_aux(monitor: &Monitor) -> ConfigureWindowAux {
    ConfigureWindowAux::new()
        .x(monitor.x)
        .y(monitor.y)
        .width(monitor.width)
        .height(monitor.height)
        .border_width(0)
        .stack_mode(StackMode::ABOVE)
}

/// Constructs a `ConfigureWindowAux` to restore a window's previous geometry and border width.
pub fn restore_configure_aux(geom: Geometry, border_width: u32) -> ConfigureWindowAux {
    ConfigureWindowAux::new()
        .x(geom.x)
        .y(geom.y)
        .width(geom.width)
        .height(geom.height)
        .border_width(border_width)
}

/// Updates the `_NET_WM_STATE` property for a window to add or remove `_NET_WM_STATE_FULLSCREEN`.
pub fn update_net_wm_state_fullscreen(
    conn: &impl x11rb::connection::Connection,
    client_id: Window,
    is_fullscreen: bool,
    atoms: &crate::ewmh::EwmhAtoms,
) -> Result<(), crate::x11::X11Error> {
    crate::ewmh::update_net_wm_state_fullscreen(conn, client_id, is_fullscreen, atoms)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geometry_conversions() {
        let rect = Rect::new(10, 20, 300, 400);
        let geom: Geometry = rect.into();
        assert_eq!(geom.x, 10);
        assert_eq!(geom.y, 20);
        assert_eq!(geom.width, 300);
        assert_eq!(geom.height, 400);

        let rect_back: Rect = geom.into();
        assert_eq!(rect_back, rect);
    }

    #[test]
    fn test_fullscreen_state_transitions() {
        let mut state = FullscreenState::None;
        assert_eq!(state, FullscreenState::None);

        state = FullscreenState::Fullscreen {
            previous_geometry: Geometry::new(50, 60, 800, 600),
            previous_border_width: 2,
        };

        match state {
            FullscreenState::Fullscreen {
                previous_geometry,
                previous_border_width,
            } => {
                assert_eq!(previous_geometry.x, 50);
                assert_eq!(previous_geometry.width, 800);
                assert_eq!(previous_border_width, 2);
            }
            FullscreenState::None => panic!("Expected Fullscreen variant"),
        }
    }
}
