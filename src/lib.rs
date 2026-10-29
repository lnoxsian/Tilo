//! tilo - Minimal Rust X11 Tiling Window Manager

pub mod audio;
pub mod autostart;
pub mod bar;
pub mod client;
pub mod config;
pub mod input;
pub mod layout;
pub mod launcher;
pub mod monitor;
pub mod process;
pub mod render;
pub mod session;
pub mod wm;
pub mod workspace;
pub mod x11;

pub use audio::{get_volume, adjust_volume, toggle_mute, VolumeState, AudioBackend};
pub use bar as statusbar;
pub use workspace::focus;
pub use input::keybind;
pub use input::mouse;

pub use client::decoration;
pub use client::floating;
pub use client::fullscreen;
pub use client::rules;
pub use client::scratchpad;

pub use x11::atoms;
pub use x11::cursor;
pub use x11::error;
pub use x11::ewmh;
pub use x11::icccm;

pub use error::WmError;
pub use wm::{manage_existing_windows, perform_desktop_shutdown, perform_graceful_shutdown};

/// WM state type alias conforming to Plan.md specification.
pub type WmState = wm::WindowManager;
