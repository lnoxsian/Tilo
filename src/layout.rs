//! Layout engine and tiling algorithms.
//!
//! Provides geometric calculations for arranging windows in tiling layouts,
//! starting with the master/stack layout with configurable gaps and borders.

use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window};

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

/// Available layout types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum LayoutType {
    /// Traditional master and stack tiling (Master Left, Stack Right).
    #[default]
    MasterStack,
    /// Master Right, Stack Left (Horizontal mirror).
    MasterRight,
    /// Master Top, Stack Bottom.
    MasterTop,
    /// Master Bottom, Stack Top.
    MasterBottom,
    /// Centered Master (Stack Left, Master Center, Stack Right).
    CenterMaster,
    /// Grid layout (divides space into rows and columns evenly).
    Grid,
    /// Evenly divide space vertically.
    Vertical,
    /// Evenly divide space horizontally.
    Horizontal,
    /// One window takes up the whole screen.
    Monocle,
    /// All windows share the full area with a horizontal tab bar at the top.
    /// Requires window decoration rendering. Deferred to post-Phase 25.
    Tabbed,
    /// Window title bars stacked vertically; focused window takes remaining body.
    /// Requires window decoration rendering. Deferred to post-Phase 25.
    Stacked,
    /// Recursive binary space partitioning tree.
    Bsp,
}

impl std::fmt::Display for LayoutType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MasterStack => write!(f, "master-left"),
            Self::MasterRight => write!(f, "master-right"),
            Self::MasterTop => write!(f, "master-top"),
            Self::MasterBottom => write!(f, "master-bottom"),
            Self::CenterMaster => write!(f, "center-master"),
            Self::Grid => write!(f, "grid"),
            Self::Vertical => write!(f, "vertical"),
            Self::Horizontal => write!(f, "horizontal"),
            Self::Monocle => write!(f, "monocle"),
            Self::Tabbed => write!(f, "tabbed"),
            Self::Stacked => write!(f, "stacking"),
            Self::Bsp => write!(f, "bsp"),
        }
    }
}

impl std::str::FromStr for LayoutType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "master" | "master-left" | "master_left" | "masterleft" | "masterstack" | "master_stack" | "default" => Ok(Self::MasterStack),
            "master-right" | "master_right" | "masterright" => Ok(Self::MasterRight),
            "master-top" | "master_top" | "mastertop" => Ok(Self::MasterTop),
            "master-bottom" | "master_bottom" | "masterbottom" => Ok(Self::MasterBottom),
            "center-master" | "center_master" | "centermaster" | "centered-master" | "centered_master" | "centeredmaster" => Ok(Self::CenterMaster),
            "grid" | "g" => Ok(Self::Grid),
            "columns" | "column" | "cols" | "col" | "vertical" | "vert" | "v" => Ok(Self::Vertical),
            "rows" | "row" | "horizontal" | "horiz" | "h" => Ok(Self::Horizontal),
            "monocle" | "mono" | "fullscreen_layout" => Ok(Self::Monocle),
            "tabbed" | "tabs" => Ok(Self::Tabbed),
            "stacking" | "stacked" => Ok(Self::Stacked),
            "bsp" | "bsptree" => Ok(Self::Bsp),
            other => Err(format!("Unknown layout type: '{}'", other)),
        }
    }
}

impl LayoutType {
    /// Alias for MasterStack representing Master Left — Stack Right.
    #[allow(non_upper_case_globals)]
    pub const MasterLeft: Self = Self::MasterStack;

    /// Alias for CenterMaster representing Centered Master.
    #[allow(non_upper_case_globals)]
    pub const CenteredMaster: Self = Self::CenterMaster;

    /// Alias for Vertical representing Columns.
    #[allow(non_upper_case_globals)]
    pub const Columns: Self = Self::Vertical;

    /// Alias for Horizontal representing Rows.
    #[allow(non_upper_case_globals)]
    pub const Rows: Self = Self::Horizontal;

    /// Returns true if this layout is any orientation of master/stack.
    pub fn is_master(&self) -> bool {
        matches!(
            self,
            Self::MasterStack | Self::MasterRight | Self::MasterTop | Self::MasterBottom | Self::CenterMaster
        )
    }

    /// Returns true if this layout is columns or rows.
    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Vertical | Self::Horizontal)
    }

    /// Returns true if this layout can be rotated between orientations with Mod4 + r.
    pub fn can_rotate(&self) -> bool {
        self.is_master() || self.is_linear()
    }

    /// Canonical user-facing name for this layout.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MasterStack => "master-left",
            Self::MasterRight => "master-right",
            Self::MasterTop => "master-top",
            Self::MasterBottom => "master-bottom",
            Self::CenterMaster => "center-master",
            Self::Grid => "grid",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Monocle => "monocle",
            Self::Tabbed => "tabbed",
            Self::Stacked => "stacking",
            Self::Bsp => "bsp",
        }
    }

    /// Returns the short statusbar symbol for this layout.
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::MasterStack => "[]=",
            Self::MasterRight => "=[]",
            Self::MasterTop => "TT=",
            Self::MasterBottom => "=TT",
            Self::CenterMaster => "|M|",
            Self::Grid => "[G]",
            Self::Vertical => "|||",
            Self::Horizontal => "===",
            Self::Monocle => "[M]",
            Self::Tabbed => "[T]",
            Self::Stacked => "[S]",
            Self::Bsp => "[B]",
        }
    }
}

/// Rotates the layout orientation:
/// - Master stack rotates through its 5 orientations:
///   MasterLeft -> MasterRight -> MasterTop -> MasterBottom -> CenterMaster -> MasterLeft.
/// - Linear (columns/rows) rotates between columns (Vertical) and rows (Horizontal):
///   Vertical <-> Horizontal.
pub fn rotate_layout(current: LayoutType) -> LayoutType {
    match current {
        LayoutType::MasterStack => LayoutType::MasterRight,
        LayoutType::MasterRight => LayoutType::MasterTop,
        LayoutType::MasterTop => LayoutType::MasterBottom,
        LayoutType::MasterBottom => LayoutType::CenterMaster,
        LayoutType::CenterMaster => LayoutType::MasterStack,
        LayoutType::Vertical => LayoutType::Horizontal,
        LayoutType::Horizontal => LayoutType::Vertical,
        other => other,
    }
}

/// Backwards-compatible alias for `rotate_layout`.
pub fn rotate_master_layout(current: LayoutType) -> LayoutType {
    rotate_layout(current)
}

/// Cycles to the next layout in the list.
///
/// # Arguments
/// * `current` - The current layout.
/// # Returns
/// The next layout type.
pub fn cycle_layout(current: LayoutType) -> LayoutType {
    match current {
        LayoutType::MasterStack | LayoutType::MasterRight | LayoutType::MasterTop | LayoutType::MasterBottom | LayoutType::CenterMaster => LayoutType::Vertical,
        LayoutType::Vertical | LayoutType::Horizontal => LayoutType::Monocle,
        LayoutType::Monocle => LayoutType::Bsp,
        LayoutType::Bsp => LayoutType::Grid,
        LayoutType::Grid => LayoutType::MasterStack,
        LayoutType::Tabbed => LayoutType::Stacked,
        LayoutType::Stacked => LayoutType::MasterStack,
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

/// Calculates the bounding rectangles for a monocle layout (all windows take the full screen area).
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

/// Dispatches calculation to the specified layout algorithm.
pub fn calculate_layout(
    layout: LayoutType,
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    match layout {
        LayoutType::MasterStack => calculate_master_stack(clients, screen_rect, config),
        LayoutType::MasterRight => calculate_master_right(clients, screen_rect, config),
        LayoutType::MasterTop => calculate_master_top(clients, screen_rect, config),
        LayoutType::MasterBottom => calculate_master_bottom(clients, screen_rect, config),
        LayoutType::CenterMaster => calculate_center_master(clients, screen_rect, config),
        LayoutType::Grid => calculate_grid(clients, screen_rect, config),
        LayoutType::Vertical => calculate_vertical(clients, screen_rect, config),
        LayoutType::Horizontal => calculate_horizontal(clients, screen_rect, config),
        LayoutType::Monocle | LayoutType::Tabbed | LayoutType::Stacked => {
            calculate_monocle(clients, screen_rect, config)
        }
        LayoutType::Bsp => {
            if clients.is_empty() {
                return Vec::new();
            }
            let mut tree = BspTree::new();
            tree.sync_clients(clients, clients.first().copied(), screen_rect, config.inner_gap);
            let pairs = tree.calculate_rects(screen_rect, config);
            let mut map = std::collections::HashMap::new();
            for (w, r) in pairs {
                map.insert(w, r);
            }
            clients.iter().filter_map(|w| map.get(w).copied()).collect()
        }
    }
}

/// Applies the chosen layout algorithm to the list of visible clients.
///
/// # Arguments
/// * `layout` - The layout type to apply.
/// * `clients` - List of window IDs to tile.
/// * `area` - The available screen geometry (excluding bars).
/// * `master_ratio` - The ratio of screen width given to the master area.
/// * `conn` - The X11 connection to apply positions.
pub fn apply_layout(
    layout: LayoutType,
    clients: &[u32],
    area: &Rect,
    master_ratio: f32,
    conn: &impl Connection,
) -> Result<(), X11Error> {
    if clients.is_empty() {
        return Ok(());
    }
    let config = LayoutConfig {
        master_ratio,
        inner_gap: 0,
        outer_gap: 0,
        border_width: 2,
    };
    let rects = calculate_layout(layout, clients, area, &config);
    apply_layout_with_border(conn, clients, &rects, config.border_width)?;

    if layout == LayoutType::Monocle
        && let Some(&first) = clients.first() {
            let aux = ConfigureWindowAux::default()
                .stack_mode(StackMode::ABOVE);
            let _ = conn.configure_window(first, &aux);
            conn.flush()?;
        }

    Ok(())
}

/// Applies the computed layout geometries to the actual X11 windows using default border width.
pub fn apply_layout_rects<C: Connection>(
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

/// Orientation of a binary split in the layout tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitType {
    /// Splits horizontally into Left and Right children.
    Horizontal,
    /// Splits vertically into Top and Bottom children.
    Vertical,
}

/// A node in the Binary Space Partitioning (BSP) tree.
#[derive(Debug, Clone, PartialEq)]
pub enum BspNode {
    /// A leaf node representing a single managed X11 client window.
    Leaf {
        window: Window,
    },
    /// An internal node dividing its allocated area between two child subtrees.
    Split {
        split_type: SplitType,
        /// Fraction of width or height allocated to the first child (clamped 0.05..=0.95).
        ratio: f32,
        first: Box<BspNode>,
        second: Box<BspNode>,
    },
}

impl BspNode {
    /// Checks if this subtree contains the given window.
    pub fn contains(&self, target: Window) -> bool {
        match self {
            BspNode::Leaf { window } => *window == target,
            BspNode::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    /// Returns all window IDs in this subtree in pre-order traversal.
    pub fn windows(&self) -> Vec<Window> {
        let mut result = Vec::new();
        self.collect_windows(&mut result);
        result
    }

    fn collect_windows(&self, out: &mut Vec<Window>) {
        match self {
            BspNode::Leaf { window } => out.push(*window),
            BspNode::Split { first, second, .. } => {
                first.collect_windows(out);
                second.collect_windows(out);
            }
        }
    }

    /// Returns the number of leaf windows in this subtree.
    pub fn leaf_count(&self) -> usize {
        match self {
            BspNode::Leaf { .. } => 1,
            BspNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Finds the assigned bounding rectangle of `target` within the given screen space.
    pub fn find_window_rect(&self, target: Window, rect: Rect, inner_gap: u32) -> Option<Rect> {
        match self {
            BspNode::Leaf { window } => {
                if *window == target {
                    Some(rect)
                } else {
                    None
                }
            }
            BspNode::Split { split_type, ratio, first, second } => {
                let (r1, r2) = split_rect(rect, *split_type, *ratio, inner_gap);
                if first.contains(target) {
                    first.find_window_rect(target, r1, inner_gap)
                } else if second.contains(target) {
                    second.find_window_rect(target, r2, inner_gap)
                } else {
                    None
                }
            }
        }
    }

    /// Recursively computes bounding rectangles for all leaf windows.
    pub fn calculate_rects(&self, rect: Rect, inner_gap: u32, out: &mut Vec<(Window, Rect)>) {
        match self {
            BspNode::Leaf { window } => {
                out.push((*window, rect));
            }
            BspNode::Split { split_type, ratio, first, second } => {
                let (r1, r2) = split_rect(rect, *split_type, *ratio, inner_gap);
                first.calculate_rects(r1, inner_gap, out);
                second.calculate_rects(r2, inner_gap, out);
            }
        }
    }

    /// Inserts `new_win` by replacing the leaf containing `target` with a split node.
    pub fn insert_at(&mut self, target: Window, new_win: Window, split_type: SplitType, ratio: f32) -> bool {
        match self {
            BspNode::Leaf { window } if *window == target => {
                let old_win = *window;
                *self = BspNode::Split {
                    split_type,
                    ratio: ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: old_win }),
                    second: Box::new(BspNode::Leaf { window: new_win }),
                };
                true
            }
            BspNode::Split { first, second, .. } => {
                first.insert_at(target, new_win, split_type, ratio)
                    || second.insert_at(target, new_win, split_type, ratio)
            }
            _ => false,
        }
    }

    /// Swaps the leaf positions of two windows in this subtree.
    pub fn swap(&mut self, win_a: Window, win_b: Window) -> bool {
        let mut leaf_a = None;
        let mut leaf_b = None;
        self.collect_leaf_mut(&mut |w| {
            if *w == win_a {
                leaf_a = Some(w as *mut Window);
            } else if *w == win_b {
                leaf_b = Some(w as *mut Window);
            }
        });

        if let (Some(ptr_a), Some(ptr_b)) = (leaf_a, leaf_b) {
            unsafe {
                core::ptr::swap(ptr_a, ptr_b);
            }
            true
        } else {
            false
        }
    }

    fn collect_leaf_mut<F>(&mut self, f: &mut F)
    where
        F: FnMut(&mut Window),
    {
        match self {
            BspNode::Leaf { window } => f(window),
            BspNode::Split { first, second, .. } => {
                first.collect_leaf_mut(f);
                second.collect_leaf_mut(f);
            }
        }
    }

    /// Resizes the ancestor split of `target` that matches `target_split`.
    /// Resizes the focused pane `target` by `delta` along `target_split` such that
    /// only `target` and its immediate adjacent neighbor in that linear sequence change size,
    /// leaving all other panes completely unchanged.
    pub fn resize_pane(&mut self, target: Window, delta: f32, target_split: SplitType) -> bool {
        if let Some(highest) = self.find_highest_split_mut(target, target_split) {
            highest.adjust_linear_leaf(target, delta, target_split)
        } else {
            self.resize_any_split(delta, target_split)
        }
    }

    /// Finds a mutable reference to the highest continuous ancestor split of `target_split` containing `target`.
    fn find_highest_split_mut(&mut self, target: Window, target_split: SplitType) -> Option<&mut BspNode> {
        if !self.contains(target) {
            return None;
        }

        if let BspNode::Split { split_type, .. } = self
            && *split_type == target_split {
                return Some(self);
            }

        match self {
            BspNode::Split { first, second, .. } => {
                if first.contains(target) {
                    first.find_highest_split_mut(target, target_split)
                } else if second.contains(target) {
                    second.find_highest_split_mut(target, target_split)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Adjusts the leaf corresponding to `target` within this linear subtree of `target_split`.
    fn adjust_linear_leaf(&mut self, target: Window, delta: f32, target_split: SplitType) -> bool {
        let mut units = Vec::new();
        self.collect_split_units(target_split, 1.0, &mut units);

        if units.len() < 2 {
            return false;
        }

        let target_idx = match units.iter().position(|(wins, _)| wins.contains(&target)) {
            Some(i) => i,
            None => return false,
        };

        let neighbor_idx = if target_idx + 1 < units.len() {
            target_idx + 1
        } else if target_idx > 0 {
            target_idx - 1
        } else {
            return false;
        };

        let min_weight = 0.05;
        if delta > 0.0 {
            let max_delta = (units[neighbor_idx].1 - min_weight).max(0.0);
            let actual = delta.min(max_delta);
            if actual <= 0.0001 {
                return false;
            }
            units[target_idx].1 += actual;
            units[neighbor_idx].1 -= actual;
        } else {
            let max_delta = (units[target_idx].1 - min_weight).max(0.0);
            let actual = (-delta).min(max_delta);
            if actual <= 0.0001 {
                return false;
            }
            units[target_idx].1 -= actual;
            units[neighbor_idx].1 += actual;
        }

        self.apply_split_unit_weights(target_split, &units);
        true
    }

    fn collect_split_units(&self, target_split: SplitType, factor: f32, out: &mut Vec<(Vec<Window>, f32)>) {
        match self {
            BspNode::Leaf { window } => {
                out.push((vec![*window], factor));
            }
            BspNode::Split { split_type, ratio, first, second } if *split_type == target_split => {
                first.collect_split_units(target_split, factor * *ratio, out);
                second.collect_split_units(target_split, factor * (1.0 - *ratio), out);
            }
            BspNode::Split { .. } => {
                out.push((self.windows(), factor));
            }
        }
    }

    fn apply_split_unit_weights(&mut self, target_split: SplitType, units: &[(Vec<Window>, f32)]) {
        match self {
            BspNode::Leaf { .. } => {}
            BspNode::Split { split_type, ratio, first, second } if *split_type == target_split => {
                let first_wins = first.windows();
                let second_wins = second.windows();

                let sum_first: f32 = units
                    .iter()
                    .filter(|(wins, _)| wins.iter().any(|w| first_wins.contains(w)))
                    .map(|(_, s)| *s)
                    .sum();
                let sum_second: f32 = units
                    .iter()
                    .filter(|(wins, _)| wins.iter().any(|w| second_wins.contains(w)))
                    .map(|(_, s)| *s)
                    .sum();

                let total = sum_first + sum_second;
                if total > 0.0 {
                    *ratio = (sum_first / total).clamp(0.05, 0.95);
                }

                first.apply_split_unit_weights(target_split, units);
                second.apply_split_unit_weights(target_split, units);
            }
            _ => {}
        }
    }

    /// Resizes the first split matching `target_split` in this subtree.
    pub fn resize_any_split(&mut self, delta: f32, target_split: SplitType) -> bool {
        match self {
            BspNode::Leaf { .. } => false,
            BspNode::Split { split_type, ratio, first, second } => {
                if *split_type == target_split {
                    *ratio = (*ratio + delta).clamp(0.05, 0.95);
                    true
                } else {
                    first.resize_any_split(delta, target_split)
                        || second.resize_any_split(delta, target_split)
                }
            }
        }
    }
}

/// Recursively removes `target` from the subtree, collapsing the parent node with the remaining sibling.
#[allow(clippy::boxed_local)]
fn remove_node(node: Box<BspNode>, target: Window) -> Option<Box<BspNode>> {
    match *node {
        BspNode::Leaf { window } => {
            if window == target {
                None
            } else {
                Some(Box::new(BspNode::Leaf { window }))
            }
        }
        BspNode::Split { split_type, ratio, first, second } => {
            if first.contains(target) {
                let new_first = remove_node(first, target);
                match new_first {
                    None => Some(second), // First was removed, collapse to second child
                    Some(f) => Some(Box::new(BspNode::Split {
                        split_type,
                        ratio,
                        first: f,
                        second,
                    })),
                }
            } else if second.contains(target) {
                let new_second = remove_node(second, target);
                match new_second {
                    None => Some(first), // Second was removed, collapse to first child
                    Some(s) => Some(Box::new(BspNode::Split {
                        split_type,
                        ratio,
                        first,
                        second: s,
                    })),
                }
            } else {
                Some(Box::new(BspNode::Split {
                    split_type,
                    ratio,
                    first,
                    second,
                }))
            }
        }
    }
}

/// Helper function to divide a rectangle into two children based on split type, ratio, and gap.
fn split_rect(rect: Rect, split_type: SplitType, ratio: f32, inner_gap: u32) -> (Rect, Rect) {
    match split_type {
        SplitType::Horizontal => {
            if rect.width <= inner_gap {
                let w = rect.width / 2;
                (
                    Rect::new(rect.x, rect.y, w, rect.height),
                    Rect::new(rect.x + w as i32, rect.y, rect.width.saturating_sub(w), rect.height),
                )
            } else {
                let available = rect.width - inner_gap;
                let w1 = ((available as f32 * ratio).round() as u32).clamp(1, available.saturating_sub(1));
                let w2 = available - w1;
                (
                    Rect::new(rect.x, rect.y, w1, rect.height),
                    Rect::new(rect.x + w1 as i32 + inner_gap as i32, rect.y, w2, rect.height),
                )
            }
        }
        SplitType::Vertical => {
            if rect.height <= inner_gap {
                let h = rect.height / 2;
                (
                    Rect::new(rect.x, rect.y, rect.width, h),
                    Rect::new(rect.x, rect.y + h as i32, rect.width, rect.height.saturating_sub(h)),
                )
            } else {
                let available = rect.height - inner_gap;
                let h1 = ((available as f32 * ratio).round() as u32).clamp(1, available.saturating_sub(1));
                let h2 = available - h1;
                (
                    Rect::new(rect.x, rect.y, rect.width, h1),
                    Rect::new(rect.x, rect.y + h1 as i32 + inner_gap as i32, rect.width, h2),
                )
            }
        }
    }
}

/// Binary Space Partitioning (BSP) tree layout manager for a single workspace.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BspTree {
    /// Root node of the partition tree.
    pub root: Option<Box<BspNode>>,
    /// Pre-selected split direction for the next inserted window.
    pub preselect: Option<SplitType>,
}

impl BspTree {
    /// Creates a new empty `BspTree`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the tree contains no windows.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Returns the number of tiled windows in the tree.
    pub fn len(&self) -> usize {
        self.root.as_ref().map(|r| r.leaf_count()).unwrap_or(0)
    }

    /// Checks if a window is managed by this tree.
    pub fn contains(&self, window: Window) -> bool {
        self.root.as_ref().map(|r| r.contains(window)).unwrap_or(false)
    }

    /// Returns all window IDs in pre-order traversal.
    pub fn windows(&self) -> Vec<Window> {
        self.root.as_ref().map(|r| r.windows()).unwrap_or_default()
    }

    /// Inserts a window into the tree, auto-splitting or using a pre-selected split.
    pub fn insert(&mut self, target: Option<Window>, new_win: Window, usable_rect: &Rect, inner_gap: u32) {
        if self.contains(new_win) {
            return;
        }

        let Some(ref mut root) = self.root else {
            self.root = Some(Box::new(BspNode::Leaf { window: new_win }));
            return;
        };

        let target_win = target
            .filter(|&w| root.contains(w))
            .or_else(|| root.windows().last().copied())
            .unwrap_or(new_win);

        if target_win == new_win {
            return;
        }

        let split_type = if let Some(pre) = self.preselect.take() {
            pre
        } else {
            let target_rect = root.find_window_rect(target_win, *usable_rect, inner_gap)
                .unwrap_or(*usable_rect);
            if target_rect.width >= target_rect.height {
                SplitType::Horizontal
            } else {
                SplitType::Vertical
            }
        };

        root.insert_at(target_win, new_win, split_type, 0.5);
    }

    /// Removes a window from the tree and collapses the sibling into the parent split.
    pub fn remove(&mut self, window: Window) -> bool {
        if let Some(root) = self.root.take() {
            let was_present = root.contains(window);
            self.root = remove_node(root, window);
            was_present
        } else {
            false
        }
    }

    /// Swaps two windows' positions in the tree.
    pub fn swap_windows(&mut self, win_a: Window, win_b: Window) -> bool {
        if let Some(ref mut root) = self.root {
            root.swap(win_a, win_b)
        } else {
            false
        }
    }

    /// Resizes the ancestor split of `window` by `delta_ratio` ($\pm 0.05$).
    /// Returns the active master pane ratio based on layout orientation.
    pub fn master_ratio_for_layout(&self, layout: LayoutType) -> Option<f32> {
        match (layout, self.root.as_deref()) {
            (LayoutType::MasterStack, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, .. })) => Some(*ratio),
            (LayoutType::MasterRight, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, .. })) => Some(1.0 - *ratio),
            (LayoutType::MasterTop, Some(BspNode::Split { split_type: SplitType::Vertical, ratio, .. })) => Some(*ratio),
            (LayoutType::MasterBottom, Some(BspNode::Split { split_type: SplitType::Vertical, ratio, .. })) => Some(1.0 - *ratio),
            (LayoutType::CenterMaster, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, second, .. })) => {
                if matches!(**second, BspNode::Leaf { .. }) {
                    Some(1.0 - *ratio)
                } else {
                    Some((1.0 - 2.0 * *ratio).clamp(0.05, 0.95))
                }
            }
            _ => None,
        }
    }

    /// Returns the current horizontal split ratio if the root is a horizontal split.
    pub fn master_ratio(&self) -> Option<f32> {
        self.master_ratio_for_layout(LayoutType::MasterStack)
    }

    /// Resizes the ancestor split of `window` by `delta_ratio` ($\pm 0.05$), falling back to any matching split.
    pub fn resize_window(&mut self, window: Window, delta_ratio: f32, split_type: SplitType) -> bool {
        if let Some(ref mut root) = self.root {
            root.resize_pane(window, delta_ratio, split_type)
        } else {
            false
        }
    }

    /// Dynamically resizes a window's split during mouse drag.
    pub fn resize_by_pointer(&mut self, window: Window, delta_x: i32, delta_y: i32, initial_rect: Rect) -> bool {
        let mut handled = false;
        if let Some(ref mut root) = self.root {
            if initial_rect.width > 0 && delta_x != 0 {
                let frac = delta_x as f32 / initial_rect.width as f32;
                if root.resize_pane(window, frac, SplitType::Horizontal) {
                    handled = true;
                }
            }
            if initial_rect.height > 0 && delta_y != 0 {
                let frac = delta_y as f32 / initial_rect.height as f32;
                if root.resize_pane(window, frac, SplitType::Vertical) {
                    handled = true;
                }
            }
        }
        handled
    }

    /// Calculates bounding rectangles for all windows in the tree.
    pub fn calculate_rects(&self, screen_rect: &Rect, config: &LayoutConfig) -> Vec<(Window, Rect)> {
        let Some(ref root) = self.root else {
            return Vec::new();
        };

        let double_outer = 2 * config.outer_gap;
        let (usable_x, usable_y, usable_w, usable_h) = if screen_rect.width <= double_outer || screen_rect.height <= double_outer {
            (screen_rect.x, screen_rect.y, screen_rect.width, screen_rect.height)
        } else {
            (
                screen_rect.x + config.outer_gap as i32,
                screen_rect.y + config.outer_gap as i32,
                screen_rect.width - double_outer,
                screen_rect.height - double_outer,
            )
        };

        let usable_rect = Rect::new(usable_x, usable_y, usable_w, usable_h);
        let mut pairs = Vec::new();
        root.calculate_rects(usable_rect, config.inner_gap, &mut pairs);

        for (_, rect) in &mut pairs {
            rect.width = rect.width.saturating_sub(2 * config.border_width);
            rect.height = rect.height.saturating_sub(2 * config.border_width);
        }

        pairs
    }

    /// Builds a Master/Stack binary layout tree.
    pub fn build_master_stack(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: master_ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: Self::build_linear(stack, SplitType::Vertical),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Right, Stack on Left.
    pub fn build_master_right(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: stack_ratio,
                    first: Self::build_linear(stack, SplitType::Vertical),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Top, Stack on Bottom.
    pub fn build_master_top(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Vertical,
                    ratio: master_ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: Self::build_linear(stack, SplitType::Horizontal),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Bottom, Stack on Top.
    pub fn build_master_bottom(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Vertical,
                    ratio: stack_ratio,
                    first: Self::build_linear(stack, SplitType::Horizontal),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
        }
    }

    /// Builds a Centered Master binary layout tree (Stack Left, Master Center, Stack Right).
    pub fn build_center_master(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            2 => {
                let master = clients[0];
                let stack = clients[1];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: stack_ratio,
                    first: Box::new(BspNode::Leaf { window: stack }),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
            _ => {
                let master = clients[0];
                let left_wins: Vec<Window> = clients.iter().skip(1).step_by(2).copied().collect();
                let right_wins: Vec<Window> = clients.iter().skip(2).step_by(2).copied().collect();

                let master_r = master_ratio.clamp(0.10, 0.80);
                let stack_r = 1.0 - master_r;
                let left_fraction = stack_r / 2.0;
                let remaining_fraction = 1.0 - left_fraction;
                let master_in_rem_ratio = (master_r / remaining_fraction).clamp(0.05, 0.95);

                let right_node = Self::build_linear(&right_wins, SplitType::Vertical);
                let center_and_right = Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: master_in_rem_ratio,
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: right_node,
                });

                let left_node = Self::build_linear(&left_wins, SplitType::Vertical);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: left_fraction.clamp(0.05, 0.95),
                    first: left_node,
                    second: center_and_right,
                }))
            }
        }
    }

    /// Builds a Vertical (side-by-side columns) binary layout tree.
    pub fn build_vertical(clients: &[Window]) -> Option<Box<BspNode>> {
        if clients.is_empty() {
            None
        } else {
            Some(Self::build_linear(clients, SplitType::Horizontal))
        }
    }

    /// Builds a Horizontal (stacked rows) binary layout tree.
    pub fn build_horizontal(clients: &[Window]) -> Option<Box<BspNode>> {
        if clients.is_empty() {
            None
        } else {
            Some(Self::build_linear(clients, SplitType::Vertical))
        }
    }

    /// Builds a balanced Grid binary layout tree.
    pub fn build_grid(clients: &[Window]) -> Option<Box<BspNode>> {
        let n = clients.len();
        if n == 0 {
            return None;
        }
        if n == 1 {
            return Some(Box::new(BspNode::Leaf { window: clients[0] }));
        }

        let cols = (n as f32).sqrt().ceil() as usize;
        let rows = n.div_ceil(cols);
        let base_cols = n / rows;
        let extra_rows = n % rows;

        let mut row_nodes = Vec::with_capacity(rows);
        let mut idx = 0;
        for r in 0..rows {
            let cols_in_row = if r < extra_rows {
                base_cols + 1
            } else {
                base_cols
            };
            let row_clients = &clients[idx..idx + cols_in_row];
            idx += cols_in_row;
            row_nodes.push(*Self::build_linear(row_clients, SplitType::Horizontal));
        }

        Self::combine_nodes(&row_nodes, SplitType::Vertical)
    }

    fn combine_nodes(nodes: &[BspNode], split_type: SplitType) -> Option<Box<BspNode>> {
        if nodes.is_empty() {
            return None;
        }
        if nodes.len() == 1 {
            return Some(Box::new(nodes[0].clone()));
        }
        let k = nodes.len();
        let ratio = (1.0 / k as f32).clamp(0.05, 0.95);
        let first = Box::new(nodes[0].clone());
        let second = Self::combine_nodes(&nodes[1..], split_type).unwrap();
        Some(Box::new(BspNode::Split {
            split_type,
            ratio,
            first,
            second,
        }))
    }

    fn build_linear(clients: &[Window], split_type: SplitType) -> Box<BspNode> {
        let k = clients.len();
        if k <= 1 {
            Box::new(BspNode::Leaf { window: clients[0] })
        } else {
            let ratio = (1.0 / k as f32).clamp(0.05, 0.95);
            Box::new(BspNode::Split {
                split_type,
                ratio,
                first: Box::new(BspNode::Leaf { window: clients[0] }),
                second: Self::build_linear(&clients[1..], split_type),
            })
        }
    }

    /// Rebuilds the tree structure for the specified layout type while preserving client order.
    pub fn rebuild_for_layout(&mut self, layout: LayoutType, clients: &[Window], master_ratio: f32) {
        self.root = match layout {
            LayoutType::MasterStack => Self::build_master_stack(clients, master_ratio),
            LayoutType::MasterRight => Self::build_master_right(clients, master_ratio),
            LayoutType::MasterTop => Self::build_master_top(clients, master_ratio),
            LayoutType::MasterBottom => Self::build_master_bottom(clients, master_ratio),
            LayoutType::CenterMaster => Self::build_center_master(clients, master_ratio),
            LayoutType::Grid => Self::build_grid(clients),
            LayoutType::Vertical => Self::build_vertical(clients),
            LayoutType::Horizontal => Self::build_horizontal(clients),
            LayoutType::Bsp => {
                let current_wins = self.windows();
                if current_wins.len() == clients.len() && clients.iter().all(|w| current_wins.contains(w)) && self.root.is_some() {
                    return;
                }
                let mut tree = BspTree::new();
                let dummy_rect = Rect::new(0, 0, 1920, 1080);
                for &w in clients {
                    tree.insert(None, w, &dummy_rect, 0);
                }
                tree.root
            }
            LayoutType::Monocle | LayoutType::Tabbed | LayoutType::Stacked => None,
        };
    }

    /// Synchronizes the tree with visible clients using layout-specific insertion rules.
    pub fn sync_clients_with_layout(
        &mut self,
        layout: LayoutType,
        visible_clients: &[Window],
        focused: Option<Window>,
        usable_rect: &Rect,
        inner_gap: u32,
        master_ratio: f32,
    ) {
        for w in self.windows() {
            if !visible_clients.contains(&w) {
                self.remove(w);
            }
        }

        if self.is_empty() {
            self.rebuild_for_layout(layout, visible_clients, master_ratio);
            return;
        }

        for &w in visible_clients {
            if !self.contains(w) {
                match layout {
                    LayoutType::MasterStack => {
                        let wins = self.windows();
                        if wins.len() == 1 {
                            let m = wins[0];
                            if let Some(ref mut root) = self.root {
                                root.insert_at(m, w, SplitType::Horizontal, master_ratio.clamp(0.05, 0.95));
                            }
                        } else {
                            let last = wins.last().copied().unwrap();
                            if let Some(ref mut root) = self.root {
                                root.insert_at(last, w, SplitType::Vertical, 0.5);
                            }
                        }
                    }
                    LayoutType::MasterRight | LayoutType::MasterTop | LayoutType::MasterBottom | LayoutType::CenterMaster | LayoutType::Grid => {
                        let mut all = self.windows();
                        all.push(w);
                        self.rebuild_for_layout(layout, &all, master_ratio);
                    }
                    LayoutType::Vertical => {
                        let wins = self.windows();
                        let last = wins.last().copied().unwrap();
                        if let Some(ref mut root) = self.root {
                            root.insert_at(last, w, SplitType::Horizontal, 0.5);
                        }
                    }
                    LayoutType::Horizontal => {
                        let wins = self.windows();
                        let last = wins.last().copied().unwrap();
                        if let Some(ref mut root) = self.root {
                            root.insert_at(last, w, SplitType::Vertical, 0.5);
                        }
                    }
                    _ => {
                        self.insert(focused, w, usable_rect, inner_gap);
                    }
                }
            }
        }
    }

    /// Synchronizes the tree with a list of currently visible tiled windows.
    pub fn sync_clients(&mut self, visible_clients: &[Window], focused: Option<Window>, usable_rect: &Rect, inner_gap: u32) {
        self.sync_clients_with_layout(LayoutType::Bsp, visible_clients, focused, usable_rect, inner_gap, 0.55);
    }
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

    #[test]
    fn test_bsp_tree_lifecycle_and_auto_splits() {
        let mut tree = BspTree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);

        let screen = Rect::new(0, 0, 1920, 1080);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // Window 1: fills usable area
        tree.insert(None, 100, &screen, config.inner_gap);
        assert_eq!(tree.len(), 1);
        assert!(tree.contains(100));

        let rects = tree.calculate_rects(&screen, &config);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].0, 100);

        // Window 2: splits horizontally (since 1920 >= 1080)
        tree.insert(Some(100), 200, &screen, config.inner_gap);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.windows(), vec![100, 200]);

        let rects2 = tree.calculate_rects(&screen, &config);
        assert_eq!(rects2.len(), 2);
        let r1 = rects2[0].1;
        let r2 = rects2[1].1;
        // Should be side by side
        assert!(r1.x < r2.x);
        assert!(!r1.overlaps(&r2));

        // Window 3: focused on 200 (which has height > width in a 1920x1080 split) -> auto splits vertically
        tree.insert(Some(200), 300, &screen, config.inner_gap);
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.windows(), vec![100, 200, 300]);

        let rects3 = tree.calculate_rects(&screen, &config);
        assert_eq!(rects3.len(), 3);
        // None of the 3 windows should overlap
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(!rects3[i].1.overlaps(&rects3[j].1));
            }
        }

        // Test swapping windows
        assert!(tree.swap_windows(100, 300));
        assert_eq!(tree.windows(), vec![300, 200, 100]);

        // Test resizing ancestor split
        let pre_resize = tree.calculate_rects(&screen, &config);
        assert!(tree.resize_window(300, 0.1, SplitType::Horizontal));
        let post_resize = tree.calculate_rects(&screen, &config);
        assert!(post_resize[0].1.width > pre_resize[0].1.width);

        // Test pointer resize
        let initial = post_resize[0].1;
        assert!(tree.resize_by_pointer(300, 50, 0, initial));

        // Test removal and tree collapse
        assert!(tree.remove(200));
        assert_eq!(tree.len(), 2);
        assert!(!tree.contains(200));

        let rects_post_remove = tree.calculate_rects(&screen, &config);
        assert_eq!(rects_post_remove.len(), 2);
        assert!(!rects_post_remove[0].1.overlaps(&rects_post_remove[1].1));

        // Test sync_clients
        tree.sync_clients(&[300, 400], Some(300), &screen, config.inner_gap);
        assert!(tree.contains(300));
        assert!(tree.contains(400));
        assert!(!tree.contains(100));
    }

    #[test]
    fn test_phase27_layout_types_and_cycling() {
        assert_eq!(LayoutType::default(), LayoutType::MasterStack);

        assert_eq!(cycle_layout(LayoutType::MasterStack), LayoutType::Vertical);
        assert_eq!(cycle_layout(LayoutType::Vertical), LayoutType::Monocle);
        assert_eq!(cycle_layout(LayoutType::Horizontal), LayoutType::Monocle);
        assert_eq!(cycle_layout(LayoutType::Monocle), LayoutType::Bsp);
        assert_eq!(cycle_layout(LayoutType::Bsp), LayoutType::Grid);
        assert_eq!(cycle_layout(LayoutType::Grid), LayoutType::MasterStack);

        // Deferred fallback/loop
        assert_eq!(cycle_layout(LayoutType::Tabbed), LayoutType::Stacked);
        assert_eq!(cycle_layout(LayoutType::Stacked), LayoutType::MasterStack);

        // Parsing
        assert_eq!("master".parse::<LayoutType>().unwrap(), LayoutType::MasterStack);
        assert_eq!("master-left".parse::<LayoutType>().unwrap(), LayoutType::MasterStack);
        assert_eq!("master-right".parse::<LayoutType>().unwrap(), LayoutType::MasterRight);
        assert_eq!("master-top".parse::<LayoutType>().unwrap(), LayoutType::MasterTop);
        assert_eq!("master-bottom".parse::<LayoutType>().unwrap(), LayoutType::MasterBottom);
        assert_eq!("center-master".parse::<LayoutType>().unwrap(), LayoutType::CenterMaster);
        assert_eq!("centered-master".parse::<LayoutType>().unwrap(), LayoutType::CenterMaster);
        assert_eq!("grid".parse::<LayoutType>().unwrap(), LayoutType::Grid);
        assert_eq!("g".parse::<LayoutType>().unwrap(), LayoutType::Grid);
        assert_eq!("columns".parse::<LayoutType>().unwrap(), LayoutType::Vertical);
        assert_eq!("vertical".parse::<LayoutType>().unwrap(), LayoutType::Vertical);
        assert_eq!("rows".parse::<LayoutType>().unwrap(), LayoutType::Horizontal);
        assert_eq!("horizontal".parse::<LayoutType>().unwrap(), LayoutType::Horizontal);
        assert_eq!("monocle".parse::<LayoutType>().unwrap(), LayoutType::Monocle);
        assert_eq!("bsp".parse::<LayoutType>().unwrap(), LayoutType::Bsp);
        assert!("invalid".parse::<LayoutType>().is_err());

        // Display
        assert_eq!(format!("{}", LayoutType::MasterStack), "master-left");
        assert_eq!(format!("{}", LayoutType::MasterRight), "master-right");
        assert_eq!(format!("{}", LayoutType::MasterTop), "master-top");
        assert_eq!(format!("{}", LayoutType::MasterBottom), "master-bottom");
        assert_eq!(format!("{}", LayoutType::CenterMaster), "center-master");
        assert_eq!(format!("{}", LayoutType::Grid), "grid");
        assert_eq!(format!("{}", LayoutType::Vertical), "vertical");
        assert_eq!(format!("{}", LayoutType::Horizontal), "horizontal");
        assert_eq!(format!("{}", LayoutType::Monocle), "monocle");

        // Symbols
        assert_eq!(LayoutType::MasterStack.symbol(), "[]=");
        assert_eq!(LayoutType::MasterRight.symbol(), "=[]");
        assert_eq!(LayoutType::MasterTop.symbol(), "TT=");
        assert_eq!(LayoutType::MasterBottom.symbol(), "=TT");
        assert_eq!(LayoutType::CenterMaster.symbol(), "|M|");
        assert_eq!(LayoutType::Grid.symbol(), "[G]");
        assert_eq!(LayoutType::Vertical.symbol(), "|||");
        assert_eq!(LayoutType::Horizontal.symbol(), "===");
        assert_eq!(LayoutType::Monocle.symbol(), "[M]");
        assert_eq!(LayoutType::Bsp.symbol(), "[B]");

        // Layout rotation cycling
        assert_eq!(rotate_layout(LayoutType::MasterStack), LayoutType::MasterRight);
        assert_eq!(rotate_layout(LayoutType::MasterRight), LayoutType::MasterTop);
        assert_eq!(rotate_layout(LayoutType::MasterTop), LayoutType::MasterBottom);
        assert_eq!(rotate_layout(LayoutType::MasterBottom), LayoutType::CenterMaster);
        assert_eq!(rotate_layout(LayoutType::CenterMaster), LayoutType::MasterStack);
        assert_eq!(rotate_layout(LayoutType::Vertical), LayoutType::Horizontal);
        assert_eq!(rotate_layout(LayoutType::Horizontal), LayoutType::Vertical);
        assert_eq!(rotate_layout(LayoutType::Monocle), LayoutType::Monocle);
    }

    #[test]
    fn test_vertical_layout_math() {
        let screen = Rect::new(0, 0, 1000, 600);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // 0 windows
        assert!(calculate_vertical(&[], &screen, &config).is_empty());

        // 1 window takes full usable area
        let r1 = calculate_vertical(&[1], &screen, &config);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].x, 10);
        assert_eq!(r1[0].y, 10);
        assert_eq!(r1[0].width, 1000 - 20 - 4);
        assert_eq!(r1[0].height, 600 - 20 - 4);

        // 3 windows side-by-side
        let r3 = calculate_vertical(&[1, 2, 3], &screen, &config);
        assert_eq!(r3.len(), 3);
        assert!(r3[0].x < r3[1].x);
        assert!(r3[1].x < r3[2].x);
        // All windows have same y and height
        assert_eq!(r3[0].y, r3[1].y);
        assert_eq!(r3[1].y, r3[2].y);
        assert_eq!(r3[0].height, r3[2].height);
        // No overlaps
        assert!(!r3[0].overlaps(&r3[1]));
        assert!(!r3[1].overlaps(&r3[2]));
        assert!(!r3[0].overlaps(&r3[2]));
    }

    #[test]
    fn test_horizontal_layout_math() {
        let screen = Rect::new(0, 0, 1000, 600);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // 0 windows
        assert!(calculate_horizontal(&[], &screen, &config).is_empty());

        // 1 window takes full usable area
        let r1 = calculate_horizontal(&[1], &screen, &config);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].x, 10);
        assert_eq!(r1[0].y, 10);

        // 3 windows stacked top to bottom
        let r3 = calculate_horizontal(&[1, 2, 3], &screen, &config);
        assert_eq!(r3.len(), 3);
        assert!(r3[0].y < r3[1].y);
        assert!(r3[1].y < r3[2].y);
        // All windows have same x and width
        assert_eq!(r3[0].x, r3[1].x);
        assert_eq!(r3[1].x, r3[2].x);
        assert_eq!(r3[0].width, r3[2].width);
        // No overlaps
        assert!(!r3[0].overlaps(&r3[1]));
        assert!(!r3[1].overlaps(&r3[2]));
        assert!(!r3[0].overlaps(&r3[2]));
    }

    #[test]
    fn test_monocle_layout_math() {
        let screen = Rect::new(0, 0, 1000, 600);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // 0 windows
        assert!(calculate_monocle(&[], &screen, &config).is_empty());

        // 3 windows all have identical full screen bounds
        let r3 = calculate_monocle(&[1, 2, 3], &screen, &config);
        assert_eq!(r3.len(), 3);
        assert_eq!(r3[0], r3[1]);
        assert_eq!(r3[1], r3[2]);
        assert_eq!(r3[0].width, 1000 - 20 - 4);
        assert_eq!(r3[0].height, 600 - 20 - 4);
    }

    #[test]
    fn test_resize_pane_isolates_changes_to_adjacent_neighbor_only() {
        let screen = Rect::new(0, 0, 1200, 900);
        let config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 0,
            outer_gap: 0,
            border_width: 0,
        };

        // 1. Master/Stack layout: resizing stack window 2 must NOT change master window 1
        let mut tree_ms = BspTree::new();
        tree_ms.rebuild_for_layout(LayoutType::MasterStack, &[1, 2, 3], 0.50);
        let rects_ms_init = tree_ms.calculate_rects(&screen, &config);
        let r1_init = rects_ms_init.iter().find(|(w, _)| *w == 1).unwrap().1;
        let r2_init = rects_ms_init.iter().find(|(w, _)| *w == 2).unwrap().1;
        let r3_init = rects_ms_init.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert!(tree_ms.resize_window(2, 0.05, SplitType::Vertical));
        let rects_ms_post = tree_ms.calculate_rects(&screen, &config);
        let r1_post = rects_ms_post.iter().find(|(w, _)| *w == 1).unwrap().1;
        let r2_post = rects_ms_post.iter().find(|(w, _)| *w == 2).unwrap().1;
        let r3_post = rects_ms_post.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert_eq!(r1_post.width, r1_init.width, "Master width must remain strictly unchanged");
        assert_eq!(r1_post.height, r1_init.height, "Master height must remain strictly unchanged");
        assert!(r2_post.height > r2_init.height, "Window 2 height must increase");
        assert!(r3_post.height < r3_init.height, "Window 3 height must decrease");

        // 2. Vertical layout: 3 columns. Resizing window 1 must NOT change non-adjacent window 3
        let mut tree_v = BspTree::new();
        tree_v.rebuild_for_layout(LayoutType::Vertical, &[1, 2, 3], 0.50);
        let rects_v_init = tree_v.calculate_rects(&screen, &config);
        let v1_init = rects_v_init.iter().find(|(w, _)| *w == 1).unwrap().1;
        let v2_init = rects_v_init.iter().find(|(w, _)| *w == 2).unwrap().1;
        let v3_init = rects_v_init.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert!(tree_v.resize_window(1, 0.05, SplitType::Horizontal));
        let rects_v_post = tree_v.calculate_rects(&screen, &config);
        let v1_post = rects_v_post.iter().find(|(w, _)| *w == 1).unwrap().1;
        let v2_post = rects_v_post.iter().find(|(w, _)| *w == 2).unwrap().1;
        let v3_post = rects_v_post.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert!(v1_post.width > v1_init.width, "Window 1 width must increase");
        assert!(v2_post.width < v2_init.width, "Adjacent Window 2 width must decrease");
        assert_eq!(v3_post.width, v3_init.width, "Non-adjacent Window 3 width must remain strictly unchanged");

        // 3. Horizontal layout: 3 rows. Resizing window 1 must NOT change non-adjacent window 3
        let mut tree_h = BspTree::new();
        tree_h.rebuild_for_layout(LayoutType::Horizontal, &[1, 2, 3], 0.50);
        let rects_h_init = tree_h.calculate_rects(&screen, &config);
        let h1_init = rects_h_init.iter().find(|(w, _)| *w == 1).unwrap().1;
        let h2_init = rects_h_init.iter().find(|(w, _)| *w == 2).unwrap().1;
        let h3_init = rects_h_init.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert!(tree_h.resize_window(1, 0.05, SplitType::Vertical));
        let rects_h_post = tree_h.calculate_rects(&screen, &config);
        let h1_post = rects_h_post.iter().find(|(w, _)| *w == 1).unwrap().1;
        let h2_post = rects_h_post.iter().find(|(w, _)| *w == 2).unwrap().1;
        let h3_post = rects_h_post.iter().find(|(w, _)| *w == 3).unwrap().1;

        assert!(h1_post.height > h1_init.height, "Window 1 height must increase");
        assert!(h2_post.height < h2_init.height, "Adjacent Window 2 height must decrease");
        assert_eq!(h3_post.height, h3_init.height, "Non-adjacent Window 3 height must remain strictly unchanged");
    }

    #[test]
    fn test_rotated_master_layouts_geometry_and_bsp() {
        let screen = Rect::new(0, 0, 1000, 600);
        let config = LayoutConfig {
            master_ratio: 0.60,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // 1. Master Right (Master on right, stack vertically on left)
        let r_right = calculate_master_right(&[1, 2, 3], &screen, &config);
        assert_eq!(r_right.len(), 3);
        // Master window (index 0) must be to the right of stack windows (index 1, 2)
        assert!(r_right[0].x > r_right[1].x, "Master must be to the right of stack");
        assert_eq!(r_right[1].x, r_right[2].x, "Stack windows must be in the same vertical column");
        assert!(r_right[1].y < r_right[2].y, "Stack window 2 must be above stack window 3");
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(!r_right[i].overlaps(&r_right[j]), "Windows must not overlap in master-right");
            }
        }

        // 2. Master Top (Master on top, stack horizontally on bottom)
        let r_top = calculate_master_top(&[1, 2, 3], &screen, &config);
        assert_eq!(r_top.len(), 3);
        // Master window (index 0) must be above stack windows (index 1, 2)
        assert!(r_top[0].y < r_top[1].y, "Master must be above stack");
        assert_eq!(r_top[1].y, r_top[2].y, "Stack windows must be in the same horizontal row");
        assert!(r_top[1].x < r_top[2].x, "Stack window 2 must be to the left of stack window 3");
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(!r_top[i].overlaps(&r_top[j]), "Windows must not overlap in master-top");
            }
        }

        // 3. Master Bottom (Master on bottom, stack horizontally on top)
        let r_bot = calculate_master_bottom(&[1, 2, 3], &screen, &config);
        assert_eq!(r_bot.len(), 3);
        // Master window (index 0) must be below stack windows (index 1, 2)
        assert!(r_bot[0].y > r_bot[1].y, "Master must be below stack");
        assert_eq!(r_bot[1].y, r_bot[2].y, "Stack windows must be in the same horizontal row");
        assert!(r_bot[1].x < r_bot[2].x, "Stack window 2 must be to the left of stack window 3");
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(!r_bot[i].overlaps(&r_bot[j]), "Windows must not overlap in master-bottom");
            }
        }

        // 4. BspTree rebuild and master ratio reporting for all 4 layouts
        let mut tree = BspTree::new();

        tree.rebuild_for_layout(LayoutType::MasterStack, &[1, 2, 3], 0.60);
        let ratio = tree.master_ratio_for_layout(LayoutType::MasterStack).unwrap();
        assert!((ratio - 0.60).abs() < 0.01);

        tree.rebuild_for_layout(LayoutType::MasterRight, &[1, 2, 3], 0.60);
        let ratio = tree.master_ratio_for_layout(LayoutType::MasterRight).unwrap();
        assert!((ratio - 0.60).abs() < 0.01);

        tree.rebuild_for_layout(LayoutType::MasterTop, &[1, 2, 3], 0.60);
        let ratio = tree.master_ratio_for_layout(LayoutType::MasterTop).unwrap();
        assert!((ratio - 0.60).abs() < 0.01);

        tree.rebuild_for_layout(LayoutType::MasterBottom, &[1, 2, 3], 0.60);
        let ratio = tree.master_ratio_for_layout(LayoutType::MasterBottom).unwrap();
        assert!((ratio - 0.60).abs() < 0.01);

        // 5. Center Master (Stack Left: B, D; Master Center; Stack Right: C, E)
        let r_center = calculate_center_master(&[1, 2, 3, 4, 5], &screen, &config);
        assert_eq!(r_center.len(), 5);
        let master = r_center[0];
        let b = r_center[1]; // top-left
        let c = r_center[2]; // top-right
        let d = r_center[3]; // bottom-left
        let e = r_center[4]; // bottom-right

        // Master must be horizontally between Left stack (B, D) and Right stack (C, E)
        assert!(master.x > b.x, "Master must be to the right of left stack");
        assert!(master.x < c.x, "Master must be to the left of right stack");
        assert_eq!(b.x, d.x, "B and D must be in left column");
        assert_eq!(c.x, e.x, "C and E must be in right column");
        assert!(b.y < d.y, "B must be above D");
        assert!(c.y < e.y, "C must be above E");

        for i in 0..5 {
            for j in (i + 1)..5 {
                assert!(!r_center[i].overlaps(&r_center[j]), "Windows must not overlap in center-master");
            }
        }

        tree.rebuild_for_layout(LayoutType::CenterMaster, &[1, 2, 3, 4, 5], 0.60);
        let ratio = tree.master_ratio_for_layout(LayoutType::CenterMaster).unwrap();
        assert!((ratio - 0.60).abs() < 0.05);
    }

    #[test]
    fn test_grid_layout_math() {
        let screen = Rect::new(0, 0, 1000, 600);
        let config = LayoutConfig {
            master_ratio: 0.55,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        // 0 windows: empty
        assert!(calculate_grid(&[], &screen, &config).is_empty());

        // 1 window: fills usable area
        let r1 = calculate_grid(&[1], &screen, &config);
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].x, 10);
        assert_eq!(r1[0].y, 10);
        assert_eq!(r1[0].width, 1000 - 20 - 4);
        assert_eq!(r1[0].height, 600 - 20 - 4);

        // 2 windows: 1 row, 2 columns side-by-side
        let r2 = calculate_grid(&[1, 2], &screen, &config);
        assert_eq!(r2.len(), 2);
        assert_eq!(r2[0].y, r2[1].y);
        assert!(r2[0].x < r2[1].x);
        assert!(!r2[0].overlaps(&r2[1]));

        // 3 windows: 2 rows (row 0: 2 windows, row 1: 1 window full width)
        let r3 = calculate_grid(&[1, 2, 3], &screen, &config);
        assert_eq!(r3.len(), 3);
        assert_eq!(r3[0].y, r3[1].y);
        assert!(r3[0].x < r3[1].x);
        assert!(r3[2].y > r3[0].y);
        assert!(r3[2].width > r3[0].width);
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(!r3[i].overlaps(&r3[j]), "Grid 3 windows must not overlap");
            }
        }

        // 4 windows: 2x2 grid
        let r4 = calculate_grid(&[1, 2, 3, 4], &screen, &config);
        assert_eq!(r4.len(), 4);
        assert_eq!(r4[0].y, r4[1].y);
        assert_eq!(r4[2].y, r4[3].y);
        assert!(r4[0].y < r4[2].y);
        assert_eq!(r4[0].x, r4[2].x);
        assert_eq!(r4[1].x, r4[3].x);
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert!(!r4[i].overlaps(&r4[j]), "Grid 4 windows must not overlap");
            }
        }

        // 5 windows: 2 rows (row 0: 3 windows, row 1: 2 windows)
        let r5 = calculate_grid(&[1, 2, 3, 4, 5], &screen, &config);
        assert_eq!(r5.len(), 5);
        for i in 0..5 {
            for j in (i + 1)..5 {
                assert!(!r5[i].overlaps(&r5[j]), "Grid 5 windows must not overlap");
            }
        }

        // All windows within screen boundaries
        for (idx, r) in r5.iter().enumerate() {
            assert!(r.x >= screen.x, "Window {} x out of bounds", idx);
            assert!(r.y >= screen.y, "Window {} y out of bounds", idx);
            assert!(
                r.x + (r.width + 2 * config.border_width) as i32 <= screen.x + screen.width as i32,
                "Window {} width extends beyond screen",
                idx
            );
            assert!(
                r.y + (r.height + 2 * config.border_width) as i32 <= screen.y + screen.height as i32,
                "Window {} height extends beyond screen",
                idx
            );
        }

        // BspTree rebuild_for_layout
        let mut tree = BspTree::new();
        tree.rebuild_for_layout(LayoutType::Grid, &[1, 2, 3, 4], 0.5);
        assert_eq!(tree.windows(), vec![1, 2, 3, 4]);
    }
}
