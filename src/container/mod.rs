//! Container hierarchy and layout engine.
//!
//! Provides a full container tree model supporting splits, tabs, stacks,
//! parent/child navigation, and container swapping.

use crate::ipc::json::JsonValue;
use crate::layout::{LayoutType, Rect, SplitType};
use x11rb::protocol::xproto::Window;

/// Monotonically increasing unique identifier for containers.
static NEXT_CONTAINER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100);

/// Generates a new unique container ID.
pub fn next_container_id() -> u64 {
    NEXT_CONTAINER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The layout/structural type of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerType {
    Root,
    Output,
    Workspace,
    Split(SplitType),
    Tabbed,
    Stacked,
    Leaf,
}

/// A container in the Tilo window management hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    /// Unique container ID.
    pub id: u64,
    /// Structural container type.
    pub container_type: ContainerType,
    /// Window layout algorithm.
    pub layout: LayoutType,
    /// Bounding rectangle in screen space.
    pub rect: Rect,
    /// Associated X11 Window if this is a leaf container.
    pub window: Option<Window>,
    /// Child containers.
    pub children: Vec<Container>,
    /// Ratio of division if this is a split container (0.05..0.95).
    pub ratio: f32,
    /// ID of the currently focused child container.
    pub focused_child: Option<u64>,
    /// Container marks.
    pub marks: Vec<String>,
    /// Is the container floating.
    pub floating: bool,
    /// Is the container fullscreen.
    pub fullscreen: bool,
    /// Is the container urgent.
    pub urgent: bool,
    /// Container name (e.g. workspace name or window title).
    pub name: Option<String>,
}

impl Container {
    /// Creates a new container with the given type.
    pub fn new(container_type: ContainerType) -> Self {
        Self {
            id: next_container_id(),
            container_type,
            layout: LayoutType::Bsp,
            rect: Rect::default(),
            window: None,
            children: Vec::new(),
            ratio: 0.5,
            focused_child: None,
            marks: Vec::new(),
            floating: false,
            fullscreen: false,
            urgent: false,
            name: None,
        }
    }

    /// Creates a leaf container holding an X11 client window.
    pub fn new_leaf(window: Window) -> Self {
        let mut con = Self::new(ContainerType::Leaf);
        con.window = Some(window);
        con
    }

    /// Checks if this subtree contains the given window.
    pub fn contains_window(&self, target: Window) -> bool {
        if self.window == Some(target) {
            return true;
        }
        self.children.iter().any(|c| c.contains_window(target))
    }

    /// Checks if this subtree contains the given container ID.
    pub fn contains_id(&self, target_id: u64) -> bool {
        if self.id == target_id {
            return true;
        }
        self.children.iter().any(|c| c.contains_id(target_id))
    }

    /// Finds a reference to a container containing the specified window.
    pub fn find_by_window(&self, target: Window) -> Option<&Container> {
        if self.window == Some(target) {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_by_window(target) {
                return Some(found);
            }
        }
        None
    }

    /// Finds a mutable reference to a container containing the specified window.
    pub fn find_by_window_mut(&mut self, target: Window) -> Option<&mut Container> {
        if self.window == Some(target) {
            return Some(self);
        }
        for child in &mut self.children {
            if let Some(found) = child.find_by_window_mut(target) {
                return Some(found);
            }
        }
        None
    }

    /// Finds the parent container ID of the given window or container ID.
    pub fn find_parent_of_window(&self, target: Window) -> Option<u64> {
        for child in &self.children {
            if child.window == Some(target) {
                return Some(self.id);
            }
            if let Some(p) = child.find_parent_of_window(target) {
                return Some(p);
            }
        }
        None
    }

    /// Collects all leaf windows in pre-order.
    pub fn windows(&self) -> Vec<Window> {
        let mut wins = Vec::new();
        self.collect_windows(&mut wins);
        wins
    }

    fn collect_windows(&self, out: &mut Vec<Window>) {
        if let Some(w) = self.window {
            out.push(w);
        }
        for child in &self.children {
            child.collect_windows(out);
        }
    }

    /// Removes a window from the container tree, collapsing single-child splits.
    pub fn remove_window(&mut self, target: Window) -> bool {
        let prev_len = self.children.len();
        self.children.retain(|c| c.window != Some(target));

        if self.children.len() < prev_len {
            return true;
        }

        let mut removed = false;
        for child in &mut self.children {
            if child.remove_window(target) {
                removed = true;
                break;
            }
        }

        if removed {
            // Flatten single-child split containers
            if self.children.len() == 1
                && matches!(self.container_type, ContainerType::Split(_))
            {
                let only_child = self.children.remove(0);
                *self = only_child;
            }
        }

        removed
    }

    /// Swaps the tree positions of two windows.
    pub fn swap_windows(&mut self, win_a: Window, win_b: Window) -> bool {
        let mut leaf_a = None;
        let mut leaf_b = None;

        self.collect_leaves_mut(&mut |con| {
            if con.window == Some(win_a) {
                leaf_a = Some(con as *mut Container);
            } else if con.window == Some(win_b) {
                leaf_b = Some(con as *mut Container);
            }
        });

        if let (Some(a), Some(b)) = (leaf_a, leaf_b) {
            unsafe {
                let a_win = (*a).window;
                let a_marks = std::mem::take(&mut (*a).marks);
                let b_win = (*b).window;
                let b_marks = std::mem::take(&mut (*b).marks);

                (*a).window = b_win;
                (*a).marks = b_marks;
                (*b).window = a_win;
                (*b).marks = a_marks;
            }
            true
        } else {
            false
        }
    }

    fn collect_leaves_mut<F>(&mut self, f: &mut F)
    where
        F: FnMut(&mut Container),
    {
        if self.window.is_some() {
            f(self);
        }
        for child in &mut self.children {
            child.collect_leaves_mut(f);
        }
    }

    /// Serializes this container into a `JsonValue` for tree queries.
    pub fn to_json(&self) -> JsonValue {
        let mut obj = JsonValue::object();
        obj.insert("id", JsonValue::Number(self.id as i64));
        obj.insert(
            "name",
            self.name
                .as_ref()
                .map(|n| JsonValue::Str(n.clone()))
                .unwrap_or(JsonValue::Null),
        );

        let type_str = match self.container_type {
            ContainerType::Root => "root",
            ContainerType::Output => "output",
            ContainerType::Workspace => "workspace",
            ContainerType::Split(_) | ContainerType::Tabbed | ContainerType::Stacked | ContainerType::Leaf => "con",
        };
        obj.insert("type", JsonValue::Str(type_str.to_string()));

        let layout_str = match self.container_type {
            ContainerType::Split(SplitType::Horizontal) => "splith",
            ContainerType::Split(SplitType::Vertical) => "splitv",
            ContainerType::Tabbed => "tabbed",
            ContainerType::Stacked => "stacked",
            _ => "default",
        };
        obj.insert("layout", JsonValue::Str(layout_str.to_string()));

        // Rect
        let mut rect_obj = JsonValue::object();
        rect_obj.insert("x", JsonValue::Number(self.rect.x as i64));
        rect_obj.insert("y", JsonValue::Number(self.rect.y as i64));
        rect_obj.insert("width", JsonValue::Number(self.rect.width as i64));
        rect_obj.insert("height", JsonValue::Number(self.rect.height as i64));
        obj.insert("rect", rect_obj);

        if let Some(w) = self.window {
            obj.insert("window", JsonValue::Number(w as i64));
        } else {
            obj.insert("window", JsonValue::Null);
        }

        // Marks
        let mut marks_arr = JsonValue::array();
        for m in &self.marks {
            marks_arr.push(JsonValue::Str(m.clone()));
        }
        obj.insert("marks", marks_arr);

        obj.insert("floating", JsonValue::Bool(self.floating));
        obj.insert("fullscreen", JsonValue::Bool(self.fullscreen));
        obj.insert("urgent", JsonValue::Bool(self.urgent));

        // Nodes
        let mut nodes_arr = JsonValue::array();
        for child in &self.children {
            if !child.floating {
                nodes_arr.push(child.to_json());
            }
        }
        obj.insert("nodes", nodes_arr);

        // Floating nodes
        let mut float_arr = JsonValue::array();
        for child in &self.children {
            if child.floating {
                float_arr.push(child.to_json());
            }
        }
        obj.insert("floating_nodes", float_arr);

        obj
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_hierarchy() {
        let mut root = Container::new(ContainerType::Workspace);
        let mut split = Container::new(ContainerType::Split(SplitType::Horizontal));
        split.children.push(Container::new_leaf(101));
        split.children.push(Container::new_leaf(102));
        root.children.push(split);

        assert!(root.contains_window(101));
        assert!(root.contains_window(102));
        assert!(!root.contains_window(103));
        assert_eq!(root.windows(), vec![101, 102]);

        // Swap windows 101 and 102
        assert!(root.swap_windows(101, 102));
        assert_eq!(root.windows(), vec![102, 101]);

        // Remove window 101
        assert!(root.remove_window(101));
        assert_eq!(root.windows(), vec![102]);
    }

    #[test]
    fn test_container_json_output() {
        let mut con = Container::new_leaf(42);
        con.marks.push("test_mark".to_string());
        con.name = Some("Terminal".to_string());
        let json = con.to_json().to_json_string();
        assert!(json.contains("\"window\":42"));
        assert!(json.contains("\"name\":\"Terminal\""));
        assert!(json.contains("\"marks\":[\"test_mark\"]"));
    }
}
