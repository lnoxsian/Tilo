//! Binary Space Partitioning (BSP) tree layout.

use x11rb::protocol::xproto::Window;
use super::{LayoutConfig, LayoutType, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitType {
    /// Splits horizontally into Left and Right children.
    Horizontal,
    /// Splits vertically into Top and Bottom children.
    Vertical,
}

/// A node in the Binary Space Partitioning (BSP) tree.
#[derive(Debug, Clone, PartialEq)]
pub enum BspNode {
    /// A leaf node representing a single managed X11 client window.
    Leaf {
        window: Window,
    },
    /// An internal node dividing its allocated area between two child subtrees.
    Split {
        split_type: SplitType,
        /// Fraction of width or height allocated to the first child (clamped 0.05..=0.95).
        ratio: f32,
        first: Box<BspNode>,
        second: Box<BspNode>,
    },
}

impl BspNode {
    /// Checks if this subtree contains the given window.
    pub fn contains(&self, target: Window) -> bool {
        match self {
            BspNode::Leaf { window } => *window == target,
            BspNode::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    /// Returns all window IDs in this subtree in pre-order traversal.
    pub fn windows(&self) -> Vec<Window> {
        let mut result = Vec::new();
        self.collect_windows(&mut result);
        result
    }

    fn collect_windows(&self, out: &mut Vec<Window>) {
        match self {
            BspNode::Leaf { window } => out.push(*window),
            BspNode::Split { first, second, .. } => {
                first.collect_windows(out);
                second.collect_windows(out);
            }
        }
    }

    /// Returns the number of leaf windows in this subtree.
    pub fn leaf_count(&self) -> usize {
        match self {
            BspNode::Leaf { .. } => 1,
            BspNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Finds the assigned bounding rectangle of `target` within the given screen space.
    pub fn find_window_rect(&self, target: Window, rect: Rect, inner_gap: u32) -> Option<Rect> {
        match self {
            BspNode::Leaf { window } => {
                if *window == target {
                    Some(rect)
                } else {
                    None
                }
            }
            BspNode::Split { split_type, ratio, first, second } => {
                let (r1, r2) = split_rect(rect, *split_type, *ratio, inner_gap);
                if first.contains(target) {
                    first.find_window_rect(target, r1, inner_gap)
                } else if second.contains(target) {
                    second.find_window_rect(target, r2, inner_gap)
                } else {
                    None
                }
            }
        }
    }

    /// Recursively computes bounding rectangles for all leaf windows.
    pub fn calculate_rects(&self, rect: Rect, inner_gap: u32, out: &mut Vec<(Window, Rect)>) {
        match self {
            BspNode::Leaf { window } => {
                out.push((*window, rect));
            }
            BspNode::Split { split_type, ratio, first, second } => {
                let (r1, r2) = split_rect(rect, *split_type, *ratio, inner_gap);
                first.calculate_rects(r1, inner_gap, out);
                second.calculate_rects(r2, inner_gap, out);
            }
        }
    }

    /// Inserts `new_win` by replacing the leaf containing `target` with a split node.
    pub fn insert_at(&mut self, target: Window, new_win: Window, split_type: SplitType, ratio: f32) -> bool {
        match self {
            BspNode::Leaf { window } if *window == target => {
                let old_win = *window;
                *self = BspNode::Split {
                    split_type,
                    ratio: ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: old_win }),
                    second: Box::new(BspNode::Leaf { window: new_win }),
                };
                true
            }
            BspNode::Split { first, second, .. } => {
                first.insert_at(target, new_win, split_type, ratio)
                    || second.insert_at(target, new_win, split_type, ratio)
            }
            _ => false,
        }
    }

    /// Swaps the leaf positions of two windows in this subtree.
    pub fn swap(&mut self, win_a: Window, win_b: Window) -> bool {
        let mut leaf_a = None;
        let mut leaf_b = None;
        self.collect_leaf_mut(&mut |w| {
            if *w == win_a {
                leaf_a = Some(w as *mut Window);
            } else if *w == win_b {
                leaf_b = Some(w as *mut Window);
            }
        });

        if let (Some(ptr_a), Some(ptr_b)) = (leaf_a, leaf_b) {
            unsafe {
                core::ptr::swap(ptr_a, ptr_b);
            }
            true
        } else {
            false
        }
    }

    fn collect_leaf_mut<F>(&mut self, f: &mut F)
    where
        F: FnMut(&mut Window),
    {
        match self {
            BspNode::Leaf { window } => f(window),
            BspNode::Split { first, second, .. } => {
                first.collect_leaf_mut(f);
                second.collect_leaf_mut(f);
            }
        }
    }

    /// Resizes the ancestor split of `target` that matches `target_split`.
    /// Resizes the focused pane `target` by `delta` along `target_split` such that
    /// only `target` and its immediate adjacent neighbor in that linear sequence change size,
    /// leaving all other panes completely unchanged.
    pub fn resize_pane(&mut self, target: Window, delta: f32, target_split: SplitType) -> bool {
        if let Some(highest) = self.find_highest_split_mut(target, target_split) {
            highest.adjust_linear_leaf(target, delta, target_split)
        } else {
            self.resize_any_split(delta, target_split)
        }
    }

    /// Finds a mutable reference to the highest continuous ancestor split of `target_split` containing `target`.
    fn find_highest_split_mut(&mut self, target: Window, target_split: SplitType) -> Option<&mut BspNode> {
        if !self.contains(target) {
            return None;
        }

        if let BspNode::Split { split_type, .. } = self
            && *split_type == target_split {
                return Some(self);
            }

        match self {
            BspNode::Split { first, second, .. } => {
                if first.contains(target) {
                    first.find_highest_split_mut(target, target_split)
                } else if second.contains(target) {
                    second.find_highest_split_mut(target, target_split)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Adjusts the leaf corresponding to `target` within this linear subtree of `target_split`.
    fn adjust_linear_leaf(&mut self, target: Window, delta: f32, target_split: SplitType) -> bool {
        let mut units = Vec::new();
        self.collect_split_units(target_split, 1.0, &mut units);

        if units.len() < 2 {
            return false;
        }

        let target_idx = match units.iter().position(|(wins, _)| wins.contains(&target)) {
            Some(i) => i,
            None => return false,
        };

        let neighbor_idx = if target_idx + 1 < units.len() {
            target_idx + 1
        } else if target_idx > 0 {
            target_idx - 1
        } else {
            return false;
        };

        let min_weight = 0.05;
        if delta > 0.0 {
            let max_delta = (units[neighbor_idx].1 - min_weight).max(0.0);
            let actual = delta.min(max_delta);
            if actual <= 0.0001 {
                return false;
            }
            units[target_idx].1 += actual;
            units[neighbor_idx].1 -= actual;
        } else {
            let max_delta = (units[target_idx].1 - min_weight).max(0.0);
            let actual = (-delta).min(max_delta);
            if actual <= 0.0001 {
                return false;
            }
            units[target_idx].1 -= actual;
            units[neighbor_idx].1 += actual;
        }

        self.apply_split_unit_weights(target_split, &units);
        true
    }

    fn collect_split_units(&self, target_split: SplitType, factor: f32, out: &mut Vec<(Vec<Window>, f32)>) {
        match self {
            BspNode::Leaf { window } => {
                out.push((vec![*window], factor));
            }
            BspNode::Split { split_type, ratio, first, second } if *split_type == target_split => {
                first.collect_split_units(target_split, factor * *ratio, out);
                second.collect_split_units(target_split, factor * (1.0 - *ratio), out);
            }
            BspNode::Split { .. } => {
                out.push((self.windows(), factor));
            }
        }
    }

    fn apply_split_unit_weights(&mut self, target_split: SplitType, units: &[(Vec<Window>, f32)]) {
        match self {
            BspNode::Leaf { .. } => {}
            BspNode::Split { split_type, ratio, first, second } if *split_type == target_split => {
                let first_wins = first.windows();
                let second_wins = second.windows();

                let sum_first: f32 = units
                    .iter()
                    .filter(|(wins, _)| wins.iter().any(|w| first_wins.contains(w)))
                    .map(|(_, s)| *s)
                    .sum();
                let sum_second: f32 = units
                    .iter()
                    .filter(|(wins, _)| wins.iter().any(|w| second_wins.contains(w)))
                    .map(|(_, s)| *s)
                    .sum();

                let total = sum_first + sum_second;
                if total > 0.0 {
                    *ratio = (sum_first / total).clamp(0.05, 0.95);
                }

                first.apply_split_unit_weights(target_split, units);
                second.apply_split_unit_weights(target_split, units);
            }
            _ => {}
        }
    }

    /// Resizes the first split matching `target_split` in this subtree.
    pub fn resize_any_split(&mut self, delta: f32, target_split: SplitType) -> bool {
        match self {
            BspNode::Leaf { .. } => false,
            BspNode::Split { split_type, ratio, first, second } => {
                if *split_type == target_split {
                    *ratio = (*ratio + delta).clamp(0.05, 0.95);
                    true
                } else {
                    first.resize_any_split(delta, target_split)
                        || second.resize_any_split(delta, target_split)
                }
            }
        }
    }
}

/// Recursively removes `target` from the subtree, collapsing the parent node with the remaining sibling.
#[allow(clippy::boxed_local)]
fn remove_node(node: Box<BspNode>, target: Window) -> Option<Box<BspNode>> {
    match *node {
        BspNode::Leaf { window } => {
            if window == target {
                None
            } else {
                Some(Box::new(BspNode::Leaf { window }))
            }
        }
        BspNode::Split { split_type, ratio, first, second } => {
            if first.contains(target) {
                let new_first = remove_node(first, target);
                match new_first {
                    None => Some(second), // First was removed, collapse to second child
                    Some(f) => Some(Box::new(BspNode::Split {
                        split_type,
                        ratio,
                        first: f,
                        second,
                    })),
                }
            } else if second.contains(target) {
                let new_second = remove_node(second, target);
                match new_second {
                    None => Some(first), // Second was removed, collapse to first child
                    Some(s) => Some(Box::new(BspNode::Split {
                        split_type,
                        ratio,
                        first,
                        second: s,
                    })),
                }
            } else {
                Some(Box::new(BspNode::Split {
                    split_type,
                    ratio,
                    first,
                    second,
                }))
            }
        }
    }
}

/// Helper function to divide a rectangle into two children based on split type, ratio, and gap.
fn split_rect(rect: Rect, split_type: SplitType, ratio: f32, inner_gap: u32) -> (Rect, Rect) {
    match split_type {
        SplitType::Horizontal => {
            if rect.width <= inner_gap {
                let w = rect.width / 2;
                (
                    Rect::new(rect.x, rect.y, w, rect.height),
                    Rect::new(rect.x + w as i32, rect.y, rect.width.saturating_sub(w), rect.height),
                )
            } else {
                let available = rect.width - inner_gap;
                let w1 = ((available as f32 * ratio).round() as u32).clamp(1, available.saturating_sub(1));
                let w2 = available - w1;
                (
                    Rect::new(rect.x, rect.y, w1, rect.height),
                    Rect::new(rect.x + w1 as i32 + inner_gap as i32, rect.y, w2, rect.height),
                )
            }
        }
        SplitType::Vertical => {
            if rect.height <= inner_gap {
                let h = rect.height / 2;
                (
                    Rect::new(rect.x, rect.y, rect.width, h),
                    Rect::new(rect.x, rect.y + h as i32, rect.width, rect.height.saturating_sub(h)),
                )
            } else {
                let available = rect.height - inner_gap;
                let h1 = ((available as f32 * ratio).round() as u32).clamp(1, available.saturating_sub(1));
                let h2 = available - h1;
                (
                    Rect::new(rect.x, rect.y, rect.width, h1),
                    Rect::new(rect.x, rect.y + h1 as i32 + inner_gap as i32, rect.width, h2),
                )
            }
        }
    }
}

/// Binary Space Partitioning (BSP) tree layout manager for a single workspace.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BspTree {
    /// Root node of the partition tree.
    pub root: Option<Box<BspNode>>,
    /// Pre-selected split direction for the next inserted window.
    pub preselect: Option<SplitType>,
}

impl BspTree {
    /// Creates a new empty `BspTree`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the tree contains no windows.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Returns the number of tiled windows in the tree.
    pub fn len(&self) -> usize {
        self.root.as_ref().map(|r| r.leaf_count()).unwrap_or(0)
    }

    /// Checks if a window is managed by this tree.
    pub fn contains(&self, window: Window) -> bool {
        self.root.as_ref().map(|r| r.contains(window)).unwrap_or(false)
    }

    /// Returns all window IDs in pre-order traversal.
    pub fn windows(&self) -> Vec<Window> {
        self.root.as_ref().map(|r| r.windows()).unwrap_or_default()
    }

    /// Inserts a window into the tree, auto-splitting or using a pre-selected split.
    pub fn insert(&mut self, target: Option<Window>, new_win: Window, usable_rect: &Rect, inner_gap: u32) {
        if self.contains(new_win) {
            return;
        }

        let Some(ref mut root) = self.root else {
            self.root = Some(Box::new(BspNode::Leaf { window: new_win }));
            return;
        };

        let target_win = target
            .filter(|&w| root.contains(w))
            .or_else(|| root.windows().last().copied())
            .unwrap_or(new_win);

        if target_win == new_win {
            return;
        }

        let split_type = if let Some(pre) = self.preselect.take() {
            pre
        } else {
            let target_rect = root.find_window_rect(target_win, *usable_rect, inner_gap)
                .unwrap_or(*usable_rect);
            if target_rect.width >= target_rect.height {
                SplitType::Horizontal
            } else {
                SplitType::Vertical
            }
        };

        root.insert_at(target_win, new_win, split_type, 0.5);
    }

    /// Removes a window from the tree and collapses the sibling into the parent split.
    pub fn remove(&mut self, window: Window) -> bool {
        if let Some(root) = self.root.take() {
            let was_present = root.contains(window);
            self.root = remove_node(root, window);
            was_present
        } else {
            false
        }
    }

    /// Swaps two windows' positions in the tree.
    pub fn swap_windows(&mut self, win_a: Window, win_b: Window) -> bool {
        if let Some(ref mut root) = self.root {
            root.swap(win_a, win_b)
        } else {
            false
        }
    }

    /// Resizes the ancestor split of `window` by `delta_ratio` ($\pm 0.05$).
    /// Returns the active master pane ratio based on layout orientation.
    pub fn master_ratio_for_layout(&self, layout: LayoutType) -> Option<f32> {
        match (layout, self.root.as_deref()) {
            (LayoutType::MasterStack, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, .. })) => Some(*ratio),
            (LayoutType::MasterRight, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, .. })) => Some(1.0 - *ratio),
            (LayoutType::MasterTop, Some(BspNode::Split { split_type: SplitType::Vertical, ratio, .. })) => Some(*ratio),
            (LayoutType::MasterBottom, Some(BspNode::Split { split_type: SplitType::Vertical, ratio, .. })) => Some(1.0 - *ratio),
            (LayoutType::CenterMaster, Some(BspNode::Split { split_type: SplitType::Horizontal, ratio, second, .. })) => {
                if matches!(**second, BspNode::Leaf { .. }) {
                    Some(1.0 - *ratio)
                } else {
                    Some((1.0 - 2.0 * *ratio).clamp(0.05, 0.95))
                }
            }
            _ => None,
        }
    }

    /// Returns the current horizontal split ratio if the root is a horizontal split.
    pub fn master_ratio(&self) -> Option<f32> {
        self.master_ratio_for_layout(LayoutType::MasterStack)
    }

    /// Resizes the ancestor split of `window` by `delta_ratio` ($\pm 0.05$), falling back to any matching split.
    pub fn resize_window(&mut self, window: Window, delta_ratio: f32, split_type: SplitType) -> bool {
        if let Some(ref mut root) = self.root {
            root.resize_pane(window, delta_ratio, split_type)
        } else {
            false
        }
    }

    /// Dynamically resizes a window's split during mouse drag.
    pub fn resize_by_pointer(&mut self, window: Window, delta_x: i32, delta_y: i32, initial_rect: Rect) -> bool {
        let mut handled = false;
        if let Some(ref mut root) = self.root {
            if initial_rect.width > 0 && delta_x != 0 {
                let frac = delta_x as f32 / initial_rect.width as f32;
                if root.resize_pane(window, frac, SplitType::Horizontal) {
                    handled = true;
                }
            }
            if initial_rect.height > 0 && delta_y != 0 {
                let frac = delta_y as f32 / initial_rect.height as f32;
                if root.resize_pane(window, frac, SplitType::Vertical) {
                    handled = true;
                }
            }
        }
        handled
    }

    /// Calculates bounding rectangles for all windows in the tree.
    pub fn calculate_rects(&self, screen_rect: &Rect, config: &LayoutConfig) -> Vec<(Window, Rect)> {
        let Some(ref root) = self.root else {
            return Vec::new();
        };

        let double_outer = 2 * config.outer_gap;
        let (usable_x, usable_y, usable_w, usable_h) = if screen_rect.width <= double_outer || screen_rect.height <= double_outer {
            (screen_rect.x, screen_rect.y, screen_rect.width, screen_rect.height)
        } else {
            (
                screen_rect.x + config.outer_gap as i32,
                screen_rect.y + config.outer_gap as i32,
                screen_rect.width - double_outer,
                screen_rect.height - double_outer,
            )
        };

        let usable_rect = Rect::new(usable_x, usable_y, usable_w, usable_h);
        let mut pairs = Vec::new();
        root.calculate_rects(usable_rect, config.inner_gap, &mut pairs);

        for (_, rect) in &mut pairs {
            rect.width = rect.width.saturating_sub(2 * config.border_width);
            rect.height = rect.height.saturating_sub(2 * config.border_width);
        }

        pairs
    }

    /// Builds a Master/Stack binary layout tree.
    pub fn build_master_stack(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: master_ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: Self::build_linear(stack, SplitType::Vertical),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Right, Stack on Left.
    pub fn build_master_right(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: stack_ratio,
                    first: Self::build_linear(stack, SplitType::Vertical),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Top, Stack on Bottom.
    pub fn build_master_top(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Vertical,
                    ratio: master_ratio.clamp(0.05, 0.95),
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: Self::build_linear(stack, SplitType::Horizontal),
                }))
            }
        }
    }

    /// Builds a Master/Stack binary layout tree with Master on Bottom, Stack on Top.
    pub fn build_master_bottom(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            _ => {
                let master = clients[0];
                let stack = &clients[1..];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Vertical,
                    ratio: stack_ratio,
                    first: Self::build_linear(stack, SplitType::Horizontal),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
        }
    }

    /// Builds a Centered Master binary layout tree (Stack Left, Master Center, Stack Right).
    pub fn build_center_master(clients: &[Window], master_ratio: f32) -> Option<Box<BspNode>> {
        match clients.len() {
            0 => None,
            1 => Some(Box::new(BspNode::Leaf { window: clients[0] })),
            2 => {
                let master = clients[0];
                let stack = clients[1];
                let stack_ratio = (1.0 - master_ratio).clamp(0.05, 0.95);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: stack_ratio,
                    first: Box::new(BspNode::Leaf { window: stack }),
                    second: Box::new(BspNode::Leaf { window: master }),
                }))
            }
            _ => {
                let master = clients[0];
                let left_wins: Vec<Window> = clients.iter().skip(1).step_by(2).copied().collect();
                let right_wins: Vec<Window> = clients.iter().skip(2).step_by(2).copied().collect();

                let master_r = master_ratio.clamp(0.10, 0.80);
                let stack_r = 1.0 - master_r;
                let left_fraction = stack_r / 2.0;
                let remaining_fraction = 1.0 - left_fraction;
                let master_in_rem_ratio = (master_r / remaining_fraction).clamp(0.05, 0.95);

                let right_node = Self::build_linear(&right_wins, SplitType::Vertical);
                let center_and_right = Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: master_in_rem_ratio,
                    first: Box::new(BspNode::Leaf { window: master }),
                    second: right_node,
                });

                let left_node = Self::build_linear(&left_wins, SplitType::Vertical);
                Some(Box::new(BspNode::Split {
                    split_type: SplitType::Horizontal,
                    ratio: left_fraction.clamp(0.05, 0.95),
                    first: left_node,
                    second: center_and_right,
                }))
            }
        }
    }

    /// Builds a Vertical (side-by-side columns) binary layout tree.
    pub fn build_vertical(clients: &[Window]) -> Option<Box<BspNode>> {
        if clients.is_empty() {
            None
        } else {
            Some(Self::build_linear(clients, SplitType::Horizontal))
        }
    }

    /// Builds a Horizontal (stacked rows) binary layout tree.
    pub fn build_horizontal(clients: &[Window]) -> Option<Box<BspNode>> {
        if clients.is_empty() {
            None
        } else {
            Some(Self::build_linear(clients, SplitType::Vertical))
        }
    }

    /// Builds a balanced Grid binary layout tree.
    pub fn build_grid(clients: &[Window]) -> Option<Box<BspNode>> {
        let n = clients.len();
        if n == 0 {
            return None;
        }
        if n == 1 {
            return Some(Box::new(BspNode::Leaf { window: clients[0] }));
        }

        let cols = (n as f32).sqrt().ceil() as usize;
        let rows = n.div_ceil(cols);
        let base_cols = n / rows;
        let extra_rows = n % rows;

        let mut row_nodes = Vec::with_capacity(rows);
        let mut idx = 0;
        for r in 0..rows {
            let cols_in_row = if r < extra_rows {
                base_cols + 1
            } else {
                base_cols
            };
            let row_clients = &clients[idx..idx + cols_in_row];
            idx += cols_in_row;
            row_nodes.push(*Self::build_linear(row_clients, SplitType::Horizontal));
        }

        Self::combine_nodes(&row_nodes, SplitType::Vertical)
    }

    fn combine_nodes(nodes: &[BspNode], split_type: SplitType) -> Option<Box<BspNode>> {
        if nodes.is_empty() {
            return None;
        }
        if nodes.len() == 1 {
            return Some(Box::new(nodes[0].clone()));
        }
        let k = nodes.len();
        let ratio = (1.0 / k as f32).clamp(0.05, 0.95);
        let first = Box::new(nodes[0].clone());
        let second = Self::combine_nodes(&nodes[1..], split_type)?;
        Some(Box::new(BspNode::Split {
            split_type,
            ratio,
            first,
            second,
        }))
    }

    fn build_linear(clients: &[Window], split_type: SplitType) -> Box<BspNode> {
        let k = clients.len();
        if k <= 1 {
            Box::new(BspNode::Leaf { window: clients[0] })
        } else {
            let ratio = (1.0 / k as f32).clamp(0.05, 0.95);
            Box::new(BspNode::Split {
                split_type,
                ratio,
                first: Box::new(BspNode::Leaf { window: clients[0] }),
                second: Self::build_linear(&clients[1..], split_type),
            })
        }
    }

    /// Rebuilds the tree structure for the specified layout type while preserving client order.
    pub fn rebuild_for_layout(&mut self, layout: LayoutType, clients: &[Window], master_ratio: f32) {
        self.root = match layout {
            LayoutType::MasterStack => Self::build_master_stack(clients, master_ratio),
            LayoutType::MasterRight => Self::build_master_right(clients, master_ratio),
            LayoutType::MasterTop => Self::build_master_top(clients, master_ratio),
            LayoutType::MasterBottom => Self::build_master_bottom(clients, master_ratio),
            LayoutType::CenterMaster => Self::build_center_master(clients, master_ratio),
            LayoutType::Grid => Self::build_grid(clients),
            LayoutType::Vertical => Self::build_vertical(clients),
            LayoutType::Horizontal => Self::build_horizontal(clients),
            LayoutType::Bsp => {
                let current_wins = self.windows();
                if current_wins.len() == clients.len() && clients.iter().all(|w| current_wins.contains(w)) && self.root.is_some() {
                    return;
                }
                let mut tree = BspTree::new();
                let dummy_rect = Rect::new(0, 0, 1920, 1080);
                for &w in clients {
                    tree.insert(None, w, &dummy_rect, 0);
                }
                tree.root
            }
            LayoutType::Monocle | LayoutType::Tabbed | LayoutType::Stacked => None,
        };
    }

    /// Synchronizes the tree with visible clients using layout-specific insertion rules.
    pub fn sync_clients_with_layout(
        &mut self,
        layout: LayoutType,
        visible_clients: &[Window],
        focused: Option<Window>,
        usable_rect: &Rect,
        inner_gap: u32,
        master_ratio: f32,
    ) {
        for w in self.windows() {
            if !visible_clients.contains(&w) {
                self.remove(w);
            }
        }

        if self.is_empty() {
            self.rebuild_for_layout(layout, visible_clients, master_ratio);
            return;
        }

        for &w in visible_clients {
            if !self.contains(w) {
                match layout {
                    LayoutType::MasterStack => {
                        let wins = self.windows();
                        if wins.len() == 1 {
                            let m = wins[0];
                            if let Some(ref mut root) = self.root {
                                root.insert_at(m, w, SplitType::Horizontal, master_ratio.clamp(0.05, 0.95));
                            }
                        } else if let Some(last) = wins.last().copied()
                            && let Some(ref mut root) = self.root {
                                root.insert_at(last, w, SplitType::Vertical, 0.5);
                        }
                    }
                    LayoutType::MasterRight | LayoutType::MasterTop | LayoutType::MasterBottom | LayoutType::CenterMaster | LayoutType::Grid => {
                        let mut all = self.windows();
                        all.push(w);
                        self.rebuild_for_layout(layout, &all, master_ratio);
                    }
                    LayoutType::Vertical => {
                        let wins = self.windows();
                        if let Some(last) = wins.last().copied()
                            && let Some(ref mut root) = self.root {
                                root.insert_at(last, w, SplitType::Horizontal, 0.5);
                        }
                    }
                    LayoutType::Horizontal => {
                        let wins = self.windows();
                        if let Some(last) = wins.last().copied()
                            && let Some(ref mut root) = self.root {
                                root.insert_at(last, w, SplitType::Vertical, 0.5);
                        }
                    }
                    _ => {
                        self.insert(focused, w, usable_rect, inner_gap);
                    }
                }
            }
        }
    }

    /// Synchronizes the tree with a list of currently visible tiled windows.
    pub fn sync_clients(&mut self, visible_clients: &[Window], focused: Option<Window>, usable_rect: &Rect, inner_gap: u32) {
        self.sync_clients_with_layout(LayoutType::Bsp, visible_clients, focused, usable_rect, inner_gap, 0.55);
    }
}


