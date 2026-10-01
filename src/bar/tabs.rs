//! Statusbar tab component representation and layout calculation.
//!
//! Provides lightweight tab metadata and layout calculation for the
//! Status-Bar Integrated Tabbed Layout.

use crate::layout::Rect;
use x11rb::protocol::xproto::Window;

/// Lightweight read-only metadata representing a single tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// X11 Window identifier of the client.
    pub window: Window,
    /// Current window title.
    pub title: String,
    /// 1-based index of this tab in the active workspace tab group.
    pub index: usize,
    /// True if this is the currently focused active tab.
    pub focused: bool,
    /// True if the window has requested attention (ICCCM urgency hint).
    pub urgent: bool,
}

impl TabInfo {
    /// Creates a new `TabInfo` instance.
    pub fn new(window: Window, title: String, index: usize, focused: bool, urgent: bool) -> Self {
        Self {
            window,
            title,
            index,
            focused,
            urgent,
        }
    }
}

/// Configuration options for the tabbed layout bar display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabbedConfig {
    /// Whether to display numeric tab indexes (e.g. "[1 term]" vs "[term]").
    pub show_index: bool,
    /// Maximum pixel width for an individual tab pill.
    pub max_width: u32,
    /// Whether mouse clicks on tabs focus/close them.
    pub mouse: bool,
    /// Whether mouse wheel over tab region switches tabs.
    pub wheel_switch: bool,
}

impl Default for TabbedConfig {
    fn default() -> Self {
        Self {
            show_index: false,
            max_width: 160,
            mouse: true,
            wheel_switch: true,
        }
    }
}

/// Computed geometry and display label for a tab item on the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabGeometry {
    /// Target X11 client window.
    pub window: Window,
    /// Overall bounding rectangle of the tab pill on the status bar.
    pub rect: Rect,
    /// Label text to render inside the tab pill.
    pub label: String,
    /// True if this tab is the active focused tab.
    pub focused: bool,
    /// True if this tab has an active urgency hint.
    pub urgent: bool,
}

/// Calculates tab item geometries and formatted labels to fit within available status bar space.
///
/// # Arguments
/// * `tabs` - Slice of tab metadata for the active workspace.
/// * `start_x` - Horizontal pixel coordinate where the tab region begins.
/// * `available_width` - Total horizontal pixel width allocated for the tab region.
/// * `bar_height` - Total vertical height of the status bar in pixels.
/// * `config` - Tabbed layout display configuration.
/// * `char_width` - Average character width in pixels for the loaded font.
///
/// # Returns
/// A vector of [`TabGeometry`] entries and the total width consumed.
pub fn calculate_tab_geometries(
    tabs: &[TabInfo],
    start_x: i16,
    available_width: u16,
    bar_height: u16,
    config: &TabbedConfig,
    char_width: u16,
) -> (Vec<TabGeometry>, u16) {
    if tabs.is_empty() || available_width == 0 {
        return (Vec::new(), 0);
    }

    let n = tabs.len();
    let gap = 4i16;
    let pad = 6i16;
    let cw = char_width.max(1) as i16;

    // Calculate maximum width per tab respecting available space
    let total_gaps = (n.saturating_sub(1) as i16) * gap;
    let usable_w = (available_width as i16).saturating_sub(total_gaps).max(20);
    let per_tab_budget = (usable_w / n as i16).clamp(36, config.max_width as i16);

    let mut geometries = Vec::with_capacity(n);
    let mut cur_x = start_x;
    let pill_y = 3i16;
    let pill_h = bar_height.saturating_sub(6);

    for tab in tabs {
        if cur_x >= start_x + available_width as i16 {
            break;
        }

        // 1. Build base title
        let clean_title = if tab.title.trim().is_empty() {
            "window"
        } else {
            tab.title.trim()
        };
        let base_title = if config.show_index {
            format!("{} {}", tab.index, clean_title)
        } else {
            clean_title.to_string()
        };

        let title_with_urgent = if tab.urgent && !tab.focused {
            format!("! {}", base_title)
        } else {
            base_title
        };

        // 2. Truncate title if it exceeds the budgeted width
        let max_label_chars = (((per_tab_budget - (pad * 2)) / cw).max(3)) as usize;
        let display_title = if title_with_urgent.chars().count() > max_label_chars {
            let truncated: String = title_with_urgent
                .chars()
                .take(max_label_chars.saturating_sub(3))
                .collect();
            format!("{}...", truncated)
        } else {
            title_with_urgent
        };

        let label = format!("[{}]", display_title);
        let natural_w = (label.chars().count() as i16 * cw) + (pad * 2);
        let actual_w = natural_w.min(per_tab_budget);

        // Clamp to not overflow available_width
        let remaining_space = (start_x + available_width as i16) - cur_x;
        if remaining_space < 20 {
            break;
        }
        let final_w = actual_w.min(remaining_space);

        let rect = Rect::new(cur_x as i32, pill_y as i32, final_w as u32, pill_h as u32);
        geometries.push(TabGeometry {
            window: tab.window,
            rect,
            label,
            focused: tab.focused,
            urgent: tab.urgent,
        });

        cur_x += final_w + gap;
    }

    let consumed_width = (cur_x - start_x).max(0) as u16;
    (geometries, consumed_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_info_creation() {
        let tab = TabInfo::new(0x100, "Terminal".to_string(), 1, true, false);
        assert_eq!(tab.window, 0x100);
        assert_eq!(tab.title, "Terminal");
        assert_eq!(tab.index, 1);
        assert!(tab.focused);
        assert!(!tab.urgent);
    }

    #[test]
    fn test_calculate_tab_geometries_empty() {
        let (geoms, w) = calculate_tab_geometries(&[], 100, 500, 24, &TabbedConfig::default(), 8);
        assert!(geoms.is_empty());
        assert_eq!(w, 0);
    }

    #[test]
    fn test_calculate_tab_geometries_multiple() {
        let tabs = vec![
            TabInfo::new(1, "term".to_string(), 1, true, false),
            TabInfo::new(2, "nvim".to_string(), 2, false, false),
            TabInfo::new(3, "firefox".to_string(), 3, false, true),
        ];

        let (geoms, w) = calculate_tab_geometries(&tabs, 100, 600, 24, &TabbedConfig::default(), 8);
        assert_eq!(geoms.len(), 3);
        assert!(w > 0);
        assert_eq!(geoms[0].label, "[term]");
        assert!(geoms[0].focused);
        assert_eq!(geoms[1].label, "[nvim]");
        assert!(!geoms[1].focused);
        assert_eq!(geoms[2].label, "[! firefox]");
        assert!(geoms[2].urgent);
    }

    #[test]
    fn test_calculate_tab_geometries_with_index() {
        let tabs = vec![
            TabInfo::new(1, "term".to_string(), 1, true, false),
            TabInfo::new(2, "nvim".to_string(), 2, false, false),
        ];

        let config = TabbedConfig {
            show_index: true,
            ..Default::default()
        };

        let (geoms, _) = calculate_tab_geometries(&tabs, 50, 400, 24, &config, 8);
        assert_eq!(geoms.len(), 2);
        assert_eq!(geoms[0].label, "[1 term]");
        assert_eq!(geoms[1].label, "[2 nvim]");
    }
}
