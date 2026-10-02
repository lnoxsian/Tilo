//! Client state and tracking types.
//!
//! This module manages application windows tracked by the window manager,
//! maintaining the window lookup table (`Window -> Client`) and the focus stack.

pub mod decoration;
pub mod floating;
pub mod fullscreen;
pub mod rules;
pub mod scratchpad;

pub use decoration::*;
pub use floating::*;
pub use fullscreen::*;
pub use rules::*;
pub use scratchpad::*;

use std::collections::HashMap;
use x11rb::protocol::xproto::Window;


/// Represents a single tracked application window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    /// The X11 Window ID.
    pub window: Window,
    /// The ID of the workspace this client is currently on.
    pub workspace: u32,
    /// The ID of the monitor this client is currently displayed on.
    pub monitor: u32,
    /// True if the window is floating (exempt from tiling).
    pub floating: bool,
    /// True if the window is in fullscreen mode.
    pub fullscreen: bool,
    /// Fullscreen state tracking previous geometry and borders.
    pub fullscreen_state: FullscreenState,
    /// True if the window is currently mapped (visible) to the screen.
    pub mapped: bool,
    /// True if the window currently has input focus.
    pub focused: bool,
    /// Detailed layout state (Tiled vs Floating with geometry).
    pub layout_state: ClientLayoutState,
    /// The last saved floating geometry for restoring when un-tiled.
    pub floating_geometry: Option<FloatingGeometry>,
    /// Currently applied border color (RGB/hex).
    pub border_color: Option<u32>,
    /// True if the window has an active urgency hint.
    pub urgent: bool,
    /// Cached window title to avoid repeated X11 property queries.
    pub title: Option<String>,
}

impl Client {
    /// Creates a new `Client` for the specified window.
    pub fn new(window: Window) -> Self {
        Self {
            window,
            workspace: 1,
            monitor: 0,
            floating: false,
            fullscreen: false,
            fullscreen_state: FullscreenState::None,
            mapped: false,
            focused: false,
            layout_state: ClientLayoutState::Tiled,
            floating_geometry: None,
            border_color: None,
            urgent: false,
            title: None,
        }
    }

    /// Returns true if this client is in a floating state.
    pub fn is_floating(&self) -> bool {
        self.floating || matches!(self.layout_state, ClientLayoutState::Floating(_))
    }

    /// Returns true if this client is in fullscreen mode.
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen || self.fullscreen_state.is_fullscreen()
    }

    /// Sets the client to fullscreen mode, saving its previous geometry and border width.
    pub fn set_fullscreen(&mut self, prev_geom: Geometry, prev_border_width: u32) {
        self.fullscreen = true;
        self.fullscreen_state = FullscreenState::Fullscreen {
            previous_geometry: prev_geom,
            previous_border_width: prev_border_width,
        };
    }

    /// Unsets fullscreen mode and returns the saved previous geometry and border width, if any.
    pub fn unset_fullscreen(&mut self) -> Option<(Geometry, u32)> {
        self.fullscreen = false;
        match std::mem::take(&mut self.fullscreen_state) {
            FullscreenState::Fullscreen {
                previous_geometry,
                previous_border_width,
            } => Some((previous_geometry, previous_border_width)),
            FullscreenState::None => None,
        }
    }

    /// Sets the client to floating with the given geometry.
    pub fn set_floating(&mut self, geom: FloatingGeometry) {
        self.floating = true;
        self.floating_geometry = Some(geom);
        self.layout_state = ClientLayoutState::Floating(geom);
    }

    /// Sets the client to tiled mode.
    pub fn set_tiled(&mut self) {
        self.floating = false;
        self.layout_state = ClientLayoutState::Tiled;
    }
}

/// Central registry for tracking and interacting with all managed clients.
#[derive(Debug, Default)]
pub struct ClientState {
    /// A map binding X11 Window IDs to `Client` data structures.
    pub clients: HashMap<Window, Client>,
    /// An ordered list of Window IDs for focus history/stacking.
    pub focus_stack: Vec<Window>,
    /// Insertion order of clients (useful for master/stack tiling).
    pub client_order: Vec<Window>,
}

impl ClientState {
    /// Creates a new, empty client state registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new window to be managed by the window manager.
    ///
    /// # Arguments
    /// * `window` - The X11 window ID.
    /// * `client` - The initial state of the client.
    pub fn add_client(&mut self, window: Window, client: Client) {
        self.clients.insert(window, client);
        if !self.focus_stack.contains(&window) {
            self.focus_stack.push(window);
        }
        if !self.client_order.contains(&window) {
            self.client_order.push(window);
        }
    }

    /// Deregisters and removes a window from the window manager's state.
    ///
    /// # Arguments
    /// * `window` - The X11 window ID to remove.
    ///
    /// # Returns
    /// The removed `Client` instance, or `None` if it was not tracked.
    pub fn remove_client(&mut self, window: Window) -> Option<Client> {
        self.focus_stack.retain(|&w| w != window);
        self.client_order.retain(|&w| w != window);
        self.clients.remove(&window)
    }

    /// Returns the window IDs of all managed clients across all workspaces.
    pub fn all_clients(&self) -> &[Window] {
        &self.client_order
    }

    /// Returns the window IDs of all mapped, non-floating, non-fullscreen clients on a given workspace.
    pub fn visible_tiled_clients_on_workspace(&self, workspace: u32) -> Vec<Window> {
        self.client_order
            .iter()
            .copied()
            .filter(|&w| {
                self.clients
                    .get(&w)
                    .is_some_and(|c| {
                        c.workspace == workspace && c.mapped && !c.is_floating() && !c.is_fullscreen()
                    })
            })
            .collect()
    }

    /// Returns the window IDs of all mapped floating (non-fullscreen) clients on a given workspace.
    pub fn floating_clients_on_workspace(&self, workspace: u32) -> Vec<Window> {
        self.client_order
            .iter()
            .copied()
            .filter(|&w| {
                self.clients
                    .get(&w)
                    .is_some_and(|c| c.workspace == workspace && c.mapped && c.is_floating() && !c.is_fullscreen())
            })
            .collect()
    }

    /// Returns the window IDs of all mapped fullscreen clients on a given workspace.
    pub fn fullscreen_clients_on_workspace(&self, workspace: u32) -> Vec<Window> {
        self.client_order
            .iter()
            .copied()
            .filter(|&w| {
                self.clients
                    .get(&w)
                    .is_some_and(|c| c.workspace == workspace && c.mapped && c.is_fullscreen())
            })
            .collect()
    }

    /// Retrieves a reference to a tracked client.
    pub fn get_client(&self, window: Window) -> Option<&Client> {
        self.clients.get(&window)
    }

    /// Retrieves a mutable reference to a tracked client.
    pub fn get_client_mut(&mut self, window: Window) -> Option<&mut Client> {
        self.clients.get_mut(&window)
    }

    /// Checks if a window is currently tracked.
    pub fn contains(&self, window: Window) -> bool {
        self.clients.contains_key(&window)
    }

    /// Updates focus to the given window, marking it as focused in `Client`
    /// and moving it to the top of the focus stack.
    pub fn focus(&mut self, window: Window) {
        for client in self.clients.values_mut() {
            if client.window == window {
                client.focused = true;
            } else if client.focused {
                client.focused = false;
            }
        }
        if let Some(pos) = self.focus_stack.iter().position(|&w| w == window) {
            self.focus_stack.remove(pos);
        }
        self.focus_stack.push(window);
    }

    /// Clears focus from all managed clients.
    pub fn unfocus_all(&mut self) {
        if let Some(&prev) = self.focus_stack.last()
            && let Some(client) = self.clients.get_mut(&prev) {
                client.focused = false;
        }
        self.focus_stack.clear();
    }

    /// Returns the window ID at the top of the focus stack, if any.
    pub fn focused_window(&self) -> Option<Window> {
        self.focus_stack.last().copied()
    }

    /// Returns the number of managed clients.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Returns true if no clients are currently tracked.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new_defaults() {
        let client = Client::new(0x123);
        assert_eq!(client.window, 0x123);
        assert_eq!(client.workspace, 1);
        assert_eq!(client.monitor, 0);
        assert!(!client.floating);
        assert!(!client.fullscreen);
        assert!(!client.mapped);
        assert!(!client.focused);
    }

    #[test]
    fn test_client_state_add_and_get() {
        let mut state = ClientState::new();
        assert!(state.is_empty());
        assert_eq!(state.len(), 0);

        let client = Client::new(0x100);
        state.add_client(0x100, client);

        assert_eq!(state.len(), 1);
        assert!(state.contains(0x100));
        assert_eq!(state.focused_window(), Some(0x100));

        let retrieved = state.get_client(0x100).expect("Client should exist");
        assert_eq!(retrieved.window, 0x100);

        let retrieved_mut = state.get_client_mut(0x100).expect("Client should exist");
        retrieved_mut.workspace = 2;
        assert_eq!(state.get_client(0x100).unwrap().workspace, 2);
    }

    #[test]
    fn test_client_state_remove() {
        let mut state = ClientState::new();
        state.add_client(0x100, Client::new(0x100));
        state.add_client(0x200, Client::new(0x200));
        assert_eq!(state.len(), 2);
        assert_eq!(state.focused_window(), Some(0x200));

        let removed = state.remove_client(0x200);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().window, 0x200);
        assert_eq!(state.len(), 1);
        assert_eq!(state.focused_window(), Some(0x100));

        assert!(state.remove_client(0x999).is_none());
    }

    #[test]
    fn test_client_state_focus_switching() {
        let mut state = ClientState::new();
        state.add_client(0x100, Client::new(0x100));
        state.add_client(0x200, Client::new(0x200));
        state.add_client(0x300, Client::new(0x300));

        state.focus(0x100);
        assert_eq!(state.focused_window(), Some(0x100));
        assert!(state.get_client(0x100).unwrap().focused);
        assert!(!state.get_client(0x200).unwrap().focused);
        assert!(!state.get_client(0x300).unwrap().focused);

        state.focus(0x200);
        assert_eq!(state.focused_window(), Some(0x200));
        assert!(!state.get_client(0x100).unwrap().focused);
        assert!(state.get_client(0x200).unwrap().focused);
    }
}
