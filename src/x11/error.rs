//! Application state recovery and error handling.
//!
//! Implements Phase 29 of Plan.md:
//! - [`WmError`] core error types representing fatal connection loss, recoverable protocol errors, and configuration failures.
//! - State recovery mechanisms when clients disconnect unexpectedly, windows are unmapped,
//!   or workspace/monitor changes occur during client destruction.

use crate::config::ConfigError;
use crate::x11::X11Error;
use thiserror::Error;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::ErrorKind;

/// Core application error types.
#[derive(Debug, Error)]
pub enum WmError {
    /// X server connection lost or unavailable.
    #[error("X server connection lost: {0}")]
    ConnectionLost(String),

    /// Recoverable X11 protocol error (e.g. BadWindow).
    #[error("X11 protocol error: {0}")]
    X11ProtocolError(#[from] X11Error),

    /// Configuration parse error.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<ConfigError> for WmError {
    fn from(err: ConfigError) -> Self {
        WmError::ConfigError(err.to_string())
    }
}

impl From<ConnectionError> for WmError {
    fn from(err: ConnectionError) -> Self {
        WmError::ConnectionLost(err.to_string())
    }
}

impl From<ConnectError> for WmError {
    fn from(err: ConnectError) -> Self {
        WmError::ConnectionLost(err.to_string())
    }
}

impl From<crate::statusbar::StatusbarError> for WmError {
    fn from(err: crate::statusbar::StatusbarError) -> Self {
        match err {
            crate::statusbar::StatusbarError::X11(x) => WmError::X11ProtocolError(x),
            other => WmError::ConfigError(other.to_string()),
        }
    }
}

impl From<crate::config::ReloadError> for WmError {
    fn from(err: crate::config::ReloadError) -> Self {
        match err {
            crate::config::ReloadError::ConfigInvalid(c) => WmError::ConfigError(c.to_string()),
            crate::config::ReloadError::X11Error(x) => WmError::X11ProtocolError(x),
        }
    }
}

impl WmError {
    /// Returns `true` if this error indicates that the connection to the X server was lost.
    pub fn is_fatal(&self) -> bool {
        match self {
            WmError::ConnectionLost(_) => true,
            WmError::X11ProtocolError(err) => is_connection_lost_error(err),
            WmError::ConfigError(_) => false,
        }
    }

    /// Returns `true` if this error is recoverable without terminating the window manager.
    pub fn is_recoverable(&self) -> bool {
        !self.is_fatal()
    }
}

/// Checks whether an [`X11Error`] indicates that the X11 connection was terminated or is invalid.
pub fn is_connection_lost_error(err: &X11Error) -> bool {
    matches!(err, X11Error::Connect(_) | X11Error::Connection(_))
}

/// Checks whether an [`X11Error`] is caused by a client window disconnecting or disappearing
/// (specifically `BadWindow`).
pub fn is_client_disconnect_error(err: &X11Error) -> bool {
    match err {
        X11Error::Reply(ReplyError::X11Error(xerr)) => {
            xerr.error_kind == ErrorKind::Window || xerr.error_code == 3
        }
        X11Error::ReplyOrId(ReplyOrIdError::X11Error(xerr)) => {
            xerr.error_kind == ErrorKind::Window || xerr.error_code == 3
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wm_error_display() {
        let err1 = WmError::ConnectionLost("Connection reset by peer".to_string());
        assert_eq!(err1.to_string(), "X server connection lost: Connection reset by peer");
        assert!(err1.is_fatal());
        assert!(!err1.is_recoverable());

        let err2 = WmError::ConfigError("Syntax error at line 5".to_string());
        assert_eq!(err2.to_string(), "Configuration error: Syntax error at line 5");
        assert!(!err2.is_fatal());
        assert!(err2.is_recoverable());

        let x11_err = X11Error::AnotherWmRunning;
        let err3 = WmError::X11ProtocolError(x11_err);
        assert_eq!(err3.to_string(), "X11 protocol error: Another window manager is already running");
        assert!(!err3.is_fatal());
        assert!(err3.is_recoverable());
    }

    #[test]
    fn test_error_conversions() {
        let config_err = ConfigError::IoError("File not found".to_string());
        let wm_err: WmError = config_err.into();
        assert!(matches!(wm_err, WmError::ConfigError(_)));

        let reload_err = crate::config::ReloadError::ConfigInvalid(ConfigError::InvalidValue {
            line: 2,
            value: "bad".to_string(),
        });
        let wm_err2: WmError = reload_err.into();
        assert!(matches!(wm_err2, WmError::ConfigError(_)));
    }

    #[test]
    fn test_is_connection_lost_error() {
        assert!(!is_connection_lost_error(&X11Error::AnotherWmRunning));
        assert!(!is_connection_lost_error(&X11Error::RandrNotSupported));
        assert!(!is_connection_lost_error(&X11Error::InvalidScreen(1)));
    }

    #[test]
    fn test_is_client_disconnect_error() {
        assert!(!is_client_disconnect_error(&X11Error::AnotherWmRunning));
    }
}
