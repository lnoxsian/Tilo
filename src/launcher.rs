//! Integrated application launcher.
//!
//! Implements Phase 19, 20, and 21 of Plan.md:
//! - `$PATH` scanning and executable caching ([`scan_path`]).
//! - Direct, detached process execution supporting CLI arguments ([`spawn_app`]).
//! - Fast fuzzy matching scoring algorithm ([`fuzzy_match`]).
//! - Keyboard state machine and actions ([`LauncherAction`], [`handle_launcher_keypress`]).
//! - Double-buffered overlay window with search input and filtered result list.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use crate::input::KeyboardMapping;
use crate::keybind::{
    MOD_CONTROL, MOD_SUPER, XK_A, XK_B, XK_BACKSPACE, XK_C, XK_D, XK_DOWN, XK_E,
    XK_END, XK_ESCAPE, XK_F, XK_HOME, XK_LEFT, XK_N, XK_P, XK_RETURN, XK_RIGHT,
    XK_TAB, XK_U, XK_UP, XK_W,
};
use crate::layout::Rect;
use crate::render::Renderer;
use crate::statusbar::{ColorScheme, StatusbarError, TiloError};
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, EventMask, Screen, WindowClass,
};

/// Type alias for keyboard mapping matching Plan.md specification.
pub type KeysymMap = KeyboardMapping;

/// Default width of the application launcher overlay window.
pub const LAUNCHER_WIDTH: u16 = 560;

/// Default height of the application launcher overlay window when displaying the top 8 suggestions.
pub const LAUNCHER_HEIGHT: u16 = 256;

/// Maximum number of suggestions displayed by default.
pub const LAUNCHER_MAX_RESULTS: usize = 8;

/// Core configuration and state for the application launcher.
#[derive(Debug)]
pub struct Launcher {
    /// X11 window ID for the launcher overlay (0 if unallocated).
    pub window_id: u32,
    /// Cached list of executable names discovered in `$PATH`.
    pub executable_cache: Vec<String>,
    /// Current search query typed by the user.
    pub query: String,
    /// Cursor character index within the search query.
    pub cursor_position: usize,
    /// Currently selected index in the filtered results.
    pub selected_index: usize,
    /// Whether the launcher is currently active and mapped on screen.
    pub visible: bool,
    /// The geometric bounds of the launcher window.
    pub rect: Rect,
    /// Double-buffered graphics renderer.
    pub renderer: Option<Renderer>,
    /// Visual color scheme.
    pub color_scheme: ColorScheme,
}

impl Launcher {
    /// Creates a new `Launcher` instance with scanned `$PATH` executables.
    pub fn new() -> Self {
        let executable_cache = scan_path().unwrap_or_default();
        Self {
            window_id: 0,
            executable_cache,
            query: String::new(),
            cursor_position: 0,
            selected_index: 0,
            visible: false,
            rect: Rect::new(0, 0, LAUNCHER_WIDTH as u32, LAUNCHER_HEIGHT as u32),
            renderer: None,
            color_scheme: ColorScheme::default(),
        }
    }

    /// Calculates the exact window height required for a given number of result items with no empty space.
    pub fn calculate_height(item_count: usize) -> u16 {
        let count = item_count.min(LAUNCHER_MAX_RESULTS);
        if count == 0 {
            // Header (42) + notice row (26) + bottom padding (6)
            42 + 26 + 6
        } else {
            // Header (42) + item rows (count * 26) + bottom padding (6)
            42 + (count as u16 * 26) + 6
        }
    }

    /// Attaches an initialized [`Renderer`] to this launcher.
    pub fn attach_renderer(&mut self, renderer: Renderer) {
        self.renderer = Some(renderer);
    }

    /// Returns the top filtered executable results matching the current query.
    pub fn filtered_results(&self, max_results: usize) -> Vec<String> {
        filter_executables(&self.executable_cache, &self.query, max_results)
    }

    /// Renders the launcher UI into its off-screen pixmap and copies it to the window.
    pub fn render(&mut self, conn: &impl Connection) -> Result<(), StatusbarError> {
        let renderer = match self.renderer {
            Some(ref r) => r,
            None => return Ok(()),
        };

        if self.window_id == 0 {
            return Ok(());
        }

        let results = self.filtered_results(LAUNCHER_MAX_RESULTS);

        // Keep selected index within valid bounds
        if !results.is_empty() && self.selected_index >= results.len() {
            self.selected_index = results.len() - 1;
        }

        // Dynamically compute the window height to snugly fit the suggestions with zero empty space
        let display_count = if results.is_empty() && !self.query.is_empty() {
            0
        } else {
            results.len().min(LAUNCHER_MAX_RESULTS)
        };
        let target_height = Self::calculate_height(display_count);

        if target_height != self.rect.height as u16 {
            self.rect.height = target_height as u32;
            let _ = conn.configure_window(
                self.window_id,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .height(target_height as u32),
            );
        }

        let w = self.rect.width as u16;
        let h = self.rect.height as u16;

        // 1. Fill entire launcher background
        renderer.fill_rect(
            conn,
            renderer.pixmap,
            0,
            0,
            w,
            h,
            self.color_scheme.bg,
        )?;

        // 2. Draw border around the launcher overlay
        let border_color = self.color_scheme.active_bg;
        // Top, bottom, left, right border outlines
        renderer.fill_rect(conn, renderer.pixmap, 0, 0, w, 2, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, 0, (h - 2) as i16, w, 2, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, 0, 0, 2, h, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, (w - 2) as i16, 0, 2, h, border_color)?;

        // 3. Draw input prompt box
        let input_baseline = 24i16;
        let prompt_prefix = "> ";
        renderer.draw_text(
            conn,
            renderer.pixmap,
            16,
            input_baseline,
            prompt_prefix,
            self.color_scheme.active_bg,
        )?;

        let prefix_width = renderer.text_width(prompt_prefix);
        let query_text = if self.query.is_empty() {
            "Type application name..."
        } else {
            &self.query
        };
        let query_color = if self.query.is_empty() {
            0x666677
        } else {
            self.color_scheme.fg
        };

        let text_start_x = if self.query.is_empty() {
            16 + prefix_width as i16 + 4
        } else {
            16 + prefix_width as i16
        };

        renderer.draw_text(
            conn,
            renderer.pixmap,
            text_start_x,
            input_baseline,
            query_text,
            query_color,
        )?;

        // Draw vertical cursor bar at cursor_position
        let text_before_cursor: String = self.query.chars().take(self.cursor_position).collect();
        let cursor_offset = renderer.text_width(&text_before_cursor);
        let cursor_x = 16 + prefix_width as i16 + cursor_offset as i16;
        let cursor_y = 10i16;
        let cursor_height = 16u16;
        renderer.fill_rect(
            conn,
            renderer.pixmap,
            cursor_x,
            cursor_y,
            2,
            cursor_height,
            self.color_scheme.active_bg,
        )?;

        // 4. Draw separator line below input field
        renderer.fill_rect(conn, renderer.pixmap, 8, 36, w.saturating_sub(16), 1, 0x333344)?;

        // 5. Draw filtered result items
        let mut cur_y = 42i16;
        let row_height = 26i16;

        for (idx, item) in results.iter().enumerate() {
            let is_selected = idx == self.selected_index;
            let item_baseline = cur_y + 18;

            if is_selected {
                // Highlight row background
                renderer.fill_rect(
                    conn,
                    renderer.pixmap,
                    8,
                    cur_y,
                    w.saturating_sub(16),
                    row_height as u16,
                    self.color_scheme.active_bg,
                )?;
                // Highlight text
                renderer.draw_text(
                    conn,
                    renderer.pixmap,
                    20,
                    item_baseline,
                    item,
                    self.color_scheme.active_fg,
                )?;
            } else {
                // Inactive row text
                renderer.draw_text(
                    conn,
                    renderer.pixmap,
                    20,
                    item_baseline,
                    item,
                    self.color_scheme.fg,
                )?;
            }

            cur_y += row_height;
        }

        // If no results and query is non-empty, indicate direct command execution mode
        if results.is_empty() && !self.query.is_empty() {
            let notice = format!("Run: '{}'", self.query);
            renderer.draw_text(
                conn,
                renderer.pixmap,
                20,
                cur_y + 18,
                &notice,
                self.color_scheme.active_bg,
            )?;
        }

        // 6. Blit completed double-buffer pixmap sub-area to launcher window
        renderer.flush_area_to_window(conn, self.window_id, w, h)?;

        Ok(())
    }

    /// Destroys the X11 window associated with the launcher.
    pub fn destroy<C: Connection>(&mut self, conn: &C) -> Result<(), X11Error> {
        if self.window_id != 0 {
            let _ = conn.unmap_window(self.window_id);
            let _ = conn.destroy_window(self.window_id);
            conn.flush()?;
            self.window_id = 0;
            self.visible = false;
        }
        Ok(())
    }
}

impl Default for Launcher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PATH Scanning and Caching
// ---------------------------------------------------------------------------

/// Scans the system `$PATH` for executable files and caches them.
///
/// # Returns
/// A sorted vector of unique executable names.
pub fn scan_path() -> Result<Vec<String>, TiloError> {
    let path_var = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    scan_path_from(&path_var)
}

/// Scans the specified colon-delimited path string for executable files.
pub fn scan_path_from(path_var: &str) -> Result<Vec<String>, TiloError> {
    let mut executables = HashSet::new();

    for dir in path_var.split(':') {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let file_path = entry.path();
                if let Ok(metadata) = file_path.metadata() {
                    // Check if file is executable by user, group, or others
                    if (metadata.is_file() || metadata.file_type().is_symlink())
                        && (metadata.permissions().mode() & 0o111 != 0)
                    {
                        if let Some(file_name) = file_path.file_name().and_then(|n| n.to_str()) {
                            executables.insert(file_name.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut sorted: Vec<String> = executables.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

// ---------------------------------------------------------------------------
// Command-line Parsing & Direct Process Execution
// ---------------------------------------------------------------------------

/// Parses a command line string into a list of arguments, respecting single and double quotes.
///
/// # Examples
/// ```
/// use tilo::launcher::parse_command_line;
/// assert_eq!(parse_command_line("firefox --private-window"), vec!["firefox", "--private-window"]);
/// assert_eq!(parse_command_line("xterm -e 'htop -d 10'"), vec!["xterm", "-e", "htop -d 10"]);
/// ```
pub fn parse_command_line(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for c in input.chars() {
        match c {
            '"' | '\'' if in_quotes && c == quote_char => {
                in_quotes = false;
            }
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(current);
                    current = String::new();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Spawns the selected application as a detached child process, parsing command line arguments.
///
/// Supports passing command-line flags and parameters (e.g., `"firefox --private-window"`).
/// Avoids invoking a shell (`sh -c`) when spawning the binary directly.
///
/// # Arguments
/// * `app_name` - The executable name, path, or command line with arguments to launch.
///
/// # Returns
/// The Process ID (PID) of the spawned application, or an error.
pub fn spawn_app(app_name: &str) -> Result<u32, TiloError> {
    let args = parse_command_line(app_name);
    if args.is_empty() {
        return Err(StatusbarError::SpawnError("Empty command line".to_string()));
    }

    let binary = &args[0];
    let binary_args = &args[1..];

    let child = Command::new(binary)
        .args(binary_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| StatusbarError::SpawnError(format!("Failed to spawn '{}': {}", app_name, e)))?;

    Ok(child.id())
}

// ---------------------------------------------------------------------------
// Fuzzy Matching & Filtering
// ---------------------------------------------------------------------------

/// Computes a fuzzy match score between a user query and a target string.
///
/// Higher scores represent better matches. Returns `None` if the target
/// does not contain all query characters in sequence.
pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let query_chars: Vec<char> = query.chars().collect();
    let target_chars: Vec<char> = target.chars().collect();

    if query_chars.is_empty() {
        return Some(0);
    }

    let mut q_idx = 0;
    let mut score = 0;
    let mut consecutive = 0;

    for (t_idx, &t_c) in target_chars.iter().enumerate() {
        if q_idx < query_chars.len() && t_c.eq_ignore_ascii_case(&query_chars[q_idx]) {
            score += 10;
            score += consecutive * 5;
            if t_idx == 0 {
                score += 15; // Starting character bonus
            } else if target_chars[t_idx - 1] == '-'
                || target_chars[t_idx - 1] == '_'
                || target_chars[t_idx - 1] == '.'
            {
                score += 10; // Word boundary bonus (e.g. "fc" matching "firewall-cmd")
            }
            consecutive += 1;
            q_idx += 1;

            if q_idx == query_chars.len() {
                return Some(score);
            }
        } else {
            consecutive = 0;
            score -= 1; // Gap penalty
        }
    }

    if q_idx == query_chars.len() {
        Some(score)
    } else {
        None
    }
}

/// Filters and ranks cached executable names against the user query.
///
/// If the query contains arguments (e.g. `firefox --private-window`),
/// only the binary portion is used to match against the executable cache.
pub fn filter_executables(cache: &[String], query: &str, max_results: usize) -> Vec<String> {
    let binary_query = query.split_whitespace().next().unwrap_or("").trim();
    if binary_query.is_empty() {
        return cache.iter().take(max_results).cloned().collect();
    }

    let query_lower = binary_query.to_lowercase();
    let first_char = query_lower.chars().next();
    let mut scored: Vec<(&String, i32)> = Vec::new();

    for item in cache {
        let item_lower = item.to_lowercase();

        if item_lower == query_lower {
            // Exact match
            scored.push((item, 5000));
        } else if item_lower.starts_with(&query_lower) {
            // High priority for prefix matches
            let score = 3000 - (item.len() as i32 * 2);
            scored.push((item, score));
        } else if let Some(fuzzy_score) = fuzzy_match(&query_lower, &item_lower) {
            // If the item starts with the first character of the query, it ranks above substring matches
            if first_char.map_or(false, |fc| item_lower.starts_with(fc)) {
                let score = 1500 + fuzzy_score * 10 - (item.len() as i32);
                scored.push((item, score));
            } else if let Some(pos) = item_lower.find(&query_lower) {
                // Substring matches that don't start with query char
                let score = 500 - (pos as i32 * 5) - (item.len() as i32);
                scored.push((item, score));
            } else {
                // General fuzzy match
                scored.push((item, fuzzy_score * 5 - (item.len() as i32)));
            }
        } else if let Some(pos) = item_lower.find(&query_lower) {
            let score = 500 - (pos as i32 * 5) - (item.len() as i32);
            scored.push((item, score));
        }
    }

    scored.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.len().cmp(&b.0.len()))
            .then_with(|| a.0.cmp(b.0))
    });
    scored.into_iter().take(max_results).map(|(s, _)| s.clone()).collect()
}

// ---------------------------------------------------------------------------
// Keyboard Action State Machine
// ---------------------------------------------------------------------------

/// Represents a parsed user keyboard intent in the launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherAction {
    /// Move selection up (Arrow Up, Ctrl+P)
    SelectPrevious,
    /// Move selection down (Arrow Down, Ctrl+N)
    SelectNext,
    /// Execute the currently selected item or typed command line (Enter)
    Execute,
    /// Close the launcher without executing (Escape)
    Cancel,
    /// Delete the character immediately before the cursor (Backspace)
    DeleteChar,
    /// Delete the word before the cursor (Ctrl+W)
    DeleteWord,
    /// Delete all text before the cursor (Ctrl+U)
    DeleteLine,
    /// Move cursor to start of line (Ctrl+A, Home)
    MoveToStart,
    /// Move cursor to end of line (Ctrl+E, End)
    MoveToEnd,
    /// Move cursor one character to the left (Left arrow, Ctrl+B)
    MoveCursorLeft,
    /// Move cursor one character to the right (Right arrow, Ctrl+F)
    MoveCursorRight,
    /// Auto-complete selected suggestion into query (Tab)
    AutoComplete,
    /// Insert a character into the query
    InsertChar(char),
}

/// Translates raw X11 KeyPress keycodes and modifiers into a [`LauncherAction`].
pub fn handle_launcher_keypress(
    keycode: u8,
    modifiers: u16,
    mapping: &KeyboardMapping,
) -> Option<LauncherAction> {
    let keysym = mapping.keycode_to_keysym(keycode, modifiers);
    let ctrl = (modifiers & MOD_CONTROL) != 0;

    let is_mod = (modifiers & MOD_SUPER) != 0;
    if is_mod {
        if keysym == XK_D || keysym == XK_ESCAPE {
            return Some(LauncherAction::Cancel);
        }
        return None;
    }

    if ctrl {
        match keysym {
            XK_P => return Some(LauncherAction::SelectPrevious),
            XK_N => return Some(LauncherAction::SelectNext),
            XK_W => return Some(LauncherAction::DeleteWord),
            XK_U => return Some(LauncherAction::DeleteLine),
            XK_A => return Some(LauncherAction::MoveToStart),
            XK_E => return Some(LauncherAction::MoveToEnd),
            XK_B => return Some(LauncherAction::MoveCursorLeft),
            XK_F => return Some(LauncherAction::MoveCursorRight),
            XK_C => return Some(LauncherAction::Cancel),
            _ => return None,
        }
    }

    match keysym {
        XK_RETURN => Some(LauncherAction::Execute),
        XK_ESCAPE => Some(LauncherAction::Cancel),
        XK_BACKSPACE => Some(LauncherAction::DeleteChar),
        XK_UP => Some(LauncherAction::SelectPrevious),
        XK_DOWN => Some(LauncherAction::SelectNext),
        XK_LEFT => Some(LauncherAction::MoveCursorLeft),
        XK_RIGHT => Some(LauncherAction::MoveCursorRight),
        XK_HOME => Some(LauncherAction::MoveToStart),
        XK_END => Some(LauncherAction::MoveToEnd),
        XK_TAB => Some(LauncherAction::AutoComplete),
        s if (0x20..=0x7e).contains(&s) => {
            Some(LauncherAction::InsertChar(s as u8 as char))
        }
        _ => None,
    }
}

/// Applies a [`LauncherAction`] to update the query and selection of the [`Launcher`].
pub fn update_launcher_state(launcher: &mut Launcher, action: LauncherAction) {
    match action {
        LauncherAction::InsertChar(c) => {
            let mut chars: Vec<char> = launcher.query.chars().collect();
            let pos = launcher.cursor_position.min(chars.len());
            chars.insert(pos, c);
            launcher.cursor_position = pos + 1;
            launcher.query = chars.into_iter().collect();
            launcher.selected_index = 0;
        }
        LauncherAction::DeleteChar => {
            let mut chars: Vec<char> = launcher.query.chars().collect();
            let pos = launcher.cursor_position.min(chars.len());
            if pos > 0 {
                chars.remove(pos - 1);
                launcher.cursor_position = pos - 1;
                launcher.query = chars.into_iter().collect();
                launcher.selected_index = 0;
            }
        }
        LauncherAction::DeleteWord => {
            let mut chars: Vec<char> = launcher.query.chars().collect();
            let pos = launcher.cursor_position.min(chars.len());
            if pos > 0 {
                let mut i = pos;
                // Skip trailing whitespace before cursor
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                // Skip word characters
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                chars.drain(i..pos);
                launcher.cursor_position = i;
                launcher.query = chars.into_iter().collect();
                launcher.selected_index = 0;
            }
        }
        LauncherAction::DeleteLine => {
            let mut chars: Vec<char> = launcher.query.chars().collect();
            let pos = launcher.cursor_position.min(chars.len());
            chars.drain(0..pos);
            launcher.cursor_position = 0;
            launcher.query = chars.into_iter().collect();
            launcher.selected_index = 0;
        }
        LauncherAction::MoveToStart => {
            launcher.cursor_position = 0;
        }
        LauncherAction::MoveToEnd => {
            launcher.cursor_position = launcher.query.chars().count();
        }
        LauncherAction::MoveCursorLeft => {
            launcher.cursor_position = launcher.cursor_position.saturating_sub(1);
        }
        LauncherAction::MoveCursorRight => {
            let len = launcher.query.chars().count();
            if launcher.cursor_position < len {
                launcher.cursor_position += 1;
            }
        }
        LauncherAction::AutoComplete => {
            let results = launcher.filtered_results(LAUNCHER_MAX_RESULTS);
            if let Some(selected) = results.get(launcher.selected_index) {
                let trimmed = launcher.query.trim_start();
                if let Some(space_idx) = trimmed.find(' ') {
                    let args = &trimmed[space_idx..];
                    launcher.query = format!("{}{}", selected, args);
                } else {
                    launcher.query = format!("{} ", selected);
                }
                launcher.cursor_position = launcher.query.chars().count();
                launcher.selected_index = 0;
            }
        }
        LauncherAction::SelectNext => {
            let count = launcher.filtered_results(LAUNCHER_MAX_RESULTS).len();
            if count > 0 {
                launcher.selected_index = (launcher.selected_index + 1) % count;
            }
        }
        LauncherAction::SelectPrevious => {
            let count = launcher.filtered_results(LAUNCHER_MAX_RESULTS).len();
            if count > 0 {
                if launcher.selected_index == 0 {
                    launcher.selected_index = count - 1;
                } else {
                    launcher.selected_index -= 1;
                }
            }
        }
        LauncherAction::Execute | LauncherAction::Cancel => {}
    }
}

// ---------------------------------------------------------------------------
// Window Creation with override_redirect
// ---------------------------------------------------------------------------

/// Creates an unmapped floating overlay window for the application launcher.
pub fn create_launcher_window(
    conn: &impl Connection,
    screen: &Screen,
    rect: Rect,
    background_color: u32,
) -> Result<u32, StatusbarError> {
    let window_id = conn.generate_id()?;

    let aux = CreateWindowAux::new()
        .override_redirect(1) // Avoid WM window management interception
        .background_pixel(background_color)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::STRUCTURE_NOTIFY
                | EventMask::KEY_PRESS
                | EventMask::BUTTON_PRESS,
        );

    conn.create_window(
        screen.root_depth,
        window_id,
        screen.root,
        rect.x as i16,
        rect.y as i16,
        rect.width as u16,
        rect.height as u16,
        0,
        WindowClass::COPY_FROM_PARENT,
        screen.root_visual,
        &aux,
    )?;

    conn.flush()?;
    Ok(window_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_line_simple_and_with_args() {
        assert_eq!(
            parse_command_line("firefox"),
            vec!["firefox"]
        );
        assert_eq!(
            parse_command_line("firefox --private-window"),
            vec!["firefox", "--private-window"]
        );
        assert_eq!(
            parse_command_line("xterm -bg black -fg white -e htop"),
            vec!["xterm", "-bg", "black", "-fg", "white", "-e", "htop"]
        );
        assert_eq!(
            parse_command_line("alacritty -e \"echo 'hello world'\""),
            vec!["alacritty", "-e", "echo 'hello world'"]
        );
        assert_eq!(parse_command_line("   "), Vec::<String>::new());
    }

    #[test]
    fn test_fuzzy_match_scoring() {
        assert_eq!(fuzzy_match("", "anything"), Some(0));
        assert!(fuzzy_match("ff", "firefox").is_some());
        assert!(fuzzy_match("fox", "firefox").is_some());
        assert!(fuzzy_match("xyz", "firefox").is_none());

        // Prefix match should score higher than mid-string match
        let score_prefix = fuzzy_match("fire", "firefox").unwrap();
        let score_mid = fuzzy_match("ref", "firefox").unwrap();
        assert!(score_prefix > score_mid);
    }

    #[test]
    fn test_filter_executables_with_arguments() {
        let cache = vec![
            "firefox".to_string(),
            "firewall-applet".to_string(),
            "alacritty".to_string(),
            "foot".to_string(),
            "xterm".to_string(),
        ];

        // Matching without arguments
        let res = filter_executables(&cache, "fire", 5);
        assert!(res.contains(&"firefox".to_string()));
        assert!(res.contains(&"firewall-applet".to_string()));

        // Matching with arguments should still match the binary "firefox"
        let res_args = filter_executables(&cache, "firefox --private-window", 5);
        assert_eq!(res_args.first().unwrap(), "firefox");
    }

    #[test]
    fn test_launcher_state_transitions() {
        let mut launcher = Launcher::new();
        launcher.executable_cache = vec!["firefox".to_string(), "foot".to_string(), "xterm".to_string()];

        update_launcher_state(&mut launcher, LauncherAction::InsertChar('f'));
        update_launcher_state(&mut launcher, LauncherAction::InsertChar('i'));
        assert_eq!(launcher.query, "fi");

        update_launcher_state(&mut launcher, LauncherAction::DeleteChar);
        assert_eq!(launcher.query, "f");

        update_launcher_state(&mut launcher, LauncherAction::SelectNext);
        assert_eq!(launcher.selected_index, 1);

        update_launcher_state(&mut launcher, LauncherAction::SelectPrevious);
        assert_eq!(launcher.selected_index, 0);

        update_launcher_state(&mut launcher, LauncherAction::DeleteLine);
        assert_eq!(launcher.query, "");
    }

    #[test]
    fn test_calculate_height_no_empty_space() {
        // Height for top 8 suggestions must be exactly 256 px with zero empty space
        assert_eq!(Launcher::calculate_height(8), 256);
        assert_eq!(LAUNCHER_HEIGHT, 256);

        // Height for 0 items (notice row)
        assert_eq!(Launcher::calculate_height(0), 74);

        // Height for 1 item
        assert_eq!(Launcher::calculate_height(1), 74);

        // Height for 3 items
        assert_eq!(Launcher::calculate_height(3), 126);
    }

    #[test]
    fn test_plan_md_fuzzy_match_ff_finds_firefox() {
        let cache = vec![
            "firefox".to_string(),
            "alacritty".to_string(),
            "diff".to_string(),
            "foot".to_string(),
        ];
        let res = filter_executables(&cache, "ff", 5);
        assert!(res.contains(&"firefox".to_string()));
        assert_eq!(res[0], "firefox");
    }

    #[test]
    fn test_cursor_movement_and_editing() {
        let mut launcher = Launcher::new();
        update_launcher_state(&mut launcher, LauncherAction::InsertChar('a'));
        update_launcher_state(&mut launcher, LauncherAction::InsertChar('b'));
        update_launcher_state(&mut launcher, LauncherAction::InsertChar('c'));
        assert_eq!(launcher.query, "abc");
        assert_eq!(launcher.cursor_position, 3);

        // Move cursor left: "ab|c"
        update_launcher_state(&mut launcher, LauncherAction::MoveCursorLeft);
        assert_eq!(launcher.cursor_position, 2);

        // Insert 'x' in middle: "abx|c"
        update_launcher_state(&mut launcher, LauncherAction::InsertChar('x'));
        assert_eq!(launcher.query, "abxc");
        assert_eq!(launcher.cursor_position, 3);

        // Move to start: "|abxc"
        update_launcher_state(&mut launcher, LauncherAction::MoveToStart);
        assert_eq!(launcher.cursor_position, 0);

        // Backspace at start does nothing
        update_launcher_state(&mut launcher, LauncherAction::DeleteChar);
        assert_eq!(launcher.query, "abxc");
        assert_eq!(launcher.cursor_position, 0);

        // Move to end: "abxc|"
        update_launcher_state(&mut launcher, LauncherAction::MoveToEnd);
        assert_eq!(launcher.cursor_position, 4);

        // Move cursor right at end stays at end
        update_launcher_state(&mut launcher, LauncherAction::MoveCursorRight);
        assert_eq!(launcher.cursor_position, 4);

        // Delete char at end: deletes 'c' -> "abx|"
        update_launcher_state(&mut launcher, LauncherAction::DeleteChar);
        assert_eq!(launcher.query, "abx");
        assert_eq!(launcher.cursor_position, 3);

        // Move left and delete: "ab|x" -> "a|x"
        update_launcher_state(&mut launcher, LauncherAction::MoveCursorLeft);
        assert_eq!(launcher.cursor_position, 2);
        update_launcher_state(&mut launcher, LauncherAction::DeleteChar);
        assert_eq!(launcher.query, "ax");
        assert_eq!(launcher.cursor_position, 1);
    }

    #[test]
    fn test_delete_word_and_delete_line() {
        let mut launcher = Launcher::new();
        launcher.query = "firefox --private-window".to_string();
        launcher.cursor_position = launcher.query.chars().count();

        // Ctrl+W deletes "--private-window"
        update_launcher_state(&mut launcher, LauncherAction::DeleteWord);
        assert_eq!(launcher.query, "firefox ");
        assert_eq!(launcher.cursor_position, 8);

        // Ctrl+W deletes "firefox "
        update_launcher_state(&mut launcher, LauncherAction::DeleteWord);
        assert_eq!(launcher.query, "");
        assert_eq!(launcher.cursor_position, 0);

        // Ctrl+U deletes line before cursor
        launcher.query = "echo hello world".to_string();
        launcher.cursor_position = 10; // cursor right after "hello" ("echo hello| world")
        update_launcher_state(&mut launcher, LauncherAction::DeleteLine);
        assert_eq!(launcher.query, " world");
        assert_eq!(launcher.cursor_position, 0);
    }

    #[test]
    fn test_tab_autocomplete() {
        let mut launcher = Launcher::new();
        launcher.executable_cache = vec![
            "firefox".to_string(),
            "firewall-cmd".to_string(),
            "foot".to_string(),
        ];

        // Autocomplete without arguments
        launcher.query = "fir".to_string();
        launcher.cursor_position = 3;
        update_launcher_state(&mut launcher, LauncherAction::AutoComplete);
        assert_eq!(launcher.query, "firefox ");
        assert_eq!(launcher.cursor_position, 8);

        // Autocomplete with existing arguments
        launcher.query = "fir --private-window".to_string();
        launcher.cursor_position = launcher.query.chars().count();
        update_launcher_state(&mut launcher, LauncherAction::AutoComplete);
        assert_eq!(launcher.query, "firefox --private-window");
    }

    #[test]
    fn test_selection_wraparound() {
        let mut launcher = Launcher::new();
        launcher.executable_cache = vec![
            "firefox".to_string(),
            "firewall-cmd".to_string(),
            "foot".to_string(),
        ];
        launcher.query = "f".to_string();
        launcher.selected_index = 0;

        // Wrap around previous: 0 -> 2
        update_launcher_state(&mut launcher, LauncherAction::SelectPrevious);
        assert_eq!(launcher.selected_index, 2);

        // Wrap around next: 2 -> 0
        update_launcher_state(&mut launcher, LauncherAction::SelectNext);
        assert_eq!(launcher.selected_index, 0);

        // Next: 0 -> 1
        update_launcher_state(&mut launcher, LauncherAction::SelectNext);
        assert_eq!(launcher.selected_index, 1);
    }

    #[test]
    fn test_handle_launcher_keypress_mappings() {
        let mapping = KeyboardMapping {
            min_keycode: 8,
            max_keycode: 60,
            keysyms_per_keycode: 2,
            keysyms: {
                let mut v = vec![0; (60 - 8 + 1) * 2];
                // keycode 10 = XK_RETURN
                v[(10 - 8) * 2] = XK_RETURN;
                // keycode 11 = XK_ESCAPE
                v[(11 - 8) * 2] = XK_ESCAPE;
                // keycode 12 = XK_BACKSPACE
                v[(12 - 8) * 2] = XK_BACKSPACE;
                // keycode 13 = XK_UP
                v[(13 - 8) * 2] = XK_UP;
                // keycode 14 = XK_DOWN
                v[(14 - 8) * 2] = XK_DOWN;
                // keycode 15 = XK_LEFT
                v[(15 - 8) * 2] = XK_LEFT;
                // keycode 16 = XK_RIGHT
                v[(16 - 8) * 2] = XK_RIGHT;
                // keycode 17 = XK_TAB
                v[(17 - 8) * 2] = XK_TAB;
                // keycode 18 = XK_HOME
                v[(18 - 8) * 2] = XK_HOME;
                // keycode 19 = XK_END
                v[(19 - 8) * 2] = XK_END;
                // keycode 20 = XK_W
                v[(20 - 8) * 2] = XK_W;
                // keycode 21 = XK_U
                v[(21 - 8) * 2] = XK_U;
                // keycode 22 = XK_A
                v[(22 - 8) * 2] = XK_A;
                // keycode 23 = XK_E
                v[(23 - 8) * 2] = XK_E;
                // keycode 24 = XK_P
                v[(24 - 8) * 2] = XK_P;
                // keycode 25 = XK_N
                v[(25 - 8) * 2] = XK_N;
                // keycode 26 = XK_D
                v[(26 - 8) * 2] = XK_D;
                v
            },
        };

        assert_eq!(handle_launcher_keypress(10, 0, &mapping), Some(LauncherAction::Execute));
        assert_eq!(handle_launcher_keypress(11, 0, &mapping), Some(LauncherAction::Cancel));
        assert_eq!(handle_launcher_keypress(12, 0, &mapping), Some(LauncherAction::DeleteChar));
        assert_eq!(handle_launcher_keypress(13, 0, &mapping), Some(LauncherAction::SelectPrevious));
        assert_eq!(handle_launcher_keypress(14, 0, &mapping), Some(LauncherAction::SelectNext));
        assert_eq!(handle_launcher_keypress(15, 0, &mapping), Some(LauncherAction::MoveCursorLeft));
        assert_eq!(handle_launcher_keypress(16, 0, &mapping), Some(LauncherAction::MoveCursorRight));
        assert_eq!(handle_launcher_keypress(17, 0, &mapping), Some(LauncherAction::AutoComplete));
        assert_eq!(handle_launcher_keypress(18, 0, &mapping), Some(LauncherAction::MoveToStart));
        assert_eq!(handle_launcher_keypress(19, 0, &mapping), Some(LauncherAction::MoveToEnd));

        // Ctrl combinations
        assert_eq!(handle_launcher_keypress(20, MOD_CONTROL, &mapping), Some(LauncherAction::DeleteWord));
        assert_eq!(handle_launcher_keypress(21, MOD_CONTROL, &mapping), Some(LauncherAction::DeleteLine));
        assert_eq!(handle_launcher_keypress(22, MOD_CONTROL, &mapping), Some(LauncherAction::MoveToStart));
        assert_eq!(handle_launcher_keypress(23, MOD_CONTROL, &mapping), Some(LauncherAction::MoveToEnd));
        assert_eq!(handle_launcher_keypress(24, MOD_CONTROL, &mapping), Some(LauncherAction::SelectPrevious));
        assert_eq!(handle_launcher_keypress(25, MOD_CONTROL, &mapping), Some(LauncherAction::SelectNext));

        // Mod4 + D cancels launcher
        assert_eq!(handle_launcher_keypress(26, MOD_SUPER, &mapping), Some(LauncherAction::Cancel));
    }
}
