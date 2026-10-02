//! Integrated application launcher.
//!
//! Implements Phase 19, 20, and 21 of Plan.md:
//! - `$PATH` scanning and executable caching ([`scan_path`]).
//! - Direct, detached process execution supporting CLI arguments ([`spawn_app`]).
//! - Fast fuzzy matching scoring algorithm ([`fuzzy_match`]).
//! - Double-buffered overlay window with search input and filtered result list.

pub mod fuzzy;
pub mod scanner;

pub use fuzzy::*;
pub use scanner::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::input::KeyboardMapping;
use crate::keybind::{
    MOD_CONTROL, MOD_SHIFT, MOD_SUPER, XK_A, XK_B, XK_BACKSPACE, XK_C, XK_D, XK_DOWN, XK_E,
    XK_END, XK_ESCAPE, XK_F, XK_F5, XK_HOME, XK_LEFT, XK_N, XK_P, XK_Q, XK_R, XK_RETURN, XK_RIGHT,
    XK_TAB, XK_U, XK_UP, XK_W,
};
use crate::layout::Rect;
use crate::render::Renderer;
use crate::statusbar::{ColorScheme, StatusbarError};
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

/// Cached scan results for a single directory (either binary PATH or desktop application directory).
#[derive(Debug, Clone, Default)]
pub struct DirCacheEntry {
    /// Modification timestamp recorded for this directory.
    pub mtime: Option<SystemTime>,
    /// Discovered item names in this directory.
    pub items: Vec<String>,
    /// Discovered application command mappings in this directory.
    pub commands: HashMap<String, String>,
}

/// Operating mode for the launcher overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherMode {
    /// Normal application launcher searching $PATH binaries and desktop entries.
    #[default]
    App,
    /// Power / Exit menu offering Logout, Reboot, and Poweroff options.
    Exit,
}

/// Core configuration and state for the application launcher.
#[derive(Debug)]
pub struct Launcher {
    /// X11 window ID for the launcher overlay (0 if unallocated).
    pub window_id: u32,
    /// Cached list of executable and application names.
    pub executable_cache: Vec<String>,
    /// Current search query typed by the user.
    pub query: String,
    /// Cursor character index within the search query.
    pub cursor_position: usize,
    /// Currently selected index in the filtered results.
    pub selected_index: usize,
    /// Whether the launcher is currently active and mapped on screen.
    pub visible: bool,
    /// Operating mode of the launcher (App search or Exit menu).
    pub mode: LauncherMode,
    /// The geometric bounds of the launcher window.
    pub rect: Rect,
    /// Double-buffered graphics renderer.
    pub renderer: Option<Renderer>,
    /// Visual color scheme.
    pub color_scheme: ColorScheme,
    /// Optional window border color override.
    pub border_color: Option<u32>,
    /// Mapping of application display names (or desktop IDs) to executable command lines.
    pub app_commands: HashMap<String, String>,
    /// Monitored directories and their last-seen modification times.
    pub dir_mtimes: HashMap<PathBuf, Option<SystemTime>>,
    /// Monitored directory paths kept in order to avoid re-allocations on check.
    pub monitored_dirs: Vec<PathBuf>,
    /// Last value of the PATH environment variable.
    pub last_path_var: String,
    /// Per-directory incremental cache.
    pub dir_cache: HashMap<PathBuf, DirCacheEntry>,
    /// Timestamp of last filesystem check for rapid keystroke throttling.
    pub last_check_instant: Option<std::time::Instant>,
}

impl Launcher {
    /// Creates a new `Launcher` instance without pre-scanning the filesystem.
    ///
    /// Filesystem scanning is deferred until the launcher is opened or refreshed,
    /// eliminating startup latency and idle heap memory usage.
    pub fn new() -> Self {
        Self {
            window_id: 0,
            executable_cache: Vec::new(),
            query: String::new(),
            cursor_position: 0,
            selected_index: 0,
            visible: false,
            mode: LauncherMode::App,
            rect: Rect::new(0, 0, LAUNCHER_WIDTH as u32, LAUNCHER_HEIGHT as u32),
            renderer: None,
            color_scheme: ColorScheme::default(),
            border_color: None,
            app_commands: HashMap::new(),
            dir_mtimes: HashMap::new(),
            monitored_dirs: Vec::new(),
            last_path_var: String::new(),
            dir_cache: HashMap::new(),
            last_check_instant: None,
        }
    }

    /// Returns the executable command line for a given item name.
    ///
    /// If the item is an application display name, returns its associated clean command.
    /// Otherwise returns the item name itself as fallback.
    pub fn command_for(&self, name: &str) -> String {
        if self.mode == LauncherMode::Exit {
            return name.to_string();
        }
        self.app_commands
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Refreshes the executable and application cache using the specified PATH string and desktop directories.
    ///
    /// Optimizes rescan by only reading directories whose `mtime` has changed,
    /// reusing cached results for unmodified directories.
    pub fn refresh_from(&mut self, path_var: &str, desktop_dirs: &[PathBuf]) -> bool {
        self.last_path_var = path_var.to_string();

        let mut monitored = Vec::new();
        let mut bin_dirs_set = HashSet::new();
        for p in path_var.split(':') {
            if !p.is_empty() {
                let pb = PathBuf::from(p);
                bin_dirs_set.insert(pb.clone());
                monitored.push(pb);
            }
        }
        for d in desktop_dirs {
            monitored.push(d.clone());
        }
        let mut seen = HashSet::new();
        monitored.retain(|d| seen.insert(d.clone()));
        self.monitored_dirs = monitored;

        let mut any_changed = false;
        let mut new_mtimes = HashMap::with_capacity(self.monitored_dirs.len());

        for dir in &self.monitored_dirs {
            let current_mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
            new_mtimes.insert(dir.clone(), current_mtime);

            let cached_mtime = self.dir_cache.get(dir).and_then(|e| e.mtime);
            if cached_mtime != current_mtime || !self.dir_cache.contains_key(dir) {
                // Only scan this specific directory because it changed
                any_changed = true;
                let (items, commands) = if bin_dirs_set.contains(dir) {
                    scan_single_bin_dir(dir)
                } else {
                    scan_single_desktop_dir(dir)
                };
                self.dir_cache.insert(
                    dir.clone(),
                    DirCacheEntry {
                        mtime: current_mtime,
                        items,
                        commands,
                    },
                );
            }
        }

        self.dir_mtimes = new_mtimes;

        // Clean up any removed directories from cache
        self.dir_cache.retain(|k, _| self.monitored_dirs.contains(k));

        // Re-merge all cached entries
        let mut all_names = HashSet::new();
        let mut app_commands = HashMap::new();

        for dir in &self.monitored_dirs {
            if let Some(entry) = self.dir_cache.get(dir) {
                for item in &entry.items {
                    all_names.insert(item.clone());
                }
                for (k, v) in &entry.commands {
                    if !app_commands.contains_key(k) {
                        app_commands.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        let mut sorted: Vec<String> = all_names.into_iter().collect();
        sorted.sort();

        self.executable_cache = sorted;
        self.app_commands = app_commands;

        let count = self.filtered_results(LAUNCHER_MAX_RESULTS).len();
        if self.selected_index >= count {
            self.selected_index = count.saturating_sub(1);
        }

        any_changed
    }

    /// Checks if any monitored directory or PATH string has changed compared to the last scan.
    ///
    /// Avoids heap allocations when no changes are detected.
    pub fn refresh_if_needed_from(&mut self, path_var: &str, desktop_dirs: &[PathBuf]) -> bool {
        if path_var != self.last_path_var || self.monitored_dirs.is_empty() {
            return self.refresh_from(path_var, desktop_dirs);
        }

        let mut changed = false;
        for dir in &self.monitored_dirs {
            let current_mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
            if self.dir_mtimes.get(dir) != Some(&current_mtime) {
                changed = true;
                break;
            }
        }

        if changed {
            self.refresh_from(path_var, desktop_dirs)
        } else {
            false
        }
    }

    /// Refreshes the executable cache immediately using system `$PATH`.
    pub fn refresh(&mut self) -> bool {
        let path_var = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
        self.refresh_from(&path_var, &[])
    }

    /// Checks if any system directory in `$PATH` has changed.
    pub fn refresh_if_needed(&mut self) -> bool {
        let current_path = std::env::var("PATH").unwrap_or_default();
        self.refresh_if_needed_from(&current_path, &[])
    }

    /// Checks for directory modifications with rate-limiting to prevent excessive
    /// filesystem stat calls during rapid keystrokes.
    pub fn refresh_if_needed_throttled(&mut self, min_interval: std::time::Duration) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_check_instant
            && now.duration_since(last) < min_interval {
                return false;
            }
        self.last_check_instant = Some(now);
        self.refresh_if_needed()
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

    /// Filters the power/exit menu options based on user query.
    pub fn filter_exit_options(query: &str) -> Vec<String> {
        let options = ["Logout", "Reboot", "Poweroff"];
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return options.iter().map(|s| s.to_string()).collect();
        }

        let mut scored: Vec<(i32, String)> = Vec::new();
        for opt in &options {
            let opt_lower = opt.to_lowercase();
            let q_lower = trimmed.to_lowercase();
            if opt_lower == q_lower {
                scored.push((1000, opt.to_string()));
            } else if opt_lower.starts_with(&q_lower) {
                scored.push((800, opt.to_string()));
            } else if opt_lower.contains(&q_lower) {
                scored.push((500, opt.to_string()));
            } else if (q_lower == "shutdown" && *opt == "Poweroff")
                || (q_lower == "quit" && *opt == "Logout")
                || (q_lower == "exit" && *opt == "Logout")
                || (q_lower == "restart" && *opt == "Reboot")
            {
                scored.push((900, opt.to_string()));
            } else if let Some(score) = fuzzy_match(opt, trimmed) {
                scored.push((score, opt.to_string()));
            }
        }

        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().map(|(_, name)| name).collect()
    }

    /// Returns the top filtered executable results matching the current query.
    pub fn filtered_results(&self, max_results: usize) -> Vec<String> {
        if self.mode == LauncherMode::Exit {
            Self::filter_exit_options(&self.query)
        } else {
            filter_executables(&self.executable_cache, &self.query, max_results)
        }
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

        // Launcher maintains a fixed height and position so it doesn't move or resize
        // when search results are 1 or fewer than 8.
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
        let border_color = self.border_color.unwrap_or(self.color_scheme.border);
        // Top, bottom, left, right border outlines
        renderer.fill_rect(conn, renderer.pixmap, 0, 0, w, 2, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, 0, (h - 2) as i16, w, 2, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, 0, 0, 2, h, border_color)?;
        renderer.fill_rect(conn, renderer.pixmap, (w - 2) as i16, 0, 2, h, border_color)?;

        // 3. Draw input prompt box
        let input_baseline = 24i16;
        let prompt_prefix = if self.mode == LauncherMode::Exit {
            "Exit > "
        } else {
            "> "
        };
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
            if self.mode == LauncherMode::Exit {
                "Select action: Logout, Reboot, Poweroff..."
            } else {
                "Type application name..."
            }
        } else {
            &self.query
        };
        let query_color = if self.query.is_empty() {
            self.color_scheme.inactive_fg
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
        let byte_offset = self.query.char_indices().nth(self.cursor_position).map(|(i, _)| i).unwrap_or(self.query.len());
        let cursor_offset = renderer.text_width(&self.query[..byte_offset]);
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
        renderer.fill_rect(conn, renderer.pixmap, 8, 36, w.saturating_sub(16), 1, self.color_scheme.border)?;

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

        // If no results and query is non-empty, indicate notice / direct command execution mode
        if results.is_empty() && !self.query.is_empty() {
            let notice = if self.mode == LauncherMode::Exit {
                format!("Unknown action: '{}' (options: Logout, Reboot, Poweroff)", self.query)
            } else {
                format!("Run: '{}'", self.query)
            };
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
        if let Some(renderer) = self.renderer.take() {
            let _ = renderer.destroy(conn);
        }
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
    /// Force refresh the executable and application cache (Ctrl+R, F5)
    Refresh,
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
    let shift = (modifiers & MOD_SHIFT) != 0;

    let is_mod = (modifiers & MOD_SUPER) != 0;
    if is_mod {
        if keysym == XK_D || keysym == XK_ESCAPE || (shift && keysym == XK_Q) {
            return Some(LauncherAction::Cancel);
        }
        return None;
    }

    if ctrl && shift && keysym == XK_Q {
        return Some(LauncherAction::Cancel);
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
            XK_R => return Some(LauncherAction::Refresh),
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
        XK_F5 => Some(LauncherAction::Refresh),
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
            let byte_offset = launcher.query.char_indices().nth(launcher.cursor_position).map(|(i, _)| i).unwrap_or(launcher.query.len());
            launcher.query.insert(byte_offset, c);
            launcher.cursor_position += 1;
            launcher.selected_index = 0;
        }
        LauncherAction::DeleteChar => {
            if launcher.cursor_position > 0 {
                let char_idx = launcher.cursor_position - 1;
                if let Some((byte_offset, _)) = launcher.query.char_indices().nth(char_idx) {
                    launcher.query.remove(byte_offset);
                    launcher.cursor_position = char_idx;
                    launcher.selected_index = 0;
                }
            }
        }
        LauncherAction::DeleteWord => {
            if launcher.cursor_position > 0 {
                let total_chars = launcher.query.chars().count();
                let pos = launcher.cursor_position.min(total_chars);
                let chars: Vec<char> = launcher.query.chars().collect();
                let mut i = pos;
                // Skip trailing whitespace before cursor
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                // Skip word characters
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                let start_byte = launcher.query.char_indices().nth(i).map(|(b, _)| b).unwrap_or(0);
                let end_byte = launcher.query.char_indices().nth(pos).map(|(b, _)| b).unwrap_or(launcher.query.len());
                launcher.query.drain(start_byte..end_byte);
                launcher.cursor_position = i;
                launcher.selected_index = 0;
            }
        }
        LauncherAction::DeleteLine => {
            let total_chars = launcher.query.chars().count();
            let pos = launcher.cursor_position.min(total_chars);
            let end_byte = launcher.query.char_indices().nth(pos).map(|(b, _)| b).unwrap_or(launcher.query.len());
            launcher.query.drain(0..end_byte);
            launcher.cursor_position = 0;
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
        LauncherAction::Refresh => {
            launcher.refresh();
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
    use std::os::unix::fs::PermissionsExt;

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
    fn test_launcher_fixed_height_on_few_results() {
        let mut launcher = Launcher::new();
        assert_eq!(launcher.rect.height, LAUNCHER_HEIGHT as u32);

        // When 1 result is present, rect height must remain fixed at LAUNCHER_HEIGHT (256)
        launcher.executable_cache = vec!["single_app".to_string()];
        launcher.query = "single".to_string();
        let res = launcher.filtered_results(LAUNCHER_MAX_RESULTS);
        assert_eq!(res.len(), 1);
        assert_eq!(launcher.rect.height, 256);

        // When 0 results match, rect height must still remain fixed at 256
        launcher.query = "non_existent_query_xyz".to_string();
        let res_none = launcher.filtered_results(LAUNCHER_MAX_RESULTS);
        assert!(res_none.is_empty());
        assert_eq!(launcher.rect.height, 256);
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
                // keycode 27 = XK_R
                v[(27 - 8) * 2] = XK_R;
                // keycode 28 = XK_F5
                v[(28 - 8) * 2] = XK_F5;
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
        assert_eq!(handle_launcher_keypress(28, 0, &mapping), Some(LauncherAction::Refresh));

        // Ctrl combinations
        assert_eq!(handle_launcher_keypress(20, MOD_CONTROL, &mapping), Some(LauncherAction::DeleteWord));
        assert_eq!(handle_launcher_keypress(21, MOD_CONTROL, &mapping), Some(LauncherAction::DeleteLine));
        assert_eq!(handle_launcher_keypress(22, MOD_CONTROL, &mapping), Some(LauncherAction::MoveToStart));
        assert_eq!(handle_launcher_keypress(23, MOD_CONTROL, &mapping), Some(LauncherAction::MoveToEnd));
        assert_eq!(handle_launcher_keypress(24, MOD_CONTROL, &mapping), Some(LauncherAction::SelectPrevious));
        assert_eq!(handle_launcher_keypress(25, MOD_CONTROL, &mapping), Some(LauncherAction::SelectNext));
        assert_eq!(handle_launcher_keypress(27, MOD_CONTROL, &mapping), Some(LauncherAction::Refresh));

        // Mod4 + D cancels launcher
        assert_eq!(handle_launcher_keypress(26, MOD_SUPER, &mapping), Some(LauncherAction::Cancel));
    }

    #[test]
    fn test_clean_exec_field() {
        assert_eq!(clean_exec_field("firefox %u"), "firefox");
        assert_eq!(clean_exec_field("vlc --started-from-file %U"), "vlc --started-from-file");
        assert_eq!(clean_exec_field("/usr/bin/google-chrome-stable %U"), "/usr/bin/google-chrome-stable");
        assert_eq!(
            clean_exec_field("/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=steam com.valvesoftware.Steam @@u %U @@"),
            "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=steam com.valvesoftware.Steam"
        );
        assert_eq!(clean_exec_field("code --unity-launch %F"), "code --unity-launch");
    }

    #[test]
    fn test_parse_desktop_file() {
        let content_valid = r#"
[Desktop Entry]
Version=1.0
Type=Application
Name=Visual Studio Code
Exec=code --new-window %F
Icon=vscode
"#;
        let parsed = parse_desktop_file(content_valid);
        assert_eq!(
            parsed,
            Some(("Visual Studio Code".to_string(), "code --new-window".to_string()))
        );

        let content_no_display = r#"
[Desktop Entry]
Type=Application
Name=Hidden Tool
Exec=hidden-tool
NoDisplay=true
"#;
        assert_eq!(parse_desktop_file(content_no_display), None);

        let content_hidden = r#"
[Desktop Entry]
Type=Application
Name=Hidden Tool 2
Exec=hidden-tool-2
Hidden=true
"#;
        assert_eq!(parse_desktop_file(content_hidden), None);

        let content_link = r#"
[Desktop Entry]
Type=Link
Name=URL Link
Exec=link
"#;
        assert_eq!(parse_desktop_file(content_link), None);
    }

    #[test]
    fn test_dynamic_refresh_on_new_binary() {
        let temp_dir = std::env::temp_dir().join(format!("tilo_launcher_dyn_refresh_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        let path_var = temp_dir.to_str().unwrap();

        // 1. Initial state: create bin1
        let bin1 = temp_dir.join("bin1");
        std::fs::write(&bin1, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&bin1).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin1, perms).unwrap();

        let mut launcher = Launcher::new();
        launcher.refresh_from(path_var, &[]);
        assert!(launcher.executable_cache.contains(&"bin1".to_string()));
        assert!(!launcher.executable_cache.contains(&"bin2_new".to_string()));

        // Check if needed returns false when nothing changed
        assert!(!launcher.refresh_if_needed_from(path_var, &[]));

        // 2. Wait briefly to guarantee mtime change, then install/create bin2_new
        std::thread::sleep(std::time::Duration::from_millis(50));
        let bin2 = temp_dir.join("bin2_new");
        std::fs::write(&bin2, "#!/bin/sh\n").unwrap();
        let mut perms2 = std::fs::metadata(&bin2).unwrap().permissions();
        perms2.set_mode(0o755);
        std::fs::set_permissions(&bin2, perms2).unwrap();

        // 3. refresh_if_needed_from should detect the directory change and refresh!
        assert!(launcher.refresh_if_needed_from(path_var, &[]));
        assert!(launcher.executable_cache.contains(&"bin2_new".to_string()));

        // Searching for newly installed binary works immediately
        let results = filter_executables(&launcher.executable_cache, "bin2", 5);
        assert!(results.contains(&"bin2_new".to_string()));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_desktop_directories_are_not_searched_or_monitored() {
        assert!(get_desktop_directories().is_empty());

        let monitored = get_monitored_directories();
        // Monitored directories must not contain standard .desktop application paths
        for dir in &monitored {
            let s = dir.to_string_lossy();
            assert!(!s.ends_with("share/applications"), "Monitored dirs should not contain desktop application dirs: {}", s);
        }
    }
}
