//! ICCCM (Inter-Client Communication Conventions Manual) protocol support.
//!
//! This module handles standard ICCCM client-window protocols:
//! - `WM_PROTOCOLS` negotiation (`WM_DELETE_WINDOW`, `WM_TAKE_FOCUS`)
//! - `WM_STATE` tracking (`Withdrawn`, `Normal`, `Iconic`)
//! - `WM_NORMAL_HINTS` (size hints: min, max, base, resize increments, aspect ratio)
//! - `WM_HINTS` (urgency, input focus, initial state)

use crate::atoms::Atoms;
use crate::layout::Rect;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConfigureNotifyEvent, ConnectionExt,
    EventMask, PropMode, Window, CONFIGURE_NOTIFY_EVENT,
};
use x11rb::wrapper::ConnectionExt as _;

/// Represents the `WM_STATE` property values as specified in ICCCM 4.1.3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum WmState {
    /// Window is withdrawn (hidden or unmanaged).
    Withdrawn = 0,
    /// Window is in its normal state (mapped and visible).
    Normal = 1,
    /// Window is minimized (iconified or hidden on inactive workspace).
    Iconic = 3,
}

impl WmState {
    /// Converts a raw u32 to `WmState`.
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::Withdrawn),
            1 => Some(Self::Normal),
            3 => Some(Self::Iconic),
            _ => None,
        }
    }
}

/// Custom struct caching interned X11 atoms for ICCCM protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcccmAtoms {
    /// `WM_PROTOCOLS` atom.
    pub wm_protocols: Atom,
    /// `WM_DELETE_WINDOW` atom.
    pub wm_delete_window: Atom,
    /// `WM_TAKE_FOCUS` atom.
    pub wm_take_focus: Atom,
    /// `WM_STATE` atom.
    pub wm_state: Atom,
}

impl From<&Atoms> for IcccmAtoms {
    fn from(atoms: &Atoms) -> Self {
        Self {
            wm_protocols: atoms.wm_protocols,
            wm_delete_window: atoms.wm_delete_window,
            wm_take_focus: atoms.wm_take_focus,
            wm_state: atoms.wm_state,
        }
    }
}

impl From<Atoms> for IcccmAtoms {
    fn from(atoms: Atoms) -> Self {
        (&atoms).into()
    }
}

/// Window size hints parsed from `WM_NORMAL_HINTS`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WmSizeHints {
    /// Minimum allowed window width.
    pub min_width: Option<u32>,
    /// Minimum allowed window height.
    pub min_height: Option<u32>,
    /// Maximum allowed window width.
    pub max_width: Option<u32>,
    /// Maximum allowed window height.
    pub max_height: Option<u32>,
    /// Base window width for resizing.
    pub base_width: Option<u32>,
    /// Base window height for resizing.
    pub base_height: Option<u32>,
    /// Horizontal resizing increment.
    pub width_inc: Option<u32>,
    /// Vertical resizing increment.
    pub height_inc: Option<u32>,
    /// Minimum aspect ratio (numerator, denominator).
    pub min_aspect: Option<(u32, u32)>,
    /// Maximum aspect ratio (numerator, denominator).
    pub max_aspect: Option<(u32, u32)>,
}

impl WmSizeHints {
    /// Returns true if this window has fixed dimensions (min_size == max_size > 0).
    pub fn is_fixed_size(&self) -> bool {
        match (self.min_width, self.max_width, self.min_height, self.max_height) {
            (Some(min_w), Some(max_w), Some(min_h), Some(max_h)) => {
                min_w > 0 && min_h > 0 && min_w == max_w && min_h == max_h
            }
            _ => false,
        }
    }

    /// Clamps and snaps dimensions according to size hints.
    pub fn apply_hints(&self, mut width: u32, mut height: u32) -> (u32, u32) {
        let base_w = self.base_width.unwrap_or(0);
        let base_h = self.base_height.unwrap_or(0);

        // Aspect ratio constraints: ICCCM 4.1.2.3
        let (mut delta_w, mut delta_h) = if self.base_width.is_some() || self.base_height.is_some() {
            (
                width.saturating_sub(base_w).max(1),
                height.saturating_sub(base_h).max(1),
            )
        } else {
            (width.max(1), height.max(1))
        };

        if let Some((min_x, min_y)) = self.min_aspect
            && min_x > 0
            && min_y > 0
        {
            // delta_w / delta_h >= min_x / min_y => delta_w * min_y >= delta_h * min_x
            if (delta_w as u64) * (min_y as u64) < (delta_h as u64) * (min_x as u64) {
                // Too narrow: reduce height to satisfy minimum ratio
                delta_h = ((delta_w as u64) * (min_y as u64) / (min_x as u64)) as u32;
            }
        }

        if let Some((max_x, max_y)) = self.max_aspect
            && max_x > 0
            && max_y > 0
        {
            // delta_w / delta_h <= max_x / max_y => delta_w * max_y <= delta_h * max_x
            if (delta_w as u64) * (max_y as u64) > (delta_h as u64) * (max_x as u64) {
                // Too wide: reduce width to satisfy maximum ratio
                delta_w = ((delta_h as u64) * (max_x as u64) / (max_y as u64)) as u32;
            }
        }

        if self.base_width.is_some() || self.base_height.is_some() {
            width = base_w + delta_w;
            height = base_h + delta_h;
        } else {
            width = delta_w;
            height = delta_h;
        }

        // Apply resize increments
        let inc_base_w = self.base_width.or(self.min_width).unwrap_or(0);
        if let Some(w_inc) = self.width_inc
            && w_inc > 1
            && width >= inc_base_w
        {
            width = inc_base_w + ((width - inc_base_w) / w_inc) * w_inc;
        }

        let inc_base_h = self.base_height.or(self.min_height).unwrap_or(0);
        if let Some(h_inc) = self.height_inc
            && h_inc > 1
            && height >= inc_base_h
        {
            height = inc_base_h + ((height - inc_base_h) / h_inc) * h_inc;
        }

        // Clamp to min and max bounds
        if let Some(min_w) = self.min_width {
            width = width.max(min_w);
        }
        if let Some(min_h) = self.min_height {
            height = height.max(min_h);
        }
        if let Some(max_w) = self.max_width
            && max_w > 0
        {
            width = width.min(max_w);
        }
        if let Some(max_h) = self.max_height
            && max_h > 0
        {
            height = height.min(max_h);
        }

        (width, height)
    }
}

/// Parses the raw 32-bit values of `WM_NORMAL_HINTS` into `WmSizeHints`.
pub fn parse_wm_size_hints(data: &[u32]) -> WmSizeHints {
    let mut hints = WmSizeHints::default();
    if data.is_empty() {
        return hints;
    }

    let flags = data[0];
    const P_MIN_SIZE: u32 = 1 << 4;
    const P_MAX_SIZE: u32 = 1 << 5;
    const P_RESIZE_INC: u32 = 1 << 6;
    const P_ASPECT: u32 = 1 << 7;
    const P_BASE_SIZE: u32 = 1 << 8;

    if (flags & P_MIN_SIZE) != 0 && data.len() >= 7 {
        hints.min_width = Some(data[5]);
        hints.min_height = Some(data[6]);
    }
    if (flags & P_MAX_SIZE) != 0 && data.len() >= 9 {
        hints.max_width = Some(data[7]);
        hints.max_height = Some(data[8]);
    }
    if (flags & P_RESIZE_INC) != 0 && data.len() >= 11 {
        hints.width_inc = Some(data[9]);
        hints.height_inc = Some(data[10]);
    }
    if (flags & P_ASPECT) != 0 && data.len() >= 15 {
        hints.min_aspect = Some((data[11], data[12]));
        hints.max_aspect = Some((data[13], data[14]));
    }
    if (flags & P_BASE_SIZE) != 0 && data.len() >= 17 {
        hints.base_width = Some(data[15]);
        hints.base_height = Some(data[16]);
    }

    hints
}

/// Parsed window manager hints from `WM_HINTS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WmHints {
    /// True if the window accepts keyboard input focus.
    pub input: bool,
    /// Initial desired window state.
    pub initial_state: Option<WmState>,
    /// True if the window has requested urgency attention.
    pub is_urgent: bool,
}

impl Default for WmHints {
    fn default() -> Self {
        Self {
            input: true,
            initial_state: None,
            is_urgent: false,
        }
    }
}

/// Parses the raw 32-bit values of `WM_HINTS` into `WmHints`.
pub fn parse_wm_hints(data: &[u32]) -> WmHints {
    let mut hints = WmHints::default();
    if data.is_empty() {
        return hints;
    }

    let flags = data[0];
    const INPUT_HINT: u32 = 1 << 0;
    const STATE_HINT: u32 = 1 << 1;
    const URGENCY_HINT: u32 = 1 << 8;

    if (flags & INPUT_HINT) != 0 && data.len() >= 2 {
        hints.input = data[1] != 0;
    }
    if (flags & STATE_HINT) != 0 && data.len() >= 3 {
        hints.initial_state = WmState::from_u32(data[2]);
    }
    if (flags & URGENCY_HINT) != 0 {
        hints.is_urgent = true;
    }

    hints
}

/// Queries `WM_PROTOCOLS` on a window to determine if it supports a specific protocol.
pub fn supports_protocol(
    conn: &impl Connection,
    window: Window,
    protocol: Atom,
    wm_protocols: Atom,
) -> Result<bool, X11Error> {
    let reply = conn
        .get_property(false, window, wm_protocols, AtomEnum::ATOM, 0, 32)?
        .reply();

    if let Ok(prop) = reply
        && let Some(mut atoms) = prop.value32()
    {
        return Ok(atoms.any(|a| a == protocol));
    }
    Ok(false)
}

/// Attempts to close a client window gracefully via `WM_DELETE_WINDOW`.
///
/// If the client supports `WM_DELETE_WINDOW` in its `WM_PROTOCOLS`, sends a `ClientMessage`
/// event requesting graceful shutdown. Otherwise, falls back to forcibly killing the client
/// via `KillClient`.
pub fn send_wm_delete(
    conn: &impl Connection,
    window: Window,
    atoms: &IcccmAtoms,
) -> Result<(), X11Error> {
    if supports_protocol(conn, window, atoms.wm_delete_window, atoms.wm_protocols)? {
        let data = [atoms.wm_delete_window, x11rb::CURRENT_TIME, 0, 0, 0];
        let event = ClientMessageEvent::new(
            32,
            window,
            atoms.wm_protocols,
            ClientMessageData::from(data),
        );
        conn.send_event(false, window, EventMask::NO_EVENT, event)?;
        conn.flush()?;
    } else {
        conn.kill_client(window)?;
        conn.flush()?;
    }
    Ok(())
}

/// Sends ICCCM `WM_TAKE_FOCUS` `ClientMessage` to a client window.
pub fn send_wm_take_focus(
    conn: &impl Connection,
    window: Window,
    atoms: &IcccmAtoms,
    timestamp: x11rb::protocol::xproto::Timestamp,
) -> Result<(), X11Error> {
    let data = [atoms.wm_take_focus, timestamp, 0, 0, 0];
    let event = ClientMessageEvent::new(
        32,
        window,
        atoms.wm_protocols,
        ClientMessageData::from(data),
    );
    conn.send_event(false, window, EventMask::NO_EVENT, event)?;
    conn.flush()?;
    Ok(())
}

/// Sends a synthetic `ConfigureNotify` event to a client window according to ICCCM 4.1.5.
pub fn send_synthetic_configure_notify(
    conn: &impl Connection,
    window: Window,
    rect: Rect,
    border_width: u32,
) -> Result<(), X11Error> {
    let event = ConfigureNotifyEvent {
        response_type: CONFIGURE_NOTIFY_EVENT,
        sequence: 0,
        event: window,
        window,
        above_sibling: x11rb::NONE,
        x: rect.x as i16,
        y: rect.y as i16,
        width: rect.width.min(u16::MAX as u32) as u16,
        height: rect.height.min(u16::MAX as u32) as u16,
        border_width: border_width.min(u16::MAX as u32) as u16,
        override_redirect: false,
    };
    conn.send_event(false, window, EventMask::STRUCTURE_NOTIFY, event)?;
    conn.flush()?;
    Ok(())
}

/// Updates the `WM_STATE` property on a client window.
///
/// Follows ICCCM 4.1.3.1: format 32, type `WM_STATE`, containing `[state, icon_window]`.
pub fn set_wm_state(
    conn: &impl Connection,
    window: Window,
    state: WmState,
    atoms: &IcccmAtoms,
) -> Result<(), X11Error> {
    let data = [state as u32, 0];
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.wm_state,
        atoms.wm_state,
        &data,
    )?;
    conn.flush()?;
    Ok(())
}

/// Retrieves the current `WM_STATE` property from a client window.
pub fn get_wm_state(
    conn: &impl Connection,
    window: Window,
    atoms: &IcccmAtoms,
) -> Result<Option<WmState>, X11Error> {
    let reply = conn
        .get_property(false, window, atoms.wm_state, atoms.wm_state, 0, 2)?
        .reply();

    if let Ok(prop) = reply
        && let Some(mut val32) = prop.value32()
        && let Some(state_num) = val32.next()
    {
        return Ok(WmState::from_u32(state_num));
    }
    Ok(None)
}

/// Retrieves and parses `WM_NORMAL_HINTS` from a client window.
pub fn get_wm_normal_hints(
    conn: &impl Connection,
    window: Window,
) -> Result<Option<WmSizeHints>, X11Error> {
    if let Ok(reply) = conn.get_property(
        false,
        window,
        AtomEnum::WM_NORMAL_HINTS,
        AtomEnum::ANY,
        0,
        32,
    ) && let Ok(prop) = reply.reply()
        && let Some(val32) = prop.value32()
    {
        let vec: Vec<u32> = val32.collect();
        return Ok(Some(parse_wm_size_hints(&vec)));
    }
    Ok(None)
}

/// Retrieves and parses `WM_HINTS` from a client window.
pub fn get_wm_hints(conn: &impl Connection, window: Window) -> Result<Option<WmHints>, X11Error> {
    if let Ok(reply) = conn.get_property(false, window, AtomEnum::WM_HINTS, AtomEnum::ANY, 0, 32)
        && let Ok(prop) = reply.reply()
        && let Some(val32) = prop.value32()
    {
        let vec: Vec<u32> = val32.collect();
        return Ok(Some(parse_wm_hints(&vec)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wm_state_conversion() {
        assert_eq!(WmState::from_u32(0), Some(WmState::Withdrawn));
        assert_eq!(WmState::from_u32(1), Some(WmState::Normal));
        assert_eq!(WmState::from_u32(3), Some(WmState::Iconic));
        assert_eq!(WmState::from_u32(2), None);
        assert_eq!(WmState::from_u32(99), None);
    }

    #[test]
    fn test_parse_wm_size_hints() {
        // flags: PMinSize (1<<4) | PMaxSize (1<<5) | PResizeInc (1<<6)
        let flags = (1 << 4) | (1 << 5) | (1 << 6);
        let mut data = vec![0u32; 18];
        data[0] = flags;
        data[5] = 200; // min_width
        data[6] = 150; // min_height
        data[7] = 800; // max_width
        data[8] = 600; // max_height
        data[9] = 10; // width_inc
        data[10] = 5; // height_inc

        let hints = parse_wm_size_hints(&data);
        assert_eq!(hints.min_width, Some(200));
        assert_eq!(hints.min_height, Some(150));
        assert_eq!(hints.max_width, Some(800));
        assert_eq!(hints.max_height, Some(600));
        assert_eq!(hints.width_inc, Some(10));
        assert_eq!(hints.height_inc, Some(5));

        // Clamping & increment snapping
        let (w, h) = hints.apply_hints(100, 100);
        assert_eq!(w, 200); // clamped to min
        assert_eq!(h, 150); // clamped to min

        let (w2, h2) = hints.apply_hints(1000, 900);
        assert_eq!(w2, 800); // clamped to max
        assert_eq!(h2, 600); // clamped to max

        let (w3, h3) = hints.apply_hints(255, 173);
        assert_eq!(w3, 250); // 200 + (55 / 10)*10 = 250
        assert_eq!(h3, 170); // 150 + (23 / 5)*5 = 170
    }

    #[test]
    fn test_parse_wm_hints() {
        // flags: InputHint (1<<0) | StateHint (1<<1) | UrgencyHint (1<<8)
        let flags = (1 << 0) | (1 << 1) | (1 << 8);
        let data = vec![flags, 0, 1]; // input = false, initial_state = Normal, urgent = true

        let hints = parse_wm_hints(&data);
        assert!(!hints.input);
        assert_eq!(hints.initial_state, Some(WmState::Normal));
        assert!(hints.is_urgent);

        let default_hints = parse_wm_hints(&[]);
        assert!(default_hints.input);
        assert_eq!(default_hints.initial_state, None);
        assert!(!default_hints.is_urgent);
    }

    #[test]
    fn test_wm_size_hints_aspect_ratio_min_and_max() {
        let mut hints = WmSizeHints::default();
        // Aspect ratio 16:9 min
        hints.min_aspect = Some((16, 9));
        // 800x600 is 4:3 (12:9), too narrow for 16:9 -> height should be reduced to 450
        let (w, h) = hints.apply_hints(800, 600);
        assert_eq!(w, 800);
        assert_eq!(h, 450);

        // Aspect ratio 4:3 max
        let mut hints_max = WmSizeHints::default();
        hints_max.max_aspect = Some((4, 3));
        // 800x400 is 2:1 (6:3), too wide for 4:3 -> width should be reduced to 533
        let (w2, h2) = hints_max.apply_hints(800, 400);
        assert_eq!(w2, 533);
        assert_eq!(h2, 400);
    }

    #[test]
    fn test_wm_size_hints_fixed_size() {
        let mut hints = WmSizeHints::default();
        assert!(!hints.is_fixed_size());

        hints.min_width = Some(400);
        hints.max_width = Some(400);
        hints.min_height = Some(300);
        hints.max_height = Some(300);
        assert!(hints.is_fixed_size());

        hints.max_width = Some(500);
        assert!(!hints.is_fixed_size());
    }
}
