//! tilo - Minimal Rust X11 Tiling Window Manager

pub mod atoms;
pub mod autostart;
pub mod bar;
pub mod client;
pub mod config;
pub mod cursor;
pub mod decoration;
pub mod ewmh;
pub mod floating;
pub mod focus;
pub mod fullscreen;
pub mod icccm;
pub mod input;
pub mod keybind;
pub mod layout;
pub mod launcher;
pub mod monitor;
pub mod mouse;
pub mod render;
pub mod rules;
pub mod scratchpad;
pub mod error;
pub mod statusbar;
pub mod wm;
pub mod workspace;
pub mod x11;

pub use error::WmError;



