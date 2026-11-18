//! Window and container marks management.

use super::ClientState;
use x11rb::protocol::xproto::Window;

impl ClientState {
    /// Marks a window with the given mark name.
    ///
    /// If another window has this mark, it is removed from that window first.
    /// - If `toggle` is true: if `window` already has `mark`, it is removed; otherwise added.
    /// - If `add` is true: appends `mark` to the window if not already present.
    /// - If neither (replace mode): sets the window's marks to just `[mark]`.
    pub fn mark_window(&mut self, window: Window, mark: &str, add: bool, toggle: bool) -> bool {
        if !self.clients.contains_key(&window) {
            return false;
        }

        // Remove this mark from any other client (enforce uniqueness across windows)
        for (w, client) in self.clients.iter_mut() {
            if *w != window {
                client.marks.retain(|m| m != mark);
            }
        }

        let client = match self.clients.get_mut(&window) {
            Some(c) => c,
            None => return false,
        };

        if toggle {
            if let Some(pos) = client.marks.iter().position(|m| m == mark) {
                client.marks.remove(pos);
            } else {
                client.marks.push(mark.to_string());
            }
        } else if add {
            if !client.marks.iter().any(|m| m == mark) {
                client.marks.push(mark.to_string());
            }
        } else {
            // Replace
            client.marks.clear();
            client.marks.push(mark.to_string());
        }
        true
    }

    /// Removes a mark (or all marks if `mark` is None) from a window.
    pub fn unmark_window(&mut self, window: Window, mark: Option<&str>) -> bool {
        if let Some(client) = self.clients.get_mut(&window) {
            if let Some(m) = mark {
                client.marks.retain(|existing| existing != m);
            } else {
                client.marks.clear();
            }
            true
        } else {
            false
        }
    }

    /// Finds a window by its mark.
    pub fn find_window_by_mark(&self, mark: &str) -> Option<Window> {
        for (&w, client) in &self.clients {
            if client.marks.iter().any(|m| m == mark) {
                return Some(w);
            }
        }
        None
    }

    /// Returns a sorted list of all active marks across all windows.
    pub fn all_marks(&self) -> Vec<String> {
        let mut marks = Vec::new();
        for client in self.clients.values() {
            for m in &client.marks {
                if !marks.contains(m) {
                    marks.push(m.clone());
                }
            }
        }
        marks.sort();
        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Client;

    #[test]
    fn test_marks_lifecycle() {
        let mut state = ClientState::new();
        state.add_client(10, Client::new(10));
        state.add_client(20, Client::new(20));

        // Add mark to window 10
        assert!(state.mark_window(10, "browser", false, false));
        assert_eq!(state.all_marks(), vec!["browser".to_string()]);
        assert_eq!(state.find_window_by_mark("browser"), Some(10));

        // Add second mark to window 10 with add=true
        assert!(state.mark_window(10, "work", true, false));
        assert_eq!(state.all_marks(), vec!["browser".to_string(), "work".to_string()]);

        // Steal mark "browser" to window 20
        assert!(state.mark_window(20, "browser", false, false));
        assert_eq!(state.find_window_by_mark("browser"), Some(20));
        // Window 10 still has "work"
        assert_eq!(state.find_window_by_mark("work"), Some(10));

        // Toggle mark on window 20 (removes "browser")
        assert!(state.mark_window(20, "browser", false, true));
        assert_eq!(state.find_window_by_mark("browser"), None);

        // Toggle again (re-adds "browser")
        assert!(state.mark_window(20, "browser", false, true));
        assert_eq!(state.find_window_by_mark("browser"), Some(20));

        // Unmark window 10
        assert!(state.unmark_window(10, None));
        assert_eq!(state.find_window_by_mark("work"), None);

        // Window destruction purges marks
        state.remove_client(20);
        assert!(state.all_marks().is_empty());
    }
}
