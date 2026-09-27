//! Focus management, spatial navigation, and window appearance.
//!
//! This module implements predictable keyboard-driven focus management:
//! - [`Direction`]: 2D directional navigation (`Up`, `Down`, `Left`, `Right`).
//! - [`FocusState`]: Tracking active window and focus history restoration.
//! - [`WorkspaceFocus`]: Maintaining one focused client per workspace.
//! - [`BorderConfig`]: Window border colors for focused, normal, and urgent states.
//! - Directional spatial focus algorithm with projection overlap heuristic and optional wrapping.
//! - ICCCM `WM_TAKE_FOCUS` protocol and X11 input focus helpers.

use std::collections::HashMap;
use crate::layout::Rect;
use crate::x11::X11Error;
use x11rb::protocol::xproto::{
    Atom, ChangeWindowAttributesAux, ClientMessageData, ClientMessageEvent, ConfigureWindowAux,
    ConnectionExt, EventMask, InputFocus, Timestamp, Window,
};

/// Direction for spatial focus and movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Returns the opposite direction.
    pub fn opposite(&self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }

    /// Returns `true` if the direction is horizontal (`Left` or `Right`).
    pub fn is_horizontal(&self) -> bool {
        matches!(self, Direction::Left | Direction::Right)
    }

    /// Returns `true` if the direction is vertical (`Up` or `Down`).
    pub fn is_vertical(&self) -> bool {
        matches!(self, Direction::Up | Direction::Down)
    }
}

/// Configuration for window borders and focus colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderConfig {
    /// Border width in pixels.
    pub width: u32,
    /// Pixel color for the currently focused window (default `#ffffff`).
    pub focused_color: u32,
    /// Pixel color for inactive/unfocused windows (default `#444444`).
    pub normal_color: u32,
    /// Pixel color for urgent windows (default `#ff0000`).
    pub urgent_color: u32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            width: 2,
            focused_color: 0x00ffffff,
            normal_color: 0x00444444,
            urgent_color: 0x00ff0000,
        }
    }
}

/// Represents the focus state of a given workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusState {
    /// The currently focused window, if any.
    pub focused_window: Option<Window>,
    /// A history of focused windows to restore focus when a window closes.
    pub focus_history: Vec<Window>,
}

impl FocusState {
    /// Creates a new, empty `FocusState`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the focused window, pushing any previously focused window onto the history stack.
    pub fn focus(&mut self, window: Window) {
        if self.focused_window == Some(window) {
            return;
        }
        if let Some(prev) = self.focused_window {
            self.focus_history.retain(|&w| w != prev);
            self.focus_history.push(prev);
        }
        self.focus_history.retain(|&w| w != window);
        self.focused_window = Some(window);
    }

    /// Clears the currently focused window, pushing it to history if present.
    pub fn unfocus(&mut self) -> Option<Window> {
        let prev = self.focused_window.take();
        if let Some(w) = prev {
            self.focus_history.retain(|&x| x != w);
            self.focus_history.push(w);
        }
        prev
    }

    /// Removes a window from tracking (e.g. when closed or destroyed).
    /// If the removed window was the active focus, restores focus from the most recent history entry.
    /// Returns the new focused window, or `None` if no window remains.
    pub fn remove_window(&mut self, window: Window) -> Option<Window> {
        self.focus_history.retain(|&w| w != window);
        if self.focused_window == Some(window) {
            self.focused_window = self.focus_history.pop();
        }
        self.focused_window
    }

    /// Returns the currently focused window.
    pub fn focused(&self) -> Option<Window> {
        self.focused_window
    }

    /// Returns the history of previously focused windows.
    pub fn history(&self) -> &[Window] {
        &self.focus_history
    }

    /// Clears all focus state and history.
    pub fn clear(&mut self) {
        self.focused_window = None;
        self.focus_history.clear();
    }
}

/// Manages focus state per workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceFocus {
    /// Map of workspace ID -> FocusState.
    workspaces: HashMap<u32, FocusState>,
}

impl WorkspaceFocus {
    /// Creates a new `WorkspaceFocus` manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gets or creates a mutable reference to the `FocusState` for the given workspace.
    pub fn get_or_create(&mut self, workspace_id: u32) -> &mut FocusState {
        self.workspaces.entry(workspace_id).or_default()
    }

    /// Returns a reference to the `FocusState` for the given workspace.
    pub fn get(&self, workspace_id: u32) -> Option<&FocusState> {
        self.workspaces.get(&workspace_id)
    }

    /// Returns the currently focused window on the given workspace.
    pub fn focused_window_on(&self, workspace_id: u32) -> Option<Window> {
        self.workspaces.get(&workspace_id).and_then(|s| s.focused())
    }

    /// Sets the focused window on the specified workspace.
    pub fn focus(&mut self, workspace_id: u32, window: Window) {
        self.get_or_create(workspace_id).focus(window);
    }

    /// Unfocuses the specified workspace.
    pub fn unfocus(&mut self, workspace_id: u32) -> Option<Window> {
        self.get_or_create(workspace_id).unfocus()
    }

    /// Removes a window from the specified workspace's focus tracking.
    pub fn remove_window(&mut self, workspace_id: u32, window: Window) -> Option<Window> {
        self.get_or_create(workspace_id).remove_window(window)
    }

    /// Removes a window across all workspace focus trackings.
    pub fn remove_window_all(&mut self, window: Window) {
        for state in self.workspaces.values_mut() {
            state.remove_window(window);
        }
    }
}

/// Finds the adjacent window in the given spatial direction among visible clients.
///
/// # Arguments
/// * `direction` - The spatial direction to navigate (`Up`, `Down`, `Left`, `Right`).
/// * `current` - The currently focused window ID.
/// * `clients` - Slice of `(Window, Rect)` pairs representing visible windows and their geometries.
/// * `wrap` - If `true`, wrap around to the opposite side if no candidate exists in `direction`.
///
/// # Returns
/// The window ID of the best spatial neighbor, or `None` if no suitable window is found.
pub fn find_neighbor(
    direction: Direction,
    current: Option<Window>,
    clients: &[(Window, Rect)],
    wrap: bool,
) -> Option<Window> {
    if clients.is_empty() {
        return None;
    }

    let current_window = match current {
        Some(w) => w,
        None => return Some(clients[0].0),
    };

    // Find the current window's rect
    let current_rect = match clients.iter().find(|(w, _)| *w == current_window) {
        Some((_, r)) => *r,
        None => return Some(clients[0].0),
    };

    let cx_c = current_rect.x as f64 + (current_rect.width as f64) / 2.0;
    let cy_c = current_rect.y as f64 + (current_rect.height as f64) / 2.0;

    let mut best_candidate: Option<(Window, f64)> = None;

    for &(win, rect) in clients {
        if win == current_window {
            continue;
        }

        let cx_i = rect.x as f64 + (rect.width as f64) / 2.0;
        let cy_i = rect.y as f64 + (rect.height as f64) / 2.0;

        let is_valid = match direction {
            Direction::Left => cx_i < cx_c - 1.0,
            Direction::Right => cx_i > cx_c + 1.0,
            Direction::Up => cy_i < cy_c - 1.0,
            Direction::Down => cy_i > cy_c + 1.0,
        };

        if !is_valid {
            continue;
        }

        let score = match direction {
            Direction::Left | Direction::Right => {
                let dx = (cx_i - cx_c).abs();
                let dy = (cy_i - cy_c).abs();
                // Overlap along Y
                let y1 = current_rect.y;
                let y2 = current_rect.y + current_rect.height as i32;
                let ry1 = rect.y;
                let ry2 = rect.y + rect.height as i32;
                let overlaps = y1 < ry2 && ry1 < y2;
                let weight = if overlaps { 1.0 } else { 5.0 };
                dx * dx + (weight * dy) * (weight * dy)
            }
            Direction::Up | Direction::Down => {
                let dx = (cx_i - cx_c).abs();
                let dy = (cy_i - cy_c).abs();
                // Overlap along X
                let x1 = current_rect.x;
                let x2 = current_rect.x + current_rect.width as i32;
                let rx1 = rect.x;
                let rx2 = rect.x + rect.width as i32;
                let overlaps = x1 < rx2 && rx1 < x2;
                let weight = if overlaps { 1.0 } else { 5.0 };
                dy * dy + (weight * dx) * (weight * dx)
            }
        };

        match best_candidate {
            Some((_, best_score)) => {
                if score < best_score {
                    best_candidate = Some((win, score));
                }
            }
            None => {
                best_candidate = Some((win, score));
            }
        }
    }

    if let Some((best_win, _)) = best_candidate {
        return Some(best_win);
    }

    // No direct candidate found in that direction.
    if !wrap {
        return None;
    }

    // Wrap around to the opposite edge
    let mut wrap_best: Option<(Window, f64)> = None;

    for &(win, rect) in clients {
        if win == current_window {
            continue;
        }
        let cx_i = rect.x as f64 + (rect.width as f64) / 2.0;
        let cy_i = rect.y as f64 + (rect.height as f64) / 2.0;

        let (primary, secondary) = match direction {
            // Wrapping Left: pick the rightmost window (largest cx_i)
            Direction::Left => (-cx_i, (cy_i - cy_c).abs()),
            // Wrapping Right: pick the leftmost window (smallest cx_i)
            Direction::Right => (cx_i, (cy_i - cy_c).abs()),
            // Wrapping Up: pick the bottommost window (largest cy_i)
            Direction::Up => (-cy_i, (cx_i - cx_c).abs()),
            // Wrapping Down: pick the topmost window (smallest cy_i)
            Direction::Down => (cy_i, (cx_i - cx_c).abs()),
        };

        let score = primary * 10000.0 + secondary;

        match wrap_best {
            Some((_, best_score)) => {
                if score < best_score {
                    wrap_best = Some((win, score));
                }
            }
            None => {
                wrap_best = Some((win, score));
            }
        }
    }

    wrap_best.map(|(w, _)| w)
}

/// Focuses the next window in a given spatial direction without wrapping.
pub fn focus_direction(
    direction: Direction,
    current: Option<Window>,
    clients: &[(Window, Rect)],
) -> Option<Window> {
    find_neighbor(direction, current, clients, false)
}

/// Focuses the next window in a given spatial direction with wrapping around screen edges.
pub fn focus_direction_wrap(
    direction: Direction,
    current: Option<Window>,
    clients: &[(Window, Rect)],
) -> Option<Window> {
    find_neighbor(direction, current, clients, true)
}

/// Updates a window's border pixel color in X11.
pub fn set_window_border_color<C: ConnectionExt>(
    conn: &C,
    window: Window,
    color: u32,
) -> Result<(), X11Error> {
    let aux = ChangeWindowAttributesAux::new().border_pixel(color);
    conn.change_window_attributes(window, &aux)?;
    Ok(())
}

/// Updates a window's border width in X11.
pub fn set_window_border_width<C: ConnectionExt>(
    conn: &C,
    window: Window,
    width: u32,
) -> Result<(), X11Error> {
    let aux = ConfigureWindowAux::new().border_width(width);
    conn.configure_window(window, &aux)?;
    Ok(())
}

/// Sets input focus to the specified window with `POINTER_ROOT` revert mode.
pub fn set_input_focus<C: ConnectionExt>(
    conn: &C,
    window: Window,
) -> Result<(), X11Error> {
    conn.set_input_focus(InputFocus::POINTER_ROOT, window, x11rb::CURRENT_TIME)?;
    Ok(())
}

/// Sends ICCCM `WM_TAKE_FOCUS` ClientMessage to the client window.
pub fn send_take_focus<C: ConnectionExt>(
    conn: &C,
    window: Window,
    wm_protocols: Atom,
    wm_take_focus: Atom,
    timestamp: Timestamp,
) -> Result<(), X11Error> {
    let data = [
        wm_take_focus,
        timestamp,
        0,
        0,
        0,
    ];
    let event = ClientMessageEvent {
        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window,
        type_: wm_protocols,
        data: ClientMessageData::from(data),
    };
    conn.send_event(false, window, EventMask::NO_EVENT, event)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_state_lifecycle() {
        let mut state = FocusState::new();
        assert_eq!(state.focused(), None);
        assert!(state.history().is_empty());

        // Focus first window
        state.focus(100);
        assert_eq!(state.focused(), Some(100));
        assert!(state.history().is_empty());

        // Focusing same window does not duplicate history
        state.focus(100);
        assert_eq!(state.focused(), Some(100));
        assert!(state.history().is_empty());

        // Focus second window
        state.focus(200);
        assert_eq!(state.focused(), Some(200));
        assert_eq!(state.history(), &[100]);

        // Focus third window
        state.focus(300);
        assert_eq!(state.focused(), Some(300));
        assert_eq!(state.history(), &[100, 200]);

        // Refocus window from history moves it to top without duplicates
        state.focus(100);
        assert_eq!(state.focused(), Some(100));
        assert_eq!(state.history(), &[200, 300]);

        // Removing active window restores from history
        let next = state.remove_window(100);
        assert_eq!(next, Some(300));
        assert_eq!(state.focused(), Some(300));
        assert_eq!(state.history(), &[200]);

        // Removing non-active window cleans history
        state.remove_window(200);
        assert_eq!(state.focused(), Some(300));
        assert!(state.history().is_empty());

        // Removing last window leaves focus empty
        let next = state.remove_window(300);
        assert_eq!(next, None);
        assert_eq!(state.focused(), None);
    }

    #[test]
    fn test_workspace_focus_per_workspace() {
        let mut ws_focus = WorkspaceFocus::new();

        // Workspace 1
        ws_focus.focus(1, 101);
        ws_focus.focus(1, 102);

        // Workspace 2
        ws_focus.focus(2, 201);

        assert_eq!(ws_focus.focused_window_on(1), Some(102));
        assert_eq!(ws_focus.focused_window_on(2), Some(201));
        assert_eq!(ws_focus.focused_window_on(3), None);

        // Closing window on ws 1 restores previous focus on ws 1
        let restored = ws_focus.remove_window(1, 102);
        assert_eq!(restored, Some(101));
        assert_eq!(ws_focus.focused_window_on(1), Some(101));

        // Workspace 2 focus is unaffected
        assert_eq!(ws_focus.focused_window_on(2), Some(201));
    }

    #[test]
    fn test_direction_helpers() {
        assert_eq!(Direction::Up.opposite(), Direction::Down);
        assert_eq!(Direction::Down.opposite(), Direction::Up);
        assert_eq!(Direction::Left.opposite(), Direction::Right);
        assert_eq!(Direction::Right.opposite(), Direction::Left);

        assert!(Direction::Left.is_horizontal());
        assert!(Direction::Right.is_horizontal());
        assert!(!Direction::Up.is_horizontal());

        assert!(Direction::Up.is_vertical());
        assert!(Direction::Down.is_vertical());
        assert!(!Direction::Left.is_vertical());
    }

    #[test]
    fn test_spatial_navigation_master_stack() {
        // Master window on the left, Stack 1 and Stack 2 on the right
        // Master: [0, 0, 960, 1080]
        // S1:     [960, 0, 960, 540]
        // S2:     [960, 540, 960, 540]
        let master = 1;
        let s1 = 2;
        let s2 = 3;

        let clients = vec![
            (master, Rect { x: 0, y: 0, width: 960, height: 1080 }),
            (s1, Rect { x: 960, y: 0, width: 960, height: 540 }),
            (s2, Rect { x: 960, y: 540, width: 960, height: 540 }),
        ];

        // From Master -> Right goes to S1 (first candidate in order)
        assert_eq!(focus_direction(Direction::Right, Some(master), &clients), Some(s1));

        // From Master -> Left (no wrap) is None
        assert_eq!(focus_direction(Direction::Left, Some(master), &clients), None);

        // From Master -> Left (with wrap) wraps to right stack window
        assert_eq!(focus_direction_wrap(Direction::Left, Some(master), &clients), Some(s1));

        // From S1 -> Down goes to S2
        assert_eq!(focus_direction(Direction::Down, Some(s1), &clients), Some(s2));

        // From S1 -> Left goes to Master
        assert_eq!(focus_direction(Direction::Left, Some(s1), &clients), Some(master));

        // From S2 -> Up goes to S1
        assert_eq!(focus_direction(Direction::Up, Some(s2), &clients), Some(s1));

        // From S2 -> Left goes to Master
        assert_eq!(focus_direction(Direction::Left, Some(s2), &clients), Some(master));

        // From S2 -> Right (no wrap) is None
        assert_eq!(focus_direction(Direction::Right, Some(s2), &clients), None);

        // From S2 -> Right (with wrap) wraps to Master
        assert_eq!(focus_direction_wrap(Direction::Right, Some(s2), &clients), Some(master));
    }

    #[test]
    fn test_spatial_navigation_2x2_grid() {
        // Grid:
        // [A] [B]
        // [C] [D]
        let a = 10;
        let b = 20;
        let c = 30;
        let d = 40;

        let clients = vec![
            (a, Rect { x: 0, y: 0, width: 500, height: 500 }),
            (b, Rect { x: 500, y: 0, width: 500, height: 500 }),
            (c, Rect { x: 0, y: 500, width: 500, height: 500 }),
            (d, Rect { x: 500, y: 500, width: 500, height: 500 }),
        ];

        // From A
        assert_eq!(focus_direction(Direction::Right, Some(a), &clients), Some(b));
        assert_eq!(focus_direction(Direction::Down, Some(a), &clients), Some(c));
        assert_eq!(focus_direction(Direction::Left, Some(a), &clients), None);
        assert_eq!(focus_direction(Direction::Up, Some(a), &clients), None);

        // From B
        assert_eq!(focus_direction(Direction::Left, Some(b), &clients), Some(a));
        assert_eq!(focus_direction(Direction::Down, Some(b), &clients), Some(d));

        // From C
        assert_eq!(focus_direction(Direction::Up, Some(c), &clients), Some(a));
        assert_eq!(focus_direction(Direction::Right, Some(c), &clients), Some(d));

        // From D
        assert_eq!(focus_direction(Direction::Up, Some(d), &clients), Some(b));
        assert_eq!(focus_direction(Direction::Left, Some(d), &clients), Some(c));
    }
}
