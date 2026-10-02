//! Filesystem scanning, `.desktop` file parsing, and process execution for the launcher.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use crate::statusbar::{StatusbarError, TiloError};

/// Scans a single binary directory for executable files.
pub fn scan_single_bin_dir(dir: &std::path::Path) -> (Vec<String>, HashMap<String, String>) {
    let mut items = Vec::new();
    let commands = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let file_path = entry.path();
            if let Ok(metadata) = file_path.metadata()
                && (metadata.is_file() || metadata.file_type().is_symlink())
                && (metadata.permissions().mode() & 0o111 != 0)
                && let Some(file_name) = file_path.file_name().and_then(|n| n.to_str())
            {
                let name_str = file_name.to_string();
                items.push(name_str);
            }
        }
    }
    items.sort();
    (items, commands)
}

/// Scans the specified colon-delimited path string for executable files.
pub fn scan_path_from(path_var: &str) -> Result<Vec<String>, TiloError> {
    let mut executables = HashSet::new();

    for dir in path_var.split(':') {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let (items, _) = scan_single_bin_dir(dir_path);
        for item in items {
            executables.insert(item);
        }
    }

    let mut sorted: Vec<String> = executables.into_iter().collect();
    sorted.sort();
    Ok(sorted)
}

/// Returns the list of standard desktop entry directories to search for `.desktop` files.
///
/// Since the launcher directly searches all executables in `$PATH`,
/// scanning `.desktop` directories (`~/.local/share/applications`,
/// `/usr/local/share/applications`, `/usr/share/applications`, etc.) is disabled.
pub fn get_desktop_directories() -> Vec<PathBuf> {
    Vec::new()
}

/// Returns all directories monitored for dynamic launcher refreshes,
/// which consists of all valid `$PATH` directories.
pub fn get_monitored_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // PATH directories
    let path_var = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    for p in path_var.split(':') {
        if !p.is_empty() {
            dirs.push(PathBuf::from(p));
        }
    }

    // Deduplicate
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn is_field_code(arg: &str) -> bool {
    matches!(
        arg,
        "%f" | "%F"
            | "%u"
            | "%U"
            | "%d"
            | "%D"
            | "%n"
            | "%N"
            | "%i"
            | "%c"
            | "%k"
            | "%v"
            | "%m"
            | "@@"
            | "@@u"
            | "@@f"
            | "@@U"
            | "@@F"
    ) || (arg.starts_with('%')
        && arg.len() == 2
        && arg.chars().nth(1).is_some_and(|c| {
            matches!(
                c,
                'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
            )
        }))
}

/// Strips field codes (e.g. `%u`, `%U`, `%f`, `%F`, `@@`) from an `Exec` line in a `.desktop` file.
pub fn clean_exec_field(raw: &str) -> String {
    let tokens = parse_command_line(raw);
    let mut clean_tokens = Vec::with_capacity(tokens.len());

    for token in tokens {
        if is_field_code(&token) {
            continue;
        }
        if token == "%%" {
            clean_tokens.push("%".to_string());
        } else {
            clean_tokens.push(token);
        }
    }

    let mut result = String::new();
    for token in clean_tokens {
        if !result.is_empty() {
            result.push(' ');
        }
        if token.contains(' ') || token.contains('\t') {
            result.push('"');
            result.push_str(&token.replace('\"', "\\\""));
            result.push('"');
        } else {
            result.push_str(&token);
        }
    }
    result
}

/// Parses the `[Desktop Entry]` section of a `.desktop` file.
///
/// Returns `Some((name, clean_exec))` if the file represents a visible application,
/// or `None` if `NoDisplay=true`, `Hidden=true`, `Type` is not `Application`,
/// or required fields are missing.
pub fn parse_desktop_file(content: &str) -> Option<(String, String)> {
    let mut in_desktop_entry = false;
    let mut name = None;
    let mut exec = None;
    let mut is_application = true;
    let mut no_display = false;
    let mut hidden = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            if line == "[Desktop Entry]" {
                in_desktop_entry = true;
            } else if in_desktop_entry {
                // Done parsing [Desktop Entry]
                break;
            }
            continue;
        }

        if !in_desktop_entry {
            continue;
        }

        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();

            match key {
                "Type" => {
                    if !val.eq_ignore_ascii_case("Application") {
                        is_application = false;
                    }
                }
                "NoDisplay" => {
                    if val.eq_ignore_ascii_case("true") {
                        no_display = true;
                    }
                }
                "Hidden" => {
                    if val.eq_ignore_ascii_case("true") {
                        hidden = true;
                    }
                }
                "Name" => {
                    if name.is_none() {
                        name = Some(val.to_string());
                    }
                }
                "Exec" if exec.is_none() => {
                    exec = Some(val.to_string());
                }
                _ => {}
            }
        }
    }

    if !is_application || no_display || hidden {
        return None;
    }

    let name = name?;
    let exec = exec?;
    let clean_exec = clean_exec_field(&exec);
    if clean_exec.is_empty() {
        return None;
    }

    Some((name, clean_exec))
}

/// Scans a single desktop entry directory for `.desktop` application files.
pub fn scan_single_desktop_dir(dir: &std::path::Path) -> (Vec<String>, HashMap<String, String>) {
    let mut items = Vec::new();
    let mut commands = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("desktop")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Some((name, clean_exec)) = parse_desktop_file(&content)
            {
                commands.insert(name.clone(), clean_exec.clone());
                items.push(name);

                let first_word = clean_exec.split_whitespace().next().unwrap_or("");
                let binary_name = std::path::Path::new(first_word)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !binary_name.is_empty() && !commands.contains_key(binary_name) {
                    commands.insert(binary_name.to_string(), clean_exec);
                }
            }
        }
    }
    items.sort();
    (items, commands)
}

/// Scans desktop entry directories and returns application names and their clean execution commands.
pub fn scan_desktop_dirs(dirs: &[PathBuf]) -> (Vec<String>, HashMap<String, String>) {
    let mut all_names = HashSet::new();
    let mut app_commands = HashMap::new();
    let mut seen_desktop_files = HashSet::new();

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
                    let file_name = match path.file_name().and_then(|s| s.to_str()) {
                        Some(f) => f.to_string(),
                        None => continue,
                    };

                    if !seen_desktop_files.insert(file_name) {
                        continue;
                    }

                    if let Ok(content) = std::fs::read_to_string(&path)
                        && let Some((name, clean_exec)) = parse_desktop_file(&content)
                    {
                        app_commands.insert(name.clone(), clean_exec.clone());
                        all_names.insert(name);

                        let first_word = clean_exec.split_whitespace().next().unwrap_or("");
                        let binary_name = std::path::Path::new(first_word)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");
                        if !binary_name.is_empty() && !app_commands.contains_key(binary_name) {
                            app_commands.insert(binary_name.to_string(), clean_exec);
                        }
                    }
                }
            }
        }
    }

    let mut sorted: Vec<String> = all_names.into_iter().collect();
    sorted.sort();
    (sorted, app_commands)
}

/// Scans `$PATH` executables.
pub fn scan_all_executables(path_var: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut all_names = HashSet::new();
    let app_commands = HashMap::new();

    if let Ok(path_execs) = scan_path_from(path_var) {
        for exe in path_execs {
            all_names.insert(exe);
        }
    }

    let mut sorted: Vec<String> = all_names.into_iter().collect();
    sorted.sort();
    (sorted, app_commands)
}

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
