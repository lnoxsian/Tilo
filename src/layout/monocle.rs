//! Monocle layout algorithm.

use x11rb::protocol::xproto::Window;
use super::{LayoutConfig, Rect};

/// Calculates the bounding rectangles for a monocle layout.
///
/// In a monocle layout, all windows are maximized to fill the full usable screen area
/// (minus outer gaps and window borders).
pub fn calculate_monocle(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    let n = clients.len();
    if n == 0 {
        return Vec::new();
    }

    let double_outer = 2 * config.outer_gap;
    if screen_rect.width <= double_outer || screen_rect.height <= double_outer {
        return vec![*screen_rect; n];
    }

    let usable_x = screen_rect.x + config.outer_gap as i32;
    let usable_y = screen_rect.y + config.outer_gap as i32;
    let usable_w = screen_rect.width - double_outer;
    let usable_h = screen_rect.height - double_outer;

    let mut r = Rect::new(usable_x, usable_y, usable_w, usable_h);
    r.width = r.width.saturating_sub(2 * config.border_width);
    r.height = r.height.saturating_sub(2 * config.border_width);
    vec![r; n]
}
