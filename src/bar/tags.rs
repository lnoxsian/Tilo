//! UI tag and layout status modules.

use std::time::Instant;
use super::modules::{ModuleUpdate, StatusModule};
use super::StatusbarError;

/// Workspace status module displaying active and occupied workspace tags.
#[derive(Debug, Clone)]
pub struct WorkspaceModule {
    pub current_workspace: u32,
    pub occupied_workspaces: Vec<u32>,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl WorkspaceModule {
    pub fn new() -> Self {
        let mut s = Self {
            current_workspace: 1,
            occupied_workspaces: vec![1],
            cached_text: String::new(),
            last_updated: None,
        };
        s.rebuild_text();
        s
    }

    pub fn set_workspace(&mut self, current: u32, occupied: &[u32]) {
        self.current_workspace = current;
        self.occupied_workspaces = occupied.to_vec();
        self.rebuild_text();
    }

    fn rebuild_text(&mut self) {
        let mut parts = Vec::new();
        for ws in 1..=9 {
            if ws == self.current_workspace {
                parts.push(format!("[{}]", ws));
            } else if self.occupied_workspaces.contains(&ws) {
                parts.push(format!("{}:*", ws));
            } else {
                parts.push(format!("{}", ws));
            }
        }
        self.cached_text = parts.join(" ");
    }
}

impl Default for WorkspaceModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for WorkspaceModule {
    fn name(&self) -> &'static str {
        "workspace"
    }

    fn update_interval(&self) -> u64 {
        100
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        Ok(ModuleUpdate::Unchanged)
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Layout status module displaying the active window tiling layout symbol.
#[derive(Debug, Clone)]
pub struct LayoutModule {
    pub layout_name: String,
    pub layout_symbol: String,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl LayoutModule {
    pub fn new() -> Self {
        Self {
            layout_name: "tiling".to_string(),
            layout_symbol: "[]=".to_string(),
            cached_text: "[]=".to_string(),
            last_updated: None,
        }
    }

    pub fn set_layout(&mut self, layout: &str) {
        self.layout_name = layout.to_string();
        self.layout_symbol = match layout {
            "floating" | "><>" => "><>".to_string(),
            "tabbed" | "[T]" => "[T]".to_string(),
            "stacking" | "stacked" | "[S]" => "[S]".to_string(),
            "columns" | "vertical" | "|||" => "|||".to_string(),
            "rows" | "horizontal" | "===" => "===".to_string(),
            "monocle" | "[M]" => "[M]".to_string(),
            "bsp" | "[B]" => "[B]".to_string(),
            "master" | "master-left" | "[]=" => "[]=".to_string(),
            "master-right" | "=[]" => "=[]".to_string(),
            "master-top" | "TT=" => "TT=".to_string(),
            "master-bottom" | "=TT" => "=TT".to_string(),
            "center-master" | "centered-master" | "|M|" | "=M=" => "|M|".to_string(),
            "grid" | "[G]" => "[G]".to_string(),
            _ => {
                if layout.starts_with('[') && layout.ends_with(']') {
                    layout.to_string()
                } else {
                    "[]=".to_string()
                }
            }
        };
        self.cached_text = self.layout_symbol.clone();
    }
}

impl Default for LayoutModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for LayoutModule {
    fn name(&self) -> &'static str {
        "layout"
    }

    fn update_interval(&self) -> u64 {
        200
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        Ok(ModuleUpdate::Unchanged)
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Window Title status module displaying the currently focused client title.
#[derive(Debug, Clone)]
pub struct TitleModule {
    pub title: String,
    pub max_length: usize,
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl TitleModule {
    pub fn new() -> Self {
        Self {
            title: String::new(),
            max_length: 50,
            cached_text: String::new(),
            last_updated: None,
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        if self.title.chars().count() > self.max_length {
            let truncated: String = self.title.chars().take(self.max_length - 3).collect();
            self.cached_text = format!("{}...", truncated);
        } else {
            self.cached_text = self.title.clone();
        }
    }
}

impl Default for TitleModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for TitleModule {
    fn name(&self) -> &'static str {
        "title"
    }

    fn update_interval(&self) -> u64 {
        100
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        Ok(ModuleUpdate::Unchanged)
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
