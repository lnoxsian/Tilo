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
    MOD_CONTROL, MOD_SUPER, XK_A, XK_BACKSPACE, XK_D, XK_DOWN, XK_ESCAPE, XK_N,
    XK_P, XK_RETURN, XK_U, XK_UP, XK_W,
};
use crate::layout::Rect;
use crate::render::Renderer;
use crate::statusbar::{ColorScheme, StatusbarError, TiloError};
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, EventMask, Screen, WindowClass,
};

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

        renderer.draw_text(
            conn,
            renderer.pixmap,
            16 + prefix_width as i16,
            input_baseline,
            query_text,
            query_color,
        )?;

        // Draw cursor if query is being typed
        if !self.query.is_empty() {
            let cursor_x = 16 + prefix_width as i16 + renderer.text_width(&self.query) as i16;
            renderer.draw_text(
                conn,
                renderer.pixmap,
                cursor_x,
                input_baseline,
                "_",
                self.color_scheme.active_bg,
            )?;
        }

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
    /// Move cursor to start of line (Ctrl+A)
    MoveToStart,
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
            _ => {}
        }
    }

    match keysym {
        XK_RETURN => Some(LauncherAction::Execute),
        XK_ESCAPE => Some(LauncherAction::Cancel),
        XK_BACKSPACE => Some(LauncherAction::DeleteChar),
        XK_UP => Some(LauncherAction::SelectPrevious),
        XK_DOWN => Some(LauncherAction::SelectNext),
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
            launcher.query.push(c);
            launcher.selected_index = 0;
        }
        LauncherAction::DeleteChar => {
            launcher.query.pop();
            launcher.selected_index = 0;
        }
        LauncherAction::DeleteWord => {
            if let Some(idx) = launcher.query.rfind(' ') {
                launcher.query.truncate(idx);
            } else {
                launcher.query.clear();
            }
            launcher.selected_index = 0;
        }
        LauncherAction::DeleteLine | LauncherAction::MoveToStart => {
            launcher.query.clear();
            launcher.selected_index = 0;
        }
        LauncherAction::SelectNext => {
            let count = launcher.filtered_results(8).len();
            if count > 0 {
                launcher.selected_index = (launcher.selected_index + 1) % count;
            }
        }
        LauncherAction::SelectPrevious => {
            let count = launcher.filtered_results(8).len();
            if count > 0 {
                if launcher.selected_index == 0 {
                    launcher.selected_index = count.saturating_sub(1);
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
}
