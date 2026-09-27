//! Client state and tracking types.
//!
//! This module manages application windows tracked by the window manager,
//! maintaining the window lookup table (`Window -> Client`) and the focus stack.

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
    /// True if the window is currently mapped (visible) to the screen.
    pub mapped: bool,
    /// True if the window currently has input focus.
    pub focused: bool,
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
            mapped: false,
            focused: false,
        }
    }
}

/// Central registry for tracking and interacting with all managed clients.
#[derive(Debug, Default)]
pub struct ClientState {
    /// A map binding X11 Window IDs to `Client` data structures.
    pub clients: HashMap<Window, Client>,
    /// An ordered list of Window IDs for focus history/stacking.
    pub focus_stack: Vec<Window>,
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
        self.clients.remove(&window)
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
        for (w, client) in self.clients.iter_mut() {
            client.focused = *w == window;
        }
        self.focus_stack.retain(|&w| w != window);
        self.focus_stack.push(window);
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
