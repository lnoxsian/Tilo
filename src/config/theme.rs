//! Global and component-specific theme configuration.
//!
//! Provides unified color and font control across:
//! - Window decorations (focused, normal, and urgent window borders)
//! - Status bar (background, foreground, active/urgent workspace & tabs, fonts)
//! - Application launcher (overlay background, input/result foreground, selected item, border, fonts)

use crate::bar::ColorScheme;

/// Comprehensive theme configuration allowing global color palette definitions
/// and granular per-component overrides for window decorations, statusbar, and launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeConfig {
    // --- Global Palette ---
    /// Global default background color (0xRRGGBB).
    pub bg: u32,
    /// Global default text foreground color (0xRRGGBB).
    pub fg: u32,
    /// Global accent/active background color.
    pub active_bg: u32,
    /// Global accent/active text foreground color.
    pub active_fg: u32,
    /// Global urgent background color.
    pub urgent_bg: u32,
    /// Global urgent text foreground color.
    pub urgent_fg: u32,
    /// Global inactive / dimmed text color.
    pub inactive_fg: u32,
    /// Window border color when focused.
    pub border_focused: u32,
    /// Window border color when inactive/normal.
    pub border_normal: u32,
    /// Window border color when urgent.
    pub border_urgent: u32,

    // --- Statusbar Overrides (None = inherit from global palette) ---
    /// Specific background color for the statusbar.
    pub bar_bg: Option<u32>,
    /// Specific text foreground color for the statusbar.
    pub bar_fg: Option<u32>,
    /// Specific active workspace / tab pill background for the statusbar.
    pub bar_active_bg: Option<u32>,
    /// Specific active workspace / tab text foreground for the statusbar.
    pub bar_active_fg: Option<u32>,
    /// Specific urgent workspace / tab pill background for the statusbar.
    pub bar_urgent_bg: Option<u32>,
    /// Specific urgent workspace / tab text foreground for the statusbar.
    pub bar_urgent_fg: Option<u32>,
    /// Specific inactive/dimmed text color for the statusbar.
    pub bar_inactive_fg: Option<u32>,
    /// Specific border/separator color for the statusbar.
    pub bar_border: Option<u32>,
    /// Specific font specification for the statusbar.
    pub bar_font: Option<String>,

    // --- Launcher Overrides (None = inherit from global palette) ---
    /// Specific background color for the application launcher overlay.
    pub launcher_bg: Option<u32>,
    /// Specific text foreground color for the launcher input and inactive rows.
    pub launcher_fg: Option<u32>,
    /// Specific background highlight for the selected launcher item.
    pub launcher_active_bg: Option<u32>,
    /// Specific text foreground color for the selected launcher item.
    pub launcher_active_fg: Option<u32>,
    /// Specific urgent background color for launcher items.
    pub launcher_urgent_bg: Option<u32>,
    /// Specific urgent text foreground color for launcher items.
    pub launcher_urgent_fg: Option<u32>,
    /// Specific dimmed / placeholder text color for the launcher.
    pub launcher_inactive_fg: Option<u32>,
    /// Specific window border color for the launcher.
    pub launcher_border: Option<u32>,
    /// Specific font specification for the launcher.
    pub launcher_font: Option<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg: 0x1a1b26,
            fg: 0xc0caf5,
            active_bg: 0x7aa2f7,
            active_fg: 0x15161e,
            urgent_bg: 0xf7768e,
            urgent_fg: 0x15161e,
            inactive_fg: 0x555566,
            border_focused: 0x00ffffff,
            border_normal: 0x00444444,
            border_urgent: 0x00ff0000,

            bar_bg: None,
            bar_fg: None,
            bar_active_bg: None,
            bar_active_fg: None,
            bar_urgent_bg: None,
            bar_urgent_fg: None,
            bar_inactive_fg: None,
            bar_border: None,
            bar_font: None,

            launcher_bg: None,
            launcher_fg: None,
            launcher_active_bg: None,
            launcher_active_fg: None,
            launcher_urgent_bg: None,
            launcher_urgent_fg: None,
            launcher_inactive_fg: None,
            launcher_border: None,
            launcher_font: None,
        }
    }
}

impl ThemeConfig {
    /// Resolves the effective color scheme for the statusbar.
    pub fn bar_color_scheme(&self) -> ColorScheme {
        ColorScheme {
            bg: self.bar_bg.unwrap_or(self.bg),
            fg: self.bar_fg.unwrap_or(self.fg),
            active_bg: self.bar_active_bg.unwrap_or(self.active_bg),
            active_fg: self.bar_active_fg.unwrap_or(self.active_fg),
            urgent_bg: self.bar_urgent_bg.unwrap_or(self.urgent_bg),
            urgent_fg: self.bar_urgent_fg.unwrap_or(self.urgent_fg),
            inactive_fg: self.bar_inactive_fg.unwrap_or(self.inactive_fg),
            border: self.bar_border.unwrap_or(self.active_bg),
        }
    }

    /// Resolves the effective color scheme for the application launcher.
    pub fn launcher_color_scheme(&self) -> ColorScheme {
        ColorScheme {
            bg: self.launcher_bg.unwrap_or(self.bg),
            fg: self.launcher_fg.unwrap_or(self.fg),
            active_bg: self.launcher_active_bg.unwrap_or(self.active_bg),
            active_fg: self.launcher_active_fg.unwrap_or(self.active_fg),
            urgent_bg: self.launcher_urgent_bg.unwrap_or(self.urgent_bg),
            urgent_fg: self.launcher_urgent_fg.unwrap_or(self.urgent_fg),
            inactive_fg: self.launcher_inactive_fg.unwrap_or(self.inactive_fg),
            border: self.launcher_border.unwrap_or(self.active_bg),
        }
    }
}
