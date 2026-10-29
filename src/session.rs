//! Desktop session and D-Bus integration for Tilo.
//!
//! Provides i3-parity session initialization:
//! 1. Ensures standard FreeDesktop environment variables (`XDG_CURRENT_DESKTOP`, `XDG_SESSION_TYPE`).
//! 2. Auto-detects and heals missing D-Bus user session addresses (e.g. when launched via `startx`).
//! 3. Updates the D-Bus activation environment (`dbus-update-activation-environment`) so desktop
//!    services, Polkit agents, portals, and package hooks (like `apt-listchanges` GTK/dialog frontends)
//!    can communicate with the current display and session.

use std::process::Command;

/// Initializes the desktop environment and synchronizes session state with D-Bus.
///
/// This should be called once during Tilo startup before establishing the X11 connection.
pub fn init_desktop_session() {
    // 1. Set XDG_CURRENT_DESKTOP to identify Tilo to portals, GUI toolkits, and D-Bus services
    unsafe {
        std::env::set_var("XDG_CURRENT_DESKTOP", "tilo");
        std::env::set_var("XDG_SESSION_TYPE", "x11");
    }

    // 3. Auto-detect user session bus if DBUS_SESSION_BUS_ADDRESS is missing
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        let uid = unsafe { libc::getuid() };
        let user_bus_path = format!("/run/user/{}/bus", uid);
        if std::path::Path::new(&user_bus_path).exists() {
            unsafe {
                std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", user_bus_path));
            }
        }
    }

    // 4. Update D-Bus activation environment (matches i3 desktop startup behavior)
    // Synchronize DISPLAY, XAUTHORITY, XDG_CURRENT_DESKTOP, and XDG_SESSION_TYPE
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        let systemd_update = Command::new("dbus-update-activation-environment")
            .arg("--systemd")
            .arg("DISPLAY")
            .arg("XAUTHORITY")
            .arg("XDG_CURRENT_DESKTOP")
            .arg("XDG_SESSION_TYPE")
            .status();

        if systemd_update.is_err() || !systemd_update.map(|s| s.success()).unwrap_or(false) {
            // Fallback for non-systemd environments or older dbus versions
            let _ = Command::new("dbus-update-activation-environment")
                .arg("DISPLAY")
                .arg("XAUTHORITY")
                .arg("XDG_CURRENT_DESKTOP")
                .arg("XDG_SESSION_TYPE")
                .status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_desktop_session_sets_xdg_variables() {
        init_desktop_session();
        assert_eq!(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(), "tilo");
        assert_eq!(std::env::var("XDG_SESSION_TYPE").unwrap_or_default(), "x11");
    }
}
