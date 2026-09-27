//! Window rules configuration and matching engine.
//!
//! This module allows configuration-based window behavior (e.g. automatically assigning
//! specific applications to target workspaces, spawning them in floating mode, or fullscreen).

use crate::atoms::Atoms;
use crate::workspace::WorkspaceId;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};

/// Defines the property to match against for a window rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCondition {
    /// Matches the window's WM_CLASS (class part).
    Class(String),
    /// Matches the window's WM_CLASS (instance part).
    Instance(String),
    /// Matches the window's WM_NAME or _NET_WM_NAME (title).
    Title(String),
}

/// Defines the action to take when a rule matches a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleAction {
    /// If Some, assigns the window to the specified workspace ID instead of the active one.
    pub target_workspace: Option<WorkspaceId>,
    /// If Some(true), spawns the window in a floating state.
    pub floating: Option<bool>,
    /// If Some(true), spawns the window in fullscreen.
    pub fullscreen: Option<bool>,
}

impl RuleAction {
    /// Creates a new default `RuleAction` with all overrides set to None.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the target workspace.
    pub fn with_workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.target_workspace = Some(workspace_id);
        self
    }

    /// Sets the floating override.
    pub fn with_floating(mut self, floating: bool) -> Self {
        self.floating = Some(floating);
        self
    }

    /// Sets the fullscreen override.
    pub fn with_fullscreen(mut self, fullscreen: bool) -> Self {
        self.fullscreen = Some(fullscreen);
        self
    }
}

/// A complete rule mapping a condition to an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRule {
    /// The condition to match against the window's metadata.
    pub condition: RuleCondition,
    /// The action to apply if matched.
    pub action: RuleAction,
}

impl WindowRule {
    /// Creates a new `WindowRule`.
    pub fn new(condition: RuleCondition, action: RuleAction) -> Self {
        Self { condition, action }
    }
}

/// Parses the raw bytes of an X11 `WM_CLASS` property into `(instance, class)`.
///
/// Under ICCCM, `WM_CLASS` consists of two null-terminated strings:
/// `[instance_name]\0[class_name]\0`.
pub fn parse_wm_class(bytes: &[u8]) -> (Option<String>, Option<String>) {
    let parts: Vec<&[u8]> = bytes
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .collect();

    match parts.len() {
        0 => (None, None),
        1 => {
            let s = String::from_utf8_lossy(parts[0]).to_string();
            (Some(s.clone()), Some(s))
        }
        _ => {
            let instance = String::from_utf8_lossy(parts[0]).to_string();
            let class = String::from_utf8_lossy(parts[1]).to_string();
            (Some(instance), Some(class))
        }
    }
}

/// Matches extracted window properties (instance, class, title) against a list of rules.
///
/// If multiple rules match, their actions are combined, with later matching rules
/// taking precedence for overlapping action fields.
pub fn match_rules(
    instance: Option<&str>,
    class: Option<&str>,
    title: Option<&str>,
    rules: &[WindowRule],
) -> RuleAction {
    let mut combined_action = RuleAction::default();

    for rule in rules {
        let is_match = match &rule.condition {
            RuleCondition::Class(expected) => class.map(|c| c == expected).unwrap_or(false),
            RuleCondition::Instance(expected) => instance.map(|i| i == expected).unwrap_or(false),
            RuleCondition::Title(expected) => title.map(|t| t == expected).unwrap_or(false),
        };

        if is_match {
            if let Some(target_ws) = rule.action.target_workspace {
                combined_action.target_workspace = Some(target_ws);
            }
            if let Some(floating) = rule.action.floating {
                combined_action.floating = Some(floating);
            }
            if let Some(fullscreen) = rule.action.fullscreen {
                combined_action.fullscreen = Some(fullscreen);
            }
        }
    }

    combined_action
}

/// Queries X11 properties for a window and extracts `(instance, class, title)`.
pub fn query_window_properties(
    conn: &impl Connection,
    atoms: &Atoms,
    window: Window,
) -> Result<(Option<String>, Option<String>, Option<String>), X11Error> {
    // 1. Query WM_CLASS
    let (instance, class) = if let Ok(reply) = conn.get_property(
        false,
        window,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        0,
        1024,
    ) {
        if let Ok(prop) = reply.reply() {
            parse_wm_class(&prop.value)
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    // 2. Query title (_NET_WM_NAME first, fallback to WM_NAME)
    let title = if let Ok(reply) = conn.get_property(
        false,
        window,
        atoms.net_wm_name,
        atoms.utf8_string,
        0,
        1024,
    ) {
        if let Ok(prop) = reply.reply() {
            if !prop.value.is_empty() {
                Some(String::from_utf8_lossy(&prop.value).to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let title = title.or_else(|| {
        if let Ok(reply) = conn.get_property(
            false,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            0,
            1024,
        ) {
            if let Ok(prop) = reply.reply() {
                if !prop.value.is_empty() {
                    Some(String::from_utf8_lossy(&prop.value).to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    });

    Ok((instance, class, title))
}

/// Evaluates window rules for a newly mapped client and returns the combined `RuleAction`.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `atoms` - Cached atoms for EWMH / ICCCM lookup.
/// * `window` - The newly requested window.
/// * `rules` - A slice of configured `WindowRule` objects.
///
/// # Returns
/// * `Result<RuleAction, X11Error>` - Returns a combined `RuleAction` containing all applied overrides.
pub fn apply_rules(
    conn: &impl Connection,
    atoms: &Atoms,
    window: Window,
    rules: &[WindowRule],
) -> Result<RuleAction, X11Error> {
    if rules.is_empty() {
        return Ok(RuleAction::default());
    }

    let (instance, class, title) = query_window_properties(conn, atoms, window)?;
    Ok(match_rules(
        instance.as_deref(),
        class.as_deref(),
        title.as_deref(),
        rules,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wm_class() {
        let (inst, class) = parse_wm_class(b"alacritty\0Alacritty\0");
        assert_eq!(inst.as_deref(), Some("alacritty"));
        assert_eq!(class.as_deref(), Some("Alacritty"));

        let (inst, class) = parse_wm_class(b"Navigator\0Firefox\0");
        assert_eq!(inst.as_deref(), Some("Navigator"));
        assert_eq!(class.as_deref(), Some("Firefox"));

        let (inst, class) = parse_wm_class(b"SingleName\0");
        assert_eq!(inst.as_deref(), Some("SingleName"));
        assert_eq!(class.as_deref(), Some("SingleName"));

        let (inst, class) = parse_wm_class(b"");
        assert_eq!(inst, None);
        assert_eq!(class, None);
    }

    #[test]
    fn test_match_rules_class_and_instance() {
        let rules = vec![
            WindowRule::new(
                RuleCondition::Class("Firefox".to_string()),
                RuleAction::new().with_workspace(2),
            ),
            WindowRule::new(
                RuleCondition::Class("Pavucontrol".to_string()),
                RuleAction::new().with_floating(true),
            ),
            WindowRule::new(
                RuleCondition::Instance("scratchpad".to_string()),
                RuleAction::new().with_floating(true).with_fullscreen(true),
            ),
        ];

        // Firefox matches workspace 2
        let action1 = match_rules(Some("Navigator"), Some("Firefox"), Some("Mozilla Firefox"), &rules);
        assert_eq!(action1.target_workspace, Some(2));
        assert_eq!(action1.floating, None);
        assert_eq!(action1.fullscreen, None);

        // Pavucontrol matches floating
        let action2 = match_rules(Some("pavucontrol"), Some("Pavucontrol"), Some("Volume Control"), &rules);
        assert_eq!(action2.target_workspace, None);
        assert_eq!(action2.floating, Some(true));

        // Scratchpad instance matches floating and fullscreen
        let action3 = match_rules(Some("scratchpad"), Some("Alacritty"), Some("Terminal"), &rules);
        assert_eq!(action3.floating, Some(true));
        assert_eq!(action3.fullscreen, Some(true));
        assert_eq!(action3.target_workspace, None);

        // Non-matching window gets default action
        let action4 = match_rules(Some("gedit"), Some("Gedit"), Some("Untitled Document"), &rules);
        assert_eq!(action4, RuleAction::default());
    }

    #[test]
    fn test_match_rules_title() {
        let rules = vec![WindowRule::new(
            RuleCondition::Title("Special Window".to_string()),
            RuleAction::new().with_workspace(5),
        )];

        let action = match_rules(Some("app"), Some("App"), Some("Special Window"), &rules);
        assert_eq!(action.target_workspace, Some(5));

        let action_mismatch = match_rules(Some("app"), Some("App"), Some("Other Window"), &rules);
        assert_eq!(action_mismatch, RuleAction::default());
    }

    #[test]
    fn test_match_rules_multiple_combining() {
        let rules = vec![
            WindowRule::new(
                RuleCondition::Class("Gimp".to_string()),
                RuleAction::new().with_floating(true),
            ),
            WindowRule::new(
                RuleCondition::Title("GIMP Startup".to_string()),
                RuleAction::new().with_workspace(3),
            ),
        ];

        let action = match_rules(Some("gimp"), Some("Gimp"), Some("GIMP Startup"), &rules);
        assert_eq!(action.floating, Some(true));
        assert_eq!(action.target_workspace, Some(3));
    }
}
