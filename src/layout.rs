//! Layout engine and tiling algorithms.
//!
//! Provides geometric calculations for arranging windows in tiling layouts,
//! starting with the master/stack layout with configurable gaps and borders.

use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, Window};

/// Represents a 2D rectangle in screen space, typically used for window geometries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// The X coordinate of the top-left corner.
    pub x: i32,
    /// The Y coordinate of the top-left corner.
    pub y: i32,
    /// The width of the rectangle in pixels.
    pub width: u32,
    /// The height of the rectangle in pixels.
    pub height: u32,
}

impl Rect {
    /// Creates a new rectangle.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// Checks if this rectangle overlaps with another rectangle (considering borders).
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.width as i32
            && self.x + self.width as i32 > other.x
            && self.y < other.y + other.height as i32
            && self.y + self.height as i32 > other.y
    }
}

/// Configuration settings for the layout engine.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    /// Percentage of screen width dedicated to the master pane (0.0 to 1.0).
    pub master_ratio: f32,
    /// Number of pixels between adjacent windows.
    pub inner_gap: u32,
    /// Number of pixels between windows and the screen edge.
    pub outer_gap: u32,
    /// Width of the window borders in pixels.
    pub border_width: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            master_ratio: 0.55,
            inner_gap: 8,
            outer_gap: 12,
            border_width: 2,
        }
    }
}

/// Calculates the bounding rectangles for a master/stack layout.
///
/// # Arguments
/// * `clients` - A slice of managed window IDs to layout.
/// * `screen_rect` - The usable screen area (excluding docks/bars).
/// * `config` - The layout configuration parameters (gaps, ratios).
///
/// # Returns
/// A vector of rectangles corresponding to each client in the same order.
pub fn calculate_master_stack(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    let n = clients.len();
    if n == 0 {
        return Vec::new();
    }

    let mut rects = Vec::with_capacity(n);

    // Calculate total usable area after outer gaps
    let double_outer = 2 * config.outer_gap;
    if screen_rect.width <= double_outer || screen_rect.height <= double_outer {
        return vec![*screen_rect; n];
    }

    let usable_x = screen_rect.x + config.outer_gap as i32;
    let usable_y = screen_rect.y + config.outer_gap as i32;
    let usable_w = screen_rect.width - double_outer;
    let usable_h = screen_rect.height - double_outer;

    if n == 1 {
        // Only one window, takes up the entire usable area
        let mut r = Rect::new(usable_x, usable_y, usable_w, usable_h);
        r.width = r.width.saturating_sub(2 * config.border_width);
        r.height = r.height.saturating_sub(2 * config.border_width);
        rects.push(r);
        return rects;
    }

    // Calculate master dimensions
    let half_inner = config.inner_gap / 2;
    let master_w = ((usable_w as f32 * config.master_ratio) as u32).saturating_sub(half_inner);
    rects.push(Rect::new(usable_x, usable_y, master_w, usable_h));

    // Calculate stack dimensions
    let stack_x = usable_x + master_w as i32 + config.inner_gap as i32;
    let stack_w = usable_w.saturating_sub(master_w + config.inner_gap);

    let stack_count = (n - 1) as u32;
    let total_inner_gaps = config.inner_gap * (stack_count - 1);
    let total_stack_height = usable_h.saturating_sub(total_inner_gaps);
    let stack_h = total_stack_height / stack_count;
    let remainder = total_stack_height % stack_count;

    for i in 0..stack_count {
        let current_y = usable_y + (i * (stack_h + config.inner_gap)) as i32;
        // Last window absorbs any pixel division remainder for exact alignment
        let h = if i == stack_count - 1 {
            stack_h + remainder
        } else {
            stack_h
        };
        rects.push(Rect::new(stack_x, current_y, stack_w, h));
    }

    // Adjust dimensions to account for X11 borders inside config logic
    for rect in &mut rects {
        rect.width = rect.width.saturating_sub(2 * config.border_width);
        rect.height = rect.height.saturating_sub(2 * config.border_width);
    }

    rects
}

/// Applies the computed layout geometries to the actual X11 windows using default border width.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `clients` - The windows to configure.
/// * `rects` - The computed geometry for each window.
///
/// # Returns
/// `Result<(), X11Error>` - Returns an error if an X11 configure request fails fatally.
pub fn apply_layout<C: Connection>(
    conn: &C,
    clients: &[Window],
    rects: &[Rect],
) -> Result<(), X11Error> {
    apply_layout_with_border(conn, clients, rects, 2)
}

/// Applies the computed layout geometries and border widths to the actual X11 windows.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `clients` - The windows to configure.
/// * `rects` - The computed geometry for each window.
/// * `border_width` - The window border width in pixels.
pub fn apply_layout_with_border<C: Connection>(
    conn: &C,
    clients: &[Window],
    rects: &[Rect],
    border_width: u32,
) -> Result<(), X11Error> {
    for (&window, rect) in clients.iter().zip(rects.iter()) {
        let aux = ConfigureWindowAux::default()
            .x(rect.x)
            .y(rect.y)
            .width(rect.width)
            .height(rect.height)
            .border_width(border_width);

        if let Err(err) = conn.configure_window(window, &aux) {
            eprintln!(
                "[WARN  tilo::layout] Failed to configure window 0x{:x}: {:?}",
                window, err
            );
        }
    }
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_clients() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let config = LayoutConfig::default();
        let rects = calculate_master_stack(&[], &screen, &config);
        assert!(rects.is_empty());
    }

    #[test]
    fn test_single_window_layout() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 8,
            outer_gap: 12,
            border_width: 2,
        };
        let rects = calculate_master_stack(&[1], &screen, &config);
        assert_eq!(rects.len(), 1);

        let r = rects[0];
        assert_eq!(r.x, 12);
        assert_eq!(r.y, 12);
        assert_eq!(r.width, 1920 - 24 - 4); // screen - 2*outer_gap - 2*border_width
        assert_eq!(r.height, 1080 - 24 - 4);
    }

    #[test]
    fn test_master_stack_two_windows() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let config = LayoutConfig {
            master_ratio: 0.60,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };
        let rects = calculate_master_stack(&[1, 2], &screen, &config);
        assert_eq!(rects.len(), 2);

        let master = rects[0];
        let stack = rects[1];

        // Master should be on the left
        assert_eq!(master.x, 10);
        assert_eq!(master.y, 10);

        // Stack should be on the right, starting after master + 2*border + inner_gap
        let master_outer_width = master.width + 2 * config.border_width;
        let expected_stack_x = master.x + master_outer_width as i32 + config.inner_gap as i32;
        assert_eq!(stack.x, expected_stack_x);
        assert_eq!(stack.y, 10);

        // Neither window should overlap
        assert!(!master.overlaps(&stack));
    }

    #[test]
    fn test_three_windows_no_overlap_and_within_bounds() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let config = LayoutConfig {
            master_ratio: 0.60,
            inner_gap: 8,
            outer_gap: 8,
            border_width: 2,
        };
        let rects = calculate_master_stack(&[1, 2, 3], &screen, &config);
        assert_eq!(rects.len(), 3);

        // Ensure no overlapping rectangles
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(
                    !rects[i].overlaps(&rects[j]),
                    "Rect {} and {} must not overlap",
                    i,
                    j
                );
            }
        }

        // Ensure all rectangles are within the screen boundaries
        for (idx, r) in rects.iter().enumerate() {
            assert!(r.x >= screen.x, "Rect {} x out of bounds", idx);
            assert!(r.y >= screen.y, "Rect {} y out of bounds", idx);
            assert!(
                r.x + (r.width + 2 * config.border_width) as i32 <= screen.x + screen.width as i32,
                "Rect {} extends beyond right edge",
                idx
            );
            assert!(
                r.y + (r.height + 2 * config.border_width) as i32 <= screen.y + screen.height as i32,
                "Rect {} extends beyond bottom edge",
                idx
            );
        }
    }
}
