//! EWMH (Extended Window Manager Hints) protocol support.
//!
//! Implements root window and client window EWMH properties:
//! - `_NET_SUPPORTED`
//! - `_NET_CLIENT_LIST`
//! - `_NET_ACTIVE_WINDOW`
//! - `_NET_CURRENT_DESKTOP`
//! - `_NET_NUMBER_OF_DESKTOPS`
//! - `_NET_DESKTOP_NAMES`
//! - `_NET_DESKTOP_GEOMETRY`
//! - `_NET_WORKAREA`
//! - `_NET_SUPPORTING_WM_CHECK`
//! - `_NET_WM_STATE` (`_NET_WM_STATE_FULLSCREEN`, `_NET_WM_STATE_ABOVE`)
//! - ClientMessages: `_NET_ACTIVE_WINDOW`, `_NET_CURRENT_DESKTOP`, `_NET_WM_STATE`, `_NET_CLOSE_WINDOW`

use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, PropMode, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

/// Action to apply when receiving a `_NET_WM_STATE` ClientMessage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NetWmStateAction {
    /// 0 = Remove state.
    Remove = 0,
    /// 1 = Add state.
    Add = 1,
    /// 2 = Toggle state.
    Toggle = 2,
}

impl NetWmStateAction {
    /// Converts a raw u32 to `NetWmStateAction`.
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::Remove),
            1 => Some(Self::Add),
            2 => Some(Self::Toggle),
            _ => None,
        }
    }
}

/// Custom struct caching interned X11 atoms for EWMH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EwmhAtoms {
    pub net_supported: Atom,
    pub net_client_list: Atom,
    pub net_active_window: Atom,
    pub net_current_desktop: Atom,
    pub net_number_of_desktops: Atom,
    pub net_desktop_names: Atom,
    pub net_desktop_geometry: Atom,
    pub net_workarea: Atom,
    pub net_supporting_wm_check: Atom,
    pub net_wm_name: Atom,
    pub net_wm_state: Atom,
    pub net_wm_state_fullscreen: Atom,
    pub net_wm_state_above: Atom,
    pub net_close_window: Atom,
    pub net_wm_pid: Atom,
    pub utf8_string: Atom,
}

impl EwmhAtoms {
    /// Interns all required EWMH atoms from the X11 connection.
    pub fn intern(conn: &impl Connection) -> Result<Self, X11Error> {
        let net_supported = conn.intern_atom(false, b"_NET_SUPPORTED")?;
        let net_client_list = conn.intern_atom(false, b"_NET_CLIENT_LIST")?;
        let net_active_window = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?;
        let net_current_desktop = conn.intern_atom(false, b"_NET_CURRENT_DESKTOP")?;
        let net_number_of_desktops = conn.intern_atom(false, b"_NET_NUMBER_OF_DESKTOPS")?;
        let net_desktop_names = conn.intern_atom(false, b"_NET_DESKTOP_NAMES")?;
        let net_desktop_geometry = conn.intern_atom(false, b"_NET_DESKTOP_GEOMETRY")?;
        let net_workarea = conn.intern_atom(false, b"_NET_WORKAREA")?;
        let net_supporting_wm_check = conn.intern_atom(false, b"_NET_SUPPORTING_WM_CHECK")?;
        let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?;
        let net_wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?;
        let net_wm_state_fullscreen = conn.intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")?;
        let net_wm_state_above = conn.intern_atom(false, b"_NET_WM_STATE_ABOVE")?;
        let net_close_window = conn.intern_atom(false, b"_NET_CLOSE_WINDOW")?;
        let net_wm_pid = conn.intern_atom(false, b"_NET_WM_PID")?;
        let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?;

        Ok(Self {
            net_supported: net_supported.reply()?.atom,
            net_client_list: net_client_list.reply()?.atom,
            net_active_window: net_active_window.reply()?.atom,
            net_current_desktop: net_current_desktop.reply()?.atom,
            net_number_of_desktops: net_number_of_desktops.reply()?.atom,
            net_desktop_names: net_desktop_names.reply()?.atom,
            net_desktop_geometry: net_desktop_geometry.reply()?.atom,
            net_workarea: net_workarea.reply()?.atom,
            net_supporting_wm_check: net_supporting_wm_check.reply()?.atom,
            net_wm_name: net_wm_name.reply()?.atom,
            net_wm_state: net_wm_state.reply()?.atom,
            net_wm_state_fullscreen: net_wm_state_fullscreen.reply()?.atom,
            net_wm_state_above: net_wm_state_above.reply()?.atom,
            net_close_window: net_close_window.reply()?.atom,
            net_wm_pid: net_wm_pid.reply()?.atom,
            utf8_string: utf8_string.reply()?.atom,
        })
    }

    /// Returns the array of supported EWMH atoms to set on `_NET_SUPPORTED`.
    pub fn supported_atoms(&self) -> [Atom; 15] {
        [
            self.net_supported,
            self.net_client_list,
            self.net_active_window,
            self.net_current_desktop,
            self.net_number_of_desktops,
            self.net_desktop_names,
            self.net_desktop_geometry,
            self.net_workarea,
            self.net_supporting_wm_check,
            self.net_wm_name,
            self.net_wm_state,
            self.net_wm_state_fullscreen,
            self.net_wm_state_above,
            self.net_close_window,
            self.net_wm_pid,
        ]
    }
}

/// Initializes EWMH properties on the root window and creates the supporting WM check window.
pub fn init_ewmh(
    conn: &impl Connection,
    root: Window,
    atoms: &EwmhAtoms,
    screen_width: u16,
    screen_height: u16,
    num_desktops: u32,
    current_desktop: u32,
) -> Result<Window, X11Error> {
    // 1. Create hidden child window for _NET_SUPPORTING_WM_CHECK
    let supporting_win = conn.generate_id()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        supporting_win,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::default(),
    )?;

    // Set _NET_SUPPORTING_WM_CHECK on child pointing to itself
    conn.change_property32(
        PropMode::REPLACE,
        supporting_win,
        atoms.net_supporting_wm_check,
        AtomEnum::WINDOW,
        &[supporting_win],
    )?;

    // Set _NET_WM_NAME on child to "tilo"
    conn.change_property8(
        PropMode::REPLACE,
        supporting_win,
        atoms.net_wm_name,
        atoms.utf8_string,
        b"tilo",
    )?;

    // Set _NET_SUPPORTING_WM_CHECK on root pointing to child
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_supporting_wm_check,
        AtomEnum::WINDOW,
        &[supporting_win],
    )?;

    // 2. Set _NET_SUPPORTED on root
    let supported = atoms.supported_atoms();
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_supported,
        AtomEnum::ATOM,
        &supported,
    )?;

    // 3. Set _NET_NUMBER_OF_DESKTOPS on root
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_number_of_desktops,
        AtomEnum::CARDINAL,
        &[num_desktops],
    )?;

    // 4. Set _NET_CURRENT_DESKTOP on root (0-indexed)
    let desktop_idx = current_desktop.saturating_sub(1);
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_current_desktop,
        AtomEnum::CARDINAL,
        &[desktop_idx],
    )?;

    // 5. Set _NET_DESKTOP_NAMES on root (null-separated strings)
    let mut names = Vec::new();
    for i in 1..=num_desktops {
        names.extend_from_slice(i.to_string().as_bytes());
        names.push(0);
    }
    conn.change_property8(
        PropMode::REPLACE,
        root,
        atoms.net_desktop_names,
        atoms.utf8_string,
        &names,
    )?;

    // 6. Set _NET_DESKTOP_GEOMETRY on root ([width, height])
    let geometry = [screen_width as u32, screen_height as u32];
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_desktop_geometry,
        AtomEnum::CARDINAL,
        &geometry,
    )?;

    // 7. Set _NET_WORKAREA on root (x, y, width, height for each desktop)
    let mut workarea = Vec::with_capacity((num_desktops * 4) as usize);
    for _ in 0..num_desktops {
        workarea.push(0);
        workarea.push(0);
        workarea.push(screen_width as u32);
        workarea.push(screen_height as u32);
    }
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_workarea,
        AtomEnum::CARDINAL,
        &workarea,
    )?;

    // 8. Set initial empty _NET_CLIENT_LIST and empty _NET_ACTIVE_WINDOW
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_client_list,
        AtomEnum::WINDOW,
        &[],
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_active_window,
        AtomEnum::WINDOW,
        &[0],
    )?;

    conn.flush()?;
    Ok(supporting_win)
}

/// Updates the root window's `_NET_ACTIVE_WINDOW` property.
pub fn update_net_active_window(
    conn: &impl Connection,
    root: Window,
    window: Option<Window>,
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    let val = [window.unwrap_or(0)];
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_active_window,
        AtomEnum::WINDOW,
        &val,
    )?;
    conn.flush()?;
    Ok(())
}

/// Updates the root window's `_NET_CLIENT_LIST` property.
pub fn update_net_client_list(
    conn: &impl Connection,
    root: Window,
    clients: &[Window],
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_client_list,
        AtomEnum::WINDOW,
        clients,
    )?;
    conn.flush()?;
    Ok(())
}

/// Updates the root window's `_NET_CURRENT_DESKTOP` property (0-indexed).
pub fn update_net_current_desktop(
    conn: &impl Connection,
    root: Window,
    desktop_idx: u32,
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms.net_current_desktop,
        AtomEnum::CARDINAL,
        &[desktop_idx],
    )?;
    conn.flush()?;
    Ok(())
}

/// Reads the `_NET_WM_STATE` property from a client window.
pub fn get_net_wm_state(
    conn: &impl Connection,
    window: Window,
    atoms: &EwmhAtoms,
) -> Result<Vec<Atom>, X11Error> {
    let reply = conn.get_property(
        false,
        window,
        atoms.net_wm_state,
        AtomEnum::ATOM,
        0,
        32,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(val32) = prop.value32() {
            return Ok(val32.collect());
        }
    Ok(Vec::new())
}

/// Sets the `_NET_WM_STATE` property on a client window.
pub fn set_net_wm_state(
    conn: &impl Connection,
    window: Window,
    states: &[Atom],
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.net_wm_state,
        AtomEnum::ATOM,
        states,
    )?;
    conn.flush()?;
    Ok(())
}

/// Modifies a specific state atom in a client window's `_NET_WM_STATE`.
pub fn update_net_wm_state_atom(
    conn: &impl Connection,
    window: Window,
    state_atom: Atom,
    enable: bool,
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    let mut current = get_net_wm_state(conn, window, atoms)?;
    if enable {
        if !current.contains(&state_atom) {
            current.push(state_atom);
        }
    } else {
        current.retain(|&a| a != state_atom);
    }
    set_net_wm_state(conn, window, &current, atoms)
}

/// Updates `_NET_WM_STATE_FULLSCREEN` on a window.
pub fn update_net_wm_state_fullscreen(
    conn: &impl Connection,
    window: Window,
    is_fullscreen: bool,
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    update_net_wm_state_atom(conn, window, atoms.net_wm_state_fullscreen, is_fullscreen, atoms)
}

/// Updates `_NET_WM_STATE_ABOVE` on a window.
pub fn update_net_wm_state_above(
    conn: &impl Connection,
    window: Window,
    is_above: bool,
    atoms: &EwmhAtoms,
) -> Result<(), X11Error> {
    update_net_wm_state_atom(conn, window, atoms.net_wm_state_above, is_above, atoms)
}

/// Reads the root window's `_NET_ACTIVE_WINDOW` property.
pub fn get_net_active_window(
    conn: &impl Connection,
    root: Window,
    atoms: &EwmhAtoms,
) -> Result<Option<Window>, X11Error> {
    let reply = conn.get_property(
        false,
        root,
        atoms.net_active_window,
        AtomEnum::WINDOW,
        0,
        1,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(mut val32) = prop.value32()
            && let Some(w) = val32.next()
                && w != 0 {
                    return Ok(Some(w));
                }
    Ok(None)
}

/// Reads the root window's `_NET_CLIENT_LIST` property.
pub fn get_net_client_list(
    conn: &impl Connection,
    root: Window,
    atoms: &EwmhAtoms,
) -> Result<Vec<Window>, X11Error> {
    let reply = conn.get_property(
        false,
        root,
        atoms.net_client_list,
        AtomEnum::WINDOW,
        0,
        1024,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(val32) = prop.value32() {
            return Ok(val32.collect());
        }
    Ok(Vec::new())
}

/// Reads the root window's `_NET_CURRENT_DESKTOP` property.
pub fn get_net_current_desktop(
    conn: &impl Connection,
    root: Window,
    atoms: &EwmhAtoms,
) -> Result<Option<u32>, X11Error> {
    let reply = conn.get_property(
        false,
        root,
        atoms.net_current_desktop,
        AtomEnum::CARDINAL,
        0,
        1,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(mut val32) = prop.value32() {
            return Ok(val32.next());
        }
    Ok(None)
}

/// Reads the root window's `_NET_SUPPORTING_WM_CHECK` property.
pub fn get_supporting_wm_check(
    conn: &impl Connection,
    root: Window,
    atoms: &EwmhAtoms,
) -> Result<Option<Window>, X11Error> {
    let reply = conn.get_property(
        false,
        root,
        atoms.net_supporting_wm_check,
        AtomEnum::WINDOW,
        0,
        1,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(mut val32) = prop.value32() {
            return Ok(val32.next());
        }
    Ok(None)
}

/// Reads the application window's `_NET_WM_PID` property.
pub fn get_wm_pid(
    conn: &impl Connection,
    window: Window,
    net_wm_pid: Atom,
) -> Result<Option<u32>, X11Error> {
    let reply = conn.get_property(
        false,
        window,
        net_wm_pid,
        AtomEnum::CARDINAL,
        0,
        1,
    )?.reply();

    if let Ok(prop) = reply
        && let Some(mut val32) = prop.value32() {
            return Ok(val32.next());
        }
    Ok(None)
}
