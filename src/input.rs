//! X11 keyboard input processing, key grabbing, and action execution.
//!
//! This module provides:
//! - [`KeyboardMapping`]: Queries and caches X11 keycode-to-keysym mappings.
//! - [`InputManager`]: Registers global key grabs with NumLock/CapsLock modifier variations.
//! - [`grab_keys`]: Binds configured shortcuts on the root window.
//! - [`handle_key_press`]: Translates raw X11 `KeyPress` events into [`Action`] commands.

use crate::keybind::{
    Action, KeyBind, MOD_LOCK, MOD_MOD2, MOD_SHIFT, XK_NUM_LOCK,
};
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, GrabMode, KeyPressEvent, Keysym, ModMask, Window,
};

/// Caches X11 keyboard layout mappings to translate keycodes into keysyms.
#[derive(Debug, Clone)]
pub struct KeyboardMapping {
    /// Lowest keycode recognized by the X server.
    pub min_keycode: u8,
    /// Highest keycode recognized by the X server.
    pub max_keycode: u8,
    /// Number of keysyms assigned to each keycode column.
    pub keysyms_per_keycode: usize,
    /// Flat array of keysyms indexed by `(keycode - min_keycode) * keysyms_per_keycode`.
    pub keysyms: Vec<Keysym>,
}

impl KeyboardMapping {
    /// Queries the full keyboard mapping from the active X server.
    pub fn query<C: ConnectionExt>(
        conn: &C,
        min_keycode: u8,
        max_keycode: u8,
    ) -> Result<Self, X11Error> {
        let count = max_keycode.saturating_sub(min_keycode) + 1;
        let reply = conn.get_keyboard_mapping(min_keycode, count)?.reply()?;
        Ok(Self {
            min_keycode,
            max_keycode,
            keysyms_per_keycode: reply.keysyms_per_keycode as usize,
            keysyms: reply.keysyms,
        })
    }

    /// Returns the slice of keysyms associated with a specific keycode.
    pub fn keysyms_for_keycode(&self, keycode: u8) -> &[Keysym] {
        if keycode < self.min_keycode
            || keycode > self.max_keycode
            || self.keysyms_per_keycode == 0
        {
            return &[];
        }
        let index = (keycode - self.min_keycode) as usize * self.keysyms_per_keycode;
        if index + self.keysyms_per_keycode <= self.keysyms.len() {
            &self.keysyms[index..index + self.keysyms_per_keycode]
        } else {
            &[]
        }
    }

    /// Translates a keycode and modifier state into a logical Keysym.
    pub fn keycode_to_keysym(&self, keycode: u8, state: u16) -> Keysym {
        let syms = self.keysyms_for_keycode(keycode);
        if syms.is_empty() {
            return 0;
        }
        let shift = (state & MOD_SHIFT) != 0;
        if shift && syms.len() > 1 && syms[1] != 0 {
            syms[1]
        } else {
            syms[0]
        }
    }

    /// Returns all keycodes that can produce the given keysym.
    pub fn keysym_to_keycodes(&self, target_keysym: Keysym) -> Vec<u8> {
        let mut keycodes = Vec::new();
        for kc in self.min_keycode..=self.max_keycode {
            let syms = self.keysyms_for_keycode(kc);
            if syms.contains(&target_keysym) {
                keycodes.push(kc);
            }
        }
        keycodes
    }
}

/// Detects which modifier bit represents `NumLock` on the current X server.
pub fn detect_num_lock_mask<C: Connection>(
    conn: &C,
    mapping: &KeyboardMapping,
) -> u16 {
    if let Ok(reply) = conn.get_modifier_mapping() {
        if let Ok(reply) = reply.reply() {
            let kpm = reply.keycodes_per_modifier() as usize;
            for (mod_idx, chunk) in reply.keycodes.chunks(kpm).enumerate() {
                for &kc in chunk {
                    if kc != 0 {
                        let syms = mapping.keysyms_for_keycode(kc);
                        if syms.contains(&XK_NUM_LOCK) {
                            return 1 << mod_idx;
                        }
                    }
                }
            }
        }
    }
    MOD_MOD2 // Fallback to standard Mod2
}

/// Grabs all registered keybindings globally on the root window.
///
/// Binds 4 variations of each combination to handle active NumLock and CapsLock.
pub fn grab_keys<C: Connection>(
    conn: &C,
    root: Window,
    bindings: &[KeyBind],
) -> Result<(), X11Error> {
    let setup = conn.setup();
    let mapping = KeyboardMapping::query(conn, setup.min_keycode, setup.max_keycode)?;
    let num_lock = detect_num_lock_mask(conn, &mapping);
    let caps_lock = MOD_LOCK;

    for bind in bindings {
        let keycodes = mapping.keysym_to_keycodes(bind.keysym);
        if keycodes.is_empty() {
            continue;
        }

        let base_mask = bind.modifiers;
        let masks = [
            base_mask,
            base_mask | num_lock,
            base_mask | caps_lock,
            base_mask | num_lock | caps_lock,
        ];

        for &kc in &keycodes {
            for &mask in &masks {
                let _ = conn.grab_key(
                    true,
                    root,
                    ModMask::from(mask),
                    kc,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                );
            }
        }
    }

    conn.flush()?;
    Ok(())
}

/// Ungrabs all keys from the root window.
pub fn ungrab_keys<C: Connection>(conn: &C, root: Window) -> Result<(), X11Error> {
    conn.ungrab_key(0, root, ModMask::ANY)?;
    conn.flush()?;
    Ok(())
}

/// Un-registers all current keybindings and registers the new ones with the X server.
///
/// # Arguments
/// * `conn` - The active X11 connection.
/// * `bindings` - The list of new parsed keybindings.
/// * `root` - The root window ID.
pub fn re_register_keybindings<C: Connection>(
    conn: &C,
    bindings: &[KeyBind],
    root: Window,
) -> Result<(), X11Error> {
    let _ = ungrab_keys(conn, root);
    grab_keys(conn, root, bindings)?;
    Ok(())
}

/// Translates a raw X11 `KeyPressEvent` into an actionable [`Action`].
pub fn handle_key_press(
    event: &KeyPressEvent,
    bindings: &[KeyBind],
    mapping: &KeyboardMapping,
    num_lock_mask: u16,
) -> Option<Action> {
    let clean_modifiers = u16::from(event.state) & !(num_lock_mask | MOD_LOCK);
    let keysym = mapping.keycode_to_keysym(event.detail, u16::from(event.state));
    let syms = mapping.keysyms_for_keycode(event.detail);

    for bind in bindings {
        if bind.modifiers == clean_modifiers
            && (bind.keysym == keysym || syms.contains(&bind.keysym))
        {
            return Some(bind.action.clone());
        }
    }

    None
}

/// Manages keyboard configuration, key grabs, and event translation.
#[derive(Debug, Clone)]
pub struct InputManager {
    /// Configured keybindings.
    pub bindings: Vec<KeyBind>,
    /// Active keyboard mapping cached from the X server.
    pub mapping: Option<KeyboardMapping>,
    /// Bitmask for the NumLock modifier.
    pub num_lock_mask: u16,
}

impl InputManager {
    /// Creates a new `InputManager` with the provided keybindings.
    pub fn new(bindings: Vec<KeyBind>) -> Self {
        Self {
            bindings,
            mapping: None,
            num_lock_mask: MOD_MOD2,
        }
    }

    /// Queries the keyboard layout, detects modifiers, and grabs all configured keys on `root`.
    pub fn setup<C: Connection>(&mut self, conn: &C, root: Window) -> Result<(), X11Error> {
        let setup = conn.setup();
        let mapping = KeyboardMapping::query(conn, setup.min_keycode, setup.max_keycode)?;
        self.num_lock_mask = detect_num_lock_mask(conn, &mapping);

        grab_keys(conn, root, &self.bindings)?;
        self.mapping = Some(mapping);
        Ok(())
    }

    /// Re-registers all keybindings with the X server.
    pub fn re_register<C: Connection>(
        &mut self,
        conn: &C,
        root: Window,
        new_bindings: Vec<KeyBind>,
    ) -> Result<(), X11Error> {
        let setup = conn.setup();
        let mapping = KeyboardMapping::query(conn, setup.min_keycode, setup.max_keycode)?;
        self.num_lock_mask = detect_num_lock_mask(conn, &mapping);
        self.mapping = Some(mapping);
        self.bindings = new_bindings;
        re_register_keybindings(conn, &self.bindings, root)
    }

    /// Processes an incoming `KeyPressEvent` and returns the matching action, if any.
    pub fn handle_event(&self, event: &KeyPressEvent) -> Option<Action> {
        let mapping = self.mapping.as_ref()?;
        handle_key_press(event, &self.bindings, mapping, self.num_lock_mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybind::{MOD_SUPER, XK_RETURN, XK_Q};

    #[test]
    fn test_keyboard_mapping_logic() {
        // Mock keyboard mapping
        // Keycode 10 -> 'q' (0x71)
        // Keycode 36 -> 'Return' (0xff0d)
        let mapping = KeyboardMapping {
            min_keycode: 8,
            max_keycode: 40,
            keysyms_per_keycode: 2,
            keysyms: {
                let mut v = vec![0; (40 - 8 + 1) * 2];
                // index for 10 is (10 - 8) * 2 = 4
                v[4] = XK_Q;
                v[5] = XK_Q;
                // index for 36 is (36 - 8) * 2 = 56
                v[56] = XK_RETURN;
                v[57] = XK_RETURN;
                v
            },
        };

        assert_eq!(mapping.keycode_to_keysym(10, 0), XK_Q);
        assert_eq!(mapping.keycode_to_keysym(36, 0), XK_RETURN);
        assert_eq!(mapping.keysym_to_keycodes(XK_Q), vec![10]);
        assert_eq!(mapping.keysym_to_keycodes(XK_RETURN), vec![36]);
    }

    #[test]
    fn test_handle_key_press_matching_and_numlock_stripping() {
        let mapping = KeyboardMapping {
            min_keycode: 8,
            max_keycode: 40,
            keysyms_per_keycode: 2,
            keysyms: {
                let mut v = vec![0; (40 - 8 + 1) * 2];
                v[4] = XK_Q;
                v[5] = XK_Q;
                v[56] = XK_RETURN;
                v[57] = XK_RETURN;
                v
            },
        };

        let bindings = vec![
            KeyBind::new(MOD_SUPER, XK_RETURN, Action::Spawn("alacritty".to_string())),
            KeyBind::new(MOD_SUPER, XK_Q, Action::Close),
        ];

        // 1. Exact match without NumLock
        let ev1 = KeyPressEvent {
            response_type: 2,
            detail: 36, // Return
            sequence: 1,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: x11rb::protocol::xproto::KeyButMask::from(MOD_SUPER),
            same_screen: true,
        };
        assert_eq!(
            handle_key_press(&ev1, &bindings, &mapping, MOD_MOD2),
            Some(Action::Spawn("alacritty".to_string()))
        );

        // 2. Match with active NumLock (Mod2) and CapsLock
        let ev2 = KeyPressEvent {
            response_type: 2,
            detail: 10, // Q
            sequence: 2,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: x11rb::protocol::xproto::KeyButMask::from(MOD_SUPER | MOD_MOD2 | MOD_LOCK),
            same_screen: true,
        };
        assert_eq!(
            handle_key_press(&ev2, &bindings, &mapping, MOD_MOD2),
            Some(Action::Close)
        );

        // 3. Unregistered key
        let ev3 = KeyPressEvent {
            response_type: 2,
            detail: 20,
            sequence: 3,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: x11rb::protocol::xproto::KeyButMask::from(MOD_SUPER),
            same_screen: true,
        };
        assert_eq!(handle_key_press(&ev3, &bindings, &mapping, MOD_MOD2), None);
    }

    #[test]
    fn test_handle_key_press_shifted_numbers() {
        const XK_1: Keysym = 0x0031;
        const XK_EXCLAM: Keysym = 0x0021;

        let mapping = KeyboardMapping {
            min_keycode: 8,
            max_keycode: 40,
            keysyms_per_keycode: 2,
            keysyms: {
                let mut v = vec![0; (40 - 8 + 1) * 2];
                // keycode 10 is '1' (column 0 = '1', column 1 = '!')
                v[(10 - 8) * 2] = XK_1;
                v[(10 - 8) * 2 + 1] = XK_EXCLAM;
                v
            },
        };

        let bindings = vec![
            KeyBind::new(MOD_SUPER, XK_1, Action::Workspace(1)),
            KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_1, Action::MoveToWorkspace(1)),
        ];

        // 1. Mod4 + 1 -> Workspace(1)
        let ev_ws1 = KeyPressEvent {
            response_type: 2,
            detail: 10,
            sequence: 1,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: x11rb::protocol::xproto::KeyButMask::from(MOD_SUPER),
            same_screen: true,
        };
        assert_eq!(
            handle_key_press(&ev_ws1, &bindings, &mapping, MOD_MOD2),
            Some(Action::Workspace(1))
        );

        // 2. Mod4 + Shift + 1 -> MoveToWorkspace(1)
        let ev_move_ws1 = KeyPressEvent {
            response_type: 2,
            detail: 10,
            sequence: 2,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: x11rb::protocol::xproto::KeyButMask::from(MOD_SUPER | MOD_SHIFT),
            same_screen: true,
        };
        assert_eq!(
            handle_key_press(&ev_move_ws1, &bindings, &mapping, MOD_MOD2),
            Some(Action::MoveToWorkspace(1))
        );
    }
}
