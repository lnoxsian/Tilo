//! Floating window management and geometry tracking.
//!
//! This module defines structures and helper routines for managing windows
//! that bypass the tiling layout and float freely at arbitrary positions and sizes.

use crate::layout::Rect;
use x11rb::protocol::xproto::{ConfigureWindowAux, StackMode};

/// Represents the saved geometry of a floating window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FloatingGeometry {
    /// X coordinate of the window's top-left corner.
    pub x: i32,
    /// Y coordinate of the window's top-left corner.
    pub y: i32,
    /// Width of the window in pixels.
    pub width: u32,
    /// Height of the window in pixels.
    pub height: u32,
}

impl FloatingGeometry {
    /// Creates a new `FloatingGeometry`.
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl From<Rect> for FloatingGeometry {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

impl From<FloatingGeometry> for Rect {
    fn from(f: FloatingGeometry) -> Self {
        Self {
            x: f.x,
            y: f.y,
            width: f.width,
            height: f.height,
        }
    }
}

/// Indicates whether a client is tiled or floating.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClientLayoutState {
    /// Window is managed by the tiling algorithm.
    #[default]
    Tiled,
    /// Window is floating and placed freely. Contains its last known geometry.
    Floating(FloatingGeometry),
}

/// Calculates the default centered geometry for a floating window based on screen dimensions.
///
/// Uses 60% of the screen width and height by default, clamped to reasonable minimums.
pub fn calculate_default_floating_geometry(
    screen_width: u16,
    screen_height: u16,
) -> FloatingGeometry {
    let width = ((screen_width as u32) * 60 / 100)
        .max(300)
        .min(screen_width as u32);
    let height = ((screen_height as u32) * 60 / 100)
        .max(200)
        .min(screen_height as u32);
    let x = ((screen_width as i32) - (width as i32)) / 2;
    let y = ((screen_height as i32) - (height as i32)) / 2;

    FloatingGeometry {
        x: x.max(0),
        y: y.max(0),
        width,
        height,
    }
}

/// Clamps a floating window geometry so it remains visible within the screen bounds.
pub fn clamp_floating_geometry(
    mut geom: FloatingGeometry,
    screen_width: u16,
    screen_height: u16,
) -> FloatingGeometry {
    let min_visible = 50i32;
    let screen_w = screen_width as i32;
    let screen_h = screen_height as i32;

    geom.width = geom.width.max(100).min(screen_width as u32);
    geom.height = geom.height.max(80).min(screen_height as u32);

    // Keep at least min_visible pixels on screen horizontally
    if geom.x + (geom.width as i32) < min_visible {
        geom.x = min_visible - (geom.width as i32);
    } else if geom.x > screen_w - min_visible {
        geom.x = screen_w - min_visible;
    }

    // Keep at least min_visible pixels on screen vertically
    if geom.y + (geom.height as i32) < min_visible {
        geom.y = min_visible - (geom.height as i32);
    } else if geom.y > screen_h - min_visible {
        geom.y = screen_h - min_visible;
    }

    geom
}

/// Creates a `ConfigureWindowAux` configuration to apply floating window dimensions,
/// optional border width, and top-of-stack stacking mode.
pub fn floating_configure_aux(
    geom: &FloatingGeometry,
    border_width: Option<u32>,
    raise: bool,
) -> ConfigureWindowAux {
    let mut aux = ConfigureWindowAux::new()
        .x(geom.x)
        .y(geom.y)
        .width(geom.width)
        .height(geom.height);

    if let Some(bw) = border_width {
        aux = aux.border_width(bw);
    }

    if raise {
        aux = aux.stack_mode(StackMode::ABOVE);
    }

    aux
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_floating_geometry_basic() {
        let geom = FloatingGeometry::new(100, 150, 600, 400);
        assert_eq!(geom.x, 100);
        assert_eq!(geom.y, 150);
        assert_eq!(geom.width, 600);
        assert_eq!(geom.height, 400);

        let rect: Rect = geom.into();
        assert_eq!(rect.x, 100);
        assert_eq!(rect.y, 150);
        assert_eq!(rect.width, 600);
        assert_eq!(rect.height, 400);

        let roundtrip: FloatingGeometry = rect.into();
        assert_eq!(roundtrip, geom);
    }

    #[test]
    fn test_client_layout_state() {
        let default_state = ClientLayoutState::default();
        assert_eq!(default_state, ClientLayoutState::Tiled);

        let geom = FloatingGeometry::new(50, 50, 400, 300);
        let float_state = ClientLayoutState::Floating(geom);
        assert_eq!(float_state, ClientLayoutState::Floating(geom));
        assert_ne!(float_state, ClientLayoutState::Tiled);
    }

    #[test]
    fn test_calculate_default_floating_geometry() {
        let geom = calculate_default_floating_geometry(1024, 768);
        assert_eq!(geom.width, (1024 * 60) / 100); // 614
        assert_eq!(geom.height, (768 * 60) / 100); // 460
        assert_eq!(geom.x, (1024 - 614) / 2); // 205
        assert_eq!(geom.y, (768 - 460) / 2); // 154
    }

    #[test]
    fn test_clamp_floating_geometry() {
        let screen_w = 1000;
        let screen_h = 800;

        // Normal geometry inside screen
        let normal = FloatingGeometry::new(100, 100, 400, 300);
        assert_eq!(clamp_floating_geometry(normal, screen_w, screen_h), normal);

        // Window dragged far to the left
        let far_left = FloatingGeometry::new(-500, 100, 400, 300);
        let clamped_left = clamp_floating_geometry(far_left, screen_w, screen_h);
        assert_eq!(clamped_left.x, 50 - 400); // -350, leaves 50px visible

        // Window dragged far to the right
        let far_right = FloatingGeometry::new(1500, 100, 400, 300);
        let clamped_right = clamp_floating_geometry(far_right, screen_w, screen_h);
        assert_eq!(clamped_right.x, 1000 - 50); // 950, leaves 50px visible
    }
}
