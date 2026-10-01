//! X11 Atoms cache for ICCCM and EWMH protocols.

use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, ConnectionExt};

/// Cached X11 atoms used for window management protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Atoms {
    /// `WM_PROTOCOLS` atom.
    pub wm_protocols: Atom,
    /// `WM_DELETE_WINDOW` atom for graceful application closure.
    pub wm_delete_window: Atom,
    /// `WM_TAKE_FOCUS` atom for focus messaging.
    pub wm_take_focus: Atom,
    /// `WM_STATE` atom for window state flags.
    pub wm_state: Atom,
    /// `_NET_WM_NAME` atom for window titles.
    pub net_wm_name: Atom,
    /// `UTF8_STRING` atom for text encoding.
    pub utf8_string: Atom,
}

impl Atoms {
    /// Interns all standard ICCCM and EWMH atoms used by the window manager.
    pub fn intern(conn: &impl Connection) -> Result<Self, X11Error> {
        let wm_protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?;
        let wm_delete_window = conn.intern_atom(false, b"WM_DELETE_WINDOW")?;
        let wm_take_focus = conn.intern_atom(false, b"WM_TAKE_FOCUS")?;
        let wm_state = conn.intern_atom(false, b"WM_STATE")?;
        let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?;
        let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?;

        Ok(Self {
            wm_protocols: wm_protocols.reply()?.atom,
            wm_delete_window: wm_delete_window.reply()?.atom,
            wm_take_focus: wm_take_focus.reply()?.atom,
            wm_state: wm_state.reply()?.atom,
            net_wm_name: net_wm_name.reply()?.atom,
            utf8_string: utf8_string.reply()?.atom,
        })
    }
}
