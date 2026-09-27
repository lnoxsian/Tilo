//! Workspace management for i3-style virtual desktops.
//!
//! Each workspace manages a set of client windows and tracks its own
//! focused client and layout. Workspaces 1 through 9 are created by default,
//! and additional workspaces are created on demand.

use std::collections::BTreeMap;
use x11rb::protocol::xproto::Window;

/// Represents a unique identifier for a workspace.
pub type WorkspaceId = u32;

/// Defines the state and layout of a single workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Unique identifier for this workspace (e.g., 1-9).
    pub id: WorkspaceId,
    /// The name of the workspace, typically matching the ID as a string, but configurable.
    pub name: String,
    /// Ordered list of client windows currently managed in this workspace.
    pub clients: Vec<Window>,
    /// The currently focused window in this workspace, if any.
    pub focused_client: Option<Window>,
}

impl Workspace {
    /// Creates a new workspace with the given ID and default name.
    pub fn new(id: WorkspaceId) -> Self {
        Self {
            id,
            name: id.to_string(),
            clients: Vec::new(),
            focused_client: None,
        }
    }

    /// Creates a new workspace with a custom name.
    pub fn with_name(id: WorkspaceId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            clients: Vec::new(),
            focused_client: None,
        }
    }

    /// Adds a client window to this workspace.
    pub fn add_client(&mut self, window: Window) {
        if !self.clients.contains(&window) {
            self.clients.push(window);
        }
    }

    /// Removes a client window from this workspace.
    pub fn remove_client(&mut self, window: Window) -> bool {
        let prev_len = self.clients.len();
        self.clients.retain(|&w| w != window);
        if self.focused_client == Some(window) {
            self.focused_client = self.clients.last().copied();
        }
        self.clients.len() < prev_len
    }

    /// Checks if this workspace contains the given window.
    pub fn contains(&self, window: Window) -> bool {
        self.clients.contains(&window)
    }

    /// Sets the focused window for this workspace.
    pub fn set_focused(&mut self, window: Option<Window>) {
        if let Some(w) = window {
            if self.contains(w) {
                self.focused_client = Some(w);
            }
        } else {
            self.focused_client = None;
        }
    }

    /// Returns the number of client windows on this workspace.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Returns `true` if this workspace contains no windows.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            id: 1,
            name: "1".to_string(),
            clients: Vec::new(),
            focused_client: None,
        }
    }
}

/// Manages multiple workspaces, switching, and window movement between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceManager {
    /// Map of workspace ID to workspace object, ordered by ID.
    pub workspaces: BTreeMap<WorkspaceId, Workspace>,
    /// Currently active workspace ID.
    pub current_workspace_id: WorkspaceId,
}

impl WorkspaceManager {
    /// Creates a new `WorkspaceManager` with default workspaces 1 through 9.
    pub fn new() -> Self {
        let mut workspaces = BTreeMap::new();
        for id in 1..=9 {
            workspaces.insert(id, Workspace::new(id));
        }
        Self {
            workspaces,
            current_workspace_id: 1,
        }
    }

    /// Returns the currently active workspace.
    pub fn current_workspace(&self) -> &Workspace {
        self.workspaces
            .get(&self.current_workspace_id)
            .expect("Current workspace must always exist")
    }

    /// Returns a mutable reference to the currently active workspace.
    pub fn current_workspace_mut(&mut self) -> &mut Workspace {
        self.workspaces
            .get_mut(&self.current_workspace_id)
            .expect("Current workspace must always exist")
    }

    /// Returns the ID of the currently active workspace.
    pub fn current_id(&self) -> WorkspaceId {
        self.current_workspace_id
    }

    /// Checks if a workspace with the given ID exists.
    pub fn workspace_exists(&self, id: WorkspaceId) -> bool {
        self.workspaces.contains_key(&id)
    }

    /// Retrieves a reference to a workspace by ID.
    pub fn get(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.get(&id)
    }

    /// Retrieves a mutable reference to a workspace by ID.
    pub fn get_mut(&mut self, id: WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.get_mut(&id)
    }

    /// Gets an existing workspace or creates a new one with the given ID.
    pub fn get_or_create(&mut self, id: WorkspaceId) -> &mut Workspace {
        self.workspaces.entry(id).or_insert_with(|| Workspace::new(id))
    }

    /// Adds a window to a specific workspace.
    pub fn add_client(&mut self, window: Window, ws_id: WorkspaceId) {
        self.get_or_create(ws_id).add_client(window);
    }

    /// Removes a window from whichever workspace contains it.
    ///
    /// Returns the workspace ID from which the window was removed, if any.
    pub fn remove_client(&mut self, window: Window) -> Option<WorkspaceId> {
        for (id, ws) in &mut self.workspaces {
            if ws.remove_client(window) {
                return Some(*id);
            }
        }
        None
    }

    /// Finds which workspace contains the given window.
    pub fn find_workspace_for(&self, window: Window) -> Option<WorkspaceId> {
        for (id, ws) in &self.workspaces {
            if ws.contains(window) {
                return Some(*id);
            }
        }
        None
    }

    /// Moves a client window from its current workspace to `target_id`.
    ///
    /// Returns `true` if the window was successfully moved.
    pub fn move_client(&mut self, window: Window, target_id: WorkspaceId) -> bool {
        let source_id = self.find_workspace_for(window);
        if let Some(src) = source_id {
            if src == target_id {
                return false;
            }
            if let Some(ws) = self.workspaces.get_mut(&src) {
                ws.remove_client(window);
            }
            self.get_or_create(target_id).add_client(window);
            true
        } else {
            // Window wasn't in any workspace, add it directly to target
            self.get_or_create(target_id).add_client(window);
            true
        }
    }

    /// Returns a list of all existing workspaces ordered by ID.
    pub fn all_workspaces(&self) -> Vec<&Workspace> {
        self.workspaces.values().collect()
    }
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_creation_and_defaults() {
        let ws = Workspace::new(1);
        assert_eq!(ws.id, 1);
        assert_eq!(ws.name, "1");
        assert!(ws.is_empty());
        assert_eq!(ws.focused_client, None);

        let ws_custom = Workspace::with_name(2, "web");
        assert_eq!(ws_custom.id, 2);
        assert_eq!(ws_custom.name, "web");
    }

    #[test]
    fn test_workspace_client_addition_and_removal() {
        let mut ws = Workspace::new(1);
        ws.add_client(100);
        ws.add_client(200);
        ws.set_focused(Some(200));

        assert_eq!(ws.len(), 2);
        assert!(ws.contains(100));
        assert!(ws.contains(200));
        assert_eq!(ws.focused_client, Some(200));

        // Adding duplicate is no-op
        ws.add_client(100);
        assert_eq!(ws.len(), 2);

        // Remove focused client falls back to last remaining client
        assert!(ws.remove_client(200));
        assert_eq!(ws.len(), 1);
        assert!(!ws.contains(200));
        assert_eq!(ws.focused_client, Some(100));

        // Removing non-existent client returns false
        assert!(!ws.remove_client(999));
    }

    #[test]
    fn test_workspace_manager_defaults() {
        let wm = WorkspaceManager::new();
        assert_eq!(wm.current_id(), 1);
        assert_eq!(wm.current_workspace().id, 1);
        assert_eq!(wm.all_workspaces().len(), 9);

        for id in 1..=9 {
            assert!(wm.workspace_exists(id));
        }
        assert!(!wm.workspace_exists(10));
    }

    #[test]
    fn test_workspace_manager_move_and_find() {
        let mut wm = WorkspaceManager::new();
        wm.add_client(101, 1);
        wm.add_client(201, 2);

        assert_eq!(wm.find_workspace_for(101), Some(1));
        assert_eq!(wm.find_workspace_for(201), Some(2));
        assert_eq!(wm.find_workspace_for(301), None);

        // Move 101 from workspace 1 to 2
        assert!(wm.move_client(101, 2));
        assert_eq!(wm.find_workspace_for(101), Some(2));
        assert!(!wm.get(1).unwrap().contains(101));
        assert!(wm.get(2).unwrap().contains(101));

        // Moving to same workspace returns false
        assert!(!wm.move_client(101, 2));

        // Move to uncreated workspace 10 implicitly creates it
        assert!(wm.move_client(101, 10));
        assert!(wm.workspace_exists(10));
        assert_eq!(wm.find_workspace_for(101), Some(10));
    }

    #[test]
    fn test_workspace_manager_remove_client() {
        let mut wm = WorkspaceManager::new();
        wm.add_client(100, 3);
        assert_eq!(wm.remove_client(100), Some(3));
        assert_eq!(wm.find_workspace_for(100), None);
        assert_eq!(wm.remove_client(100), None);
    }
}
