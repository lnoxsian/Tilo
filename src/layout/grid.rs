//! Grid layout algorithm.

use x11rb::protocol::xproto::Window;
use super::{LayoutConfig, Rect};

/// Calculates the bounding rectangles for a grid layout.
///
/// Divides `n` windows into an even grid of rows and columns.
/// Windows in each row share the full usable width evenly.
pub fn calculate_grid(
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

    if n == 1 {
        let mut r = Rect::new(usable_x, usable_y, usable_w, usable_h);
        r.width = r.width.saturating_sub(2 * config.border_width);
        r.height = r.height.saturating_sub(2 * config.border_width);
        return vec![r];
    }

    let cols = (n as f32).sqrt().ceil() as usize;
    let rows = n.div_ceil(cols);

    let total_v_gaps = config.inner_gap * (rows as u32 - 1);
    let total_available_h = usable_h.saturating_sub(total_v_gaps);
    let row_h = total_available_h / rows as u32;
    let row_rem = total_available_h % rows as u32;

    let base_cols = n / rows;
    let extra_rows = n % rows;

    let mut rects = Vec::with_capacity(n);
    let mut current_y = usable_y;

    for r in 0..rows {
        let h = if r == rows - 1 {
            row_h + row_rem
        } else {
            row_h
        };

        let cols_in_row = if r < extra_rows {
            base_cols + 1
        } else {
            base_cols
        };

        if cols_in_row > 0 {
            let total_h_gaps = config.inner_gap * (cols_in_row as u32 - 1);
            let total_available_w = usable_w.saturating_sub(total_h_gaps);
            let col_w = total_available_w / cols_in_row as u32;
            let col_rem = total_available_w % cols_in_row as u32;

            let mut current_x = usable_x;
            for c in 0..cols_in_row {
                let w = if c == cols_in_row - 1 {
                    col_w + col_rem
                } else {
                    col_w
                };
                let mut rect = Rect::new(current_x, current_y, w, h);
                rect.width = rect.width.saturating_sub(2 * config.border_width);
                rect.height = rect.height.saturating_sub(2 * config.border_width);
                rects.push(rect);

                current_x += (w + config.inner_gap) as i32;
            }
        }

        current_y += (h + config.inner_gap) as i32;
    }

    rects
}
