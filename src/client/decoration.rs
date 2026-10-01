//! Minimal window decoration using native X11 window borders.
//!
//! Implements Phase 25 of Plan.md:
//! - Minimal borders around managed client windows.
//! - Direct X11 border attribute modification (`border_width` and `border_pixel`).
//! - Support for focused, normal (unfocused), and urgent border colors.
//! - Color parsing supporting hex formats (`#RRGGBB`, `#RGB`, `#AARRGGBB`, `0xRRGGBB`) and decimal values.
//! - No client-side titlebar decorations by default, preserving lightweight performance.

use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, Window};
use crate::x11::X11Error;

/// Configuration for window borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderConfig {
    /// Border width in pixels.
    pub width: u32,
    /// Pixel color for the currently focused window (default `#ffffff`).
    pub focused_color: u32,
    /// Pixel color for inactive/unfocused windows (default `#444444`).
    pub normal_color: u32,
    /// Pixel color for urgent windows (default `#ff0000`).
    pub urgent_color: u32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            width: 2,
            focused_color: 0x00ffffff,
            normal_color: 0x00444444,
            urgent_color: 0x00ff0000,
        }
    }
}

/// Updates the border color of a specific window.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `color` - The RGB color value (e.g., 0xFFFFFF).
pub fn set_window_border_color<C: ConnectionExt>(
    conn: &C,
    window: Window,
    color: u32,
) -> Result<(), X11Error> {
    let aux = ChangeWindowAttributesAux::new().border_pixel(color);
    conn.change_window_attributes(window, &aux)?;
    Ok(())
}

/// Applies the border width configuration to a window.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `width` - The border width in pixels.
pub fn set_window_border_width<C: ConnectionExt>(
    conn: &C,
    window: Window,
    width: u32,
) -> Result<(), X11Error> {
    let aux = ConfigureWindowAux::new().border_width(width);
    conn.configure_window(window, &aux)?;
    Ok(())
}

/// Updates a window's border width and appropriate color based on state.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `border_config` - The border configuration.
/// * `is_focused` - Whether the window is currently focused.
/// * `is_urgent` - Whether the window has an active urgency hint.
pub fn update_window_border<C: ConnectionExt>(
    conn: &C,
    window: Window,
    border_config: &BorderConfig,
    is_focused: bool,
    is_urgent: bool,
) -> Result<(), X11Error> {
    set_window_border_width(conn, window, border_config.width)?;
    let color = if is_urgent {
        border_config.urgent_color
    } else if is_focused {
        border_config.focused_color
    } else {
        border_config.normal_color
    };
    set_window_border_color(conn, window, color)?;
    Ok(())
}

/// Parses a color string into a 24-bit/32-bit RGB integer value.
///
/// Supports:
/// - Standard 6-digit hex: `#ffffff` or `0xffffff`
/// - Shorthand 3-digit hex: `#fff` (expanded to `#ffffff`)
/// - 8-digit hex (e.g. ARGB): `#00ffffff`
/// - Base-10 integer string: `"16777215"`
/// - Common color names: `"white"`, `"black"`, `"red"`, `"green"`, `"blue"`, `"gray"`
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            u32::from_str_radix(hex, 16).ok()
        } else if hex.len() == 3 {
            let mut expanded = String::with_capacity(6);
            for c in hex.chars() {
                expanded.push(c);
                expanded.push(c);
            }
            u32::from_str_radix(&expanded, 16).ok()
        } else if hex.len() == 8 {
            u32::from_str_radix(hex, 16).ok()
        } else {
            None
        }
    } else if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if let Ok(num) = s.parse::<u32>() {
        Some(num)
    } else {
        match s.to_ascii_lowercase().as_str() {
            "white" => Some(0x00ffffff),
            "black" => Some(0x00000000),
            "red" => Some(0x00ff0000),
            "green" => Some(0x0000ff00),
            "blue" => Some(0x000000ff),
            "gray" | "grey" => Some(0x00808080),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_border_config_default() {
        let config = BorderConfig::default();
        assert_eq!(config.width, 2);
        assert_eq!(config.focused_color, 0x00ffffff);
        assert_eq!(config.normal_color, 0x00444444);
        assert_eq!(config.urgent_color, 0x00ff0000);
    }

    #[test]
    fn test_parse_color() {
        assert_eq!(parse_color("#ffffff"), Some(0x00ffffff));
        assert_eq!(parse_color("#444444"), Some(0x00444444));
        assert_eq!(parse_color("#ff0000"), Some(0x00ff0000));
        assert_eq!(parse_color("#fff"), Some(0x00ffffff));
        assert_eq!(parse_color("#000"), Some(0x00000000));
        assert_eq!(parse_color("0x123456"), Some(0x00123456));
        assert_eq!(parse_color("0Xabcdef"), Some(0x00abcdef));
        assert_eq!(parse_color("16777215"), Some(0x00ffffff));
        assert_eq!(parse_color("white"), Some(0x00ffffff));
        assert_eq!(parse_color("black"), Some(0x00000000));
        assert_eq!(parse_color("red"), Some(0x00ff0000));
        assert_eq!(parse_color("invalid"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#12345"), None);
    }
}
