//! Configuration system for tilo.
//!
//! Implements Phase 15 & 16 of Plan.md:
//! - Custom line-oriented, whitespace-delimited configuration syntax.
//! - Directives for modifiers, fonts, border widths, inner/outer gaps, master ratios, and keybindings.
//! - Lexical tokenizer ([`tokenize`]), recursive-descent parser ([`parse`]), and disk loader ([`load_config`]).
//! - Safe configuration reload without restarting the window manager ([`ReloadError`]).

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
    /// A list of defined keybindings.
    pub bindings: Vec<KeyBind>,
    /// Optional startup/autostart commands.
    pub autostart: Vec<AutostartCmd>,
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
            bindings: default_keybindings(),
            autostart: Vec::new(),
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
                let is_end_of_token = next_char.map_or(true, |ch| {
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
        "mod" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'mod' requires a key argument (e.g. 'mod Mod4')".to_string(),
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
        "focused_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'focused_color' requires a color argument (e.g. #ffffff)".to_string(),
                });
            }
            let color = token_to_color(line[1], line_number, "focused_color")?;
            config.focused_color = color;
        }
        "normal_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'normal_color' requires a color argument (e.g. #444444)".to_string(),
                });
            }
            let color = token_to_color(line[1], line_number, "normal_color")?;
            config.normal_color = color;
        }
        "urgent_color" => {
            if line.len() < 2 {
                return Err(ConfigError::ParseError {
                    line: line_number,
                    message: "Directive 'urgent_color' requires a color argument (e.g. #ff0000)".to_string(),
                });
            }
            let color = token_to_color(line[1], line_number, "urgent_color")?;
            config.urgent_color = color;
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
        "move_workspace" | "move_to_workspace" => {
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
        "exit" | "quit" => Ok(Action::Exit),
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

/// Loads configuration from disk, falling back to default configuration if not found.
pub fn load_config_or_default() -> Config {
    load_config().unwrap_or_default()
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
}
