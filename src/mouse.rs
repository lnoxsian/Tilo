//! Mouse interaction and window manipulation support.
//!
//! Implements Phase 24 of Plan.md:
//! - Optional mouse interaction without compromising keyboard-first workflow.
//! - `Mod + Left Click`: Focus and drag to move floating windows.
//! - `Mod + Right Click`: Focus and drag to resize floating windows.
//! - Grabs mouse button combinations on client windows.
//! - Tracks pointer drag states via [`MouseAction`].

use crate::keybind::MOD_LOCK;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ButtonIndex, ButtonPressEvent, ConfigureWindowAux, ConnectionExt, EventMask, GrabMode,
    ModMask, MotionNotifyEvent, StackMode, Window,
};

/// The current mouse interaction state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseAction {
    /// No action is currently in progress.
    None,
    /// Dragging a window to move it.
    Moving {
        window: u32,
        start_x: i32,
        start_y: i32,
        start_win_x: i32,
        start_win_y: i32,
    },
    /// Dragging a window to resize it.
    Resizing {
        window: u32,
        start_x: i32,
        start_y: i32,
        start_width: u32,
        start_height: u32,
    },
    /// Dragging a tiled window boundary to resize it in the BSP tree.
    ResizingTiled {
        window: u32,
        start_x: i32,
        start_y: i32,
        initial_rect: crate::layout::Rect,
    },
}

impl Default for MouseAction {
    fn default() -> Self {
        Self::None
    }
}

impl MouseAction {
    /// Returns `true` if a drag operation (move or resize) is currently active.
    pub fn is_active(&self) -> bool {
        !matches!(self, MouseAction::None)
    }

    /// Returns the window ID being manipulated, if any.
    pub fn window(&self) -> Option<u32> {
        match self {
            MouseAction::Moving { window, .. }
            | MouseAction::Resizing { window, .. }
            | MouseAction::ResizingTiled { window, .. } => Some(*window),
            MouseAction::None => None,
        }
    }
}

/// Represents a mouse button binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseBinding {
    /// Modifier mask (e.g., Mod4, Mod1).
    pub modifiers: u16,
    /// Mouse button code (1 for Left, 2 for Middle, 3 for Right).
    pub button: u8,
    /// The action triggered by this binding.
    pub action_type: MouseActionType,
}

impl MouseBinding {
    /// Creates a new `MouseBinding`.
    pub fn new(modifiers: u16, button: u8, action_type: MouseActionType) -> Self {
        Self {
            modifiers,
            button,
            action_type,
        }
    }
}

/// Action types associated with mouse button drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseActionType {
    /// Move a floating window by dragging.
    Move,
    /// Resize a floating window by dragging.
    Resize,
}

/// Returns the standard default mouse bindings for the given modifier mask.
///
/// Defaults:
/// - `Mod + Button1` (Left Click): Move
/// - `Mod + Button3` (Right Click): Resize
pub fn default_mouse_bindings(mod_mask: u16) -> Vec<MouseBinding> {
    vec![
        MouseBinding::new(mod_mask, 1, MouseActionType::Move),
        MouseBinding::new(mod_mask, 3, MouseActionType::Resize),
    ]
}

/// Handles a mouse button press event.
///
/// Inspects the pressed button detail and pointer coordinates, transitioning
/// `action_state` to [`MouseAction::Moving`] or [`MouseAction::Resizing`].
///
/// # Arguments
/// * `event` - The X11 ButtonPress event data.
/// * `action_state` - Mutable reference to the current mouse action state.
pub fn handle_button_press(
    event: &ButtonPressEvent,
    action_state: &mut MouseAction,
) -> Result<(), X11Error> {
    let window = if event.child != 0 {
        event.child
    } else {
        event.event
    };

    let start_x = event.root_x as i32;
    let start_y = event.root_y as i32;
    let start_win_x = event.root_x as i32 - event.event_x as i32;
    let start_win_y = event.root_y as i32 - event.event_y as i32;

    match event.detail {
        1 => {
            // Button 1 (Left Click): Move
            *action_state = MouseAction::Moving {
                window,
                start_x,
                start_y,
                start_win_x,
                start_win_y,
            };
        }
        3 => {
            // Button 3 (Right Click): Resize (initial default 400x300 if untracked)
            *action_state = MouseAction::Resizing {
                window,
                start_x,
                start_y,
                start_width: 400,
                start_height: 300,
            };
        }
        _ => {
            *action_state = MouseAction::None;
        }
    }
    Ok(())
}

/// Handles pointer motion events during a drag operation.
///
/// Computes delta movement from initial click position and dispatches a
/// `ConfigureWindow` request to move or resize the target window.
///
/// # Arguments
/// * `event` - The X11 MotionNotify event data.
/// * `action_state` - The current mouse action state.
/// * `conn` - The X11 connection to apply move/resize operations.
pub fn handle_motion_notify(
    event: &MotionNotifyEvent,
    action_state: &MouseAction,
    conn: &impl Connection,
) -> Result<(), X11Error> {
    match action_state {
        MouseAction::Moving {
            window,
            start_x,
            start_y,
            start_win_x,
            start_win_y,
        } => {
            let delta_x = event.root_x as i32 - start_x;
            let delta_y = event.root_y as i32 - start_y;
            let new_x = start_win_x + delta_x;
            let new_y = start_win_y + delta_y;

            let aux = ConfigureWindowAux::new()
                .x(new_x)
                .y(new_y)
                .stack_mode(StackMode::ABOVE);
            conn.configure_window(*window, &aux)?;
            conn.flush()?;
        }
        MouseAction::Resizing {
            window,
            start_x,
            start_y,
            start_width,
            start_height,
        } => {
            let delta_x = event.root_x as i32 - start_x;
            let delta_y = event.root_y as i32 - start_y;
            let new_w = (*start_width as i32 + delta_x).max(100) as u32;
            let new_h = (*start_height as i32 + delta_y).max(80) as u32;

            let aux = ConfigureWindowAux::new()
                .width(new_w)
                .height(new_h)
                .stack_mode(StackMode::ABOVE);
            conn.configure_window(*window, &aux)?;
            conn.flush()?;
        }
        MouseAction::ResizingTiled { .. } | MouseAction::None => {}
    }
    Ok(())
}

/// Updates mouse button grabs on a client window depending on whether it is focused.
///
/// - **Always**: Grabs `Mod + Button1` (Move) and `Mod + Button3` (Resize) in `ASYNC` mode
///   with `BUTTON_PRESS | BUTTON_RELEASE | POINTER_MOTION`.
/// - **Unfocused (`focused = false`)**: Additionally grabs `Button 1` (Left Click) in `SYNC` mode
///   for click-to-focus.
/// - **Focused (`focused = true`)**: Ungrabs unmodified `Button 1`, allowing the client
///   application to receive all normal mouse clicks, selections, and drags with zero WM interference.
pub fn update_client_button_grabs(
    conn: &impl Connection,
    window: Window,
    bindings: &[MouseBinding],
    num_lock_mask: u16,
    focused: bool,
) -> Result<(), X11Error> {
    // 1. Ungrab all existing buttons first to prevent stale grabs
    let _ = conn.ungrab_button(ButtonIndex::ANY, window, ModMask::ANY);

    let mut modifier_masks = vec![0u16, MOD_LOCK];
    if num_lock_mask != 0 {
        modifier_masks.push(num_lock_mask);
        modifier_masks.push(num_lock_mask | MOD_LOCK);
    }

    // 2. Always grab configured Mod + Button shortcuts in ASYNC mode
    for b in bindings {
        for &m in &modifier_masks {
            let mask = b.modifiers | m;
            let _ = conn.grab_button(
                false,
                window,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                ButtonIndex::from(b.button),
                ModMask::from(mask),
            );
        }
    }

    // 3. For unfocused windows, grab Button 1 without Mod in SYNC mode for click-to-focus
    if !focused {
        for &m in &modifier_masks {
            let _ = conn.grab_button(
                false,
                window,
                EventMask::BUTTON_PRESS,
                GrabMode::SYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                ButtonIndex::M1,
                ModMask::from(m),
            );
        }
    }

    conn.flush()?;
    Ok(())
}

/// Grabs configured mouse button shortcuts and the focus button on a client window.
pub fn grab_client_buttons(
    conn: &impl Connection,
    window: Window,
    bindings: &[MouseBinding],
    num_lock_mask: u16,
) -> Result<(), X11Error> {
    update_client_button_grabs(conn, window, bindings, num_lock_mask, false)
}

/// Releases all mouse button grabs on a client window.
pub fn ungrab_client_buttons(
    conn: &impl Connection,
    window: Window,
) -> Result<(), X11Error> {
    let _ = conn.ungrab_button(ButtonIndex::ANY, window, ModMask::ANY);
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybind::MOD_SUPER;
    use x11rb::protocol::xproto::KeyButMask;

    #[test]
    fn test_mouse_action_default_and_helpers() {
        let action = MouseAction::default();
        assert_eq!(action, MouseAction::None);
        assert!(!action.is_active());
        assert_eq!(action.window(), None);

        let moving = MouseAction::Moving {
            window: 0x100,
            start_x: 10,
            start_y: 20,
            start_win_x: 100,
            start_win_y: 200,
        };
        assert!(moving.is_active());
        assert_eq!(moving.window(), Some(0x100));

        let resizing = MouseAction::Resizing {
            window: 0x200,
            start_x: 50,
            start_y: 60,
            start_width: 800,
            start_height: 600,
        };
        assert!(resizing.is_active());
        assert_eq!(resizing.window(), Some(0x200));
    }

    #[test]
    fn test_default_mouse_bindings() {
        let bindings = default_mouse_bindings(MOD_SUPER);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].button, 1);
        assert_eq!(bindings[0].action_type, MouseActionType::Move);
        assert_eq!(bindings[0].modifiers, MOD_SUPER);

        assert_eq!(bindings[1].button, 3);
        assert_eq!(bindings[1].action_type, MouseActionType::Resize);
        assert_eq!(bindings[1].modifiers, MOD_SUPER);
    }

    #[test]
    fn test_handle_button_press_move_and_resize() {
        let mut action = MouseAction::None;
        let mut ev = ButtonPressEvent {
            response_type: 4,
            detail: 1, // Button 1
            sequence: 1,
            time: 0,
            root: 1,
            event: 100,
            child: 200,
            root_x: 500,
            root_y: 300,
            event_x: 50,
            event_y: 30,
            state: KeyButMask::from(0u16),
            same_screen: true,
        };

        handle_button_press(&ev, &mut action).unwrap();
        assert_eq!(
            action,
            MouseAction::Moving {
                window: 200,
                start_x: 500,
                start_y: 300,
                start_win_x: 450,
                start_win_y: 270,
            }
        );

        ev.detail = 3; // Button 3
        handle_button_press(&ev, &mut action).unwrap();
        match action {
            MouseAction::Resizing {
                window,
                start_x,
                start_y,
                start_width,
                start_height,
            } => {
                assert_eq!(window, 200);
                assert_eq!(start_x, 500);
                assert_eq!(start_y, 300);
                assert_eq!(start_width, 400);
                assert_eq!(start_height, 300);
            }
            _ => panic!("Expected MouseAction::Resizing"),
        }
    }
}
