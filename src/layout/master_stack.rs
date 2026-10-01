//! Master and stack layout variations.

use x11rb::protocol::xproto::Window;
use super::{LayoutConfig, Rect};

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

/// Calculates the bounding rectangles for a master/stack layout with Master on Right, Stack on Left.
pub fn calculate_master_right(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    let n = clients.len();
    if n == 0 {
        return Vec::new();
    }

    let mut rects = Vec::with_capacity(n);

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
        rects.push(r);
        return rects;
    }

    let half_inner = config.inner_gap / 2;
    let master_w = ((usable_w as f32 * config.master_ratio) as u32).saturating_sub(half_inner);
    let stack_w = usable_w.saturating_sub(master_w + config.inner_gap);
    let stack_x = usable_x;
    let master_x = usable_x + stack_w as i32 + config.inner_gap as i32;

    // Master is window 0 on the right
    rects.push(Rect::new(master_x, usable_y, master_w, usable_h));

    // Stack windows 1..n on the left, stacked vertically into rows
    let stack_count = (n - 1) as u32;
    let total_inner_gaps = config.inner_gap * (stack_count - 1);
    let total_stack_height = usable_h.saturating_sub(total_inner_gaps);
    let stack_h = total_stack_height / stack_count;
    let remainder = total_stack_height % stack_count;

    for i in 0..stack_count {
        let current_y = usable_y + (i * (stack_h + config.inner_gap)) as i32;
        let h = if i == stack_count - 1 {
            stack_h + remainder
        } else {
            stack_h
        };
        rects.push(Rect::new(stack_x, current_y, stack_w, h));
    }

    for rect in &mut rects {
        rect.width = rect.width.saturating_sub(2 * config.border_width);
        rect.height = rect.height.saturating_sub(2 * config.border_width);
    }

    rects
}

/// Calculates the bounding rectangles for a master/stack layout with Master on Top, Stack on Bottom.
pub fn calculate_master_top(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    let n = clients.len();
    if n == 0 {
        return Vec::new();
    }

    let mut rects = Vec::with_capacity(n);

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
        rects.push(r);
        return rects;
    }

    let half_inner = config.inner_gap / 2;
    let master_h = ((usable_h as f32 * config.master_ratio) as u32).saturating_sub(half_inner);
    let stack_h = usable_h.saturating_sub(master_h + config.inner_gap);
    let master_y = usable_y;
    let stack_y = usable_y + master_h as i32 + config.inner_gap as i32;

    // Master is window 0 on top
    rects.push(Rect::new(usable_x, master_y, usable_w, master_h));

    // Stack windows 1..n on bottom, divided horizontally into columns
    let stack_count = (n - 1) as u32;
    let total_inner_gaps = config.inner_gap * (stack_count - 1);
    let total_stack_width = usable_w.saturating_sub(total_inner_gaps);
    let stack_col_w = total_stack_width / stack_count;
    let remainder = total_stack_width % stack_count;

    for i in 0..stack_count {
        let current_x = usable_x + (i * (stack_col_w + config.inner_gap)) as i32;
        let w = if i == stack_count - 1 {
            stack_col_w + remainder
        } else {
            stack_col_w
        };
        rects.push(Rect::new(current_x, stack_y, w, stack_h));
    }

    for rect in &mut rects {
        rect.width = rect.width.saturating_sub(2 * config.border_width);
        rect.height = rect.height.saturating_sub(2 * config.border_width);
    }

    rects
}

/// Calculates the bounding rectangles for a master/stack layout with Master on Bottom, Stack on Top.
pub fn calculate_master_bottom(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    let n = clients.len();
    if n == 0 {
        return Vec::new();
    }

    let mut rects = Vec::with_capacity(n);

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
        rects.push(r);
        return rects;
    }

    let half_inner = config.inner_gap / 2;
    let master_h = ((usable_h as f32 * config.master_ratio) as u32).saturating_sub(half_inner);
    let stack_h = usable_h.saturating_sub(master_h + config.inner_gap);
    let stack_y = usable_y;
    let master_y = usable_y + stack_h as i32 + config.inner_gap as i32;

    // Master is window 0 on bottom
    rects.push(Rect::new(usable_x, master_y, usable_w, master_h));

    // Stack windows 1..n on top, divided horizontally into columns
    let stack_count = (n - 1) as u32;
    let total_inner_gaps = config.inner_gap * (stack_count - 1);
    let total_stack_width = usable_w.saturating_sub(total_inner_gaps);
    let stack_col_w = total_stack_width / stack_count;
    let remainder = total_stack_width % stack_count;

    for i in 0..stack_count {
        let current_x = usable_x + (i * (stack_col_w + config.inner_gap)) as i32;
        let w = if i == stack_count - 1 {
            stack_col_w + remainder
        } else {
            stack_col_w
        };
        rects.push(Rect::new(current_x, stack_y, w, stack_h));
    }

    for rect in &mut rects {
        rect.width = rect.width.saturating_sub(2 * config.border_width);
        rect.height = rect.height.saturating_sub(2 * config.border_width);
    }

    rects
}

/// Calculates the bounding rectangles for a vertical layout (side-by-side columns).
pub fn calculate_vertical(
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

    let total_inner = config.inner_gap * (n as u32 - 1);
    let total_available_w = usable_w.saturating_sub(total_inner);
    let col_w = total_available_w / n as u32;
    let remainder = total_available_w % n as u32;

    let mut rects = Vec::with_capacity(n);
    for i in 0..n {
        let cur_x = usable_x + (i as u32 * (col_w + config.inner_gap)) as i32;
        let w = if i == n - 1 { col_w + remainder } else { col_w };
        let mut r = Rect::new(cur_x, usable_y, w, usable_h);
        r.width = r.width.saturating_sub(2 * config.border_width);
        r.height = r.height.saturating_sub(2 * config.border_width);
        rects.push(r);
    }
    rects
}

/// Calculates the bounding rectangles for a horizontal layout (stacked rows).
pub fn calculate_horizontal(
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

    let total_inner = config.inner_gap * (n as u32 - 1);
    let total_available_h = usable_h.saturating_sub(total_inner);
    let row_h = total_available_h / n as u32;
    let remainder = total_available_h % n as u32;

    let mut rects = Vec::with_capacity(n);
    for i in 0..n {
        let cur_y = usable_y + (i as u32 * (row_h + config.inner_gap)) as i32;
        let h = if i == n - 1 { row_h + remainder } else { row_h };
        let mut r = Rect::new(usable_x, cur_y, usable_w, h);
        r.width = r.width.saturating_sub(2 * config.border_width);
        r.height = r.height.saturating_sub(2 * config.border_width);
        rects.push(r);
    }
    rects
}

/// Calculates the bounding rectangles for a centered master layout (Stack Left, Master Center, Stack Right).
pub fn calculate_center_master(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    if clients.is_empty() {
        return Vec::new();
    }

    let n = clients.len();
    let mut rects = Vec::with_capacity(n);

    let usable_x = screen_rect.x + config.outer_gap as i32;
    let usable_y = screen_rect.y + config.outer_gap as i32;
    let usable_w = screen_rect.width.saturating_sub(2 * config.outer_gap);
    let usable_h = screen_rect.height.saturating_sub(2 * config.outer_gap);

    if n == 1 {
        let mut r = Rect::new(usable_x, usable_y, usable_w, usable_h);
        r.width = r.width.saturating_sub(2 * config.border_width);
        r.height = r.height.saturating_sub(2 * config.border_width);
        rects.push(r);
        return rects;
    }

    if n == 2 {
        let half_inner = config.inner_gap / 2;
        let master_w = ((usable_w as f32 * config.master_ratio) as u32).saturating_sub(half_inner);
        let stack_w = usable_w.saturating_sub(master_w + config.inner_gap);

        let master_x = usable_x + stack_w as i32 + config.inner_gap as i32;
        rects.push(Rect::new(master_x, usable_y, master_w, usable_h));
        rects.push(Rect::new(usable_x, usable_y, stack_w, usable_h));

        for rect in &mut rects {
            rect.width = rect.width.saturating_sub(2 * config.border_width);
            rect.height = rect.height.saturating_sub(2 * config.border_width);
        }
        return rects;
    }

    let left_count = (n - 1).div_ceil(2);
    let right_count = (n - 1) - left_count;

    let total_horizontal_gaps = 2 * config.inner_gap;
    let available_w = usable_w.saturating_sub(total_horizontal_gaps);
    let master_w = (available_w as f32 * config.master_ratio.clamp(0.1, 0.8)) as u32;
    let remaining_w = available_w.saturating_sub(master_w);
    let left_w = remaining_w / 2;
    let right_w = remaining_w.saturating_sub(left_w);

    let left_x = usable_x;
    let master_x = left_x + left_w as i32 + config.inner_gap as i32;
    let right_x = master_x + master_w as i32 + config.inner_gap as i32;

    rects.push(Rect::new(master_x, usable_y, master_w, usable_h));

    let calc_column_rects = |count: usize, x: i32, w: u32| -> Vec<Rect> {
        if count == 0 {
            return Vec::new();
        }
        let count_u = count as u32;
        let total_inner_gaps = config.inner_gap * (count_u - 1);
        let total_h = usable_h.saturating_sub(total_inner_gaps);
        let row_h = total_h / count_u;
        let remainder = total_h % count_u;

        let mut col_rects = Vec::with_capacity(count);
        for i in 0..count_u {
            let current_y = usable_y + (i * (row_h + config.inner_gap)) as i32;
            let h = if i == count_u - 1 {
                row_h + remainder
            } else {
                row_h
            };
            col_rects.push(Rect::new(x, current_y, w, h));
        }
        col_rects
    };

    let left_rects = calc_column_rects(left_count, left_x, left_w);
    let right_rects = calc_column_rects(right_count, right_x, right_w);

    let mut left_idx = 0;
    let mut right_idx = 0;
    for i in 1..n {
        if i % 2 == 1 {
            rects.push(left_rects[left_idx]);
            left_idx += 1;
        } else {
            rects.push(right_rects[right_idx]);
            right_idx += 1;
        }
    }

    for rect in &mut rects {
        rect.width = rect.width.saturating_sub(2 * config.border_width);
        rect.height = rect.height.saturating_sub(2 * config.border_width);
    }

    rects
}

