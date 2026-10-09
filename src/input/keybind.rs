//! Keybinding definitions, actions, and key parsing.
//!
//! This module defines the high-level [`Action`] commands, the [`KeyBind`]
//! structure, standard keysym and modifier constants, and default keybinding configurations.

use crate::focus::Direction;
pub use x11rb::protocol::xproto::Keysym;

// Standard Modifier Masks
pub const MOD_SHIFT: u16 = 0x0001;
pub const MOD_LOCK: u16 = 0x0002;
pub const MOD_CONTROL: u16 = 0x0004;
pub const MOD_ALT: u16 = 0x0008; // Mod1
pub const MOD_MOD1: u16 = 0x0008;
pub const MOD_MOD2: u16 = 0x0010;
pub const MOD_MOD3: u16 = 0x0020;
pub const MOD_SUPER: u16 = 0x0040; // Mod4 (Windows key)
pub const MOD_MOD4: u16 = 0x0040;
pub const MOD_MOD5: u16 = 0x0080;

// Standard Keysym Constants
pub const XK_BACKSPACE: Keysym = 0xff08;
pub const XK_TAB: Keysym = 0xff09;
pub const XK_RETURN: Keysym = 0xff0d;
pub const XK_ESCAPE: Keysym = 0xff1b;
pub const XK_SPACE: Keysym = 0x0020;
pub const XK_DELETE: Keysym = 0xffff;
pub const XK_COMMA: Keysym = 0x002c;
pub const XK_PERIOD: Keysym = 0x002e;
pub const XK_GRAVE: Keysym = 0x0060; // ` (grave accent / backtick)
pub const XK_ASCIITILDE: Keysym = 0x007e; // ~ (tilde)

pub const XK_HOME: Keysym = 0xff50;
pub const XK_LEFT: Keysym = 0xff51;
pub const XK_UP: Keysym = 0xff52;
pub const XK_RIGHT: Keysym = 0xff53;
pub const XK_DOWN: Keysym = 0xff54;
pub const XK_END: Keysym = 0xff57;

pub const XK_NUM_LOCK: Keysym = 0xff7f;

pub const XK_F5: Keysym = 0xffc2;
pub const XK_F10: Keysym = 0xffc7;
pub const XK_F11: Keysym = 0xffc8;
pub const XK_F12: Keysym = 0xffc9;

// Multimedia Keysym Constants (XF86Audio*)
pub const XF86_AUDIO_LOWER_VOLUME: Keysym = 0x1008_ff11;
pub const XF86_AUDIO_MUTE:         Keysym = 0x1008_ff12;
pub const XF86_AUDIO_RAISE_VOLUME: Keysym = 0x1008_ff13;
pub const XF86_AUDIO_MIC_MUTE:     Keysym = 0x1008_ffb2;

pub const XK_A: Keysym = 0x0061;
pub const XK_B: Keysym = 0x0062;
pub const XK_C: Keysym = 0x0063;
pub const XK_D: Keysym = 0x0064;
pub const XK_E: Keysym = 0x0065;
pub const XK_F: Keysym = 0x0066;
pub const XK_G: Keysym = 0x0067;
pub const XK_H: Keysym = 0x0068;
pub const XK_I: Keysym = 0x0069;
pub const XK_J: Keysym = 0x006a;
pub const XK_K: Keysym = 0x006b;
pub const XK_L: Keysym = 0x006c;
pub const XK_M: Keysym = 0x006d;
pub const XK_N: Keysym = 0x006e;
pub const XK_O: Keysym = 0x006f;
pub const XK_P: Keysym = 0x0070;
pub const XK_Q: Keysym = 0x0071;
pub const XK_R: Keysym = 0x0072;
pub const XK_S: Keysym = 0x0073;
pub const XK_T: Keysym = 0x0074;
pub const XK_U: Keysym = 0x0075;
pub const XK_V: Keysym = 0x0076;
pub const XK_W: Keysym = 0x0077;
pub const XK_X: Keysym = 0x0078;
pub const XK_Y: Keysym = 0x0079;
pub const XK_Z: Keysym = 0x007a;

pub const XK_0: Keysym = 0x0030;
pub const XK_1: Keysym = 0x0031;
pub const XK_2: Keysym = 0x0032;
pub const XK_3: Keysym = 0x0033;
pub const XK_4: Keysym = 0x0034;
pub const XK_5: Keysym = 0x0035;
pub const XK_6: Keysym = 0x0036;
pub const XK_7: Keysym = 0x0037;
pub const XK_8: Keysym = 0x0038;
pub const XK_9: Keysym = 0x0039;

/// High-level actions that can be triggered by keybindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Spawn an external command.
    Spawn(String),
    /// Close the currently focused window.
    Close,
    /// Move focus in a specific spatial direction.
    Focus(Direction),
    /// Move the currently focused window in a specific direction.
    Move(Direction),
    /// Switch to a different workspace by ID.
    Workspace(u32),
    /// Move the currently focused window to a different workspace.
    MoveToWorkspace(u32),
    /// Toggle floating state for the focused window.
    ToggleFloating,
    /// Toggle fullscreen state for the focused window.
    ToggleFullscreen,
    /// Focus the monitor visually to the left of the active monitor.
    FocusMonitorLeft,
    /// Focus the monitor visually to the right of the active monitor.
    FocusMonitorRight,
    /// Move the currently focused window to the specified monitor ID.
    MoveToMonitor(u32),
    /// Reload the window manager configuration.
    Reload,
    /// Toggle the integrated application launcher.
    Launcher,
    /// Toggle the scratchpad window visibility (Phase 23).
    ToggleScratchpad,
    /// Assign the currently focused window as the scratchpad.
    SetScratchpad,
    /// Open the integrated exit/power menu (Logout, Reboot, Poweroff).
    ExitMenu,
    /// Open the exit launcher menu (Phase 21).
    Exit,
    /// Gracefully shutdown and exit the window manager immediately (Phase 32).
    Quit,
    /// Resize the focused tile along a direction by a delta (e.g. 0.05).
    ResizeTile(Direction),
    /// Preselect the split orientation for the next tiled window.
    PreselectSplit(crate::layout::SplitType),
    /// Cycle to the next window layout (Phase 27).
    CycleLayout,
    /// Rotate the master-stack layout orientation.
    RotateMaster,
    /// Set the layout of the current workspace (Phase 27).
    SetLayout(crate::layout::LayoutType),
    /// Focus the next tab in a tabbed layout.
    FocusNextTab,
    /// Focus the previous tab in a tabbed layout.
    FocusPrevTab,
    /// Move the active tab to the left (earlier in client order).
    MoveTabLeft,
    /// Move the active tab to the right (later in client order).
    MoveTabRight,
    /// Directly select a tab by its 1-based index.
    SelectTab(usize),
    /// Increase audio volume by the specified percentage step.
    VolumeUp(u8),
    /// Decrease audio volume by the specified percentage step.
    VolumeDown(u8),
    /// Toggle audio output mute state.
    VolumeToggleMute,
}

/// Represents a single keybinding configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBind {
    /// Modifier mask (e.g., Mod4, Shift).
    pub modifiers: u16,
    /// The specific keysym (e.g., Return, Q, j, k).
    pub keysym: Keysym,
    /// The action to execute when this keybind is pressed.
    pub action: Action,
}

impl KeyBind {
    /// Creates a new `KeyBind`.
    pub fn new(modifiers: u16, keysym: Keysym, action: Action) -> Self {
        Self {
            modifiers,
            keysym,
            action,
        }
    }
}

/// Parses a string representation of a modifier key into its bitmask.
pub fn parse_modifier(name: &str) -> Option<u16> {
    if name.eq_ignore_ascii_case("shift") {
        Some(MOD_SHIFT)
    } else if name.eq_ignore_ascii_case("lock") || name.eq_ignore_ascii_case("capslock") {
        Some(MOD_LOCK)
    } else if name.eq_ignore_ascii_case("control") || name.eq_ignore_ascii_case("ctrl") {
        Some(MOD_CONTROL)
    } else if name.eq_ignore_ascii_case("mod1") || name.eq_ignore_ascii_case("alt") {
        Some(MOD_ALT)
    } else if name.eq_ignore_ascii_case("mod2") {
        Some(MOD_MOD2)
    } else if name.eq_ignore_ascii_case("mod3") {
        Some(MOD_MOD3)
    } else if name.eq_ignore_ascii_case("mod4")
        || name.eq_ignore_ascii_case("super")
        || name.eq_ignore_ascii_case("win")
    {
        Some(MOD_SUPER)
    } else if name.eq_ignore_ascii_case("mod5") {
        Some(MOD_MOD5)
    } else {
        None
    }
}

/// Parses a keysym name into its corresponding `Keysym` code.
pub fn parse_keysym(name: &str) -> Option<Keysym> {
    match name {
        "Return" | "Enter" => Some(XK_RETURN),
        "Escape" | "Esc" => Some(XK_ESCAPE),
        "BackSpace" | "Backspace" => Some(XK_BACKSPACE),
        "Tab" => Some(XK_TAB),
        "Space" | "space" => Some(XK_SPACE),
        "Delete" => Some(XK_DELETE),
        "Left" | "left" => Some(XK_LEFT),
        "Up" | "up" => Some(XK_UP),
        "Right" | "right" => Some(XK_RIGHT),
        "Down" | "down" => Some(XK_DOWN),
        "comma" | "," => Some(XK_COMMA),
        "period" | "." => Some(XK_PERIOD),
        "grave" | "`" => Some(XK_GRAVE),
        "asciitilde" | "~" | "tilde" => Some(XK_ASCIITILDE),
        "Num_Lock" | "NumLock" => Some(XK_NUM_LOCK),
        s if s.len() > 1 && (s.starts_with('F') || s.starts_with('f')) => {
            if let Ok(num) = s[1..].parse::<u32>() {
                if (1..=12).contains(&num) {
                    Some(0xffbe + (num - 1))
                } else {
                    None
                }
            } else {
                None
            }
        }
        "XF86AudioRaiseVolume" | "AudioRaiseVolume" | "VolumeUp" => Some(XF86_AUDIO_RAISE_VOLUME),
        "XF86AudioLowerVolume" | "AudioLowerVolume" | "VolumeDown" => Some(XF86_AUDIO_LOWER_VOLUME),
        "XF86AudioMute" | "AudioMute" | "VolumeMute" => Some(XF86_AUDIO_MUTE),
        "XF86AudioMicMute" | "AudioMicMute" => Some(XF86_AUDIO_MIC_MUTE),
        s if s.len() == 1 => {
            if let Some(c) = s.chars().next() {
                if c.is_ascii() {
                    Some(c.to_ascii_lowercase() as Keysym)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parses a keybinding specification (e.g., `"Mod4+Return"`, `"Mod4+Shift+q"`) with an Action.
pub fn parse_keybind(spec: &str, action: Action) -> Result<KeyBind, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("Empty keybind specification".to_string());
    }

    let (mods_part, key_part) = match trimmed.rsplit_once('+') {
        Some((m, k)) => (Some(m), k.trim()),
        None => (None, trimmed),
    };

    let mut modifiers = 0u16;
    if let Some(mods) = mods_part {
        for mod_name in mods.split('+').map(|s| s.trim()) {
            if mod_name.is_empty() {
                continue;
            }
            match parse_modifier(mod_name) {
                Some(mask) => modifiers |= mask,
                None => return Err(format!("Unknown modifier: {}", mod_name)),
            }
        }
    }

    let keysym = parse_keysym(key_part)
        .ok_or_else(|| format!("Unknown keysym: {}", key_part))?;

    Ok(KeyBind::new(modifiers, keysym, action))
}

/// Returns the default set of keybindings for the window manager.
pub fn default_keybindings() -> Vec<KeyBind> {
    vec![
        // Applications
        KeyBind::new(
            MOD_SUPER,
            XK_RETURN,
            Action::Spawn("alacritty".to_string()),
        ),
        // Window Management & Exit
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_Q, Action::Exit),
        KeyBind::new(MOD_SUPER, XK_Q, Action::Close),
        // Spatial Focus (Vim keys)
        KeyBind::new(MOD_SUPER, XK_H, Action::Focus(Direction::Left)),
        KeyBind::new(MOD_SUPER, XK_J, Action::Focus(Direction::Down)),
        KeyBind::new(MOD_SUPER, XK_K, Action::Focus(Direction::Up)),
        KeyBind::new(MOD_SUPER, XK_L, Action::Focus(Direction::Right)),
        // Spatial Focus (Arrow keys)
        KeyBind::new(MOD_SUPER, XK_LEFT, Action::Focus(Direction::Left)),
        KeyBind::new(MOD_SUPER, XK_DOWN, Action::Focus(Direction::Down)),
        KeyBind::new(MOD_SUPER, XK_UP, Action::Focus(Direction::Up)),
        KeyBind::new(MOD_SUPER, XK_RIGHT, Action::Focus(Direction::Right)),
        // Window Movement (Vim keys)
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_H, Action::Move(Direction::Left)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_J, Action::Move(Direction::Down)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_K, Action::Move(Direction::Up)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_L, Action::Move(Direction::Right)),
        // Window Movement (Arrow keys)
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_LEFT, Action::Move(Direction::Left)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_DOWN, Action::Move(Direction::Down)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_UP, Action::Move(Direction::Up)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_RIGHT, Action::Move(Direction::Right)),
        // Window States & Launcher & Scratchpad
        KeyBind::new(MOD_SUPER, XK_F, Action::ToggleFullscreen),
        KeyBind::new(MOD_SUPER, XK_D, Action::Launcher),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_SPACE, Action::ToggleFloating),
        KeyBind::new(MOD_SUPER, XK_SPACE, Action::CycleLayout),
        KeyBind::new(MOD_SUPER, XK_R, Action::RotateMaster),
        KeyBind::new(MOD_SUPER, XK_TAB, Action::FocusNextTab),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_TAB, Action::FocusPrevTab),
        KeyBind::new(MOD_SUPER, XK_GRAVE, Action::ToggleScratchpad),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_GRAVE, Action::SetScratchpad),
        // Workspaces 1-9
        KeyBind::new(MOD_SUPER, XK_1, Action::Workspace(1)),
        KeyBind::new(MOD_SUPER, XK_2, Action::Workspace(2)),
        KeyBind::new(MOD_SUPER, XK_3, Action::Workspace(3)),
        KeyBind::new(MOD_SUPER, XK_4, Action::Workspace(4)),
        KeyBind::new(MOD_SUPER, XK_5, Action::Workspace(5)),
        KeyBind::new(MOD_SUPER, XK_6, Action::Workspace(6)),
        KeyBind::new(MOD_SUPER, XK_7, Action::Workspace(7)),
        KeyBind::new(MOD_SUPER, XK_8, Action::Workspace(8)),
        KeyBind::new(MOD_SUPER, XK_9, Action::Workspace(9)),
        // Move to Workspace 1-9
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_1, Action::MoveToWorkspace(1)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_2, Action::MoveToWorkspace(2)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_3, Action::MoveToWorkspace(3)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_4, Action::MoveToWorkspace(4)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_5, Action::MoveToWorkspace(5)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_6, Action::MoveToWorkspace(6)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_7, Action::MoveToWorkspace(7)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_8, Action::MoveToWorkspace(8)),
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_9, Action::MoveToWorkspace(9)),
        // Multi-Monitor Focus
        KeyBind::new(MOD_SUPER, XK_COMMA, Action::FocusMonitorLeft),
        KeyBind::new(MOD_SUPER, XK_PERIOD, Action::FocusMonitorRight),
        // BSP Tile Resizing
        KeyBind::new(MOD_SUPER | MOD_CONTROL, XK_LEFT, Action::ResizeTile(Direction::Left)),
        KeyBind::new(MOD_SUPER | MOD_CONTROL, XK_RIGHT, Action::ResizeTile(Direction::Right)),
        KeyBind::new(MOD_SUPER | MOD_CONTROL, XK_UP, Action::ResizeTile(Direction::Up)),
        KeyBind::new(MOD_SUPER | MOD_CONTROL, XK_DOWN, Action::ResizeTile(Direction::Down)),
        // BSP Split Preselection
        KeyBind::new(MOD_SUPER, XK_V, Action::PreselectSplit(crate::layout::SplitType::Vertical)),
        KeyBind::new(MOD_SUPER, XK_B, Action::PreselectSplit(crate::layout::SplitType::Horizontal)),
        // WM Controls
        KeyBind::new(MOD_SUPER | MOD_SHIFT, XK_R, Action::Reload),
        // Audio & Media Keys
        KeyBind::new(0, XF86_AUDIO_RAISE_VOLUME, Action::VolumeUp(5)),
        KeyBind::new(0, XF86_AUDIO_LOWER_VOLUME, Action::VolumeDown(5)),
        KeyBind::new(0, XF86_AUDIO_MUTE, Action::VolumeToggleMute),
        KeyBind::new(MOD_SUPER, XK_F12, Action::VolumeUp(5)),
        KeyBind::new(MOD_SUPER, XK_F11, Action::VolumeDown(5)),
        KeyBind::new(MOD_SUPER, XK_F10, Action::VolumeToggleMute),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modifier() {
        assert_eq!(parse_modifier("Mod4"), Some(MOD_SUPER));
        assert_eq!(parse_modifier("super"), Some(MOD_SUPER));
        assert_eq!(parse_modifier("Shift"), Some(MOD_SHIFT));
        assert_eq!(parse_modifier("Control"), Some(MOD_CONTROL));
        assert_eq!(parse_modifier("Alt"), Some(MOD_ALT));
        assert_eq!(parse_modifier("InvalidMod"), None);
    }

    #[test]
    fn test_parse_keysym() {
        assert_eq!(parse_keysym("Return"), Some(XK_RETURN));
        assert_eq!(parse_keysym("q"), Some(XK_Q));
        assert_eq!(parse_keysym("Q"), Some(XK_Q));
        assert_eq!(parse_keysym("1"), Some(XK_1));
        assert_eq!(parse_keysym("Space"), Some(XK_SPACE));
        assert_eq!(parse_keysym("F5"), Some(XK_F5));
        assert_eq!(parse_keysym("XF86AudioRaiseVolume"), Some(XF86_AUDIO_RAISE_VOLUME));
        assert_eq!(parse_keysym("VolumeDown"), Some(XF86_AUDIO_LOWER_VOLUME));
        assert_eq!(parse_keysym("VolumeMute"), Some(XF86_AUDIO_MUTE));
        assert_eq!(parse_keysym("NonExistentKey"), None);
    }

    #[test]
    fn test_parse_keybind() {
        let bind = parse_keybind("Mod4+Return", Action::Spawn("alacritty".to_string())).unwrap();
        assert_eq!(bind.modifiers, MOD_SUPER);
        assert_eq!(bind.keysym, XK_RETURN);
        assert_eq!(bind.action, Action::Spawn("alacritty".to_string()));

        let bind2 = parse_keybind("Mod4+Shift+q", Action::Close).unwrap();
        assert_eq!(bind2.modifiers, MOD_SUPER | MOD_SHIFT);
        assert_eq!(bind2.keysym, XK_Q);
        assert_eq!(bind2.action, Action::Close);

        let bind_vol = parse_keybind("XF86AudioRaiseVolume", Action::VolumeUp(5)).unwrap();
        assert_eq!(bind_vol.modifiers, 0);
        assert_eq!(bind_vol.keysym, XF86_AUDIO_RAISE_VOLUME);
        assert_eq!(bind_vol.action, Action::VolumeUp(5));
    }

    #[test]
    fn test_default_keybindings_not_empty() {
        let binds = default_keybindings();
        assert!(!binds.is_empty());
        assert!(binds.iter().any(|b| b.action == Action::Close));
        assert!(binds.iter().any(|b| b.action == Action::Exit));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER && b.keysym == XK_R && b.action == Action::RotateMaster));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER | MOD_SHIFT && b.keysym == XK_R && b.action == Action::Reload));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER && b.keysym == XK_SPACE && b.action == Action::CycleLayout));
        assert!(binds.iter().any(|b| b.modifiers == 0 && b.keysym == XF86_AUDIO_RAISE_VOLUME && b.action == Action::VolumeUp(5)));
        assert!(binds.iter().any(|b| b.modifiers == 0 && b.keysym == XF86_AUDIO_LOWER_VOLUME && b.action == Action::VolumeDown(5)));
        assert!(binds.iter().any(|b| b.modifiers == 0 && b.keysym == XF86_AUDIO_MUTE && b.action == Action::VolumeToggleMute));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER && b.keysym == XK_F12 && b.action == Action::VolumeUp(5)));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER && b.keysym == XK_F11 && b.action == Action::VolumeDown(5)));
        assert!(binds.iter().any(|b| b.modifiers == MOD_SUPER && b.keysym == XK_F10 && b.action == Action::VolumeToggleMute));
    }
}
