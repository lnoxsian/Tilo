//! Layout engine and tiling algorithms.
//!
//! Provides geometric calculations for arranging windows in tiling layouts,
//! starting with the master/stack layout with configurable gaps and borders.

pub mod bsp;
pub mod grid;
pub mod master_stack;
pub mod monocle;
pub mod tabbed;

pub use bsp::*;
pub use grid::*;
pub use master_stack::*;
pub use monocle::*;
pub use tabbed::*;

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
        LayoutType::Monocle => LayoutType::Tabbed,
        LayoutType::Tabbed => LayoutType::Bsp,
        LayoutType::Bsp => LayoutType::Grid,
        LayoutType::Grid => LayoutType::MasterStack,
        LayoutType::Stacked => LayoutType::MasterStack,
    }
}

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
        LayoutType::Tabbed => calculate_tabbed(clients, screen_rect, config),
        LayoutType::Monocle | LayoutType::Stacked => {
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
        assert_eq!(cycle_layout(LayoutType::Monocle), LayoutType::Tabbed);
        assert_eq!(cycle_layout(LayoutType::Tabbed), LayoutType::Bsp);
        assert_eq!(cycle_layout(LayoutType::Bsp), LayoutType::Grid);
        assert_eq!(cycle_layout(LayoutType::Grid), LayoutType::MasterStack);

        // Deferred fallback/loop
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
