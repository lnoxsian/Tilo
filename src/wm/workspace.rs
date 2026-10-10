//! Window Manager workspace, monitor, focus, and tiling management.

use super::WindowManager;
use crate::ewmh::{
    update_net_active_window, update_net_client_list, update_net_current_desktop,
};
use crate::floating::{calculate_default_floating_geometry, clamp_floating_geometry};
use crate::focus::{find_neighbor, set_window_border_color, Direction};
use crate::fullscreen::fullscreen_configure_aux;
use crate::icccm::WmState;
use crate::layout::{
    apply_layout_with_border, calculate_master_stack, calculate_monocle, LayoutType, Rect,
};
use crate::monitor::{query_monitors, MonitorId};
use crate::workspace::WorkspaceId;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, InputFocus, StackMode, Window,
};
use x11rb::wrapper::ConnectionExt as _;

impl WindowManager {
    /// Checks if screen dimensions or statusbar width have changed, triggering refresh if needed.
    pub fn check_screen_resize(&mut self) -> bool {
        let (geom_w, geom_h) = match self.context.connection.get_geometry(self.context.root_window) {
            Ok(cookie) => match cookie.reply() {
                Ok(geom) => (geom.width, geom.height),
                Err(_) => return false,
            },
            Err(_) => return false,
        };

        let bar_mismatch = self.statusbar.as_ref().map(|b| b.rect.width != geom_w as u32).unwrap_or(false);
        if geom_w != self.context.screen_width || geom_h != self.context.screen_height || bar_mismatch {
            let _ = self.refresh_monitors();
            return true;
        }
        false
    }

    /// Updates the root window's `_NET_ACTIVE_WINDOW` property.
    pub fn update_ewmh_active_window(&self) -> Result<(), X11Error> {
        let focused = self.focused_window();
        update_net_active_window(
            &self.context.connection,
            self.context.root_window,
            focused,
            &self.ewmh_atoms,
        )
    }

    /// Updates the root window's `_NET_CLIENT_LIST` property.
    pub fn update_ewmh_client_list(&self) -> Result<(), X11Error> {
        let clients = self.client_state.all_clients();
        update_net_client_list(
            &self.context.connection,
            self.context.root_window,
            clients,
            &self.ewmh_atoms,
        )
    }

    /// Updates the root window's `_NET_CURRENT_DESKTOP` property (0-indexed).
    pub fn update_ewmh_current_desktop(&self) -> Result<(), X11Error> {
        let desktop_idx = self.current_workspace.saturating_sub(1);
        update_net_current_desktop(
            &self.context.connection,
            self.context.root_window,
            desktop_idx,
            &self.ewmh_atoms,
        )
    }

    /// Executes configured autostart commands in the background without blocking.
    /// Computes the bounding rectangles for visible tiled windows on a workspace using its active layout.
    pub fn get_workspace_window_geometries(
        &mut self,
        ws_id: WorkspaceId,
        usable_rect: &Rect,
    ) -> Vec<(Window, Rect)> {
        let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(ws_id);
        if visible_tiled.is_empty() {
            return Vec::new();
        }

        if let Some(ws) = self.workspace_manager.get_mut(ws_id) {
            match ws.layout {
                LayoutType::Monocle | LayoutType::Tabbed | LayoutType::Stacked => {
                    let rects = calculate_monocle(&visible_tiled, usable_rect, &self.layout_config);
                    visible_tiled.into_iter().zip(rects).collect()
                }
                layout => {
                    let focused = self.workspace_focus.focused_window_on(ws_id);
                    ws.bsp_tree.sync_clients_with_layout(
                        layout,
                        &visible_tiled,
                        focused,
                        usable_rect,
                        self.layout_config.inner_gap,
                        self.layout_config.master_ratio,
                    );
                    ws.bsp_tree.calculate_rects(usable_rect, &self.layout_config)
                }
            }
        } else {
            let rects = calculate_master_stack(&visible_tiled, usable_rect, &self.layout_config);
            visible_tiled.into_iter().zip(rects).collect()
        }
    }

    /// Returns lightweight tab metadata for all managed tiled windows on the specified workspace.
    pub fn get_workspace_tabs(&self, ws_id: WorkspaceId) -> Vec<crate::bar::tabs::TabInfo> {
        let ws = match self.workspace_manager.get(ws_id) {
            Some(ws) => ws,
            None => return Vec::new(),
        };

        let focused = self.workspace_focus.focused_window_on(ws_id);
        let mut tabs = Vec::new();
        let mut idx = 1;

        for &w in &ws.clients {
            if let Some(c) = self.client_state.get_client(w) {
                if c.is_floating() {
                    continue;
                }
                let title = self.get_window_title(w);
                let is_focused = Some(w) == focused;
                tabs.push(crate::bar::tabs::TabInfo::new(w, title, idx, is_focused, c.urgent));
                idx += 1;
            }
        }

        tabs
    }

    /// Focuses the next tab in the active workspace tab list (with wrap-around).
    pub fn focus_next_tab(&mut self) -> Result<(), X11Error> {
        let current_ws = self.current_workspace;
        let tabs = self.get_workspace_tabs(current_ws);
        if tabs.len() < 2 {
            return Ok(());
        }
        let current_idx = tabs.iter().position(|t| t.focused).unwrap_or(0);
        let next_idx = (current_idx + 1) % tabs.len();
        self.focus_client(tabs[next_idx].window)?;
        Ok(())
    }

    /// Focuses the previous tab in the active workspace tab list (with wrap-around).
    pub fn focus_prev_tab(&mut self) -> Result<(), X11Error> {
        let current_ws = self.current_workspace;
        let tabs = self.get_workspace_tabs(current_ws);
        if tabs.len() < 2 {
            return Ok(());
        }
        let current_idx = tabs.iter().position(|t| t.focused).unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            tabs.len() - 1
        } else {
            current_idx - 1
        };
        self.focus_client(tabs[prev_idx].window)?;
        Ok(())
    }

    /// Moves the active tab one position to the left in workspace tab ordering.
    pub fn move_tab_left(&mut self) -> Result<(), X11Error> {
        let current_ws = self.current_workspace;
        let focused = match self.focused_window() {
            Some(w) => w,
            None => return Ok(()),
        };
        if let Some(ws) = self.workspace_manager.get_mut(current_ws)
            && let Some(idx) = ws.clients.iter().position(|&w| w == focused)
            && idx > 0
        {
            ws.clients.swap(idx, idx - 1);
        }
        let _ = self.render_statusbar();
        Ok(())
    }

    /// Moves the active tab one position to the right in workspace tab ordering.
    pub fn move_tab_right(&mut self) -> Result<(), X11Error> {
        let current_ws = self.current_workspace;
        let focused = match self.focused_window() {
            Some(w) => w,
            None => return Ok(()),
        };
        if let Some(ws) = self.workspace_manager.get_mut(current_ws)
            && let Some(idx) = ws.clients.iter().position(|&w| w == focused)
            && idx + 1 < ws.clients.len()
        {
            ws.clients.swap(idx, idx + 1);
        }
        let _ = self.render_statusbar();
        Ok(())
    }

    /// Directly selects and focuses a tab by 1-based index (e.g. 1..9).
    pub fn select_tab(&mut self, index: usize) -> Result<(), X11Error> {
        let current_ws = self.current_workspace;
        let tabs = self.get_workspace_tabs(current_ws);
        if index >= 1 && index <= tabs.len() {
            self.focus_client(tabs[index - 1].window)?;
        }
        Ok(())
    }

    /// Recalculates and applies the tiling layout to all visible managed clients on each monitor's active workspace.
    pub fn retile(&mut self) -> Result<(), X11Error> {
        let monitors = self.monitor_manager.monitors.clone();
        for monitor in &monitors {
            let mon_ws = monitor.workspace;
            let monitor_rect = monitor.rect();
            let usable_rect = if let Some(ref bar) = self.statusbar {
                if bar.rect.y <= monitor.y
                    && (bar.rect.y + bar.rect.height as i32) > monitor.y
                {
                    let offset = (bar.rect.y + bar.rect.height as i32) - monitor.y;
                    Rect::new(
                        monitor_rect.x,
                        monitor_rect.y + offset,
                        monitor_rect.width,
                        monitor_rect.height.saturating_sub(offset as u32),
                    )
                } else {
                    monitor_rect
                }
            } else {
                monitor_rect
            };

            let ws_layout = self.workspace_manager.get(mon_ws).map(|ws| ws.layout).unwrap_or_default();

            if ws_layout == LayoutType::Tabbed {
                let ws_clients = self.workspace_manager.get(mon_ws).map(|ws| ws.clients.clone()).unwrap_or_default();
                let tiled_clients: Vec<Window> = ws_clients
                    .into_iter()
                    .filter(|&w| {
                        self.client_state
                            .get_client(w)
                            .map(|c| !c.is_floating() && !c.is_fullscreen())
                            .unwrap_or(false)
                    })
                    .collect();

                let active_tab = self
                    .workspace_focus
                    .focused_window_on(mon_ws)
                    .filter(|w| tiled_clients.contains(w))
                    .or_else(|| tiled_clients.first().copied());

                for &w in &tiled_clients {
                    if Some(w) == active_tab {
                        if let Some(c) = self.client_state.get_client_mut(w) {
                            c.mapped = true;
                        }
                        let _ = self.context.connection.map_window(w);
                        let _ = self.set_wm_state(w, WmState::Normal);
                        let mut geom = usable_rect;
                        geom.width = geom.width.saturating_sub(2 * self.layout_config.border_width);
                        geom.height = geom.height.saturating_sub(2 * self.layout_config.border_width);
                        apply_layout_with_border(
                            &self.context.connection,
                            &[w],
                            &[geom],
                            self.layout_config.border_width,
                        )?;
                    } else if let Some(c) = self.client_state.get_client_mut(w)
                        && c.mapped {
                            c.mapped = false;
                            let _ = self.context.connection.unmap_window(w);
                            let _ = self.set_wm_state(w, WmState::Iconic);
                        }
                }
            } else {
                // Leaving or outside Tabbed layout: ensure all managed tiled windows on current workspace are mapped
                if mon_ws == self.current_workspace {
                    let ws_clients = self.workspace_manager.get(mon_ws).map(|ws| ws.clients.clone()).unwrap_or_default();
                    for &w in &ws_clients {
                        if let Some(c) = self.client_state.get_client_mut(w)
                            && !c.mapped && !c.is_floating() && !c.is_fullscreen() {
                                c.mapped = true;
                                let _ = self.context.connection.map_window(w);
                                let _ = self.set_wm_state(w, WmState::Normal);
                            }
                    }
                }

                let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(mon_ws);

                if !visible_tiled.is_empty() {
                    let pairs = self.get_workspace_window_geometries(mon_ws, &usable_rect);

                    let windows: Vec<Window> = pairs.iter().map(|(w, _)| *w).collect();
                    let geometries: Vec<Rect> = pairs.iter().map(|(_, r)| *r).collect();
                    apply_layout_with_border(
                        &self.context.connection,
                        &windows,
                        &geometries,
                        self.layout_config.border_width,
                    )?;

                    // If Monocle layout, raise the focused window above other tiled windows
                    if ws_layout == LayoutType::Monocle
                        && let Some(focused) = self.workspace_focus.focused_window_on(mon_ws) {
                            let _ = self.context.connection.configure_window(
                                focused,
                                &ConfigureWindowAux::default().stack_mode(StackMode::ABOVE),
                            );
                        }
                }
            }

            // Keep all mapped floating windows on this monitor's workspace stacked above tiled windows
            for &w in &self.client_state.floating_clients_on_workspace(mon_ws) {
                let _ = self.context.connection.configure_window(
                    w,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                );
            }

            // Keep all mapped fullscreen windows on this monitor's workspace filling this monitor and above
            for &w in &self.client_state.fullscreen_clients_on_workspace(mon_ws) {
                let aux = fullscreen_configure_aux(monitor);
                let _ = self.context.connection.configure_window(w, &aux);
            }
        }

        self.context.flush()?;
        Ok(())
    }

    /// Sets input focus to a specific client window, raises it, and updates border colors.
    pub fn focus_client(&mut self, window: Window) -> Result<(), X11Error> {
        if !self.client_state.contains(window) {
            return Ok(());
        }

        // 1. Reset all other windows' borders to normal color and configure passive click-to-focus grabs
        let other_windows: Vec<Window> = self
            .client_state
            .clients
            .keys()
            .copied()
            .filter(|&w| w != window)
            .collect();

        for w in other_windows {
            let _ = set_window_border_color(
                &self.context.connection,
                w,
                self.border_config.normal_color,
            );
            if let Some(c) = self.client_state.get_client_mut(w) {
                c.border_color = Some(self.border_config.normal_color);
            }
            let _ = crate::mouse::update_client_button_grabs(
                &self.context.connection,
                w,
                &self.mouse_bindings,
                self.input_manager.num_lock_mask,
                false,
            );
        }

        // 2. Set new window's border to active focused color
        let _ = set_window_border_color(
            &self.context.connection,
            window,
            self.border_config.focused_color,
        );
        if let Some(c) = self.client_state.get_client_mut(window) {
            c.border_color = Some(self.border_config.focused_color);
        }

        // 3. Update internal focus tracking first so retile() knows the new active window
        self.client_state.focus(window);
        let ws = self
            .client_state
            .get_client(window)
            .map(|c| c.workspace)
            .unwrap_or(self.current_workspace);
        self.workspace_focus.focus(ws, window);

        // In Tabbed layout, retile immediately to map the new tab and unmap inactive tabs
        if self.workspace_manager.get(ws).map(|w| w.layout) == Some(LayoutType::Tabbed) {
            let _ = self.retile();
        }

        // 4. Set X11 input focus
        let hints = crate::icccm::get_wm_hints(&self.context.connection, window)
            .ok()
            .flatten()
            .unwrap_or_default();

        if hints.input {
            let _ = self.context.connection.set_input_focus(
                InputFocus::POINTER_ROOT,
                window,
                x11rb::CURRENT_TIME,
            );
        }

        // 5. Raise window in stacking order
        let aux = ConfigureWindowAux::default().stack_mode(StackMode::ABOVE);
        let _ = self.context.connection.configure_window(window, &aux);

        // 6. Send WM_TAKE_FOCUS if supported by client (ICCCM)
        let _ = self.send_take_focus_if_supported(window);

        // 7. Update EWMH _NET_ACTIVE_WINDOW
        let _ = self.update_ewmh_active_window();

        // 8. Update statusbar window title if active
        let title = self.get_window_title(window);
        if let Some(c) = self.client_state.get_client_mut(window) {
            c.title = Some(title.clone());
        }
        if let Some(ref mut bar) = self.statusbar {
            bar.set_title(&title);
        }
        let _ = self.render_statusbar();

        // 9. Update mouse button grabs for focused window (transparent clicks)
        let _ = crate::mouse::update_client_button_grabs(
            &self.context.connection,
            window,
            &self.mouse_bindings,
            self.input_manager.num_lock_mask,
            true,
        );

        self.context.flush()?;
        Ok(())
    }

    /// Returns the currently focused window on the active workspace, if any.
    pub fn focused_window(&self) -> Option<Window> {
        self.client_state
            .clients
            .iter()
            .find(|(_, c)| c.focused && c.workspace == self.current_workspace && c.mapped)
            .map(|(&w, _)| w)
    }

    /// Clears input focus from all clients on the active screen, reverting to root window
    /// and clearing the statusbar title.
    pub fn clear_focus(&mut self) -> Result<(), X11Error> {
        // Reset all clients' focused flag and borders to normal
        for (&w, client) in self.client_state.clients.iter_mut() {
            if client.focused {
                client.focused = false;
                client.border_color = Some(self.border_config.normal_color);
                let _ = set_window_border_color(
                    &self.context.connection,
                    w,
                    self.border_config.normal_color,
                );
                let _ = crate::mouse::update_client_button_grabs(
                    &self.context.connection,
                    w,
                    &self.mouse_bindings,
                    self.input_manager.num_lock_mask,
                    false,
                );
            }
        }

        self.client_state.unfocus_all();
        self.workspace_focus.unfocus(self.current_workspace);

        // Revert X11 input focus to root
        let _ = self.context.connection.set_input_focus(
            InputFocus::POINTER_ROOT,
            self.context.root_window,
            x11rb::CURRENT_TIME,
        );

        // Update EWMH _NET_ACTIVE_WINDOW
        let _ = self.update_ewmh_active_window();

        // Clear statusbar window title
        if let Some(ref mut bar) = self.statusbar {
            bar.set_title("");
        }
        let _ = self.render_statusbar();

        self.context.flush()?;
        Ok(())
    }

    /// Queries the window title from X11 (`_NET_WM_NAME` or `WM_NAME`).
    pub fn get_window_title(&self, window: Window) -> String {
        if let Ok((_, _, Some(title))) = crate::rules::query_window_properties(
            &self.context.connection,
            &self.atoms,
            window,
        ) {
            title
        } else {
            String::new()
        }
    }

    /// Focuses the adjacent window in the given spatial direction on the active workspace.
    ///
    /// If `wrap` is true, wraps around to the opposite edge when no adjacent window exists.
    /// Focuses the adjacent window in the given spatial direction on the active workspace.
    ///
    /// If `wrap` is true, wraps around to the opposite edge when no adjacent window exists.
    pub fn focus_in_direction(
        &mut self,
        direction: Direction,
        wrap: bool,
    ) -> Result<Option<Window>, X11Error> {
        let current_ws = self.current_workspace;
        if self.workspace_manager.get(current_ws).map(|ws| ws.layout) == Some(LayoutType::Tabbed) {
            match direction {
                Direction::Right | Direction::Down => self.focus_next_tab()?,
                Direction::Left | Direction::Up => self.focus_prev_tab()?,
            }
            return Ok(self.focused_window());
        }
        let current = self.focused_window();
        let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
        if visible_tiled.is_empty() {
            return Ok(None);
        }

        let mon = self
            .monitor_manager
            .monitor_for_workspace(current_ws)
            .cloned()
            .unwrap_or_else(|| self.monitor_manager.active_monitor().clone());
        let monitor_rect = mon.rect();
        let usable_rect = if let Some(ref bar) = self.statusbar {
            if bar.rect.y <= mon.y
                && (bar.rect.y + bar.rect.height as i32) > mon.y
            {
                let offset = (bar.rect.y + bar.rect.height as i32) - mon.y;
                Rect::new(
                    monitor_rect.x,
                    monitor_rect.y + offset,
                    monitor_rect.width,
                    monitor_rect.height.saturating_sub(offset as u32),
                )
            } else {
                monitor_rect
            }
        } else {
            monitor_rect
        };
        let pairs = self.get_workspace_window_geometries(current_ws, &usable_rect);

        if let Some(target) = find_neighbor(direction, current, &pairs, wrap) {
            self.focus_client(target)?;
            Ok(Some(target))
        } else {
            Ok(None)
        }
    }

    /// Switches the currently active workspace to the specified target.
    ///
    /// Follows the Phase 8 switching algorithm:
    /// 1. Unmaps (hides) all windows on the current workspace.
    /// 2. Maps (shows) all windows on the target workspace.
    /// 3. Updates active workspace state.
    /// 4. Retiles the target workspace.
    /// 5. Restores previous focus where possible or reverts to root.
    pub fn switch_workspace(&mut self, target_id: WorkspaceId) -> Result<(), X11Error> {
        let current_id = self.workspace_manager.current_id();
        if target_id == current_id {
            return Ok(());
        }

        // 1. If target workspace is already assigned to a different monitor, switch focus to that monitor
        let target_mon_id = self
            .monitor_manager
            .monitor_for_workspace(target_id)
            .map(|m| m.id);

        if let Some(target_mon_id) = target_mon_id
            && target_mon_id != self.monitor_manager.active_monitor_id {
                self.monitor_manager.active_monitor_id = target_mon_id;
                self.workspace_manager.current_workspace_id = target_id;
                self.current_workspace = target_id;
                let _ = self.update_ewmh_current_desktop();

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(target_id)
                    .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.focused_client))
                    .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.clients.first().copied()));

                if let Some(focus_win) = target_focus.filter(|w| self.client_state.contains(*w)) {
                    self.focus_client(focus_win)?;
                } else {
                    self.clear_focus()?;
                }
                let _ = self.render_statusbar();
                self.context.flush()?;
                return Ok(());
            }

        // 2. Target workspace is not on any other monitor.
        // Active monitor replaces its workspace from current_id to target_id.
        let current_windows = self
            .workspace_manager
            .get(current_id)
            .map(|ws| ws.clients.clone())
            .unwrap_or_default();

        for &win in &current_windows {
            if let Some(client) = self.client_state.get_client_mut(win) {
                client.mapped = false;
            }
            let _ = self.context.connection.unmap_window(win);
            let _ = self.set_wm_state(win, WmState::Iconic);
        }

        self.workspace_manager.get_or_create(target_id);
        let target_windows = self
            .workspace_manager
            .get(target_id)
            .map(|ws| ws.clients.clone())
            .unwrap_or_default();

        let active_mon_id = self.monitor_manager.active_monitor_id;
        for &win in &target_windows {
            if let Some(client) = self.client_state.get_client_mut(win) {
                client.mapped = true;
                client.workspace = target_id;
                client.monitor = active_mon_id;
            }
            let _ = self.context.connection.map_window(win);
            let _ = self.set_wm_state(win, WmState::Normal);
        }

        // If scratchpad is currently visible, ensure it remains visible on target workspace
        if let Some(scratch_win) = self.scratchpad.client
            && self.scratchpad.is_visible {
                self.workspace_manager.move_client(scratch_win, target_id);
                if let Some(c) = self.client_state.get_client_mut(scratch_win) {
                    c.workspace = target_id;
                    c.mapped = true;
                }
                let _ = self.context.connection.map_window(scratch_win);
                let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
                let _ = self.context.connection.configure_window(scratch_win, &aux);
                let _ = self.set_wm_state(scratch_win, WmState::Normal);
            }

        self.monitor_manager.active_monitor_mut().workspace = target_id;
        self.workspace_manager.current_workspace_id = target_id;
        self.current_workspace = target_id;
        let _ = self.update_ewmh_current_desktop();
        if let Some(ref mut bar) = self.statusbar {
            let occupied = self.workspace_manager.occupied_workspaces();
            bar.set_workspace(target_id, &occupied);
        }

        self.retile()?;

        let target_focus = self
            .workspace_focus
            .focused_window_on(target_id)
            .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.focused_client))
            .or_else(|| target_windows.first().copied());

        if let Some(focus_win) = target_focus.filter(|w| self.client_state.contains(*w)) {
            self.focus_client(focus_win)?;
        } else {
            self.clear_focus()?;
        }

        let _ = self.render_statusbar();

        self.context.flush()?;
        Ok(())
    }

    /// Moves a client window to a target workspace, hiding it from the screen
    /// if the target workspace is inactive, or displaying it if active on another monitor.
    pub fn move_to_workspace(
        &mut self,
        window: Window,
        target_id: WorkspaceId,
    ) -> Result<(), X11Error> {
        let current_id = self
            .client_state
            .get_client(window)
            .map(|c| c.workspace)
            .or_else(|| self.workspace_manager.find_workspace_for(window))
            .unwrap_or(self.current_workspace);
        if current_id == target_id {
            return Ok(());
        }

        if self.scratchpad.is_scratchpad(window) {
            self.scratchpad.clear_client();
        }

        self.workspace_manager.move_client(window, target_id);

        let target_mon = self.monitor_manager.monitor_for_workspace(target_id).cloned();
        let target_is_visible = target_mon.is_some();

        if let Some(client) = self.client_state.get_client_mut(window) {
            client.workspace = target_id;
            if let Some(ref m) = target_mon {
                client.monitor = m.id;
            }
            client.mapped = target_is_visible;
        }

        if target_is_visible {
            let _ = self.context.connection.map_window(window);
            let _ = self.set_wm_state(window, WmState::Normal);
        } else {
            let _ = self.context.connection.unmap_window(window);
            let _ = self.set_wm_state(window, WmState::Iconic);
        }

        // Restore focus on current workspace
        let next_focus = self.workspace_focus.remove_window(current_id, window)
            .or_else(|| self.workspace_manager.get(current_id).and_then(|ws| ws.clients.first().copied()))
            .filter(|&w| self.client_state.contains(w));

        if let Some(next_win) = next_focus {
            let _ = self.focus_client(next_win);
        } else {
            let _ = self.clear_focus();
        }

        // Update statusbar tags
        if let Some(ref mut bar) = self.statusbar {
            let occupied = self.workspace_manager.occupied_workspaces();
            bar.set_workspace(self.current_workspace, &occupied);
        }
        let _ = self.render_statusbar();

        // Retile all workspaces
        self.retile()?;
        self.context.flush()?;
        Ok(())
    }

    /// Switches the active workspace, restoring the previously focused client on that workspace.
    pub fn set_current_workspace(&mut self, workspace_id: u32) -> Result<(), X11Error> {
        self.switch_workspace(workspace_id)
    }

    /// Checks if a workspace with the given ID exists.
    pub fn workspace_exists(&self, id: WorkspaceId) -> bool {
        self.workspace_manager.workspace_exists(id)
    }

    /// Returns the ID of the currently active workspace.
    pub fn current_workspace_id(&self) -> WorkspaceId {
        self.workspace_manager.current_id()
    }

    /// Focuses the monitor visually to the left of the active monitor.
    pub fn focus_monitor_left(&mut self) -> Result<(), X11Error> {
        if let Some(target_id) = self.monitor_manager.focus_monitor_left()
            && let Some(mon) = self.monitor_manager.get_monitor(target_id) {
                let ws = mon.workspace;
                self.current_workspace = ws;
                self.workspace_manager.current_workspace_id = ws;
                let _ = self.update_ewmh_current_desktop();
                if let Some(ref mut bar) = self.statusbar {
                    let occupied = self.workspace_manager.occupied_workspaces();
                    bar.set_workspace(ws, &occupied);
                }

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(ws)
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.focused_client))
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.clients.first().copied()));

                if let Some(w) = target_focus.filter(|w| self.client_state.contains(*w)) {
                    self.focus_client(w)?;
                } else {
                    self.clear_focus()?;
                }
            }
        self.context.flush()?;
        Ok(())
    }

    /// Focuses the monitor visually to the right of the active monitor.
    pub fn focus_monitor_right(&mut self) -> Result<(), X11Error> {
        if let Some(target_id) = self.monitor_manager.focus_monitor_right()
            && let Some(mon) = self.monitor_manager.get_monitor(target_id) {
                let ws = mon.workspace;
                self.current_workspace = ws;
                self.workspace_manager.current_workspace_id = ws;
                let _ = self.update_ewmh_current_desktop();
                if let Some(ref mut bar) = self.statusbar {
                    let occupied = self.workspace_manager.occupied_workspaces();
                    bar.set_workspace(ws, &occupied);
                }

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(ws)
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.focused_client))
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.clients.first().copied()));

                if let Some(w) = target_focus.filter(|w| self.client_state.contains(*w)) {
                    self.focus_client(w)?;
                } else {
                    self.clear_focus()?;
                }
            }
        self.context.flush()?;
        Ok(())
    }

    /// Moves the currently focused client to the specified monitor.
    pub fn move_to_monitor(&mut self, target_monitor_id: MonitorId) -> Result<(), X11Error> {
        let win = match self.focused_window() {
            Some(w) => w,
            None => return Ok(()),
        };

        let target_ws = match self.monitor_manager.get_monitor(target_monitor_id) {
            Some(m) => m.workspace,
            None => return Ok(()),
        };

        let current_ws = self.current_workspace;
        if current_ws == target_ws {
            return Ok(());
        }

        self.workspace_manager.move_client(win, target_ws);

        if let Some(client) = self.client_state.get_client_mut(win) {
            client.workspace = target_ws;
            client.monitor = target_monitor_id;
            client.mapped = true;
        }

        if let Some(client) = self.client_state.get_client(win)
            && client.is_floating()
                && let Some(target_mon) = self.monitor_manager.get_monitor(target_monitor_id) {
                    let mut geom = client.floating_geometry.unwrap_or_else(|| {
                        calculate_default_floating_geometry(
                            target_mon.width as u16,
                            target_mon.height as u16,
                        )
                    });
                    geom.x = target_mon.x + ((target_mon.width as i32 - geom.width as i32) / 2).max(0);
                    geom.y = target_mon.y + ((target_mon.height as i32 - geom.height as i32) / 2).max(0);
                    let aux = ConfigureWindowAux::new().x(geom.x).y(geom.y);
                    let _ = self.context.connection.configure_window(win, &aux);
                    if let Some(c_mut) = self.client_state.get_client_mut(win) {
                        c_mut.set_floating(geom);
                    }
                }

        let next_focus = self.workspace_focus.remove_window(current_ws, win)
            .or_else(|| self.workspace_manager.get(current_ws).and_then(|ws| ws.clients.first().copied()))
            .filter(|&w| self.client_state.contains(w));
        if let Some(next_win) = next_focus {
            let _ = self.focus_client(next_win);
        } else {
            let _ = self.clear_focus();
        }

        self.retile()?;
        self.context.flush()?;
        Ok(())
    }

    /// Refreshes monitor geometry by querying XRandR and reconciles state.
    pub fn refresh_monitors(&mut self) -> Result<(), X11Error> {
        // 1. Query the live root window geometry to get the actual current resolution
        if let Ok(reply) = self.context.connection.get_geometry(self.context.root_window)
            && let Ok(geom) = reply.reply() {
                self.context.screen_width = geom.width;
                self.context.screen_height = geom.height;
            }

        // 2. Query updated monitors from XRandR (using the updated screen dimensions as fallback)
        let new_monitors = match query_monitors(
            &self.context.connection,
            self.context.root_window,
            self.context.screen_width as u32,
            self.context.screen_height as u32,
        ) {
            Ok(monitors) => monitors,
            Err(err) => {
                eprintln!(
                    "[WARN  tilo::wm] Failed to query monitors from RandR: {:?}; using screen dimensions fallback",
                    err
                );
                vec![crate::monitor::Monitor::new_primary(
                    1,
                    0,
                    0,
                    self.context.screen_width as u32,
                    self.context.screen_height as u32,
                )]
            }
        };

        let disconnected = self.monitor_manager.reconcile_monitors(new_monitors);

        // For disconnected monitors, migrate their clients to the primary monitor's workspace
        let primary_ws = self.monitor_manager.active_monitor().workspace;
        for (_disc_id, old_ws) in disconnected {
            if old_ws != primary_ws {
                let clients = self.workspace_manager.get(old_ws).map(|ws| ws.clients.clone()).unwrap_or_default();
                for win in clients {
                    self.workspace_manager.move_client(win, primary_ws);
                    if let Some(client) = self.client_state.get_client_mut(win) {
                        client.workspace = primary_ws;
                        client.mapped = true;
                    }
                    let _ = self.context.connection.map_window(win);
                    let _ = self.set_wm_state(win, WmState::Normal);
                }
            }
        }

        // 3. Update EWMH _NET_DESKTOP_GEOMETRY
        let desktop_geom = [self.context.screen_width as u32, self.context.screen_height as u32];
        let _ = self.context.connection.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            self.context.root_window,
            self.ewmh_atoms.net_desktop_geometry,
            x11rb::protocol::xproto::AtomEnum::CARDINAL,
            &desktop_geom,
        );

        // 4. Update the Statusbar size and position to match the primary monitor (or new screen resolution)
        if let Some(ref mut bar) = self.statusbar {
            let primary_mon = self.monitor_manager.monitors.iter().find(|m| m.is_primary)
                .or_else(|| self.monitor_manager.monitors.first());
            let (bar_x, bar_y, bar_w) = if let Some(m) = primary_mon {
                (m.x, m.y, m.width)
            } else {
                (0, 0, self.context.screen_width as u32)
            };
            let new_bar_rect = Rect::new(bar_x, bar_y, bar_w, bar.rect.height);
            let screen = &self.context.connection.setup().roots[self.context.screen_num];
            let _ = bar.resize(&self.context.connection, new_bar_rect, screen.root_depth);
            let _ = self.render_statusbar();
        }

        // 5. If the application launcher is open, re-center it on the active monitor
        if let Some(ref mut launcher) = self.launcher
            && launcher.visible {
                let active_mon = self.monitor_manager.active_monitor().clone();
                let width = launcher.rect.width as u16;
                let height = launcher.rect.height as u16;
                let x = (active_mon.x + (active_mon.width as i32 - width as i32) / 2).max(active_mon.x) as i16;
                let y = (active_mon.y + (active_mon.height as i32 - height as i32) / 3).max(active_mon.y) as i16;
                launcher.rect.x = x as i32;
                launcher.rect.y = y as i32;
                let _ = self.context.connection.configure_window(
                    launcher.window_id,
                    &x11rb::protocol::xproto::ConfigureWindowAux::new().x(x as i32).y(y as i32),
                );
                let _ = launcher.render(&self.context.connection);
            }

        // 6. Clamp floating windows to new screen dimensions
        for client in self.client_state.clients.values_mut() {
            if client.floating
                && let Some(geom) = client.floating_geometry {
                    let clamped = clamp_floating_geometry(
                        geom,
                        self.context.screen_width,
                        self.context.screen_height,
                    );
                    client.floating_geometry = Some(clamped);
                    if client.mapped {
                        let aux = ConfigureWindowAux::new()
                            .x(clamped.x)
                            .y(clamped.y)
                            .width(clamped.width)
                            .height(clamped.height);
                        let _ = self.context.connection.configure_window(client.window, &aux);
                    }
                }
        }

        self.retile()?;
        self.context.flush()?;
        Ok(())
    }

    /// Swaps the currently focused window with its spatial neighbor in the given direction.
    pub fn swap_in_direction(&mut self, direction: Direction) -> Result<Option<Window>, X11Error> {
        let current_ws = self.current_workspace;
        if self.workspace_manager.get(current_ws).map(|ws| ws.layout) == Some(LayoutType::Tabbed) {
            match direction {
                Direction::Right | Direction::Down => self.move_tab_right()?,
                Direction::Left | Direction::Up => self.move_tab_left()?,
            }
            return Ok(self.focused_window());
        }
        let current = match self.focused_window() {
            Some(w) => w,
            None => return Ok(None),
        };
        let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
        if visible_tiled.len() < 2 {
            return Ok(None);
        }

        let mon = self
            .monitor_manager
            .monitor_for_workspace(current_ws)
            .cloned()
            .unwrap_or_else(|| self.monitor_manager.active_monitor().clone());
        let monitor_rect = mon.rect();
        let usable_rect = if let Some(ref bar) = self.statusbar {
            if bar.rect.y <= mon.y
                && (bar.rect.y + bar.rect.height as i32) > mon.y
            {
                let offset = (bar.rect.y + bar.rect.height as i32) - mon.y;
                Rect::new(
                    monitor_rect.x,
                    monitor_rect.y + offset,
                    monitor_rect.width,
                    monitor_rect.height.saturating_sub(offset as u32),
                )
            } else {
                monitor_rect
            }
        } else {
            monitor_rect
        };

        let pairs = self.get_workspace_window_geometries(current_ws, &usable_rect);

        let neighbor = find_neighbor(direction, Some(current), &pairs, false)
            .or_else(|| find_neighbor(direction, Some(current), &pairs, true));

        if let Some(target) = neighbor {
            if let Some(ws) = self.workspace_manager.get_mut(current_ws) {
                ws.bsp_tree.swap_windows(current, target);
                let ws_idx_a = ws.clients.iter().position(|&w| w == current);
                let ws_idx_b = ws.clients.iter().position(|&w| w == target);
                if let (Some(wa), Some(wb)) = (ws_idx_a, ws_idx_b) {
                    ws.clients.swap(wa, wb);
                }
            }
            let idx_a = self.client_state.client_order.iter().position(|&w| w == current);
            let idx_b = self.client_state.client_order.iter().position(|&w| w == target);
            if let (Some(a), Some(b)) = (idx_a, idx_b) {
                self.client_state.client_order.swap(a, b);
            }
            self.retile()?;
            self.focus_client(current)?;
            return Ok(Some(target));
        }
        Ok(None)
    }
}
