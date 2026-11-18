//! Window Manager IPC dispatch, queries, tree serialization, and command execution.

use super::WindowManager;
use crate::focus::Direction;
use crate::ipc::command::{CommandResult, ParsedCommand, parse_command};
use crate::ipc::json::JsonValue;
use crate::layout::SplitType;
use crate::x11::X11Error;
use x11rb::protocol::xproto::{ConnectionExt as _, Window};

impl WindowManager {
    /// Switches the active workspace by exact name, creating it on demand if absent.
    pub fn switch_workspace_by_name(&mut self, name: &str) -> Result<(), X11Error> {
        let ws_id = self.workspace_manager.get_or_create_by_name(name).id;
        self.workspace_manager.previous_workspace_id = Some(self.current_workspace);
        self.switch_workspace(ws_id)
    }

    /// Switches back and forth to the previously active workspace.
    pub fn switch_workspace_back_and_forth(&mut self) -> Result<(), X11Error> {
        if let Some(prev) = self.workspace_manager.previous_workspace_id {
            self.switch_workspace(prev)?;
        }
        Ok(())
    }

    /// Moves a window to the workspace specified by name.
    pub fn move_window_to_workspace_name(
        &mut self,
        window: Window,
        name: &str,
    ) -> Result<(), X11Error> {
        let target_id = self.workspace_manager.get_or_create_by_name(name).id;
        self.move_to_workspace(window, target_id)
    }

    /// Swaps the tree positions of two windows across workspaces or monitors safely.
    pub fn swap_windows(&mut self, win_a: Window, win_b: Window) -> Result<bool, X11Error> {
        if win_a == win_b || !self.client_state.contains(win_a) || !self.client_state.contains(win_b) {
            return Ok(false);
        }

        let ws_a = self.client_state.get_client(win_a).map(|c| c.workspace);
        let ws_b = self.client_state.get_client(win_b).map(|c| c.workspace);

        if ws_a == ws_b && let Some(ws_id) = ws_a {
            if let Some(ws) = self.workspace_manager.get_mut(ws_id) {
                ws.bsp_tree.swap_windows(win_a, win_b);
                let idx_a = ws.clients.iter().position(|&w| w == win_a);
                let idx_b = ws.clients.iter().position(|&w| w == win_b);
                if let (Some(a), Some(b)) = (idx_a, idx_b) {
                    ws.clients.swap(a, b);
                }
            }
        } else if let (Some(a_id), Some(b_id)) = (ws_a, ws_b) {
            // Swap across different workspaces
            self.workspace_manager.move_client(win_a, b_id);
            self.workspace_manager.move_client(win_b, a_id);
            if let Some(ca) = self.client_state.get_client_mut(win_a) {
                ca.workspace = b_id;
            }
            if let Some(cb) = self.client_state.get_client_mut(win_b) {
                cb.workspace = a_id;
            }
        }

        // Swap order in focus/client order
        let ord_a = self.client_state.client_order.iter().position(|&w| w == win_a);
        let ord_b = self.client_state.client_order.iter().position(|&w| w == win_b);
        if let (Some(a), Some(b)) = (ord_a, ord_b) {
            self.client_state.client_order.swap(a, b);
        }

        self.retile()?;
        self.focus_client(win_a)?;
        Ok(true)
    }

    /// Dumps the full layout tree as a JSON string.
    pub fn dump_tree_json(&self) -> String {
        let mut root = JsonValue::object();
        root.insert("id", JsonValue::Number(1));
        root.insert("name", JsonValue::Str("root".to_string()));
        root.insert("type", JsonValue::Str("root".to_string()));

        let mut root_rect = JsonValue::object();
        root_rect.insert("x", JsonValue::Number(0));
        root_rect.insert("y", JsonValue::Number(0));
        root_rect.insert("width", JsonValue::Number(self.context.screen_width as i64));
        root_rect.insert("height", JsonValue::Number(self.context.screen_height as i64));
        root.insert("rect", root_rect);

        let mut output_nodes = JsonValue::array();
        for mon in &self.monitor_manager.monitors {
            let mut out_obj = JsonValue::object();
            out_obj.insert("id", JsonValue::Number(10 + mon.id as i64));
            let mon_name = format!("output-{}", mon.id);
            out_obj.insert("name", JsonValue::Str(mon_name));
            out_obj.insert("type", JsonValue::Str("output".to_string()));

            let mut out_rect = JsonValue::object();
            out_rect.insert("x", JsonValue::Number(mon.x as i64));
            out_rect.insert("y", JsonValue::Number(mon.y as i64));
            out_rect.insert("width", JsonValue::Number(mon.width as i64));
            out_rect.insert("height", JsonValue::Number(mon.height as i64));
            out_obj.insert("rect", out_rect);

            let mut ws_nodes = JsonValue::array();
            for ws in self.workspace_manager.all_workspaces() {
                // If workspace is on this monitor or belongs to it
                if crate::workspace::workspace_to_monitor_id(ws.id) == mon.id
                    || (mon.id == self.monitor_manager.active_monitor_id && ws.id == self.current_workspace)
                {
                    let mut ws_obj = JsonValue::object();
                    ws_obj.insert("id", JsonValue::Number(ws.id as i64));
                    ws_obj.insert("num", JsonValue::Number(ws.num as i64));
                    ws_obj.insert("name", JsonValue::Str(ws.name.clone()));
                    ws_obj.insert("type", JsonValue::Str("workspace".to_string()));

                    // Nodes on workspace
                    let mut win_nodes = JsonValue::array();
                    for &w in &ws.clients {
                        if let Some(c) = self.client_state.get_client(w) {
                            if !c.is_floating() {
                                win_nodes.push(self.client_to_json_node(c));
                            }
                        }
                    }
                    ws_obj.insert("nodes", win_nodes);

                    // Floating nodes on workspace
                    let mut float_nodes = JsonValue::array();
                    for &w in &ws.clients {
                        if let Some(c) = self.client_state.get_client(w) {
                            if c.is_floating() {
                                float_nodes.push(self.client_to_json_node(c));
                            }
                        }
                    }
                    ws_obj.insert("floating_nodes", float_nodes);
                    ws_nodes.push(ws_obj);
                }
            }
            out_obj.insert("nodes", ws_nodes);
            output_nodes.push(out_obj);
        }

        root.insert("nodes", output_nodes);
        root.to_json_string()
    }

    fn client_to_json_node(&self, client: &crate::client::Client) -> JsonValue {
        let mut obj = JsonValue::object();
        obj.insert("id", JsonValue::Number(client.window as i64));
        obj.insert("window", JsonValue::Number(client.window as i64));
        obj.insert("type", JsonValue::Str("con".to_string()));
        obj.insert(
            "name",
            client
                .title
                .as_ref()
                .map(|t| JsonValue::Str(t.clone()))
                .unwrap_or(JsonValue::Null),
        );
        obj.insert("floating", JsonValue::Bool(client.is_floating()));
        obj.insert("fullscreen", JsonValue::Bool(client.is_fullscreen()));
        obj.insert("urgent", JsonValue::Bool(client.urgent));
        obj.insert("focused", JsonValue::Bool(self.focused_window() == Some(client.window)));

        let mut marks = JsonValue::array();
        for m in &client.marks {
            marks.push(JsonValue::Str(m.clone()));
        }
        obj.insert("marks", marks);

        let mut rect_obj = JsonValue::object();
        if let Some(geom) = client.floating_geometry {
            rect_obj.insert("x", JsonValue::Number(geom.x as i64));
            rect_obj.insert("y", JsonValue::Number(geom.y as i64));
            rect_obj.insert("width", JsonValue::Number(geom.width as i64));
            rect_obj.insert("height", JsonValue::Number(geom.height as i64));
        } else {
            rect_obj.insert("x", JsonValue::Number(0));
            rect_obj.insert("y", JsonValue::Number(0));
            rect_obj.insert("width", JsonValue::Number(100));
            rect_obj.insert("height", JsonValue::Number(100));
        }
        obj.insert("rect", rect_obj);

        obj
    }

    /// Dumps all active and configured workspaces as a JSON array string.
    pub fn dump_workspaces_json(&self) -> String {
        let mut arr = JsonValue::array();
        for ws in self.workspace_manager.all_workspaces() {
            let mut obj = JsonValue::object();
            obj.insert("id", JsonValue::Number(ws.id as i64));
            obj.insert("num", JsonValue::Number(ws.num as i64));
            obj.insert("name", JsonValue::Str(ws.name.clone()));
            obj.insert("focused", JsonValue::Bool(ws.id == self.current_workspace));
            let is_visible = self.monitor_manager.monitors.iter().any(|m| m.workspace == ws.id);
            obj.insert("visible", JsonValue::Bool(is_visible));
            let has_urgent = ws.clients.iter().any(|&w| {
                self.client_state.get_client(w).map(|c| c.urgent).unwrap_or(false)
            });
            obj.insert("urgent", JsonValue::Bool(has_urgent));
            let out_name = self
                .monitor_manager
                .monitor_for_workspace(ws.id)
                .map(|m| format!("output-{}", m.id))
                .unwrap_or_else(|| "default".to_string());
            obj.insert("output", JsonValue::Str(out_name));
            arr.push(obj);
        }
        arr.to_json_string()
    }

    /// Dumps all detected output monitors as a JSON array string.
    pub fn dump_outputs_json(&self) -> String {
        let mut arr = JsonValue::array();
        for mon in &self.monitor_manager.monitors {
            let mut obj = JsonValue::object();
            obj.insert("id", JsonValue::Number(mon.id as i64));
            let mon_name = format!("output-{}", mon.id);
            obj.insert("name", JsonValue::Str(mon_name));
            obj.insert("active", JsonValue::Bool(true));
            obj.insert("primary", JsonValue::Bool(mon.is_primary));

            let ws_name = self
                .workspace_manager
                .get(mon.workspace)
                .map(|w| w.name.clone())
                .unwrap_or_else(|| mon.workspace.to_string());
            obj.insert("current_workspace", JsonValue::Str(ws_name));

            let mut r = JsonValue::object();
            r.insert("x", JsonValue::Number(mon.x as i64));
            r.insert("y", JsonValue::Number(mon.y as i64));
            r.insert("width", JsonValue::Number(mon.width as i64));
            r.insert("height", JsonValue::Number(mon.height as i64));
            obj.insert("rect", r);

            arr.push(obj);
        }
        arr.to_json_string()
    }

    /// Dumps all active container marks as a JSON array string.
    pub fn dump_marks_json(&self) -> String {
        let marks = self.client_state.all_marks();
        let mut arr = JsonValue::array();
        for m in marks {
            arr.push(JsonValue::Str(m));
        }
        arr.to_json_string()
    }

    /// Executes an individual IPC command line with optional criteria targeting.
    pub fn execute_ipc_command_line(&mut self, line: &str) -> CommandResult {
        let (criteria, cmd) = match parse_command(line) {
            Ok(res) => res,
            Err(err) => return CommandResult::err(err),
        };

        if let Some(crit) = criteria {
            let matching_windows: Vec<Window> = self
                .client_state
                .all_clients()
                .iter()
                .copied()
                .filter(|&win| {
                    if let Some(client) = self.client_state.get_client(win) {
                        let title = client.title.as_deref();
                        let ws_name = self
                            .workspace_manager
                            .get(client.workspace)
                            .map(|w| w.name.as_str())
                            .unwrap_or("");
                        crit.matches(
                            None,
                            None,
                            title,
                            None,
                            None,
                            ws_name,
                            &client.marks,
                            client.is_floating(),
                            client.urgent,
                            win as u64,
                        )
                    } else {
                        false
                    }
                })
                .collect();

            if matching_windows.is_empty() {
                return CommandResult::err("No matching window found for criteria");
            }

            for win in matching_windows {
                if let Err(e) = self.apply_command_to_window(win, &cmd) {
                    return CommandResult::err(e);
                }
            }
            CommandResult::ok()
        } else {
            let focused = self.focused_window();
            match self.apply_command_to_context(focused, &cmd) {
                Ok(()) => CommandResult::ok(),
                Err(e) => CommandResult::err(e),
            }
        }
    }

    fn apply_command_to_window(&mut self, window: Window, cmd: &ParsedCommand) -> Result<(), String> {
        match cmd {
            ParsedCommand::Floating(mode) => {
                let is_float = self.client_state.get_client(window).map(|c| c.is_floating()).unwrap_or(false);
                let should_float = mode.unwrap_or(!is_float);
                if should_float != is_float {
                    let _ = self.toggle_floating(window);
                }
            }
            ParsedCommand::Fullscreen(mode) => {
                let is_full = self.client_state.get_client(window).map(|c| c.is_fullscreen()).unwrap_or(false);
                let should_full = mode.unwrap_or(!is_full);
                if should_full != is_full {
                    let _ = self.toggle_fullscreen(window);
                }
            }
            ParsedCommand::Close => {
                let _ = self.kill_client(window);
            }
            ParsedCommand::Kill => {
                let _ = self.context.connection.kill_client(window);
            }
            ParsedCommand::Mark { name, add, toggle } => {
                self.client_state.mark_window(window, name, *add, *toggle);
            }
            ParsedCommand::Unmark(name) => {
                self.client_state.unmark_window(window, name.as_deref());
            }
            ParsedCommand::MoveToWorkspace(name) => {
                let _ = self.move_window_to_workspace_name(window, name);
            }
            _ => {
                // Focus target window and run context command
                let _ = self.focus_client(window);
                return self.apply_command_to_context(Some(window), cmd);
            }
        }
        Ok(())
    }

    fn apply_command_to_context(&mut self, focused: Option<Window>, cmd: &ParsedCommand) -> Result<(), String> {
        match cmd {
            ParsedCommand::Workspace(name) => {
                self.switch_workspace_by_name(name)
                    .map_err(|e| format!("Workspace switch error: {}", e))?;
                if let Some(ref srv) = self.ipc_server {
                    srv.broadcast_event(
                        "workspace",
                        &format!("{{\"event\":\"workspace\",\"change\":\"focus\",\"current\":\"{}\"}}", name),
                    );
                }
            }
            ParsedCommand::WorkspaceBackAndForth => {
                self.switch_workspace_back_and_forth()
                    .map_err(|e| format!("Workspace switch error: {}", e))?;
            }
            ParsedCommand::WorkspaceNumber(num) => {
                let ws_id = self
                    .workspace_manager
                    .find_by_number(*num)
                    .map(|w| w.id)
                    .unwrap_or_else(|| {
                        self.workspace_manager.get_or_create_by_name(&num.to_string()).id
                    });
                self.workspace_manager.previous_workspace_id = Some(self.current_workspace);
                self.switch_workspace(ws_id)
                    .map_err(|e| format!("Workspace switch error: {}", e))?;
            }
            ParsedCommand::Split(split_type) => {
                if let Some(ws) = self.workspace_manager.get_mut(self.current_workspace) {
                    ws.bsp_tree.preselect = Some(*split_type);
                }
            }
            ParsedCommand::SplitToggle => {
                if let Some(ws) = self.workspace_manager.get_mut(self.current_workspace) {
                    ws.bsp_tree.preselect = match ws.bsp_tree.preselect {
                        Some(SplitType::Horizontal) => Some(SplitType::Vertical),
                        _ => Some(SplitType::Horizontal),
                    };
                }
            }
            ParsedCommand::Layout(layout) => {
                self.workspace_manager.set_current_layout(*layout);
            }
            ParsedCommand::LayoutToggle => {
                self.workspace_manager.cycle_current_layout();
            }
            ParsedCommand::FocusDirection(dir) => {
                let _ = self.focus_in_direction(*dir, false);
            }
            ParsedCommand::FocusNext => {
                let _ = self.focus_next_tab();
            }
            ParsedCommand::FocusPrev => {
                let _ = self.focus_prev_tab();
            }
            ParsedCommand::FocusParent | ParsedCommand::FocusChild => {
                // Sibling navigation / parent focus
                if let Some(w) = focused {
                    if let Some(ws) = self.workspace_manager.get(self.current_workspace) {
                        let wins = ws.bsp_tree.windows();
                        if let Some(pos) = wins.iter().position(|&x| x == w) {
                            let next = if pos + 1 < wins.len() { wins[pos + 1] } else { wins[0] };
                            let _ = self.focus_client(next);
                        }
                    }
                }
            }
            ParsedCommand::FocusModeToggle => {
                let is_curr_floating = focused
                    .and_then(|w| self.client_state.get_client(w))
                    .map(|c| c.is_floating())
                    .unwrap_or(false);
                let ws_id = self.current_workspace;
                if is_curr_floating {
                    if let Some(&first_tiled) = self.client_state.visible_tiled_clients_on_workspace(ws_id).first() {
                        let _ = self.focus_client(first_tiled);
                    }
                } else if let Some(&first_float) = self.client_state.floating_clients_on_workspace(ws_id).first() {
                    let _ = self.focus_client(first_float);
                }
            }
            ParsedCommand::MoveDirection(dir, px) => {
                if let Some(w) = focused {
                    let is_floating = self.client_state.get_client(w).map(|c| c.is_floating()).unwrap_or(false);
                    if is_floating {
                        let step = px.unwrap_or(30);
                        if let Some(c) = self.client_state.get_client_mut(w) {
                            if let Some(ref mut geom) = c.floating_geometry {
                                match dir {
                                    Direction::Left => geom.x -= step,
                                    Direction::Right => geom.x += step,
                                    Direction::Up => geom.y -= step,
                                    Direction::Down => geom.y += step,
                                }
                            }
                        }
                    } else {
                        let _ = self.swap_in_direction(*dir);
                    }
                }
            }
            ParsedCommand::MovePosition { center, x, y } => {
                if let Some(w) = focused {
                    if let Some(c) = self.client_state.get_client_mut(w) {
                        if let Some(ref mut geom) = c.floating_geometry {
                            if *center {
                                geom.x = (self.context.screen_width as i32 - geom.width as i32) / 2;
                                geom.y = (self.context.screen_height as i32 - geom.height as i32) / 2;
                            } else {
                                if let Some(nx) = x {
                                    geom.x = *nx;
                                }
                                if let Some(ny) = y {
                                    geom.y = *ny;
                                }
                            }
                        }
                    }
                }
            }
            ParsedCommand::MoveToWorkspace(name) => {
                if let Some(w) = focused {
                    self.move_window_to_workspace_name(w, name)
                        .map_err(|e| format!("Move to workspace error: {}", e))?;
                }
            }
            ParsedCommand::MoveToOutput(out_name) => {
                let target_mon = self
                    .monitor_manager
                    .monitors
                    .iter()
                    .find(|m| format!("output-{}", m.id) == *out_name || m.id.to_string() == *out_name)
                    .cloned();
                if let Some(mon) = target_mon {
                    self.move_to_monitor(mon.id)
                        .map_err(|e| format!("Move to monitor error: {}", e))?;
                }
            }
            ParsedCommand::MoveScratchpad => {
                if let Some(w) = focused {
                    self.set_scratchpad_client(w);
                }
            }
            ParsedCommand::SwapWithMark(mark) => {
                if let Some(target) = self.client_state.find_window_by_mark(mark) {
                    if let Some(w) = focused {
                        self.swap_windows(w, target)
                            .map_err(|e| format!("Swap error: {}", e))?;
                    }
                } else {
                    return Err(format!("No window found with mark '{}'", mark));
                }
            }
            ParsedCommand::SwapWithId(target_id) => {
                let target = *target_id as Window;
                if self.client_state.contains(target) {
                    if let Some(w) = focused {
                        self.swap_windows(w, target)
                            .map_err(|e| format!("Swap error: {}", e))?;
                    }
                } else {
                    return Err(format!("Window 0x{:x} not found", target));
                }
            }
            ParsedCommand::Fullscreen(mode) => {
                if let Some(w) = focused {
                    let is_full = self.client_state.get_client(w).map(|c| c.is_fullscreen()).unwrap_or(false);
                    let should_full = mode.unwrap_or(!is_full);
                    if should_full != is_full {
                        let _ = self.toggle_fullscreen(w);
                    }
                }
            }
            ParsedCommand::Floating(mode) => {
                if let Some(w) = focused {
                    let is_float = self.client_state.get_client(w).map(|c| c.is_floating()).unwrap_or(false);
                    let should_float = mode.unwrap_or(!is_float);
                    if should_float != is_float {
                        let _ = self.toggle_floating(w);
                    }
                }
            }
            ParsedCommand::Close => {
                if let Some(w) = focused {
                    let _ = self.kill_client(w);
                }
            }
            ParsedCommand::Kill => {
                if let Some(w) = focused {
                    let _ = self.context.connection.kill_client(w);
                }
            }
            ParsedCommand::Mark { name, add, toggle } => {
                if let Some(w) = focused {
                    self.client_state.mark_window(w, name, *add, *toggle);
                }
            }
            ParsedCommand::Unmark(name) => {
                if let Some(w) = focused {
                    self.client_state.unmark_window(w, name.as_deref());
                }
            }
            ParsedCommand::Exec(cmd) => {
                let _ = crate::process::spawn_detached_shell(cmd);
            }
            ParsedCommand::Reload | ParsedCommand::Restart => {
                let _ = self.reload_config();
            }
            ParsedCommand::Nop => {}
            _ => {}
        }
        Ok(())
    }
}
