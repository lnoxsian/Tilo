//! X11 connection and context management.
//!
//! This module provides [`X11Context`], which manages the connection to the X11
//! server, caches screen geometry, root window ID, and centralizes X11 calls.

pub mod atoms;
pub mod cursor;
pub mod error;
pub mod ewmh;
pub mod icccm;

pub use atoms::Atoms;
pub use cursor::CursorManager;
pub use ewmh::*;
pub use icccm::*;

use thiserror::Error;
use x11rb::connection::Connection;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, Colormap, ColormapAlloc, ConnectionExt, EventMask,
    PropMode, Screen, VisualClass, Visualid, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// Represents errors that can occur during X11 initialization and operation.
#[derive(Debug, Error)]
pub enum X11Error {
    /// Failed to connect to the X11 server.
    #[error("Failed to connect to X server: {0}")]
    Connect(#[from] ConnectError),

    /// The X11 connection was lost or is invalid.
    #[error("X11 connection lost or invalid: {0}")]
    Connection(#[from] ConnectionError),

    /// Failed to receive a reply from the X server.
    #[error("Failed to get a reply from the X server: {0}")]
    Reply(#[from] ReplyError),

    /// Failed to execute an X11 request or allocate an ID.
    #[error("Failed to execute an X11 request: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),

    /// The requested screen number is not available on this X server.
    #[error("Screen {0} is not available")]
    InvalidScreen(usize),

    /// Another window manager is already managing this display/root window.
    #[error("Another window manager is already running")]
    AnotherWmRunning,

    /// XRandR extension is not supported by the X server.
    #[error("XRandR extension is not supported")]
    RandrNotSupported,
}

/// Manages the underlying X11 connection and root screen details.
///
/// This struct holds the connection trait object, allowing the WM to send
/// requests to the X server. It also caches the initial screen dimensions
/// and root window ID.
pub struct X11Context {
    /// The active X11 connection (typically a `RustConnection` in x11rb).
    pub connection: RustConnection,
    /// The screen number the WM is managing.
    pub screen_num: usize,
    /// The root window ID of the primary screen.
    pub root_window: Window,
    /// The initial width of the screen in pixels.
    pub screen_width: u16,
    /// The initial height of the screen in pixels.
    pub screen_height: u16,
}

impl std::fmt::Debug for X11Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X11Context")
            .field("screen_num", &self.screen_num)
            .field("root_window", &self.root_window)
            .field("screen_width", &self.screen_width)
            .field("screen_height", &self.screen_height)
            .finish()
    }
}

impl X11Context {
    /// Initializes a new connection to the X11 server using the default `$DISPLAY`.
    ///
    /// It attempts to connect to the server specified by the `DISPLAY` environment
    /// variable (or the default display if omitted). It extracts the root window and
    /// screen dimensions from the connection setup.
    ///
    /// # Returns
    /// A configured `X11Context` on success, or an `X11Error` on failure.
    pub fn new() -> Result<Self, X11Error> {
        Self::connect_to(None)
    }

    /// Connects to a specific X11 display name (e.g. `Some(":0")` or `None` for default).
    ///
    /// # Arguments
    /// * `display_name` - Optional display name string.
    ///
    /// # Returns
    /// A configured `X11Context` on success, or an `X11Error` on failure.
    pub fn connect_to(display_name: Option<&str>) -> Result<Self, X11Error> {
        let (connection, screen_num) = x11rb::connect(display_name)?;

        let screen = connection
            .setup()
            .roots
            .get(screen_num)
            .ok_or(X11Error::InvalidScreen(screen_num))?;

        let root_window = screen.root;
        let screen_width = screen.width_in_pixels;
        let screen_height = screen.height_in_pixels;

        Ok(Self {
            connection,
            screen_num,
            root_window,
            screen_width,
            screen_height,
        })
    }

    /// Gets a reference to the active screen structure.
    ///
    /// # Returns
    /// An `x11rb::protocol::xproto::Screen` instance containing detailed
    /// visual and dimension information.
    pub fn screen(&self) -> &Screen {
        &self.connection.setup().roots[self.screen_num]
    }

    /// Gets the root visual ID of the active screen.
    pub fn root_visual_id(&self) -> Visualid {
        self.screen().root_visual
    }

    /// Gets the default colormap of the active screen.
    pub fn default_colormap(&self) -> Colormap {
        self.screen().default_colormap
    }

    /// Returns the cached screen dimensions as `(width, height)`.
    pub fn dimensions(&self) -> (u16, u16) {
        (self.screen_width, self.screen_height)
    }

    /// Flushes the output buffer, ensuring all pending requests are sent to the X server.
    pub fn flush(&self) -> Result<(), X11Error> {
        self.connection.flush()?;
        Ok(())
    }

    /// Claims the Window Manager role by registering for substructure redirection
    /// and essential events on the root window.
    ///
    /// If another window manager is already running, the X server will return an
    /// `Access` error. This function intercepts that error to exit gracefully.
    ///
    /// # Events Selected:
    /// - `SubstructureRedirect`: Intercepts map and configure requests (core WM capability)
    /// - `SubstructureNotify`: Receives notifications about child window creation/destruction
    /// - `StructureNotify`: Receives notifications about root window changes
    /// - `PropertyChange`: Tracks property changes on windows (e.g. titles, hints)
    /// - `KeyPress`: Receives keyboard input for keybindings
    /// - `ButtonPress`: Receives mouse button events for window interactions
    /// - `PointerMotion`: Receives pointer movement during window dragging/resizing
    ///
    /// # Returns
    /// `Ok(())` if ownership is successfully claimed. Returns `Err(X11Error::AnotherWmRunning)`
    /// if another window manager is active, or another `X11Error` on protocol failure.
    pub fn become_wm(&self) -> Result<(), X11Error> {
        let event_mask = EventMask::SUBSTRUCTURE_REDIRECT
            | EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::PROPERTY_CHANGE
            | EventMask::KEY_PRESS
            | EventMask::BUTTON_PRESS;

        let values = ChangeWindowAttributesAux::default().event_mask(event_mask);

        let cookie = self
            .connection
            .change_window_attributes(self.root_window, &values)?;

        match cookie.check() {
            Ok(()) => Ok(()),
            Err(ReplyError::X11Error(ref err))
                if err.error_kind == ErrorKind::Access || err.error_code == 10 =>
            {
                Err(X11Error::AnotherWmRunning)
            }
            Err(err) => Err(X11Error::from(err)),
        }
    }
}

/// Finds a 32-bit `TrueColor` ARGB visual supported by `screen`, if available.
pub fn find_argb_visual(screen: &Screen) -> Option<(u8, Visualid)> {
    for depth_info in &screen.allowed_depths {
        if depth_info.depth == 32 {
            for visual in &depth_info.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
                    return Some((32, visual.visual_id));
                }
            }
        }
    }
    None
}

/// Creates an unallocated X11 colormap for a non-default visual (such as 32-bit ARGB).
pub fn create_argb_colormap(
    conn: &impl Connection,
    root: Window,
    visual: Visualid,
) -> Result<Colormap, X11Error> {
    let cmap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, cmap, root, visual)?;
    Ok(cmap)
}

/// Sets or clears `_NET_WM_WINDOW_OPACITY` (`CARDINAL`) on an X11 window for compositors.
///
/// When `opacity >= 1.0`, the property is removed so the window remains completely opaque.
pub fn set_window_opacity(
    conn: &impl Connection,
    window: Window,
    opacity: f32,
) -> Result<(), X11Error> {
    let atom = conn
        .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?
        .reply()?
        .atom;
    let clamped = opacity.clamp(0.0, 1.0);
    if clamped >= 1.0 {
        conn.delete_property(window, atom)?;
    } else {
        let cardinal = (clamped as f64 * u32::MAX as f64).round() as u32;
        conn.change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &[cardinal])?;
    }
    Ok(())
}

/// Resolved X11 visual, colormap, and alpha settings for a compositor-transparent UI window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTransparencySurface {
    /// Window and off-screen pixmap depth (24 or 32).
    pub depth: u8,
    /// Visual ID used when creating the window.
    pub visual_id: Visualid,
    /// Optional colormap created for a 32-bit ARGB visual (must be freed on destroy).
    pub colormap: Option<Colormap>,
    /// Whether `_NET_WM_WINDOW_OPACITY` should be applied (`Window` mode or fallback).
    pub use_window_opacity: bool,
    /// Background alpha (`0..=255`) for 32-bit ARGB software rendering.
    pub bg_alpha: u8,
}

impl WindowTransparencySurface {
    /// Computes `(depth, use_window_opacity, bg_alpha)` without allocating any X11 colormap.
    pub fn plan(
        screen: &Screen,
        opacity: f32,
        mode: crate::config::TransparencyMode,
        is_core_font: bool,
    ) -> (u8, bool, u8) {
        let clamped = opacity.clamp(0.0, 1.0);
        if mode == crate::config::TransparencyMode::None || clamped >= 1.0 {
            return (screen.root_depth, false, 255);
        }

        if mode == crate::config::TransparencyMode::Background
            && !is_core_font
            && let Some((depth, _)) = find_argb_visual(screen)
        {
            let bg_alpha = (clamped * 255.0).round() as u8;
            return (depth, false, bg_alpha);
        }

        (screen.root_depth, true, 255)
    }

    /// Resolves the visual and opacity strategy for a UI overlay window.
    pub fn resolve(
        conn: &impl Connection,
        screen: &Screen,
        opacity: f32,
        mode: crate::config::TransparencyMode,
        is_core_font: bool,
    ) -> Result<Self, X11Error> {
        let clamped = opacity.clamp(0.0, 1.0);
        if mode == crate::config::TransparencyMode::None || clamped >= 1.0 {
            return Ok(Self {
                depth: screen.root_depth,
                visual_id: screen.root_visual,
                colormap: None,
                use_window_opacity: false,
                bg_alpha: 255,
            });
        }

        if mode == crate::config::TransparencyMode::Background
            && !is_core_font
            && let Some((depth, visual_id)) = find_argb_visual(screen)
        {
            let cmap = create_argb_colormap(conn, screen.root, visual_id)?;
            let bg_alpha = (clamped * 255.0).round() as u8;
            return Ok(Self {
                depth,
                visual_id,
                colormap: Some(cmap),
                use_window_opacity: false,
                bg_alpha,
            });
        }

        Ok(Self {
            depth: screen.root_depth,
            visual_id: screen.root_visual,
            colormap: None,
            use_window_opacity: true,
            bg_alpha: 255,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_invalid_screen_display() {
        let err = X11Error::InvalidScreen(3);
        assert_eq!(err.to_string(), "Screen 3 is not available");
    }

    #[test]
    fn test_connect_to_invalid_display() {
        let result = X11Context::connect_to(Some(":98"));
        assert!(result.is_err());
        match result.unwrap_err() {
            X11Error::Connect(_) => {}
            other => panic!("Expected X11Error::Connect, got: {other:?}"),
        }
    }

    #[test]
    fn test_x11_context_when_display_available() {
        // If DISPLAY is set, test real connection
        if std::env::var("DISPLAY").is_ok() {
            let ctx = X11Context::new().expect("Failed to connect to active DISPLAY");
            assert_ne!(ctx.root_window, 0, "Root window must not be 0");
            assert!(ctx.screen_width > 0, "Screen width must be positive");
            assert!(ctx.screen_height > 0, "Screen height must be positive");

            let screen = ctx.screen();
            assert_eq!(screen.root, ctx.root_window);
            assert_eq!(screen.width_in_pixels, ctx.screen_width);
            assert_eq!(screen.height_in_pixels, ctx.screen_height);
            assert_eq!(ctx.dimensions(), (ctx.screen_width, ctx.screen_height));
            assert_eq!(ctx.root_visual_id(), screen.root_visual);
            assert_eq!(ctx.default_colormap(), screen.default_colormap);

            assert!(ctx.flush().is_ok(), "Flush must succeed");
        }
    }

    #[test]
    fn test_multiple_independent_connections() {
        if std::env::var("DISPLAY").is_ok() {
            let ctx1 = X11Context::new().expect("Connection 1 failed");
            let ctx2 = X11Context::new().expect("Connection 2 failed");

            assert_eq!(ctx1.root_window, ctx2.root_window);
            assert_eq!(ctx1.screen_width, ctx2.screen_width);
            assert_eq!(ctx1.screen_height, ctx2.screen_height);
            assert!(ctx1.flush().is_ok());
            assert!(ctx2.flush().is_ok());
        }
    }

    #[test]
    fn test_error_another_wm_running_display() {
        let err = X11Error::AnotherWmRunning;
        assert_eq!(err.to_string(), "Another window manager is already running");
    }

    #[test]
    fn test_become_wm_on_active_display() {
        if std::env::var("DISPLAY").is_ok() {
            let ctx = X11Context::new().expect("Failed to connect to active DISPLAY");
            // If another WM is running (like on normal desktop), become_wm must detect it
            // and return AnotherWmRunning cleanly without crashing or panicking.
            match ctx.become_wm() {
                Ok(()) | Err(X11Error::AnotherWmRunning) => {}
                Err(other) => panic!("Unexpected error from become_wm: {other:?}"),
            }
        }
    }
}
