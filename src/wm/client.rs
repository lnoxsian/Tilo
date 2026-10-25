//! Window Manager client lifecycle and management.

use super::WindowManager;
use crate::client::{Client, ClientState};
use crate::ewmh::{update_net_wm_state_above, update_net_wm_state_fullscreen};
use crate::floating::{calculate_default_floating_geometry, clamp_floating_geometry};
use crate::focus::{set_window_border_color, set_window_border_width};
use crate::fullscreen::{fullscreen_configure_aux, restore_configure_aux, Geometry};
use crate::icccm::{
    get_wm_state, send_wm_delete as icccm_send_wm_delete,
    set_wm_state as icccm_set_wm_state, IcccmAtoms, WmState,
};
use crate::rules::{apply_rules as apply_window_rules, RuleAction, WindowRule};
use crate::x11::X11Error;
use crate::error::{is_connection_lost_error, WmError};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{
    ButtonIndex, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask,
    InputFocus, MapState, ModMask, StackMode, Window,
};

impl WindowManager {
    // -- Scratchpad (Phase 23) --

    /// Assigns a client window to be the scratchpad, ensuring it floats.
    pub fn set_scratchpad_client(&mut self, window: Window) {
        crate::scratchpad::set_scratchpad_client(&mut self.scratchpad, window);
        if let Some(c) = self.client_state.get_client_mut(window) {
            c.floating = true;
        }
        self.scratchpad.is_visible = true;
    }

    /// Toggles the scratchpad window between hidden and centered visible floating state.
    ///
    /// Follows Phase 23:
    /// - If no scratchpad is set, spawns the default terminal emulator.
    /// - If visible: unmaps window, marks iconic, restores previous focus.
    /// - If hidden: moves to current workspace, centers on active monitor, maps, raises ABOVE, focuses.
    pub fn toggle_scratchpad(&mut self) -> Result<(), X11Error> {
        if let Some(win) = self.scratchpad.client
            && !self.client_state.contains(win) {
                self.scratchpad.clear_client();
            }

        let win = match self.scratchpad.client {
            Some(w) => w,
            None => {
                // Spawn default terminal emulator as scratchpad
                let _ = crate::launcher::spawn_app(&self.config.terminal);
                return Ok(());
            }
        };
        if self.scratchpad.is_visible {
            // Hide: unmap window
            let _ = self.context.connection.unmap_window(win);
            self.scratchpad.is_visible = false;
            if let Some(client) = self.client_state.get_client_mut(win) {
                client.mapped = false;
            }
            let _ = self.set_wm_state(win, WmState::Iconic);

            // Restore focus if scratchpad had focus
            if self.focused_window() == Some(win) {
                let focus_candidate = self
                    .workspace_manager
                    .get(self.current_workspace)
                    .and_then(|ws| ws.clients.iter().copied().find(|&w| w != win));

                if let Some(next_focus) = focus_candidate {
                    let _ = self.focus_client(next_focus);
                } else {
                    let _ = self.clear_focus();
                }
            }
        } else {
            // Show: center on active monitor, map, raise ABOVE, focus
            let mon = self.monitor_manager.active_monitor().clone();
            let geom = crate::scratchpad::calculate_scratchpad_geometry(
                mon.x,
                mon.y,
                mon.width,
                mon.height,
            );

            if let Some(client) = self.client_state.get_client_mut(win) {
                client.workspace = self.current_workspace;
                client.mapped = true;
                client.floating = true;
                client.floating_geometry = Some(crate::floating::FloatingGeometry::new(
                    geom.x,
                    geom.y,
                    geom.width,
                    geom.height,
                ));
            }

            self.workspace_manager.add_client(win, self.current_workspace);

            let aux = ConfigureWindowAux::new()
                .x(geom.x)
                .y(geom.y)
                .width(geom.width)
                .height(geom.height)
                .border_width(self.border_config.width)
                .stack_mode(StackMode::ABOVE);
            let _ = self.context.connection.configure_window(win, &aux);
            let _ = self.context.connection.map_window(win);
            let _ = self.set_wm_state(win, WmState::Normal);
            let _ = update_net_wm_state_above(
                &self.context.connection,
                win,
                true,
                &self.ewmh_atoms,
            );
            let _ = self.focus_client(win);
            self.scratchpad.is_visible = true;
        }

        self.context.flush()?;
        Ok(())
    }

    // -- Client Management (Phase 4) --

    /// Registers a newly requested window to be managed by the window manager.
    ///
    /// Follows the Phase 4 client adoption algorithm:
    /// 1. Checks if the window is already tracked.
    /// 2. Queries window attributes, ignoring `override_redirect` windows.
    /// 3. Subscribes to window property changes and structure notifications.
    /// 4. Creates and stores a `Client` record in `client_state`.
    /// 5. Maps the window and sets input focus.
    ///
    /// # Returns
    /// `Ok(true)` if the window was newly managed, `Ok(false)` if it was skipped or already managed.
    pub fn manage_window(&mut self, window: Window) -> Result<bool, X11Error> {
        if self.client_state.contains(window) {
            return Ok(false);
        }

        // Check window attributes (e.g. override_redirect)
        let attrs = match self.context.connection.get_window_attributes(window) {
            Ok(cookie) => match cookie.reply() {
                Ok(attrs) => attrs,
                Err(err) => {
                    eprintln!(
                        "[WARN  tilo::wm] Failed to get attributes for window 0x{:x}: {:?}",
                        window, err
                    );
                    return Ok(false);
                }
            },
            Err(err) => {
                return Err(X11Error::Connection(err));
            }
        };

        if attrs.override_redirect {
            return Ok(false);
        }

        // Subscribe to events on the client window
        let aux = ChangeWindowAttributesAux::default().event_mask(
            EventMask::PROPERTY_CHANGE
                | EventMask::STRUCTURE_NOTIFY
                | EventMask::ENTER_WINDOW,
        );
        let _ = self
            .context
            .connection
            .change_window_attributes(window, &aux);

        // Evaluate window rules before finalizing client state
        let rule_action = match self.evaluate_window_rules(window) {
            Ok(action) => action,
            Err(err) => {
                if is_connection_lost_error(&err) {
                    return Err(err);
                }
                RuleAction::default()
            }
        };
        let active_mon = self.monitor_manager.active_monitor();
        let target_ws = rule_action.target_workspace.unwrap_or(active_mon.workspace);
        self.workspace_manager.get_or_create(target_ws);

        let target_mon_id = self
            .monitor_manager
            .monitor_for_workspace(target_ws)
            .map(|m| m.id)
            .unwrap_or(active_mon.id);

        // Construct client state
        let mut client = Client::new(window);
        client.workspace = target_ws;
        client.monitor = target_mon_id;

        if rule_action.floating == Some(true) {
            let target_mon = self
                .monitor_manager
                .get_monitor(target_mon_id)
                .unwrap_or(active_mon);
            let mut geom = calculate_default_floating_geometry(
                target_mon.width as u16,
                target_mon.height as u16,
            );
            geom.x += target_mon.x;
            geom.y += target_mon.y;
            client.set_floating(geom);

            let aux = ConfigureWindowAux::new()
                .x(geom.x)
                .y(geom.y)
                .width(geom.width)
                .height(geom.height)
                .border_width(self.border_config.width)
                .stack_mode(StackMode::ABOVE);
            let _ = self.context.connection.configure_window(window, &aux);
        }

        if rule_action.fullscreen == Some(true) {
            client.fullscreen = true;
        }

        // Set initial border width and normal (unfocused) border color
        let _ = set_window_border_width(
            &self.context.connection,
            window,
            self.border_config.width,
        );
        let _ = set_window_border_color(
            &self.context.connection,
            window,
            self.border_config.normal_color,
        );
        client.border_color = Some(self.border_config.normal_color);

        let is_current_ws = target_ws == self.current_workspace;
        let is_floating = client.floating;
        let is_fullscreen = client.fullscreen;

        if is_current_ws {
            client.mapped = true;
            self.workspace_manager.add_client(window, target_ws);
            self.client_state.add_client(window, client);

            if is_floating {
                let _ = update_net_wm_state_above(&self.context.connection, window, true, &self.ewmh_atoms);
            }
            if is_fullscreen {
                let _ = update_net_wm_state_fullscreen(&self.context.connection, window, true, &self.ewmh_atoms);
            }

            // Retile active workspace FIRST so the window is positioned before being mapped
            self.retile()?;
            self.map_client(window)?;
            let _ = self.set_wm_state(window, WmState::Normal);
            self.focus_client(window)?;
        } else {
            // Target is a background workspace: window remains unmapped until switched to
            let _ = self.context.connection.unmap_window(window);
            client.mapped = false;
            self.workspace_manager.add_client(window, target_ws);
            self.client_state.add_client(window, client);
            self.client_state.focus_stack.retain(|&w| w != window);
            if is_floating {
                let _ = update_net_wm_state_above(&self.context.connection, window, true, &self.ewmh_atoms);
            }
            if is_fullscreen {
                let _ = update_net_wm_state_fullscreen(&self.context.connection, window, true, &self.ewmh_atoms);
            }
            let _ = self.set_wm_state(window, WmState::Withdrawn);
            let _ = self.render_statusbar();
        }

        let is_focused = self.focused_window() == Some(window);
        let _ = crate::mouse::update_client_button_grabs(
            &self.context.connection,
            window,
            &self.mouse_bindings,
            self.input_manager.num_lock_mask,
            is_focused,
        );
        let _ = self.update_ewmh_client_list();
        self.context.flush()?;
        Ok(true)
    }

    /// Scans the X11 root window tree and manages pre-existing windows.
    ///
    /// Follows Phase 31 of Plan.md:
    /// 1. Query root tree.
    /// 2. Identify existing top-level windows.
    /// 3. Ignore override-redirect and unmapped windows.
    /// 4. Register valid clients via `manage_window`.
    /// 5. Apply rules (workspace assignment, floating, fullscreen).
    /// 6. Assign workspaces & configure geometries (unmap background workspace clients).
    /// 7. Map/configure clients on active workspace, set `WM_STATE`.
    /// 8. Focus the appropriate active window.
    pub fn manage_existing_windows(&mut self) -> Result<(), WmError> {
        let root = self.context.root_window;
        let tree = self
            .context
            .connection
            .query_tree(root)
            .map_err(|e| WmError::ConnectionLost(e.to_string()))?
            .reply()
            .map_err(|e| WmError::X11ProtocolError(crate::x11::X11Error::Reply(e)))?;

        for child in tree.children {
            if child == root || child == self.supporting_window {
                continue;
            }
            if self
                .statusbar
                .as_ref()
                .map(|b| b.window_id == child)
                .unwrap_or(false)
            {
                continue;
            }
            if self
                .launcher
                .as_ref()
                .map(|l| l.window_id == child)
                .unwrap_or(false)
            {
                continue;
            }

            let attrs = match self.context.connection.get_window_attributes(child) {
                Ok(cookie) => match cookie.reply() {
                    Ok(attrs) => attrs,
                    Err(err) => {
                        let xerr = crate::x11::X11Error::Reply(err);
                        if is_connection_lost_error(&xerr) {
                            return Err(WmError::ConnectionLost(xerr.to_string()));
                        }
                        continue;
                    }
                },
                Err(err) => return Err(WmError::ConnectionLost(err.to_string())),
            };

            if attrs.override_redirect || attrs.map_state == MapState::UNMAPPED {
                continue;
            }

            if let Err(err) = self.manage_window(child) {
                if is_connection_lost_error(&err) {
                    return Err(WmError::ConnectionLost(err.to_string()));
                }
                eprintln!(
                    "[WARN  tilo::wm] Failed to manage existing window 0x{:x}: {:?}",
                    child, err
                );
            }
        }

        let _ = self.retile();
        let _ = self.render_statusbar();
        let _ = self.context.flush();
        Ok(())
    }

    /// Returns the PID associated with a client window via `_NET_WM_PID` if set.
    pub fn get_client_pid(&self, window: Window) -> Option<u32> {
        crate::x11::ewmh::get_wm_pid(
            &self.context.connection,
            window,
            self.ewmh_atoms.net_wm_pid,
        ).ok().flatten()
    }

    /// Closes all running application windows gracefully using ICCCM `WM_DELETE_WINDOW`,
    /// waiting for clients to terminate, and falling back to forceful killing if they do not
    /// exit within `timeout`.
    pub fn close_all_clients(&mut self, timeout: Duration) -> Result<(), WmError> {
        let mut all_windows: Vec<Window> = self.client_state.all_clients().to_vec();
        if let Some(sp_win) = self.scratchpad.client {
            if !all_windows.contains(&sp_win) {
                all_windows.push(sp_win);
            }
        }

        if all_windows.is_empty() {
            return Ok(());
        }

        let mut pids: HashMap<Window, Option<u32>> = HashMap::new();
        for &win in &all_windows {
            pids.insert(win, self.get_client_pid(win));
        }

        // 1. Send graceful close request (WM_DELETE_WINDOW) to all client windows
        for &win in &all_windows {
            let _ = self.send_wm_delete(win);
        }
        let _ = self.context.flush();

        // 2. Event polling wait loop
        let start = Instant::now();
        let my_pid = std::process::id();

        while !self.client_state.is_empty() && start.elapsed() < timeout {
            let _ = self.process_pending_events();
            if self.client_state.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // 3. Forceful kill fallback for any stubborn clients
        let remaining: Vec<Window> = self.client_state.all_clients().to_vec();
        if !remaining.is_empty() {
            for &win in &remaining {
                let _ = self.context.connection.kill_client(win);
                if let Some(Some(pid)) = pids.get(&win) {
                    if *pid != my_pid && *pid > 1 {
                        unsafe {
                            libc::kill(*pid as i32, libc::SIGTERM);
                        }
                    }
                }
            }
            let _ = self.context.flush();

            // Brief drain period (up to 300ms)
            let drain_start = Instant::now();
            while !self.client_state.is_empty() && drain_start.elapsed() < Duration::from_millis(300) {
                let _ = self.process_pending_events();
                if self.client_state.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }

            // Final SIGKILL for remaining processes
            for &win in &remaining {
                if let Some(Some(pid)) = pids.get(&win) {
                    if *pid != my_pid && *pid > 1 {
                        unsafe {
                            if libc::kill(*pid as i32, 0) == 0 {
                                libc::kill(*pid as i32, libc::SIGKILL);
                            }
                        }
                    }
                }
            }
        }

        if self.scratchpad.client.is_some() {
            self.scratchpad.clear_client();
        }
        self.client_state = ClientState::new();
        for ws in self.workspace_manager.workspaces.values_mut() {
            ws.clients.clear();
            ws.focused_client = None;
        }

        Ok(())
    }

    /// Executes a full desktop environment shutdown:
    /// 1. Closes all running applications gracefully via ICCCM `WM_DELETE_WINDOW`.
    /// 2. Waits up to `timeout` (or `config.shutdown_timeout_ms`) for apps to exit.
    /// 3. Forcefully terminates any unresponsive clients (`KillClient` / `SIGKILL`).
    /// 4. Releases WM resources, destroys statusbar and launcher, clears root properties and grabs.
    /// 5. Reverts focus and stops the main event loop.
    pub fn perform_desktop_shutdown(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<(), WmError> {
        self.running = false;
        let timeout_duration = timeout.unwrap_or_else(|| {
            Duration::from_millis(self.config.shutdown_timeout_ms)
        });

        let _ = self.close_all_clients(timeout_duration);
        self.perform_graceful_shutdown()?;

        Ok(())
    }

    /// Executes the graceful shutdown sequence (Phase 32).
    ///
    /// Before exiting:
    /// 1. Stops launcher.
    /// 2. Removes statusbar.
    /// 3. Restores client borders (`border_width = 0`).
    /// 4. Ensures all application windows remain mapped so they are not lost.
    /// 5. Removes WM properties (`WM_STATE`, `_NET_WM_STATE`).
    /// 6. Releases `WM_Sn` selection ownership and destroys supporting window.
    /// 7. Releases root window event masks and grabs.
    /// 8. Sets input focus to `PointerRoot`.
    /// 9. Flushes X11 connection.
    /// 10. Terminates main event loop.
    pub fn perform_graceful_shutdown(&mut self) -> Result<(), WmError> {
        // 1. Stop launcher
        if let Some(ref mut launcher) = self.launcher
            && launcher.window_id != 0
        {
            let _ = self.context.connection.unmap_window(launcher.window_id);
            let _ = self.context.connection.destroy_window(launcher.window_id);
            launcher.window_id = 0;
            launcher.visible = false;
        }

        // 2. Remove statusbar
        if let Some(bar) = self.statusbar.take() {
            let _ = bar.destroy(&self.context.connection);
        }

        // 3 & 4. Iterate over all managed clients: restore borders, map background windows, remove properties
        let all_clients: Vec<Window> = self.client_state.all_clients().to_vec();
        for win in all_clients {
            let _ = set_window_border_width(&self.context.connection, win, 0);
            let _ = self.context.connection.delete_property(win, self.atoms.wm_state);
            let _ = self.context.connection.delete_property(win, self.ewmh_atoms.net_wm_state);
            let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::NO_EVENT);
            let _ = self.context.connection.change_window_attributes(win, &aux);

            if self.scratchpad.client == Some(win) && !self.scratchpad.is_visible {
                let _ = self.context.connection.unmap_window(win);
            } else {
                let _ = self.context.connection.map_window(win);
            }
        }

        if self.scratchpad.client.is_some() {
            self.scratchpad.clear_client();
        }

        self.client_state = ClientState::new();
        for ws in self.workspace_manager.workspaces.values_mut() {
            ws.clients.clear();
            ws.focused_client = None;
        }

        // 5. Release WM_Sn selection ownership
        let sel_name = format!("WM_S{}", self.context.screen_num);
        if let Ok(cookie) = self.context.connection.intern_atom(false, sel_name.as_bytes())
            && let Ok(reply) = cookie.reply()
        {
            let _ = self.context.connection.set_selection_owner(x11rb::NONE, reply.atom, x11rb::CURRENT_TIME);
        }

        // Release EWMH properties and supporting window
        let root = self.context.root_window;
        let _ = self.context.connection.delete_property(root, self.ewmh_atoms.net_supporting_wm_check);
        let _ = self.context.connection.delete_property(root, self.ewmh_atoms.net_supported);
        let _ = self.context.connection.delete_property(root, self.ewmh_atoms.net_client_list);
        let _ = self.context.connection.delete_property(root, self.ewmh_atoms.net_active_window);
        let _ = self.context.connection.delete_property(root, self.ewmh_atoms.net_workarea);

        if self.supporting_window != 0 {
            let _ = self.context.connection.destroy_window(self.supporting_window);
            self.supporting_window = 0;
        }

        let _ = self.context.connection.ungrab_key(0, root, ModMask::ANY);
        let _ = self.context.connection.ungrab_button(ButtonIndex::ANY, root, ModMask::ANY);
        let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::NO_EVENT);
        let _ = self.context.connection.change_window_attributes(root, &aux);

        let _ = self.context.connection.set_input_focus(InputFocus::POINTER_ROOT, root, x11rb::CURRENT_TIME);
        let _ = self.context.flush();
        self.running = false;
        Ok(())
    }

    /// Deregisters and unmanages a window, removing it from `client_state`.
    ///
    /// Implements Phase 29 Application State Recovery:
    /// - Gracefully handles workspace changes during client destruction: if the destroyed client
    ///   is on an inactive workspace, does not alter the active workspace's focus or layout.
    /// - If the window was active on the current workspace, transfers focus to the previous window
    ///   on the workspace/focus stack or reverts to the root window.
    ///
    /// Unmanages a client window.
    ///
    /// If `set_withdrawn` is true (e.g. on unmap), updates `WM_STATE` to `Withdrawn`.
    /// When called from `handle_destroy_notify`, `set_withdrawn` should be false because
    /// the window has already been destroyed on the X server.
    pub fn unmanage_window(&mut self, window: Window) -> Option<Client> {
        self.unmanage_window_internal(window, true)
    }

    /// Unmanages a destroyed client window without attempting to set `WM_STATE` on a destroyed window ID.
    pub fn unmanage_destroyed_window(&mut self, window: Window) -> Option<Client> {
        self.unmanage_window_internal(window, false)
    }

    fn unmanage_window_internal(&mut self, window: Window, set_withdrawn: bool) -> Option<Client> {
        // Ignore unmanaged / internal windows (e.g. launcher, statusbar, helper windows)
        if !self.client_state.contains(window) {
            return None;
        }

        let ws = self
            .workspace_manager
            .find_workspace_for(window)
            .unwrap_or(self.current_workspace);
        self.workspace_manager.remove_client(window);
        let next_ws_win = self.workspace_focus.remove_window(ws, window);
        self.workspace_focus.remove_window_all(window);

        let removed = self.client_state.remove_client(window);
        if self.scratchpad.client == Some(window) {
            self.scratchpad.clear_client();
        }
        if set_withdrawn {
            let _ = self.set_wm_state(window, WmState::Withdrawn);
        }
        let _ = self.update_ewmh_client_list();
        if removed.is_some() {
            // Only retile and shift focus if the destroyed window belonged to the current workspace
            if ws == self.current_workspace {
                let _ = self.retile();
                let next_candidate = next_ws_win
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.clients.first().copied()))
                    .filter(|&w| self.client_state.contains(w));
                if let Some(next_window) = next_candidate {
                    let _ = self.focus_client(next_window);
                } else {
                    let _ = self.clear_focus();
                }
            } else if self.focused_window() == Some(window) {
                let _ = self.clear_focus();
            } else {
                let _ = self.render_statusbar();
            }
            #[cfg(target_os = "linux")]
            unsafe {
                libc::malloc_trim(0);
            }
        }
        removed
    }



    /// Adds a window rule to the window manager.
    pub fn add_rule(&mut self, rule: WindowRule) {
        self.rules.push(rule);
    }

    /// Evaluates window rules for a window using the currently configured rules.
    pub fn evaluate_window_rules(&self, window: Window) -> Result<RuleAction, X11Error> {
        apply_window_rules(&self.context.connection, &self.atoms, window, &self.rules)
    }

    /// Evaluates window rules for a newly mapped client and applies the resulting actions.
    ///
    /// # Arguments
    /// * `window` - The newly requested window.
    /// * `rules` - A slice of configured `WindowRule` objects.
    ///
    /// # Returns
    /// * `Result<RuleAction, X11Error>` - Returns a combined `RuleAction` containing all applied overrides.
    pub fn apply_rules(
        &self,
        window: Window,
        rules: &[WindowRule],
    ) -> Result<RuleAction, X11Error> {
        apply_window_rules(&self.context.connection, &self.atoms, window, rules)
    }

    /// Returns the cached atoms needed for ICCCM operations.
    pub fn icccm_atoms(&self) -> IcccmAtoms {
        IcccmAtoms::from(&self.atoms)
    }

    /// Sets the `WM_STATE` property on a client window.
    pub fn set_wm_state(&self, window: Window, state: WmState) -> Result<(), X11Error> {
        let atoms = self.icccm_atoms();
        icccm_set_wm_state(&self.context.connection, window, state, &atoms)
    }

    /// Gets the current `WM_STATE` property from a client window.
    pub fn get_wm_state(&self, window: Window) -> Result<Option<WmState>, X11Error> {
        let atoms = self.icccm_atoms();
        get_wm_state(&self.context.connection, window, &atoms)
    }

    /// Attempts to close a client window gracefully via ICCCM `WM_DELETE_WINDOW`,
    /// falling back to `KillClient` if unsupported.
    pub fn send_wm_delete(&self, window: Window) -> Result<(), X11Error> {
        let atoms = self.icccm_atoms();
        icccm_send_wm_delete(&self.context.connection, window, &atoms)
    }

    /// Toggles the floating state of a specific window.
    ///
    /// If the window is currently tiled, it will be made floating, restoring its previous 
    /// floating geometry or using a default centered geometry. If it is floating, it will
    /// be tiled.
    pub fn toggle_floating(&mut self, window: Window) -> Result<(), X11Error> {
        let is_currently_floating = self
            .client_state
            .get_client(window)
            .map(|c| c.floating)
            .unwrap_or(false);

        if !is_currently_floating {
            // Tiled -> Floating: Restore previous floating geometry or calculate default centered geometry
            let geom = if let Some(client) = self.client_state.get_client(window) {
                client.floating_geometry.unwrap_or_else(|| {
                    calculate_default_floating_geometry(
                        self.context.screen_width,
                        self.context.screen_height,
                    )
                })
            } else {
                return Ok(());
            };

            if let Some(client) = self.client_state.get_client_mut(window) {
                client.set_floating(geom);
            }

            // Apply floating geometry, border width, and raise above tiled windows
            let aux = ConfigureWindowAux::new()
                .x(geom.x)
                .y(geom.y)
                .width(geom.width)
                .height(geom.height)
                .border_width(self.border_config.width)
                .stack_mode(StackMode::ABOVE);
            let _ = self.context.connection.configure_window(window, &aux);

            // Retile the active workspace (remaining tiled windows expand to fill space)
            self.retile()?;

            // Set focus and raise
            self.focus_client(window)?;
        } else {
            // Floating -> Tiled: Transition back to tiled layout
            if let Some(client) = self.client_state.get_client_mut(window) {
                client.set_tiled();
            }

            // Recalculate tiling layout for the active workspace
            self.retile()?;

            // Refocus window
            self.focus_client(window)?;
        }

        let is_floating_now = self.client_state.get_client(window).map(|c| c.floating).unwrap_or(false);
        let _ = update_net_wm_state_above(&self.context.connection, window, is_floating_now, &self.ewmh_atoms);

        self.context.flush()?;
        Ok(())
    }

    /// Toggles the fullscreen state of a specific window.
    pub fn toggle_fullscreen(&mut self, window: Window) -> Result<(), X11Error> {
        let is_fs = self
            .client_state
            .get_client(window)
            .map(|c| c.is_fullscreen())
            .unwrap_or(false);
        self.set_fullscreen(window, !is_fs)
    }

    /// Explicitly sets the fullscreen state of a window.
    pub fn set_fullscreen(&mut self, window: Window, fullscreen: bool) -> Result<(), X11Error> {
        let current_fs = match self.client_state.get_client(window) {
            Some(c) => c.is_fullscreen(),
            None => return Ok(()),
        };

        if current_fs == fullscreen {
            return Ok(());
        }

        let monitor = if let Some(client) = self.client_state.get_client(window) {
            self.monitor_manager
                .monitor_for_workspace(client.workspace)
                .cloned()
                .unwrap_or_else(|| self.monitor_manager.active_monitor().clone())
        } else {
            self.monitor_manager.active_monitor().clone()
        };

        if fullscreen {
            // Save client's current x, y, width, height and border_width
            let geom_reply = self.context.connection.get_geometry(window)?.reply()?;
            let prev_geom = Geometry::new(
                geom_reply.x as i32,
                geom_reply.y as i32,
                geom_reply.width as u32,
                geom_reply.height as u32,
            );
            let prev_border = geom_reply.border_width as u32;

            if let Some(client) = self.client_state.get_client_mut(window) {
                client.set_fullscreen(prev_geom, prev_border);
            }

            let aux = fullscreen_configure_aux(&monitor);
            self.context.connection.configure_window(window, &aux)?;
            update_net_wm_state_fullscreen(
                &self.context.connection,
                window,
                true,
                &self.ewmh_atoms,
            )?;
        } else {
            // Restore previous geometry and border width
            let restored = if let Some(client) = self.client_state.get_client_mut(window) {
                client.unset_fullscreen()
            } else {
                None
            };

            if let Some((prev_geom, prev_border)) = restored {
                let aux = restore_configure_aux(prev_geom, prev_border);
                let _ = self.context.connection.configure_window(window, &aux);
            }

            update_net_wm_state_fullscreen(
                &self.context.connection,
                window,
                false,
                &self.ewmh_atoms,
            )?;
        }

        // Recalculate layout for the active workspace
        self.retile()?;
        self.focus_client(window)?;
        self.context.flush()?;
        Ok(())
    }

    /// Moves a floating window to a new absolute position.
    pub fn move_floating(&mut self, window: Window, x: i32, y: i32) -> Result<(), X11Error> {
        let is_tiled = match self.client_state.get_client(window) {
            Some(c) => !c.floating,
            None => return Ok(()),
        };

        if is_tiled {
            self.toggle_floating(window)?;
        }

        let mut geom = match self.client_state.get_client(window) {
            Some(client) => client.floating_geometry.unwrap_or_else(|| {
                calculate_default_floating_geometry(
                    self.context.screen_width,
                    self.context.screen_height,
                )
            }),
            None => return Ok(()),
        };

        geom.x = x;
        geom.y = y;
        let clamped = clamp_floating_geometry(
            geom,
            self.context.screen_width,
            self.context.screen_height,
        );

        if let Some(client) = self.client_state.get_client_mut(window) {
            client.set_floating(clamped);
        }

        let aux = ConfigureWindowAux::new()
            .x(clamped.x)
            .y(clamped.y)
            .stack_mode(StackMode::ABOVE);
        self.context.connection.configure_window(window, &aux)?;
        self.context.flush()?;
        Ok(())
    }

    /// Resizes a floating window to new dimensions.
    pub fn resize_floating(
        &mut self,
        window: Window,
        width: u32,
        height: u32,
    ) -> Result<(), X11Error> {
        let is_tiled = match self.client_state.get_client(window) {
            Some(c) => !c.floating,
            None => return Ok(()),
        };

        if is_tiled {
            self.toggle_floating(window)?;
        }

        let mut geom = match self.client_state.get_client(window) {
            Some(client) => client.floating_geometry.unwrap_or_else(|| {
                calculate_default_floating_geometry(
                    self.context.screen_width,
                    self.context.screen_height,
                )
            }),
            None => return Ok(()),
        };

        geom.width = width;
        geom.height = height;
        let clamped = clamp_floating_geometry(
            geom,
            self.context.screen_width,
            self.context.screen_height,
        );

        if let Some(client) = self.client_state.get_client_mut(window) {
            client.set_floating(clamped);
        }

        let aux = ConfigureWindowAux::new()
            .width(clamped.width)
            .height(clamped.height)
            .stack_mode(StackMode::ABOVE);
        self.context.connection.configure_window(window, &aux)?;
        self.context.flush()?;
        Ok(())
    }

    /// Sends ICCCM WM_TAKE_FOCUS ClientMessage if the client specifies support in WM_PROTOCOLS.
    pub fn send_take_focus_if_supported(&self, window: Window) -> Result<bool, X11Error> {
        let atoms = self.icccm_atoms();
        let supports = crate::icccm::supports_protocol(
            &self.context.connection,
            window,
            atoms.wm_take_focus,
            atoms.wm_protocols,
        )?;

        if supports {
            crate::icccm::send_wm_take_focus(
                &self.context.connection,
                window,
                &atoms,
                x11rb::CURRENT_TIME,
            )?;
        }
        Ok(supports)
    }

    /// Maps a client window, making it visible on the screen.
    pub fn map_client(&mut self, window: Window) -> Result<(), X11Error> {
        self.context.connection.map_window(window)?;
        if let Some(client) = self.client_state.get_client_mut(window) {
            client.mapped = true;
        }
        let _ = self.set_wm_state(window, WmState::Normal);
        self.context.flush()?;
        Ok(())
    }

    /// Unmaps a client window, hiding it from the screen.
    pub fn unmap_client(&mut self, window: Window) -> Result<(), X11Error> {
        self.context.connection.unmap_window(window)?;
        if let Some(client) = self.client_state.get_client_mut(window) {
            client.mapped = false;
        }
        let _ = self.set_wm_state(window, WmState::Withdrawn);
        self.context.flush()?;
        Ok(())
    }

    /// Safely requests a client to close via ICCCM `WM_DELETE_WINDOW`, or forcefully kills it.
    pub fn kill_client(&self, window: Window) -> Result<(), X11Error> {
        self.send_wm_delete(window)
    }

    /// Moves and resizes a client window.
    pub fn move_resize_client(
        &self,
        window: Window,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> Result<(), X11Error> {
        let aux = ConfigureWindowAux::default()
            .x(x)
            .y(y)
            .width(w)
            .height(h);
        self.context.connection.configure_window(window, &aux)?;
        self.context.flush()?;
        Ok(())
    }

    /// Raises a window to the top of the stack, keeping the launcher overlay above it if active.
    pub fn raise_window(&self, window: Window) -> Result<(), X11Error> {
        let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        self.context.connection.configure_window(window, &aux)?;
        let _ = self.ensure_launcher_on_top();
        self.context.flush()?;
        Ok(())
    }
}
