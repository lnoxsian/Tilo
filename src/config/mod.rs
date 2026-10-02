//! Configuration system for tilo.
//!
//! Implements Phase 15 & 16 of Plan.md:
//! - Custom line-oriented, whitespace-delimited configuration syntax.
//! - Directives for modifiers, fonts, border widths, inner/outer gaps, master ratios, and keybindings.
//! - Lexical tokenizer ([`tokenize`]), recursive-descent parser ([`parse`]), and disk loader ([`load_config`]).
//! - Safe configuration reload without restarting the window manager ([`ReloadError`]).

pub mod theme;
pub use theme::*;

use crate::autostart::AutostartCmd;
use crate::focus::Direction;
use crate::keybind::{default_keybindings, parse_keybind, Action, KeyBind};
use crate::x11::X11Error;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Tokens produced by the lexical analyzer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A string literal or identifier.
    Word(String),
    /// A numeric integer value.
    Number(u32),
    /// A floating-point numeric value.
    Float(f32),
    /// The end of a line.
    Newline,
    /// End of the configuration file.
    Eof,
}

/// Errors that can occur during configuration loading and parsing.
#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    /// File not found or could not be read.
    #[error("IO error: {0}")]
    IoError(String),
    /// Tokenization or parsing failed at the given line number, with a descriptive message.
    #[error("Parse error at line {line}: {message}")]
    ParseError { line: usize, message: String },
    /// An unknown action or modifier was specified.
    #[error("Invalid value at line {line}: {value}")]
    InvalidValue { line: usize, value: String },
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::IoError(err.to_string())
    }
}

/// Errors that can occur during a hot configuration reload.
#[derive(Debug, Error)]
pub enum ReloadError {
    /// The configuration file was invalid. The previous configuration was kept.
    #[error("Invalid configuration: {0}")]
    ConfigInvalid(#[from] ConfigError),

    /// Failed to update X11 state (e.g., grabbing keys).
    #[error("X11 error during reload: {0}")]
    X11Error(#[from] X11Error),
}

/// Represents a parsed user configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The modifier key used for WM actions (e.g., "Mod4" for Super).
    pub mod_key: String,
    /// Font string for Xft or pango.
    pub font: String,
    /// Width of window borders in pixels.
    pub border_width: u32,
    /// Pixel color for focused window borders (default `#ffffff`).
    pub focused_color: u32,
    /// Pixel color for inactive/unfocused window borders (default `#444444`).
    pub normal_color: u32,
    /// Pixel color for urgent window borders (default `#ff0000`).
    pub urgent_color: u32,
    /// Gap between windows in pixels.
    pub inner_gap: u32,
    /// Gap between windows and screen edges in pixels.
    pub outer_gap: u32,
    /// The proportion of the screen given to the master area (0.0 to 1.0).
    pub master_ratio: f32,
    /// The default terminal emulator command.
    pub terminal: String,
    /// Default window tiling layout (Phase 27).
    pub layout: crate::layout::LayoutType,
    /// A list of defined keybindings.
    pub bindings: Vec<KeyBind>,
    /// Optional startup/autostart commands.
    pub autostart: Vec<AutostartCmd>,
    /// Configuration for tabbed layout in status bar.
    pub tab_config: crate::bar::tabs::TabbedConfig,
    /// Unified global and per-component theme configuration.
    pub theme: ThemeConfig,
    /// Enable mouse click interaction for switching workspaces on the status bar (default: true).
    pub workspace_mouse: bool,
    /// Enable mouse wheel scrolling to cycle workspaces on the status bar (default: true).
    pub workspace_wheel_switch: bool,
}

impl Config {
    /// Returns the resolved color scheme for the status bar.
    pub fn bar_color_scheme(&self) -> crate::bar::ColorScheme {
        self.theme.bar_color_scheme()
    }

    /// Returns the resolved color scheme for the application launcher.
    pub fn launcher_color_scheme(&self) -> crate::bar::ColorScheme {
        self.theme.launcher_color_scheme()
    }

    /// Returns the effective font for the status bar.
    pub fn bar_font(&self) -> &str {
        self.theme.bar_font.as_deref().unwrap_or(&self.font)
    }

    /// Returns the effective font for the application launcher.
    pub fn launcher_font(&self) -> &str {
        self.theme.launcher_font.as_deref().unwrap_or(&self.font)
    }

    /// Returns the effective window border color for the application launcher.
    pub fn launcher_border_color(&self) -> u32 {
        self.theme.launcher_border.unwrap_or(self.theme.active_bg)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mod_key: "Mod4".to_string(),
            font: "monospace".to_string(),
            border_width: 2,
            focused_color: 0x00ffffff,
            normal_color: 0x00444444,
            urgent_color: 0x00ff0000,
            inner_gap: 8,
            outer_gap: 8,
            master_ratio: 0.60,
            terminal: "foot".to_string(),
            layout: crate::layout::LayoutType::Bsp,
            bindings: default_keybindings(),
            autostart: Vec::new(),
            tab_config: crate::bar::tabs::TabbedConfig::default(),
            theme: ThemeConfig::default(),
            workspace_mouse: true,
            workspace_wheel_switch: true,
        }
    }
}

/// Tokenizes the raw string content of a configuration file.
///
/// Follows the Phase 15 tokenizer specification:
/// 1. Reads input character by character.
/// 2. Skips horizontal whitespace (spaces, tabs, carriage returns).
/// 3. Ignores comments starting with `#` until `\n`.
/// 4. Emits `Token::Newline` on `\n`.
/// 5. Accumulates words, numbers, and floats.
/// 6. Ends with `Token::Eof`.
pub fn tokenize(content: &str) -> Result<Vec<Token>, ConfigError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    let len = chars.len();
    let mut line_has_tokens = false;

    while i < len {
        let c = chars[i];

        // 1. Comments: ignore until newline (unless it is a hex color token like #ffffff)
        if c == '#' {
            if line_has_tokens {
                let mut hex_len = 0;
                while i + 1 + hex_len < len && chars[i + 1 + hex_len].is_ascii_hexdigit() {
                    hex_len += 1;
                }
                let next_char = if i + 1 + hex_len < len {
                    Some(chars[i + 1 + hex_len])
                } else {
                    None
                };
                let is_end_of_token = next_char.is_none_or(|ch| {
                    ch.is_whitespace() || ch == '\n' || ch == '\r' || ch == '#'
                });

                if (hex_len == 3 || hex_len == 6 || hex_len == 8) && is_end_of_token {
                    let mut color_token = String::with_capacity(1 + hex_len);
                    color_token.push('#');
                    for _ in 0..hex_len {
                        i += 1;
                        color_token.push(chars[i]);
                    }
                    i += 1;
                    tokens.push(Token::Word(color_token));
                    continue;
                }
            }

            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // 2. Newlines
        if c == '\n' {
            tokens.push(Token::Newline);
            line_has_tokens = false;
            i += 1;
            continue;
        }

        // 3. Skip whitespace
        if c.is_whitespace() || c == '\r' {
            i += 1;
            continue;
        }

        // 4. Quoted strings (e.g. font "JetBrainsMono 10")
        if c == '"' {
            i += 1;
            let mut word = String::new();
            while i < len && chars[i] != '"' && chars[i] != '\n' {
                word.push(chars[i]);
                i += 1;
            }
            if i < len && chars[i] == '"' {
                i += 1;
            }
            tokens.push(Token::Word(word));
            line_has_tokens = true;
            continue;
        }

        // 5. Accumulate word or numeric literal
        let mut buffer = String::new();
        while i < len && !chars[i].is_whitespace() && chars[i] != '#' && chars[i] != '\n' && chars[i] != '\r' {
            buffer.push(chars[i]);
            i += 1;
        }

        if buffer.is_empty() {
            continue;
        }

        line_has_tokens = true;
        // Attempt integer parse
        if let Ok(num) = buffer.parse::<u32>() {
            tokens.push(Token::Number(num));
        } else if buffer.contains('.') {
            // Attempt float parse if it contains a decimal point
            if let Ok(flt) = buffer.parse::<f32>() {
                tokens.push(Token::Float(flt));
            } else {
                tokens.push(Token::Word(buffer));
            }
        } else {
            tokens.push(Token::Word(buffer));
        }
    }

    tokens.push(Token::Eof);
    Ok(tokens)
}

/// Parses a slice of tokens into a structured [`Config`].
pub fn parse(tokens: &[Token]) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    let mut custom_bindings = Vec::new();
    let mut has_custom_bindings = false;

    // Split tokens by Token::Newline
    let mut line_number = 1;
    let mut current_line: Vec<&Token> = Vec::new();

    for token in tokens {
        match token {
            Token::Newline => {
                if !current_line.is_empty() {
                    parse_line(&current_line, line_number, &mut config, &mut custom_bindings, &mut has_custom_bindings)?;
                    current_line.clear();
                }
                line_number += 1;
            }
            Token::Eof => {
                if !current_line.is_empty() {
                    parse_line(&current_line, line_number, &mut config, &mut custom_bindings, &mut has_custom_bindings)?;
                    current_line.clear();
                }
                break;
            }
            _ => {
                current_line.push(token);
            }
        }
    }

    if has_custom_bindings {
        config.bindings = custom_bindings;
    }

    Ok(config)
}

fn parse_line(
    line: &[&Token],
    line_number: usize,
    config: &mut Config,
    custom_bindings: &mut Vec<KeyBind>,
    has_custom_bindings: &mut bool,
) -> Result<(), ConfigError> {
    if line.is_empty() {
        return Ok(());
    }

    let directive = match line[0] {
        Token::Word(s) => s.as_str(),
        _ => {
            return Err(ConfigError::ParseError {
                line: line_number,
                message: "Expected directive name at start of line".to_string(),
            });
        }
    };

    match directive {
        "mod" | "mod_key" | "modifier" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a key argument (e.g. '{} Mod4')", directive, directive),
                });
            }
            let key = token_to_string(line[1]);
            config.mod_key = key;
        }
        "font" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'font' requires a font specification".to_string(),
                });
            }
            let words: Vec<String> = line[1..].iter().map(|t| token_to_string(t)).collect();
            config.font = words.join(" ");
        }
        "border_width" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'border_width' requires a numeric value".to_string(),
                });
            }
            let width = token_to_u32(line[1], line_number, "border_width")?;
            config.border_width = width;
        }
        // --- Global Theme Palette ---
        "bg" | "background" | "color_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #1a1b26)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bg = color;
        }
        "fg" | "foreground" | "color_fg" | "font_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #c0caf5)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.fg = color;
        }
        "active_bg" | "accent_color" | "color_active_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #7aa2f7)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.active_bg = color;
        }
        "active_fg" | "color_active_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #15161e)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.active_fg = color;
        }
        "urgent_bg" | "color_urgent_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #f7768e)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.urgent_bg = color;
        }
        "urgent_fg" | "color_urgent_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #15161e)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.urgent_fg = color;
        }
        "inactive_fg" | "color_inactive_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #555566)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.inactive_fg = color;
        }
        "border_color" | "color_border" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #444444)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.normal_color = color;
            config.theme.border_normal = color;
        }

        // --- Window Decoration Borders ---
        "focused_color" | "border_focused" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #ffffff)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.focused_color = color;
            config.theme.border_focused = color;
        }
        "normal_color" | "border_normal" | "unfocused_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #444444)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.normal_color = color;
            config.theme.border_normal = color;
        }
        "urgent_color" | "border_urgent" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument (e.g. #ff0000)", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.urgent_color = color;
            config.theme.border_urgent = color;
        }

        // --- Statusbar Specific Overrides ---
        "bar_bg" | "statusbar_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_bg = Some(color);
        }
        "bar_fg" | "statusbar_fg" | "bar_font_color" | "statusbar_font_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_fg = Some(color);
        }
        "bar_active_bg" | "statusbar_active_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_active_bg = Some(color);
        }
        "bar_active_fg" | "statusbar_active_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_active_fg = Some(color);
        }
        "bar_urgent_bg" | "statusbar_urgent_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_urgent_bg = Some(color);
        }
        "bar_urgent_fg" | "statusbar_urgent_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_urgent_fg = Some(color);
        }
        "bar_inactive_fg" | "statusbar_inactive_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_inactive_fg = Some(color);
        }
        "bar_border" | "statusbar_border" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.bar_border = Some(color);
        }
        "bar_font" | "statusbar_font" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a font specification", directive),
                });
            }
            let words: Vec<String> = line[1..].iter().map(|t| token_to_string(t)).collect();
            config.theme.bar_font = Some(words.join(" "));
        }

        // --- Launcher Specific Overrides ---
        "launcher_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_bg = Some(color);
        }
        "launcher_fg" | "launcher_font_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_fg = Some(color);
        }
        "launcher_selected_bg" | "launcher_active_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_active_bg = Some(color);
        }
        "launcher_selected_fg" | "launcher_active_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_active_fg = Some(color);
        }
        "launcher_urgent_bg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_urgent_bg = Some(color);
        }
        "launcher_urgent_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_urgent_fg = Some(color);
        }
        "launcher_inactive_fg" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_inactive_fg = Some(color);
        }
        "launcher_border" | "launcher_border_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a color argument", directive),
                });
            }
            let color = token_to_color(line[1], line_number, directive)?;
            config.theme.launcher_border = Some(color);
        }
        "launcher_font" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: format!("Directive '{}' requires a font specification", directive),
                });
            }
            let words: Vec<String> = line[1..].iter().map(|t| token_to_string(t)).collect();
            config.theme.launcher_font = Some(words.join(" "));
        }
        "inner_gap" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'inner_gap' requires a numeric value".to_string(),
                });
            }
            let gap = token_to_u32(line[1], line_number, "inner_gap")?;
            config.inner_gap = gap;
        }
        "outer_gap" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'outer_gap' requires a numeric value".to_string(),
                });
            }
            let gap = token_to_u32(line[1], line_number, "outer_gap")?;
            config.outer_gap = gap;
        }
        "master_ratio" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'master_ratio' requires a floating-point value".to_string(),
                });
            }
            let ratio = match line[1] {
                Token::Float(f) => *f,
                Token::Number(n) => *n as f32,
                Token::Word(s) => s.parse::<f32>().map_err(|_| ConfigError::InvalidValue {
                    line: line_number,
                    value: s.clone(),
                })?,
                _ => {
                    return Err(ConfigError::ParseError {
                        line: line_number,
                        message: "Invalid master_ratio value".to_string(),
                    });
                }
            };
            if !(0.0..=1.0).contains(&ratio) {
                return Err(ConfigError::InvalidValue {
                    line: line_number,
                    value: ratio.to_string(),
                });
            }
            config.master_ratio = ratio;
        }
        "terminal" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'terminal' requires a command argument".to_string(),
                });
            }
            let words: Vec<String> = line[1..].iter().map(|t| token_to_string(t)).collect();
            config.terminal = words.join(" ");
        }
        "layout" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'layout' requires a layout type (e.g. 'layout master', 'layout monocle', 'layout vertical')".to_string(),
                });
            }
            let name = token_to_string(line[1]);
            let lt = name.parse::<crate::layout::LayoutType>().map_err(|_| ConfigError::InvalidValue {
                line: line_number,
                value: name,
            })?;
            config.layout = lt;
        }
        "bind" => {
            if line.len() < 3 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'bind' requires '<KeyCombo> <Action> [Args...]'".to_string(),
                });
            }
            let raw_combo = token_to_string(line[1]);
            // Replace $mod with current config.mod_key if present
            let resolved_combo = raw_combo.replace("$mod", &config.mod_key);

            let action_tokens = &line[2..];
            let action = parse_action(action_tokens, line_number, config)?;

            match parse_keybind(&resolved_combo, action) {
                Ok(keybind) => {
                    *has_custom_bindings = true;
                    custom_bindings.push(keybind);
                }
                Err(err) => {
                    return Err(ConfigError::ParseError {
                        line: line_number,
                        message: format!("Failed to parse keybind '{}': {}", resolved_combo, err),
                    });
                }
            }
        }
        "autostart" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'autostart' requires a command argument".to_string(),
                });
            }
            let words: Vec<String> = line[1..].iter().map(|t| token_to_string(t)).collect();
            let full_cmd = words.join(" ");
            if let Some(cmd) = AutostartCmd::from_command_line(&full_cmd) {
                config.autostart.push(cmd);
            }
        }
        "tab_show_index" | "show_tab_index" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'tab_show_index' requires 'true' or 'false'".to_string(),
                });
            }
            let val = token_to_string(line[1]).to_lowercase();
            config.tab_config.show_index = val == "true" || val == "1" || val == "yes";
        }
        "tab_max_width" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'tab_max_width' requires a numeric value in pixels".to_string(),
                });
            }
            let width = token_to_u32(line[1], line_number, "tab_max_width")?;
            config.tab_config.max_width = width;
        }
        "tab_mouse" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'tab_mouse' requires 'true' or 'false'".to_string(),
                });
            }
            let val = token_to_string(line[1]).to_lowercase();
            config.tab_config.mouse = val == "true" || val == "1" || val == "yes";
        }
        "tab_wheel_switch" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'tab_wheel_switch' requires 'true' or 'false'".to_string(),
                });
            }
            let val = token_to_string(line[1]).to_lowercase();
            config.tab_config.wheel_switch = val == "true" || val == "1" || val == "yes";
        }
        "workspace_mouse" | "bar_workspace_mouse" | "bar_mouse" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'workspace_mouse' requires 'true' or 'false'".to_string(),
                });
            }
            let val = token_to_string(line[1]).to_lowercase();
            config.workspace_mouse = val == "true" || val == "1" || val == "yes";
        }
        "workspace_wheel_switch" | "bar_workspace_wheel" | "bar_wheel_switch" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'workspace_wheel_switch' requires 'true' or 'false'".to_string(),
                });
            }
            let val = token_to_string(line[1]).to_lowercase();
            config.workspace_wheel_switch = val == "true" || val == "1" || val == "yes";
        }
        unknown => {
            return Err(ConfigError::InvalidValue {
                line: line_number,
                value: unknown.to_string(),
            });
        }
    }

    Ok(())
}

fn token_to_string(token: &Token) -> String {
    match token {
        Token::Word(s) => s.clone(),
        Token::Number(n) => n.to_string(),
        Token::Float(f) => f.to_string(),
        Token::Newline => "\n".to_string(),
        Token::Eof => "".to_string(),
    }
}

fn token_to_u32(token: &Token, line: usize, directive: &str) -> Result<u32, ConfigError> {
    match token {
        Token::Number(n) => Ok(*n),
        Token::Word(s) => s.parse::<u32>().map_err(|_| ConfigError::InvalidValue {
            line,
            value: s.clone(),
        }),
        _ => Err(ConfigError::ParseError {
            line,
            message: format!("Directive '{}' expects an integer value", directive),
        }),
    }
}

fn token_to_color(token: &Token, line: usize, directive: &str) -> Result<u32, ConfigError> {
    match token {
        Token::Number(n) => Ok(*n),
        Token::Word(s) => crate::decoration::parse_color(s).ok_or_else(|| ConfigError::InvalidValue {
            line,
            value: s.clone(),
        }),
        _ => Err(ConfigError::ParseError {
            line,
            message: format!("Directive '{}' expects a color value (e.g. #ffffff)", directive),
        }),
    }
}

fn parse_action(tokens: &[&Token], line: usize, config: &Config) -> Result<Action, ConfigError> {
    if tokens.is_empty() {
        return Err(ConfigError::ParseError {
            line,
            message: "Missing action in bind directive".to_string(),
        });
    }

    let action_name = token_to_string(tokens[0]);
    match action_name.to_lowercase().as_str() {
        "spawn" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'spawn' requires a command".to_string(),
                });
            }
            let cmd: Vec<String> = tokens[1..].iter().map(|t| token_to_string(t)).collect();
            Ok(Action::Spawn(cmd.join(" ")))
        }
        "terminal" => {
            Ok(Action::Spawn(config.terminal.clone()))
        }
        "close" | "kill" => Ok(Action::Close),
        "launcher" => {
            if tokens.len() > 1 {
                let cmd: Vec<String> = tokens[1..].iter().map(|t| token_to_string(t)).collect();
                Ok(Action::Spawn(cmd.join(" ")))
            } else {
                Ok(Action::Launcher)
            }
        }
        "focus" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'focus' requires a direction (left, right, up, down)".to_string(),
                });
            }
            let dir_str = token_to_string(tokens[1]);
            let dir = parse_direction(&dir_str, line)?;
            Ok(Action::Focus(dir))
        }
        "move" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'move' requires a direction (left, right, up, down)".to_string(),
                });
            }
            let dir_str = token_to_string(tokens[1]);
            let dir = parse_direction(&dir_str, line)?;
            Ok(Action::Move(dir))
        }
        "workspace" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'workspace' requires a workspace ID".to_string(),
                });
            }
            let ws_id = token_to_u32(tokens[1], line, "workspace")?;
            Ok(Action::Workspace(ws_id))
        }
        "move_workspace" | "move_to_workspace" | "movetoworkspace" | "move_window_to_workspace" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'move_workspace' requires a workspace ID".to_string(),
                });
            }
            let ws_id = token_to_u32(tokens[1], line, "move_workspace")?;
            Ok(Action::MoveToWorkspace(ws_id))
        }
        "toggle_floating" | "floating" => Ok(Action::ToggleFloating),
        "toggle_fullscreen" | "fullscreen" => Ok(Action::ToggleFullscreen),
        "scratchpad" | "toggle_scratchpad" => Ok(Action::ToggleScratchpad),
        "set_scratchpad" => Ok(Action::SetScratchpad),
        "focus_monitor" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'focus_monitor' requires 'left' or 'right'".to_string(),
                });
            }
            let side = token_to_string(tokens[1]).to_lowercase();
            match side.as_str() {
                "left" => Ok(Action::FocusMonitorLeft),
                "right" => Ok(Action::FocusMonitorRight),
                _ => Err(ConfigError::InvalidValue {
                    line,
                    value: side,
                }),
            }
        }
        "focus_monitor_left" => Ok(Action::FocusMonitorLeft),
        "focus_monitor_right" => Ok(Action::FocusMonitorRight),
        "move_monitor" | "move_to_monitor" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'move_monitor' requires a monitor ID".to_string(),
                });
            }
            let mon_id = token_to_u32(tokens[1], line, "move_monitor")?;
            Ok(Action::MoveToMonitor(mon_id))
        }
        "reload" => Ok(Action::Reload),
        "exit_menu" | "powermenu" | "power_menu" => Ok(Action::ExitMenu),
        "exit" => Ok(Action::Exit),
        "quit" | "shutdown" => Ok(Action::Quit),
        "resize" | "resize_tile" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'resize' requires a direction (left, right, up, down)".to_string(),
                });
            }
            let dir_str = token_to_string(tokens[1]);
            let dir = parse_direction(&dir_str, line)?;
            Ok(Action::ResizeTile(dir))
        }
        "split" | "preselect_split" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'split' requires 'vertical' or 'horizontal'".to_string(),
                });
            }
            let s = token_to_string(tokens[1]).to_lowercase();
            match s.as_str() {
                "vertical" | "v" => Ok(Action::PreselectSplit(crate::layout::SplitType::Vertical)),
                "horizontal" | "h" => Ok(Action::PreselectSplit(crate::layout::SplitType::Horizontal)),
                _ => Err(ConfigError::InvalidValue {
                    line,
                    value: s,
                }),
            }
        }
        "split_vertical" => Ok(Action::PreselectSplit(crate::layout::SplitType::Vertical)),
        "split_horizontal" => Ok(Action::PreselectSplit(crate::layout::SplitType::Horizontal)),
        "cycle_layout" | "cyclelayout" => Ok(Action::CycleLayout),
        "rotate_master" | "rotatemaster" | "rotate_layout" | "rotatelayout" => Ok(Action::RotateMaster),
        "layout" | "set_layout" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'layout' requires a layout type (master, monocle, vertical, horizontal, bsp)".to_string(),
                });
            }
            let name = token_to_string(tokens[1]);
            let lt = name.parse::<crate::layout::LayoutType>().map_err(|_| ConfigError::InvalidValue {
                line,
                value: name,
            })?;
            Ok(Action::SetLayout(lt))
        }
        "layout_master" | "layout_master_left" => Ok(Action::SetLayout(crate::layout::LayoutType::MasterStack)),
        "layout_master_right" => Ok(Action::SetLayout(crate::layout::LayoutType::MasterRight)),
        "layout_master_top" => Ok(Action::SetLayout(crate::layout::LayoutType::MasterTop)),
        "layout_master_bottom" => Ok(Action::SetLayout(crate::layout::LayoutType::MasterBottom)),
        "layout_center_master" | "layout_centered_master" | "layout_centermaster" | "layout_centeredmaster" => Ok(Action::SetLayout(crate::layout::LayoutType::CenterMaster)),
        "layout_monocle" => Ok(Action::SetLayout(crate::layout::LayoutType::Monocle)),
        "layout_tabbed" => Ok(Action::SetLayout(crate::layout::LayoutType::Tabbed)),
        "layout_vertical" | "layout_columns" => Ok(Action::SetLayout(crate::layout::LayoutType::Vertical)),
        "layout_horizontal" | "layout_rows" => Ok(Action::SetLayout(crate::layout::LayoutType::Horizontal)),
        "layout_bsp" => Ok(Action::SetLayout(crate::layout::LayoutType::Bsp)),
        "layout_grid" => Ok(Action::SetLayout(crate::layout::LayoutType::Grid)),
        "focus_next_tab" | "next_tab" => Ok(Action::FocusNextTab),
        "focus_prev_tab" | "prev_tab" | "previous_tab" => Ok(Action::FocusPrevTab),
        "move_tab_left" => Ok(Action::MoveTabLeft),
        "move_tab_right" => Ok(Action::MoveTabRight),
        "select_tab" => {
            if tokens.len() < 2 {
                return Err(ConfigError::ParseError {
                    line,
                    message: "Action 'select_tab' requires a 1-based index argument (e.g. 'select_tab 1')".to_string(),
                });
            }
            let idx = token_to_u32(tokens[1], line, "select_tab")?;
            Ok(Action::SelectTab(idx as usize))
        }
        unknown => Err(ConfigError::InvalidValue {
            line,
            value: unknown.to_string(),
        }),
    }
}

fn parse_direction(s: &str, line: usize) -> Result<Direction, ConfigError> {
    match s.to_lowercase().as_str() {
        "left" | "h" => Ok(Direction::Left),
        "down" | "j" => Ok(Direction::Down),
        "up" | "k" => Ok(Direction::Up),
        "right" | "l" => Ok(Direction::Right),
        _ => Err(ConfigError::InvalidValue {
            line,
            value: s.to_string(),
        }),
    }
}

/// Complete default configuration template created when tilo starts and no config file is found.
pub const DEFAULT_CONFIG_CONTENT: &str = r#"# ==============================================================================
# Tilo Window Manager Configuration File
# Location: ~/.config/tilo/config (or $XDG_CONFIG_HOME/tilo/config)
# ==============================================================================

# --- Modifier Key ---
# Set the primary modifier key: Mod1 (Alt) or Mod4 (Super/Windows key).
# The variable $mod can be used in keybindings below.
mod Mod4

# --- Terminal Emulator ---
# Default terminal emulator command.
terminal alacritty

# --- Default Font ---
# Font specification for the status bar, launcher, and other UI elements.
# Supports i3-style Pango font descriptions (e.g. "pango:monospace 10", "DejaVu Sans Mono 11", "JetBrainsMono Nerd Font 10").
font pango:monospace 10

# --- Window Layout ---
# Initial layout for workspaces: bsp, master, grid, tabbed, monocle.
layout bsp

# --- Window Borders & Geometry ---
# Width of window borders in pixels.
border_width 2

# Gap between adjacent windows in pixels.
inner_gap 8

# Gap between outer windows and screen borders in pixels.
outer_gap 8

# Master window ratio (0.0 to 1.0) for master-stack layouts.
master_ratio 0.60

# --- Unified Color Theme ---
# Global color palette used across window borders, status bar, and launcher.
# Format: #RRGGBB
bg #1a1b26
fg #c0caf5
active_bg #7aa2f7
active_fg #15161e
urgent_bg #f7768e
urgent_fg #15161e
inactive_fg #555566

# Window border colors
border_focused #ffffff
border_normal #444444
border_urgent #ff0000

# --- Component-Specific Overrides (Optional) ---
# Statusbar overrides:
# bar_bg #1a1b26
# bar_fg #c0caf5
# bar_active_bg #7aa2f7
# bar_active_fg #15161e
# bar_urgent_bg #f7768e
# bar_urgent_fg #15161e
# bar_border #7aa2f7
# bar_font monospace 10

# Launcher overrides:
# launcher_bg #1a1b26
# launcher_fg #c0caf5
# launcher_selected_bg #7aa2f7
# launcher_selected_fg #15161e
# launcher_urgent_bg #f7768e
# launcher_urgent_fg #15161e
# launcher_border #7aa2f7
# launcher_font monospace 10

# --- Status Bar & Workspace Behavior ---
# Enable clicking on workspace tags in the status bar to switch workspaces.
workspace_mouse true

# Enable mouse wheel scrolling over workspace tags to cycle workspaces.
workspace_wheel_switch true

# --- Tabbed Layout Settings ---
# Show numeric index in tab titles.
tab_show_index false

# Maximum width of individual tabs in pixels.
tab_max_width 200

# Enable clicking on tabs to switch active window.
tab_mouse true

# Enable mouse wheel scrolling over tabs to cycle windows.
tab_wheel_switch true

# --- Autostart Applications ---
# Execute external applications and daemons when tilo starts.
# autostart picom --daemon
# autostart feh --bg-scale ~/wallpaper.jpg

# --- Keybindings ---
# Applications
bind $mod+Return            spawn alacritty

# Window Management & Session
bind $mod+Shift+q           exit
bind $mod+q                 close

# Spatial Navigation / Window Focus (Vim keys)
bind $mod+h                 focus left
bind $mod+j                 focus down
bind $mod+k                 focus up
bind $mod+l                 focus right

# Spatial Navigation / Window Focus (Arrow keys)
bind $mod+Left              focus left
bind $mod+Down              focus down
bind $mod+Up                focus up
bind $mod+Right             focus right

# Moving Windows (Vim keys)
bind $mod+Shift+h           move left
bind $mod+Shift+j           move down
bind $mod+Shift+k           move up
bind $mod+Shift+l           move right

# Moving Windows (Arrow keys)
bind $mod+Shift+Left        move left
bind $mod+Shift+Down        move down
bind $mod+Shift+Up          move up
bind $mod+Shift+Right       move right

# Window States & Launcher & Scratchpad
bind $mod+f                 toggle_fullscreen
bind $mod+d                 launcher
bind $mod+Shift+space       toggle_floating
bind $mod+space             cycle_layout
bind $mod+r                 rotate_master
bind $mod+Tab               focus_next_tab
bind $mod+Shift+Tab         focus_prev_tab
bind $mod+grave             toggle_scratchpad
bind $mod+Shift+grave       set_scratchpad

# Workspaces (1-9)
bind $mod+1                 workspace 1
bind $mod+2                 workspace 2
bind $mod+3                 workspace 3
bind $mod+4                 workspace 4
bind $mod+5                 workspace 5
bind $mod+6                 workspace 6
bind $mod+7                 workspace 7
bind $mod+8                 workspace 8
bind $mod+9                 workspace 9

# Move Window to Workspace (1-9)
bind $mod+Shift+1           move_to_workspace 1
bind $mod+Shift+2           move_to_workspace 2
bind $mod+Shift+3           move_to_workspace 3
bind $mod+Shift+4           move_to_workspace 4
bind $mod+Shift+5           move_to_workspace 5
bind $mod+Shift+6           move_to_workspace 6
bind $mod+Shift+7           move_to_workspace 7
bind $mod+Shift+8           move_to_workspace 8
bind $mod+Shift+9           move_to_workspace 9

# Multi-Monitor Navigation
bind $mod+comma             focus_monitor left
bind $mod+period            focus_monitor right

# BSP Tiling Controls
bind $mod+Ctrl+Left         resize left
bind $mod+Ctrl+Right        resize right
bind $mod+Ctrl+Up           resize up
bind $mod+Ctrl+Down         resize down

# BSP Split Preselection
bind $mod+v                 split vertical
bind $mod+b                 split horizontal

# Window Manager Controls
bind $mod+Shift+r           reload
"#;

/// Resolves the default configuration file path (`~/.config/tilo/config`).
pub fn default_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("tilo").join("config");
        return Some(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".config").join("tilo").join("config");
        return Some(p);
    }
    None
}

/// Writes the full default configuration to the specified filesystem path.
///
/// Automatically creates any missing parent directories.
pub fn create_default_config_at_path(path: impl AsRef<Path>) -> Result<(), std::io::Error> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DEFAULT_CONFIG_CONTENT)?;
    Ok(())
}

/// Checks if the default configuration file exists (`~/.config/tilo/config`), creating it if missing.
///
/// Returns `Ok(Some(path))` if a new configuration file was created,
/// `Ok(None)` if the configuration file already existed or the default path could not be resolved,
/// or `Err(e)` if writing the default configuration failed.
pub fn create_default_config_if_missing() -> Result<Option<PathBuf>, std::io::Error> {
    if let Some(path) = default_config_path() {
        if !path.exists() {
            create_default_config_at_path(&path)?;
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Loads and parses the configuration file from disk.
///
/// Automatically resolves the path to `~/.config/tilo/config`.
pub fn load_config() -> Result<Config, ConfigError> {
    let path = default_config_path().ok_or_else(|| {
        ConfigError::IoError("Could not determine user configuration directory".to_string())
    })?;
    load_config_from_path(path)
}

/// Loads and parses a configuration file from a specified filesystem path.
pub fn load_config_from_path(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(ConfigError::IoError(format!(
            "Configuration file does not exist: {}",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(path)?;
    let tokens = tokenize(&content)?;
    parse(&tokens)
}

/// Loads configuration from disk, creating the full default configuration file if none exists,
/// and falling back to in-memory defaults if loading or parsing fails.
pub fn load_config_or_default() -> Config {
    // If no configuration file exists at the default path, generate the full default config file.
    match create_default_config_if_missing() {
        Ok(Some(path)) => {
            println!(
                "[INFO  tilo::config] No configuration file found. Created default configuration at: {}",
                path.display()
            );
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!(
                "[WARN  tilo::config] Failed to create default configuration file: {}",
                err
            );
        }
    }

    match load_config() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!(
                "[WARN  tilo::config] Failed to load configuration (falling back to defaults): {}",
                err
            );
            Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybind::MOD_SUPER;
    use crate::keybind::XK_RETURN;

    #[test]
    fn test_tokenize_basic() {
        let input = "border_width 4\ninner_gap 10\n# comment\nmaster_ratio 0.65\n";
        let tokens = tokenize(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Word("border_width".to_string()),
                Token::Number(4),
                Token::Newline,
                Token::Word("inner_gap".to_string()),
                Token::Number(10),
                Token::Newline,
                Token::Newline,
                Token::Word("master_ratio".to_string()),
                Token::Float(0.65),
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_parse_plan_example() {
        let input = r#"
mod Mod4

font JetBrainsMono Nerd Font 10

border_width 2
inner_gap 8
outer_gap 8
master_ratio 0.60

terminal foot

bind Mod4+Return spawn foot
bind Mod4+Space launcher
bind Mod4+q close

bind Mod4+h focus left
bind Mod4+j focus down
bind Mod4+k focus up
bind Mod4+l focus right

bind Mod4+Shift+h move left
bind Mod4+Shift+j move down
bind Mod4+Shift+k move up
bind Mod4+Shift+l move right

bind Mod4+1 workspace 1
bind Mod4+2 workspace 2

bind Mod4+Shift+1 move_workspace 1
bind Mod4+Shift+2 move_workspace 2
"#;
        let tokens = tokenize(input).unwrap();
        let config = parse(&tokens).unwrap();

        assert_eq!(config.mod_key, "Mod4");
        assert_eq!(config.font, "JetBrainsMono Nerd Font 10");
        assert_eq!(config.border_width, 2);
        assert_eq!(config.inner_gap, 8);
        assert_eq!(config.outer_gap, 8);
        assert_eq!(config.master_ratio, 0.60);
        assert_eq!(config.terminal, "foot");

        assert_eq!(config.bindings.len(), 15);
        assert_eq!(config.bindings[0].modifiers, MOD_SUPER);
        assert_eq!(config.bindings[0].keysym, XK_RETURN);
        assert_eq!(config.bindings[0].action, Action::Spawn("foot".to_string()));
    }

    #[test]
    fn test_parse_mod_key_alias() {
        let input1 = "mod_key Mod4\nborder_width 3\n";
        let tokens1 = tokenize(input1).unwrap();
        let config1 = parse(&tokens1).unwrap();
        assert_eq!(config1.mod_key, "Mod4");
        assert_eq!(config1.border_width, 3);

        let input2 = "modifier Mod1\nborder_width 4\n";
        let tokens2 = tokenize(input2).unwrap();
        let config2 = parse(&tokens2).unwrap();
        assert_eq!(config2.mod_key, "Mod1");
        assert_eq!(config2.border_width, 4);
    }

    #[test]
    fn test_parse_invalid_directive() {
        let input = "invalid_directive 123\n";
        let tokens = tokenize(input).unwrap();
        let err = parse(&tokens).unwrap_err();
        match err {
            ConfigError::InvalidValue { line, value } => {
                assert_eq!(line, 1);
                assert_eq!(value, "invalid_directive");
            }
            _ => panic!("Expected InvalidValue error"),
        }
    }

    #[test]
    fn test_parse_invalid_master_ratio() {
        let input = "master_ratio 1.5\n";
        let tokens = tokenize(input).unwrap();
        let err = parse(&tokens).unwrap_err();
        match err {
            ConfigError::InvalidValue { line, value } => {
                assert_eq!(line, 1);
                assert_eq!(value, "1.5");
            }
            _ => panic!("Expected InvalidValue error"),
        }
    }

    #[test]
    fn test_default_config_font_is_monospace() {
        let config = Config::default();
        assert_eq!(config.font, "monospace");
    }

    #[test]
    fn test_parse_autostart_directives() {
        let input = r#"
autostart "nm-applet"
autostart "dunst"
autostart "feh --bg-fill /path/to/wallpaper.jpg"
autostart picom -b
"#;
        let tokens = tokenize(input).unwrap();
        let config = parse(&tokens).unwrap();

        assert_eq!(config.autostart.len(), 4);
        assert_eq!(config.autostart[0].program, "nm-applet");
        assert!(config.autostart[0].args.is_empty());

        assert_eq!(config.autostart[1].program, "dunst");
        assert!(config.autostart[1].args.is_empty());

        assert_eq!(config.autostart[2].program, "feh");
        assert_eq!(config.autostart[2].args, vec!["--bg-fill", "/path/to/wallpaper.jpg"]);

        assert_eq!(config.autostart[3].program, "picom");
        assert_eq!(config.autostart[3].args, vec!["-b"]);
    }

    #[test]
    fn test_parse_border_decorations() {
        let input = r#"
border_width 3
focused_color #ffffff
normal_color #444444
urgent_color #ff0000
"#;
        let tokens = tokenize(input).unwrap();
        let config = parse(&tokens).unwrap();

        assert_eq!(config.border_width, 3);
        assert_eq!(config.focused_color, 0x00ffffff);
        assert_eq!(config.normal_color, 0x00444444);
        assert_eq!(config.urgent_color, 0x00ff0000);
    }

    #[test]
    fn test_parse_rotated_master_and_layout_bindings() {
        let input = r#"
layout master-top
bind $mod+r rotate_master
bind $mod+Shift+r rotatelayout
bind $mod+1 layout_master_left
bind $mod+2 layout_master_right
bind $mod+3 layout_master_top
bind $mod+4 layout_master_bottom
bind $mod+5 layout_center_master
bind $mod+6 layout_grid
"#;
        let tokens = tokenize(input).unwrap();
        let config = parse(&tokens).unwrap();

        assert_eq!(config.layout, crate::layout::LayoutType::MasterTop);
        assert_eq!(config.bindings.len(), 8);
        assert_eq!(config.bindings[0].action, Action::RotateMaster);
        assert_eq!(config.bindings[1].action, Action::RotateMaster);
        assert_eq!(config.bindings[2].action, Action::SetLayout(crate::layout::LayoutType::MasterStack));
        assert_eq!(config.bindings[3].action, Action::SetLayout(crate::layout::LayoutType::MasterRight));
        assert_eq!(config.bindings[4].action, Action::SetLayout(crate::layout::LayoutType::MasterTop));
        assert_eq!(config.bindings[5].action, Action::SetLayout(crate::layout::LayoutType::MasterBottom));
        assert_eq!(config.bindings[6].action, Action::SetLayout(crate::layout::LayoutType::CenterMaster));
        assert_eq!(config.bindings[7].action, Action::SetLayout(crate::layout::LayoutType::Grid));
    }

    #[test]
    fn test_parse_tabbed_config_and_bindings() {
        let input = r#"
layout tabbed
tab_show_index true
tab_max_width 200
tab_mouse false
tab_wheel_switch false
bind $mod+Tab focus_next_tab
bind $mod+Shift+Tab focus_prev_tab
bind $mod+Shift+Left move_tab_left
bind $mod+Shift+Right move_tab_right
bind $mod+1 select_tab 1
"#;
        let tokens = tokenize(input).unwrap();
        let config = parse(&tokens).unwrap();

        assert_eq!(config.layout, crate::layout::LayoutType::Tabbed);
        assert!(config.tab_config.show_index);
        assert_eq!(config.tab_config.max_width, 200);
        assert!(!config.tab_config.mouse);
        assert!(!config.tab_config.wheel_switch);

        assert_eq!(config.bindings.len(), 5);
        assert_eq!(config.bindings[0].action, Action::FocusNextTab);
        assert_eq!(config.bindings[1].action, Action::FocusPrevTab);
        assert_eq!(config.bindings[2].action, Action::MoveTabLeft);
        assert_eq!(config.bindings[3].action, Action::MoveTabRight);
        assert_eq!(config.bindings[4].action, Action::SelectTab(1));
    }

    #[test]
    fn test_default_config_template_parsing() {
        let tokens = tokenize(DEFAULT_CONFIG_CONTENT).expect("DEFAULT_CONFIG_CONTENT should tokenize successfully");
        let config = parse(&tokens).expect("DEFAULT_CONFIG_CONTENT should parse successfully");

        assert_eq!(config.mod_key, "Mod4");
        assert_eq!(config.terminal, "alacritty");
        assert_eq!(config.font, "pango:monospace 10");
        assert_eq!(config.layout, crate::layout::LayoutType::Bsp);
        assert_eq!(config.border_width, 2);
        assert_eq!(config.inner_gap, 8);
        assert_eq!(config.outer_gap, 8);
        assert!((config.master_ratio - 0.60).abs() < f32::EPSILON);

        assert_eq!(config.theme.bg, 0x1a1b26);
        assert_eq!(config.theme.fg, 0xc0caf5);
        assert_eq!(config.theme.active_bg, 0x7aa2f7);
        assert_eq!(config.theme.active_fg, 0x15161e);
        assert_eq!(config.theme.urgent_bg, 0xf7768e);
        assert_eq!(config.theme.urgent_fg, 0x15161e);
        assert_eq!(config.theme.inactive_fg, 0x555566);

        assert_eq!(config.focused_color, 0x00ffffff);
        assert_eq!(config.normal_color, 0x00444444);
        assert_eq!(config.urgent_color, 0x00ff0000);

        assert!(config.workspace_mouse);
        assert!(config.workspace_wheel_switch);

        assert!(!config.tab_config.show_index);
        assert_eq!(config.tab_config.max_width, 200);
        assert!(config.tab_config.mouse);
        assert!(config.tab_config.wheel_switch);

        let default_bindings = default_keybindings();
        assert_eq!(config.bindings.len(), default_bindings.len());
        for (idx, (parsed, expected)) in config.bindings.iter().zip(default_bindings.iter()).enumerate() {
            assert_eq!(parsed, expected, "Mismatch at binding index {}", idx);
        }
    }

    #[test]
    fn test_create_default_config_at_path() {
        let temp_dir = std::env::temp_dir().join(format!("tilo_cfg_test_{}", std::process::id()));
        let config_file = temp_dir.join("subfolder").join("config");

        assert!(!config_file.exists());
        create_default_config_at_path(&config_file).expect("Creation should succeed");
        assert!(config_file.exists());

        let content = std::fs::read_to_string(&config_file).expect("File should be readable");
        assert_eq!(content, DEFAULT_CONFIG_CONTENT);

        let loaded = load_config_from_path(&config_file).expect("Loaded config should parse");
        assert_eq!(loaded.mod_key, "Mod4");
        assert_eq!(loaded.layout, crate::layout::LayoutType::Bsp);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
