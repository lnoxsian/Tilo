//! Tabbed layout algorithm.

use x11rb::protocol::xproto::Window;
use super::{calculate_monocle, LayoutConfig, Rect};

/// Calculates the bounding rectangles for a tabbed layout.
///
/// In a tabbed layout, managed client windows each occupy the full content area,
/// while the window manager keeps only the currently active tab visible and displays
/// all tabs within the common WM status bar.
pub fn calculate_tabbed(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect> {
    calculate_monocle(clients, screen_rect, config)
}
