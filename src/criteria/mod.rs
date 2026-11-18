//! Zero-dependency criteria parsing and window matching engine.

use std::fmt;

/// Shell-style wildcard glob matching (`*` and `?`).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut p_star = None;
    let mut t_backtrack = None;

    while t_idx < t_chars.len() {
        if p_idx < p_chars.len() && (p_chars[p_idx] == '?' || p_chars[p_idx] == t_chars[t_idx]) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < p_chars.len() && p_chars[p_idx] == '*' {
            p_star = Some(p_idx);
            p_idx += 1;
            t_backtrack = Some(t_idx);
        } else if let (Some(star), Some(backtrack)) = (p_star, t_backtrack) {
            p_idx = star + 1;
            t_idx = backtrack + 1;
            t_backtrack = Some(t_idx);
        } else {
            return false;
        }
    }

    while p_idx < p_chars.len() && p_chars[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == p_chars.len()
}

/// Criteria filter to match windows and containers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Criteria {
    /// Window class (WM_CLASS second part) glob.
    pub class: Option<String>,
    /// Window instance (WM_CLASS first part) glob.
    pub instance: Option<String>,
    /// Window title (_NET_WM_NAME or WM_NAME) glob.
    pub title: Option<String>,
    /// Window role (WM_WINDOW_ROLE) glob.
    pub window_role: Option<String>,
    /// Window type (_NET_WM_WINDOW_TYPE) glob.
    pub window_type: Option<String>,
    /// Workspace name glob.
    pub workspace: Option<String>,
    /// Container mark glob.
    pub con_mark: Option<String>,
    /// X11 Window ID.
    pub id: Option<u64>,
    /// Matches floating windows if true.
    pub floating: Option<bool>,
    /// Matches tiling windows if true.
    pub tiling: Option<bool>,
    /// Matches urgent windows if true.
    pub urgent: Option<bool>,
    /// Explicit --none flag that always rejects.
    pub none: bool,
}

impl Criteria {
    /// Checks if a window with given properties satisfies this criteria.
    #[allow(clippy::too_many_arguments)]
    pub fn matches(
        &self,
        class: Option<&str>,
        instance: Option<&str>,
        title: Option<&str>,
        window_role: Option<&str>,
        window_type: Option<&str>,
        workspace: &str,
        marks: &[String],
        is_floating: bool,
        is_urgent: bool,
        window_id: u64,
    ) -> bool {
        if self.none {
            return false;
        }
        if let Some(ref pat) = self.class {
            if !class.map(|c| glob_match(pat, c)).unwrap_or(false) {
                return false;
            }
        }
        if let Some(ref pat) = self.instance {
            if !instance.map(|i| glob_match(pat, i)).unwrap_or(false) {
                return false;
            }
        }
        if let Some(ref pat) = self.title {
            if !title.map(|t| glob_match(pat, t)).unwrap_or(false) {
                return false;
            }
        }
        if let Some(ref pat) = self.window_role {
            if !window_role.map(|r| glob_match(pat, r)).unwrap_or(false) {
                return false;
            }
        }
        if let Some(ref pat) = self.window_type {
            if !window_type.map(|t| glob_match(pat, t)).unwrap_or(false) {
                return false;
            }
        }
        if let Some(ref pat) = self.workspace {
            if !glob_match(pat, workspace) {
                return false;
            }
        }
        if let Some(ref pat) = self.con_mark {
            if !marks.iter().any(|m| glob_match(pat, m)) {
                return false;
            }
        }
        if let Some(id) = self.id {
            if id != window_id {
                return false;
            }
        }
        if let Some(req_floating) = self.floating {
            if req_floating != is_floating {
                return false;
            }
        }
        if let Some(req_tiling) = self.tiling {
            if req_tiling != (!is_floating) {
                return false;
            }
        }
        if let Some(req_urgent) = self.urgent {
            if req_urgent != is_urgent {
                return false;
            }
        }
        true
    }
}

/// Parses criteria brackets and returns `(Option<Criteria>, remaining_command)`.
///
/// Example: `[class="Firefox" title="*YouTube*"] focus` -> `(Some(Criteria), "focus")`.
pub fn parse_criteria_and_command(line: &str) -> Result<(Option<Criteria>, String), String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        return Ok((None, trimmed.to_string()));
    }

    let end_idx = match find_closing_bracket(trimmed) {
        Some(idx) => idx,
        None => return Err("Unclosed criteria bracket '['".to_string()),
    };

    let criteria_body = &trimmed[1..end_idx];
    let remaining_cmd = trimmed[end_idx + 1..].trim().to_string();

    let criteria = parse_criteria_tokens(criteria_body)?;
    Ok((Some(criteria), remaining_cmd))
}

fn find_closing_bracket(s: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut quote_char = '"';
    let mut escaped = false;

    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if in_quotes {
            if c == quote_char {
                in_quotes = false;
            }
        } else if c == '"' || c == '\'' {
            in_quotes = true;
            quote_char = c;
        } else if c == ']' {
            return Some(i);
        }
    }
    None
}

fn parse_criteria_tokens(body: &str) -> Result<Criteria, String> {
    let mut criteria = Criteria::default();
    let mut chars = body.char_indices().peekable();

    while let Some(&(_i, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        // Read token key or flag
        let mut key = String::new();
        while let Some(&(_, ch)) = chars.peek() {
            if ch == '=' || ch.is_whitespace() || ch == ']' {
                break;
            }
            key.push(ch);
            chars.next();
        }

        if key.is_empty() {
            chars.next();
            continue;
        }

        // Check if standalone flag
        match key.as_str() {
            "floating" => {
                criteria.floating = Some(true);
                continue;
            }
            "tiling" => {
                criteria.tiling = Some(true);
                continue;
            }
            "urgent" => {
                criteria.urgent = Some(true);
                continue;
            }
            "--none" => {
                criteria.none = true;
                continue;
            }
            _ => {}
        }

        // Otherwise expect '='
        while let Some(&(_, ch)) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if chars.peek().map(|&(_, ch)| ch) != Some('=') {
            return Err(format!("Expected '=' after key '{}' in criteria", key));
        }
        chars.next(); // consume '='

        while let Some(&(_, ch)) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        // Read value (quoted or unquoted)
        let val = if let Some(&(_, ch)) = chars.peek() {
            if ch == '"' || ch == '\'' {
                let quote = ch;
                chars.next();
                let mut s = String::new();
                let mut esc = false;
                let mut closed = false;
                while let Some(&(_, ch)) = chars.peek() {
                    chars.next();
                    if esc {
                        s.push(ch);
                        esc = false;
                    } else if ch == '\\' {
                        esc = true;
                    } else if ch == quote {
                        closed = true;
                        break;
                    } else {
                        s.push(ch);
                    }
                }
                if !closed {
                    return Err(format!("Unclosed quote for criteria key '{}'", key));
                }
                s
            } else {
                let mut s = String::new();
                while let Some(&(_, ch)) = chars.peek() {
                    if ch.is_whitespace() || ch == ']' {
                        break;
                    }
                    s.push(ch);
                    chars.next();
                }
                s
            }
        } else {
            String::new()
        };

        match key.as_str() {
            "class" => criteria.class = Some(val),
            "instance" => criteria.instance = Some(val),
            "title" => criteria.title = Some(val),
            "window_role" => criteria.window_role = Some(val),
            "window_type" => criteria.window_type = Some(val),
            "workspace" => criteria.workspace = Some(val),
            "con_mark" => criteria.con_mark = Some(val),
            "id" | "con_id" => {
                let parsed_id = if val.starts_with("0x") || val.starts_with("0X") {
                    u64::from_str_radix(&val[2..], 16)
                } else {
                    val.parse::<u64>()
                }
                .map_err(|e| format!("Invalid id '{}': {}", val, e))?;
                criteria.id = Some(parsed_id);
            }
            unknown => return Err(format!("Unknown criteria key '{}'", unknown)),
        }
    }

    Ok(criteria)
}

impl fmt::Display for Criteria {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        let mut first = true;
        let mut write_field = |key: &str, val: &str| -> fmt::Result {
            if !first {
                write!(f, " ")?;
            }
            first = false;
            write!(f, "{}=\"{}\"", key, val)
        };
        if let Some(ref c) = self.class {
            write_field("class", c)?;
        }
        if let Some(ref i) = self.instance {
            write_field("instance", i)?;
        }
        if let Some(ref t) = self.title {
            write_field("title", t)?;
        }
        if let Some(ref r) = self.window_role {
            write_field("window_role", r)?;
        }
        if let Some(ref w) = self.workspace {
            write_field("workspace", w)?;
        }
        if let Some(ref m) = self.con_mark {
            write_field("con_mark", m)?;
        }
        if let Some(id) = self.id {
            if !first {
                write!(f, " ")?;
            }
            first = false;
            write!(f, "id={}", id)?;
        }
        if self.floating == Some(true) {
            if !first {
                write!(f, " ")?;
            }
            write!(f, "floating")?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_matching() {
        assert!(glob_match("Firefox", "Firefox"));
        assert!(glob_match("*Firefox*", "Mozilla Firefox Developer"));
        assert!(glob_match("term*", "terminal"));
        assert!(glob_match("?erm", "term"));
        assert!(!glob_match("term", "terminal"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "nonempty"));
    }

    #[test]
    fn test_parse_criteria_and_command() {
        let (crit, cmd) = parse_criteria_and_command(r#"[class="Firefox" title="*YouTube*"] floating enable"#)
            .expect("parse error");
        let crit = crit.expect("must have criteria");
        assert_eq!(crit.class, Some("Firefox".to_string()));
        assert_eq!(crit.title, Some("*YouTube*".to_string()));
        assert_eq!(cmd, "floating enable");

        let (no_crit, plain_cmd) = parse_criteria_and_command("workspace 2").expect("parse error");
        assert!(no_crit.is_none());
        assert_eq!(plain_cmd, "workspace 2");
    }

    #[test]
    fn test_criteria_matches() {
        let crit = Criteria {
            class: Some("Firefox".to_string()),
            floating: Some(false),
            ..Default::default()
        };
        assert!(crit.matches(
            Some("Firefox"),
            Some("Navigator"),
            Some("Mozilla Firefox"),
            None,
            None,
            "1",
            &[],
            false,
            false,
            123,
        ));
        assert!(!crit.matches(
            Some("Firefox"),
            Some("Navigator"),
            Some("Mozilla Firefox"),
            None,
            None,
            "1",
            &[],
            true, // floating -> fails
            false,
            123,
        ));
    }
}
