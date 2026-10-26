//! Window Manager X11 event dispatcher and action execution.

use super::WindowManager;
use crate::ewmh::NetWmStateAction;
use crate::floating::calculate_default_floating_geometry;
use crate::focus::{set_window_border_color, Direction};
use crate::icccm::{get_wm_hints, get_wm_normal_hints, WmState};
use crate::keybind::{Action, MOD_LOCK, MOD_SHIFT};
use crate::layout::{LayoutType, Rect};
use crate::x11::X11Error;
use x11rb::protocol::xproto::{
    Allow, AtomEnum, ButtonPressEvent, ButtonReleaseEvent, ClientMessageEvent,
    ConfigWindow, ConfigureRequestEvent, ConfigureWindowAux, ConnectionExt, EventMask,
    GrabMode, KeyPressEvent, MotionNotifyEvent, PropertyNotifyEvent, Window,
};
use x11rb::protocol::Event;

impl WindowManager {
    /// Dispatches a single X11 event to the appropriate handler method.
    ///
    /// # Arguments
    /// * `event` - The parsed X11 event from x11rb.
    pub fn handle_event(&mut self, event: Event) -> Result<(), X11Error> {
        self.stats.total_events += 1;
        match event {
            Event::MapRequest(ev) => {
                self.stats.map_requests += 1;
                self.handle_map_request(ev.window)?;
            }
            Event::ConfigureRequest(ev) => {
                self.stats.configure_requests += 1;
                self.handle_configure_request(&ev)?;
            }
            Event::DestroyNotify(ev) => {
                self.stats.destroy_notifications += 1;
                self.handle_destroy_notify(ev.window)?;
            }
            Event::UnmapNotify(ev) => {
                self.stats.unmap_notifications += 1;
                self.handle_unmap_notify(ev.window)?;
            }
            Event::PropertyNotify(ev) => {
                self.stats.property_notifications += 1;
                self.handle_property_notify(&ev)?;
            }
            Event::KeyPress(ev) => {
                if let Some(ref launcher) = self.launcher
                    && launcher.visible {
                        self.handle_launcher_key(&ev)?;
                        return Ok(());
                    }
                self.handle_key_press(&ev)?;
            }
            Event::ButtonPress(ev) => {
                self.stats.button_presses += 1;
                self.handle_button_press(&ev)?;
            }
            Event::MotionNotify(ev) => {
                self.handle_motion_notify(&ev)?;
            }
            Event::ButtonRelease(ev) => {
                self.handle_button_release(&ev)?;
            }
            Event::ClientMessage(ev) => {
                self.stats.client_messages += 1;
                self.handle_client_message(&ev)?;
            }
            Event::ConfigureNotify(ev) => {
                if ev.window == self.context.root_window {
                    let _ = self.refresh_monitors();
                }
            }
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => {
                let _ = self.refresh_monitors();
            }
            Event::Expose(ev) => {
                if ev.count == 0 {
                    let is_primary_bar = self.statusbar.as_ref().map(|b| ev.window == b.window_id).unwrap_or(false);
                    let is_sec_bar = self.secondary_statusbars.iter().any(|b| ev.window == b.window_id);
                    if is_primary_bar || is_sec_bar {
                        let _ = self.render_statusbar();
                    }
                    let is_launcher_expose = self
                        .launcher
                        .as_ref()
                        .map(|l| l.visible && ev.window == l.window_id)
                        .unwrap_or(false);
                    if is_launcher_expose {
                        let _ = self.render_launcher();
                    }
                }
            }
            Event::Error(err) => {
                eprintln!("[ERROR tilo::x11] X11 Protocol Error: {:?}", err);
            }
            _ => {}
        }
        Ok(())
    }

    // -- Event Handlers --

    /// Handles a request from a client to map (show) its window.
    pub fn handle_map_request(&mut self, window: Window) -> Result<(), X11Error> {
        if self.client_state.contains(window) {
            self.retile()?;
            self.map_client(window)?;
            let _ = self.set_wm_state(window, WmState::Normal);
            self.focus_client(window)?;
            return Ok(());
        }
        let _ = self.manage_window(window)?;
        Ok(())
    }

    /// Handles a request from a client to change its size, position, or stacking order.
    pub fn handle_configure_request(
        &mut self,
        event: &ConfigureRequestEvent,
    ) -> Result<(), X11Error> {
        let mut aux = ConfigureWindowAux::default();
        if event.value_mask.contains(ConfigWindow::X) {
            aux = aux.x(i32::from(event.x));
        }
        if event.value_mask.contains(ConfigWindow::Y) {
            aux = aux.y(i32::from(event.y));
        }
        if event.value_mask.contains(ConfigWindow::WIDTH) {
            aux = aux.width(u32::from(event.width));
        }
        if event.value_mask.contains(ConfigWindow::HEIGHT) {
            aux = aux.height(u32::from(event.height));
        }
        if event.value_mask.contains(ConfigWindow::BORDER_WIDTH) {
            aux = aux.border_width(u32::from(event.border_width));
        }
        if event.value_mask.contains(ConfigWindow::SIBLING) {
            aux = aux.sibling(event.sibling);
        }
        if event.value_mask.contains(ConfigWindow::STACK_MODE) {
            aux = aux.stack_mode(event.stack_mode);
        }

        if let Err(err) = self.context.connection.configure_window(event.window, &aux) {
            eprintln!(
                "[WARN  tilo::wm] Failed to configure window 0x{:x}: {}",
                event.window, err
            );
        } else if let Some(client) = self.client_state.get_client_mut(event.window)
            && client.floating {
                let mut geom = client.floating_geometry.unwrap_or_else(|| {
                    calculate_default_floating_geometry(
                        self.context.screen_width,
                        self.context.screen_height,
                    )
                });
                if event.value_mask.contains(ConfigWindow::X) {
                    geom.x = i32::from(event.x);
                }
                if event.value_mask.contains(ConfigWindow::Y) {
                    geom.y = i32::from(event.y);
                }
                if event.value_mask.contains(ConfigWindow::WIDTH) {
                    geom.width = u32::from(event.width);
                }
                if event.value_mask.contains(ConfigWindow::HEIGHT) {
                    geom.height = u32::from(event.height);
                }
                client.set_floating(geom);
            }
        Ok(())
    }

    /// Handles the notification that a window was destroyed.
    pub fn handle_destroy_notify(&mut self, window: Window) -> Result<(), X11Error> {
        // If launcher overlay window was destroyed, close and reset launcher state cleanly
        if let Some(ref mut launcher) = self.launcher
            && launcher.window_id == window {
                launcher.window_id = 0;
                launcher.visible = false;
                let _ = self.context.connection.ungrab_keyboard(x11rb::CURRENT_TIME);
                if let Some(focused) = self.focused_window() {
                    let _ = self.focus_client(focused);
                }
                return Ok(());
            }

        // If statusbar window was destroyed, ignore
        if let Some(ref bar) = self.statusbar
            && bar.window_id == window {
                return Ok(());
            }

        // Only managed clients need unmanaging on destroy
        if self.client_state.contains(window) {
            self.unmanage_destroyed_window(window);
        }
        Ok(())
    }

    /// Handles the notification that a window was unmapped.
    pub fn handle_unmap_notify(&mut self, window: Window) -> Result<(), X11Error> {
        // Ignore unmanaged windows such as the launcher or status bar overlay
        if let Some(ref launcher) = self.launcher
            && launcher.window_id == window {
                return Ok(());
            }
        if let Some(ref bar) = self.statusbar
            && bar.window_id == window {
                return Ok(());
            }
        if !self.client_state.contains(window) {
            return Ok(());
        }

        // If the window belongs to an inactive workspace, it was unmapped intentionally
        // by the window manager during a workspace switch. Keep it tracked on its workspace!
        if let Some(ws) = self.workspace_manager.find_workspace_for(window) {
            if ws != self.current_workspace {
                return Ok(());
            }
            // In Tabbed layout, inactive tabs are intentionally kept unmapped by the WM!
            if let Some(wsobj) = self.workspace_manager.get(ws)
                && wsobj.layout == LayoutType::Tabbed
                && wsobj.contains(window)
            {
                return Ok(());
            }
        }
        if self.scratchpad.client == Some(window) {
            self.scratchpad.is_visible = false;
            if let Some(c) = self.client_state.get_client_mut(window) {
                c.mapped = false;
            }
            return Ok(());
        }

        // When a normal managed window is unmapped on the active workspace by the client,
        // it transitions to Withdrawn state and is unmanaged (ICCCM 4.1.4).
        self.unmanage_window(window);
        Ok(())
    }

    /// Handles window property changes (e.g. WM_NAME, WM_HINTS, WM_NORMAL_HINTS).
    pub fn handle_property_notify(&mut self, event: &PropertyNotifyEvent) -> Result<(), X11Error> {
        self.stats.property_notifications += 1;

        if event.atom == AtomEnum::WM_HINTS.into() {
            if let Ok(Some(hints)) = get_wm_hints(&self.context.connection, event.window) {
                let is_urgent = hints.is_urgent;
                if let Some(c) = self.client_state.get_client_mut(event.window) {
                    c.urgent = is_urgent;
                }
                if is_urgent {
                    let _ = set_window_border_color(
                        &self.context.connection,
                        event.window,
                        self.border_config.urgent_color,
                    );
                }
                let _ = self.render_statusbar();
            }
        } else if event.atom == AtomEnum::WM_NORMAL_HINTS.into() {
            let _ = get_wm_normal_hints(&self.context.connection, event.window);
        } else if event.atom == AtomEnum::WM_NAME.into() || event.atom == self.atoms.net_wm_name {
            let title = self.get_window_title(event.window);
            let title_changed = if let Some(client) = self.client_state.get_client_mut(event.window) {
                let changed = client.title.as_deref() != Some(&title);
                client.title = Some(title.clone());
                changed
            } else {
                false
            };
            if title_changed {
                if Some(event.window) == self.focused_window()
                    && let Some(ref mut bar) = self.statusbar {
                        bar.set_title(&title);
                }
                if let Some(ws) = self.workspace_manager.find_workspace_for(event.window)
                    && ws == self.current_workspace
                {
                    let _ = self.render_statusbar();
                }
            }
        }

        Ok(())
    }

    /// Handles keyboard input events and executes matching keybinding actions.
    pub fn handle_key_press(&mut self, event: &KeyPressEvent) -> Result<(), X11Error> {
        self.stats.key_presses += 1;
        if let Some(action) = self.input_manager.handle_event(event) {
            self.execute_action(action)?;
        }
        Ok(())
    }

    /// Executes an [`Action`] triggered by a keybinding.
    pub fn execute_action(&mut self, action: Action) -> Result<(), X11Error> {
        match action {
            Action::Spawn(cmd) => {
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .spawn();
            }
            Action::Close => {
                if let Some(win) = self.focused_window() {
                    let _ = self.kill_client(win);
                }
            }
            Action::Focus(dir) => {
                let _ = self.focus_in_direction(dir, false)?;
            }
            Action::Move(dir) => {
                let _ = self.swap_in_direction(dir)?;
            }
            Action::Workspace(tag) => {
                let active_mon = self.monitor_manager.active_monitor_id;
                let target_ws = crate::workspace::monitor_workspace_id(active_mon, tag);
                self.switch_workspace(target_ws)?;
            }
            Action::MoveToWorkspace(tag) => {
                let win = self.focused_window()
                    .or_else(|| self.workspace_focus.focused_window_on(self.current_workspace))
                    .or_else(|| self.workspace_manager.get(self.current_workspace).and_then(|w| w.clients.first().copied()));
                if let Some(win) = win {
                    let active_mon = self.monitor_manager.active_monitor_id;
                    let target_ws = crate::workspace::monitor_workspace_id(active_mon, tag);
                    self.move_to_workspace(win, target_ws)?;
                }
            }
            Action::ToggleFloating => {
                if let Some(win) = self.focused_window() {
                    let _ = self.toggle_floating(win);
                }
            }
            Action::ToggleFullscreen => {
                if let Some(win) = self.focused_window() {
                    let _ = self.toggle_fullscreen(win);
                }
            }
            Action::FocusMonitorLeft => {
                self.focus_monitor_left()?;
            }
            Action::FocusMonitorRight => {
                self.focus_monitor_right()?;
            }
            Action::MoveToMonitor(mon_id) => {
                self.move_to_monitor(mon_id)?;
            }
            Action::Reload => {
                if let Err(err) = self.reload_config() {
                    eprintln!(
                        "[WARN  tilo::wm] Config reload aborted, keeping previous configuration: {}",
                        err
                    );
                }
            }
            Action::Launcher => {
                let _ = self.toggle_launcher();
            }
            Action::ToggleScratchpad => {
                let _ = self.toggle_scratchpad();
            }
            Action::SetScratchpad => {
                if let Some(focused) = self.focused_window() {
                    self.set_scratchpad_client(focused);
                }
            }
            Action::ExitMenu | Action::Exit => {
                let _ = self.open_exit_launcher();
            }
            Action::Quit => {
                let _ = self.perform_desktop_shutdown(None);
            }
            Action::ResizeTile(dir) => {
                let current_ws = self.workspace_manager.current_id();
                let focused = self.focused_window()
                    .or_else(|| self.workspace_focus.focused_window_on(current_ws));

                if let Some(win) = focused {
                    // 1. Floating window resizing in all 4 directions via keyboard
                    let is_floating = self.client_state.get_client(win).map(|c| c.floating).unwrap_or(false);
                    if is_floating {
                        let step = 30;
                        let geom = self.client_state.get_client(win).and_then(|c| c.floating_geometry).unwrap_or_else(|| {
                            crate::floating::calculate_default_floating_geometry(self.context.screen_width, self.context.screen_height)
                        });
                        let (new_w, new_h) = match dir {
                            Direction::Right => ((geom.width + step).min(self.context.screen_width as u32), geom.height),
                            Direction::Left => (geom.width.saturating_sub(step).max(100), geom.height),
                            Direction::Down => (geom.width, (geom.height + step).min(self.context.screen_height as u32)),
                            Direction::Up => (geom.width, geom.height.saturating_sub(step).max(80)),
                        };
                        let _ = self.resize_floating(win, new_w, new_h);
                        return Ok(());
                    }

                    // 2. Tiled window resizing across all layouts
                    if let Some(ws) = self.workspace_manager.get_mut(current_ws) {
                        let (split_type, delta) = match dir {
                            Direction::Left => (crate::layout::SplitType::Horizontal, -0.05),
                            Direction::Right => (crate::layout::SplitType::Horizontal, 0.05),
                            Direction::Up => (crate::layout::SplitType::Vertical, -0.05),
                            Direction::Down => (crate::layout::SplitType::Vertical, 0.05),
                        };
                        if ws.bsp_tree.resize_window(win, delta, split_type) {
                            if ws.layout.is_master()
                                && let Some(r) = ws.bsp_tree.master_ratio_for_layout(ws.layout) {
                                    self.layout_config.master_ratio = r;
                                }
                            let _ = self.retile();
                        }
                    }
                }
            }
            Action::PreselectSplit(split_type) => {
                let current_ws = self.workspace_manager.current_id();
                if let Some(ws) = self.workspace_manager.get_mut(current_ws) {
                    ws.layout = LayoutType::Bsp;
                    ws.bsp_tree.preselect = Some(split_type);
                }
            }
            Action::CycleLayout => {
                let current_ws = self.workspace_manager.current_id();
                let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
                let master_ratio = self.layout_config.master_ratio;
                if let Some(ws) = self.workspace_manager.get_mut(current_ws) {
                    ws.cycle_layout();
                    ws.bsp_tree.rebuild_for_layout(ws.layout, &visible_tiled, master_ratio);
                }
                self.retile()?;
                self.render_statusbar().ok();
            }
            Action::RotateMaster => {
                let current_ws = self.workspace_manager.current_id();
                let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
                let master_ratio = self.layout_config.master_ratio;
                if let Some(ws) = self.workspace_manager.get_mut(current_ws)
                    && ws.layout.can_rotate() {
                        ws.rotate_layout();
                        ws.bsp_tree.rebuild_for_layout(ws.layout, &visible_tiled, master_ratio);
                        self.retile()?;
                        self.render_statusbar().ok();
                    }
            }
            Action::SetLayout(layout) => {
                let current_ws = self.workspace_manager.current_id();
                let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
                let master_ratio = self.layout_config.master_ratio;
                if let Some(ws) = self.workspace_manager.get_mut(current_ws) {
                    ws.set_layout(layout);
                    ws.bsp_tree.rebuild_for_layout(layout, &visible_tiled, master_ratio);
                }
                self.retile()?;
                self.render_statusbar().ok();
            }
            Action::FocusNextTab => {
                self.focus_next_tab()?;
            }
            Action::FocusPrevTab => {
                self.focus_prev_tab()?;
            }
            Action::MoveTabLeft => {
                self.move_tab_left()?;
            }
            Action::MoveTabRight => {
                self.move_tab_right()?;
            }
            Action::SelectTab(index) => {
                self.select_tab(index)?;
            }
            Action::VolumeUp(step) => {
                let current_state = self.statusbar.as_ref().and_then(|b| b.volume_state());
                if let Some(mut state) = current_state {
                    state.volume = (state.volume as u16 + step as u16).min(150) as u8;
                    state.muted = false;
                    if let Some(ref mut bar) = self.statusbar {
                        bar.sync_volume(Some(state));
                    }
                    let _ = self.render_statusbar();
                    let _ = self.context.flush();
                }
                crate::audio::adjust_volume(step as i8);
                let new_vol = crate::audio::get_volume();
                if let Some(ref mut bar) = self.statusbar {
                    bar.sync_volume(new_vol);
                }
                let _ = self.render_statusbar();
                let _ = self.context.flush();
            }
            Action::VolumeDown(step) => {
                let current_state = self.statusbar.as_ref().and_then(|b| b.volume_state());
                if let Some(mut state) = current_state {
                    state.volume = state.volume.saturating_sub(step);
                    if let Some(ref mut bar) = self.statusbar {
                        bar.sync_volume(Some(state));
                    }
                    let _ = self.render_statusbar();
                    let _ = self.context.flush();
                }
                crate::audio::adjust_volume(-(step as i8));
                let new_vol = crate::audio::get_volume();
                if let Some(ref mut bar) = self.statusbar {
                    bar.sync_volume(new_vol);
                }
                let _ = self.render_statusbar();
                let _ = self.context.flush();
            }
            Action::VolumeToggleMute => {
                let current_state = self.statusbar.as_ref().and_then(|b| b.volume_state());
                if let Some(mut state) = current_state {
                    state.muted = !state.muted;
                    if let Some(ref mut bar) = self.statusbar {
                        bar.sync_volume(Some(state));
                    }
                    let _ = self.render_statusbar();
                    let _ = self.context.flush();
                }
                crate::audio::toggle_mute();
                let new_vol = crate::audio::get_volume();
                if let Some(ref mut bar) = self.statusbar {
                    bar.sync_volume(new_vol);
                }
                let _ = self.render_statusbar();
                let _ = self.context.flush();
            }
        }
        Ok(())
    }

    /// Handles mouse button press events on client windows.
    ///
    /// Supports Phase 24:
    /// - Mod + Left Click: focus and drag window to move.
    /// - Mod + Right Click: focus and drag window to resize.
    /// - Normal Left Click: focus clicked window and replay event to client.
    pub fn handle_button_press(&mut self, event: &ButtonPressEvent) -> Result<(), X11Error> {
        // Launcher mouse interaction (Wheel scroll, item click, or click outside to dismiss)
        if let Some(ref launcher) = self.launcher && launcher.visible {
            if event.event == launcher.window_id {
                if event.detail == 4 {
                    if let Some(ref mut l) = self.launcher {
                        crate::launcher::update_launcher_state(l, crate::launcher::LauncherAction::SelectPrevious);
                    }
                    let _ = self.render_launcher();
                    return Ok(());
                } else if event.detail == 5 {
                    if let Some(ref mut l) = self.launcher {
                        crate::launcher::update_launcher_state(l, crate::launcher::LauncherAction::SelectNext);
                    }
                    let _ = self.render_launcher();
                    return Ok(());
                } else if event.detail == 1 {
                    let click_x = event.event_x as i32;
                    let click_y = event.event_y as i32;
                    let mut clicked_index = None;
                    if let Some(ref l) = self.launcher {
                        for &(idx, rect) in &l.item_rects {
                            if click_x >= rect.x
                                && click_x < rect.x + rect.width as i32
                                && click_y >= rect.y
                                && click_y < rect.y + rect.height as i32
                            {
                                clicked_index = Some(idx);
                                break;
                            }
                        }
                    }
                    if let Some(idx) = clicked_index {
                        if let Some(ref mut l) = self.launcher {
                            l.selected_index = idx;
                        }
                        self.execute_launcher_selection()?;
                    }
                    return Ok(());
                }
            } else {
                let _ = self.close_launcher();
                return Ok(());
            }
        }

        // Statusbar mouse interaction (Workspace clicks, Tab clicks, and wheel switching)
        let hit_bar_info = if let Some(ref bar) = self.statusbar && bar.window_id == event.event {
            let mon_id = self.monitor_manager.monitors.iter()
                .find(|m| m.is_primary)
                .or_else(|| self.monitor_manager.monitors.first())
                .map(|m| m.id)
                .unwrap_or(0);
            Some((bar.workspace_rects.clone(), bar.tab_rects.clone(), bar.rect.height, mon_id))
        } else if let Some(bar) = self.secondary_statusbars.iter().find(|b| b.window_id == event.event) {
            let mon_id = self.monitor_manager.monitors.iter()
                .find(|m| bar.rect.x >= m.x && (bar.rect.x as i32) < (m.x + m.width as i32))
                .map(|m| m.id)
                .unwrap_or_else(|| self.monitor_manager.active_monitor_id);
            Some((bar.workspace_rects.clone(), bar.tab_rects.clone(), bar.rect.height, mon_id))
        } else {
            None
        };

        if let Some((workspace_rects, tab_rects, bar_height, hit_mon_id)) = hit_bar_info {
            // Activate the monitor hosting this statusbar if not currently active
            if hit_mon_id != self.monitor_manager.active_monitor_id {
                self.activate_monitor(hit_mon_id)?;
            }
            let mon_ws = self
                .monitor_manager
                .get_monitor(hit_mon_id)
                .map(|m| m.workspace)
                .unwrap_or(self.current_workspace);

            let click_x = event.event_x as i32;
            let click_y = event.event_y as i32;

            // 1. Mouse wheel over workspace region (Scroll up -> prev workspace, Scroll down -> next workspace)
            if self.config.workspace_wheel_switch && (event.detail == 4 || event.detail == 5)
                && let Some(first) = workspace_rects.first()
                && let Some(last) = workspace_rects.last()
            {
                let min_x = first.1.x;
                let max_x = last.1.x + last.1.width as i32;
                if click_x >= min_x && click_x <= max_x {
                    let current_tag = crate::workspace::workspace_to_tag(mon_ws);
                    let target_tag = if event.detail == 4 {
                        if current_tag > 1 { current_tag - 1 } else { 9 }
                    } else {
                        if current_tag < 9 { current_tag + 1 } else { 1 }
                    };
                    let target_ws = crate::workspace::monitor_workspace_id(hit_mon_id, target_tag);
                    self.switch_workspace(target_ws)?;
                    return Ok(());
                }
            }

            // 2. Direct workspace number clicks (1..=9)
            if self.config.workspace_mouse {
                for (idx, &(tag, rect)) in workspace_rects.iter().enumerate() {
                    let is_hit = if idx + 1 < workspace_rects.len() {
                        let next_x = workspace_rects[idx + 1].1.x;
                        click_x >= rect.x && click_x < next_x
                    } else {
                        click_x >= rect.x && click_x < rect.x + rect.width as i32 + 4
                    };

                    if is_hit && click_y >= 0 && click_y < bar_height as i32 {
                        let target_ws = crate::workspace::monitor_workspace_id(hit_mon_id, tag);
                        if event.detail == 1 {
                            let clean_modifiers = u16::from(event.state) & 0x00FF & !(self.input_manager.num_lock_mask | MOD_LOCK);
                            if clean_modifiers & MOD_SHIFT != 0 {
                                let win = self.focused_window()
                                    .or_else(|| self.workspace_focus.focused_window_on(self.current_workspace))
                                    .or_else(|| self.workspace_manager.get(self.current_workspace).and_then(|w| w.clients.first().copied()));
                                if let Some(win) = win {
                                    self.move_to_workspace(win, target_ws)?;
                                }
                            } else {
                                self.switch_workspace(target_ws)?;
                            }
                            return Ok(());
                        } else if event.detail == 3 {
                            let win = self.focused_window()
                                .or_else(|| self.workspace_focus.focused_window_on(self.current_workspace))
                                .or_else(|| self.workspace_manager.get(self.current_workspace).and_then(|w| w.clients.first().copied()));
                            if let Some(win) = win {
                                self.move_to_workspace(win, target_ws)?;
                            }
                            return Ok(());
                        }
                    }
                }
            }

            // 3. Mouse wheel over tab region
            if self.config.tab_config.wheel_switch && (event.detail == 4 || event.detail == 5)
                && let Some(first) = tab_rects.first()
                && let Some(last) = tab_rects.last()
            {
                let min_x = first.1.x;
                let max_x = last.1.x + last.1.width as i32;
                if click_x >= min_x && click_x <= max_x {
                    if event.detail == 4 {
                        self.focus_prev_tab()?;
                    } else {
                        self.focus_next_tab()?;
                    }
                    return Ok(());
                }
            }

            // 4. Direct tab pill clicks (Left click: focus, Middle click: close)
            if self.config.tab_config.mouse {
                for &(win, rect) in &tab_rects {
                    if click_x >= rect.x
                        && click_x < rect.x + rect.width as i32
                        && click_y >= rect.y
                        && click_y < rect.y + rect.height as i32
                    {
                        if event.detail == 1 {
                            self.focus_client(win)?;
                            return Ok(());
                        } else if event.detail == 2 {
                            let _ = self.kill_client(win);
                            return Ok(());
                        }
                    }
                }
            }

            return Ok(());
        }

        let window = if self.client_state.contains(event.child) {
            event.child
        } else if self.client_state.contains(event.event) {
            event.event
        } else {
            // Click occurred on root / background window or unmanaged window
            if let Some(mon) = self.monitor_manager.monitor_for_point(event.root_x as i32, event.root_y as i32) {
                if mon.id != self.monitor_manager.active_monitor_id {
                    let _ = self.activate_monitor(mon.id);
                }
            }
            let event_time = if event.time != 0 { event.time } else { x11rb::CURRENT_TIME };
            let _ = self.context.connection.allow_events(Allow::REPLAY_POINTER, event_time);
            let _ = self.context.flush();
            return Ok(());
        };

        let event_time = if event.time != 0 { event.time } else { x11rb::CURRENT_TIME };
        let clean_modifiers = u16::from(event.state) & 0x00FF & !(self.input_manager.num_lock_mask | MOD_LOCK);
        let matching_binding = self.mouse_bindings.iter().find(|b| {
            b.button == event.detail && b.modifiers == clean_modifiers
        });

        let is_floating = self
            .client_state
            .get_client(window)
            .map(|c| c.is_floating() && !c.is_fullscreen())
            .unwrap_or(false);

        match matching_binding.map(|b| b.action_type) {
            Some(crate::mouse::MouseActionType::Move) => {
                let _ = self.focus_client(window);
                let _ = self.raise_window(window);

                if !is_floating {
                    self.context.flush()?;
                    return Ok(());
                }

                let (win_x, win_y) = if let Some(reply) = self.context.connection.get_geometry(window).ok().and_then(|c| c.reply().ok()) {
                    (reply.x as i32, reply.y as i32)
                } else if let Some(fg) = self.client_state.get_client(window).and_then(|c| c.floating_geometry) {
                    (fg.x, fg.y)
                } else {
                    (event.root_x as i32 - event.event_x as i32, event.root_y as i32 - event.event_y as i32)
                };

                self.mouse_action = crate::mouse::MouseAction::Moving {
                    window,
                    start_x: event.root_x as i32,
                    start_y: event.root_y as i32,
                    start_win_x: win_x,
                    start_win_y: win_y,
                };

                let _ = self.context.connection.grab_pointer(
                    false,
                    self.context.root_window,
                    EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    self.cursor_manager.move_cursor,
                    event_time,
                );
                self.context.flush()?;
            }
            Some(crate::mouse::MouseActionType::Resize) => {
                let _ = self.focus_client(window);
                let _ = self.raise_window(window);

                if !is_floating {
                    let (win_x, win_y, win_w, win_h) = if let Some(reply) = self.context.connection.get_geometry(window).ok().and_then(|c| c.reply().ok()) {
                        (reply.x as i32, reply.y as i32, reply.width as u32, reply.height as u32)
                    } else {
                        (0, 0, 400, 300)
                    };

                    self.mouse_action = crate::mouse::MouseAction::ResizingTiled {
                        window,
                        start_x: event.root_x as i32,
                        start_y: event.root_y as i32,
                        initial_rect: Rect::new(win_x, win_y, win_w, win_h),
                    };

                    let _ = self.context.connection.grab_pointer(
                        false,
                        self.context.root_window,
                        EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                        x11rb::NONE,
                        self.cursor_manager.resize_cursor,
                        event_time,
                    );
                    self.context.flush()?;
                    return Ok(());
                }

                let (win_w, win_h) = if let Some(reply) = self.context.connection.get_geometry(window).ok().and_then(|c| c.reply().ok()) {
                    (reply.width as u32, reply.height as u32)
                } else if let Some(fg) = self.client_state.get_client(window).and_then(|c| c.floating_geometry) {
                    (fg.width, fg.height)
                } else {
                    (400, 300)
                };

                self.mouse_action = crate::mouse::MouseAction::Resizing {
                    window,
                    start_x: event.root_x as i32,
                    start_y: event.root_y as i32,
                    start_width: win_w,
                    start_height: win_h,
                };

                let _ = self.context.connection.grab_pointer(
                    false,
                    self.context.root_window,
                    EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    self.cursor_manager.resize_cursor,
                    event_time,
                );
                self.context.flush()?;
            }
            None => {
                let _ = self.focus_client(window);
                let _ = self.raise_window(window);
                let _ = self.context.connection.allow_events(Allow::REPLAY_POINTER, event_time);
                self.context.flush()?;
            }
        }

        Ok(())
    }

    /// Handles pointer motion events during a mouse drag operation.
    pub fn handle_motion_notify(&mut self, event: &MotionNotifyEvent) -> Result<(), X11Error> {
        crate::mouse::handle_motion_notify(event, &self.mouse_action, &self.context.connection)?;

        match self.mouse_action {
            crate::mouse::MouseAction::Moving {
                window,
                start_x,
                start_y,
                start_win_x,
                start_win_y,
            } => {
                let delta_x = event.root_x as i32 - start_x;
                let delta_y = event.root_y as i32 - start_y;
                let new_x = start_win_x + delta_x;
                let new_y = start_win_y + delta_y;
                if let Some(client) = self.client_state.get_client_mut(window) {
                    if let Some(ref mut fg) = client.floating_geometry {
                        fg.x = new_x;
                        fg.y = new_y;
                    } else {
                        client.floating_geometry = Some(crate::floating::FloatingGeometry::new(new_x, new_y, 400, 300));
                    }
                }
            }
            crate::mouse::MouseAction::Resizing {
                window,
                start_x,
                start_y,
                start_width,
                start_height,
            } => {
                let delta_x = event.root_x as i32 - start_x;
                let delta_y = event.root_y as i32 - start_y;
                let new_w = (start_width as i32 + delta_x).max(100) as u32;
                let new_h = (start_height as i32 + delta_y).max(80) as u32;
                let _ = self.resize_floating(window, new_w, new_h);
            }
            crate::mouse::MouseAction::ResizingTiled {
                window,
                start_x,
                start_y,
                initial_rect,
            } => {
                let delta_x = event.root_x as i32 - start_x;
                let delta_y = event.root_y as i32 - start_y;
                self.mouse_action = crate::mouse::MouseAction::ResizingTiled {
                    window,
                    start_x: event.root_x as i32,
                    start_y: event.root_y as i32,
                    initial_rect,
                };
                let current_ws = self.workspace_manager.current_id();
                if let Some(ws) = self.workspace_manager.get_mut(current_ws)
                    && ws.bsp_tree.resize_by_pointer(window, delta_x, delta_y, initial_rect) {
                        if ws.layout.is_master()
                            && let Some(r) = ws.bsp_tree.master_ratio_for_layout(ws.layout) {
                                self.layout_config.master_ratio = r;
                            }
                        let _ = self.retile();
                    }
            }
            crate::mouse::MouseAction::None => {}
        }
        Ok(())
    }

    /// Handles mouse button release events, terminating drag operations and ungrabbing the pointer.
    pub fn handle_button_release(&mut self, event: &ButtonReleaseEvent) -> Result<(), X11Error> {
        if self.mouse_action.is_active() {
            let event_time = if event.time != 0 { event.time } else { x11rb::CURRENT_TIME };
            let _ = self.context.connection.ungrab_pointer(event_time);
            self.context.flush()?;
            self.mouse_action = crate::mouse::MouseAction::None;
        }
        Ok(())
    }

    /// Handles client messages (e.g. _NET_ACTIVE_WINDOW, _NET_CURRENT_DESKTOP, _NET_WM_STATE, _NET_CLOSE_WINDOW).
    pub fn handle_client_message(&mut self, event: &ClientMessageEvent) -> Result<(), X11Error> {
        self.stats.client_messages += 1;
        let atoms = self.ewmh_atoms;

        if event.type_ == atoms.net_active_window {
            let window = event.window;
            if self.client_state.contains(window) {
                if let Some(ws) = self.workspace_manager.find_workspace_for(window)
                    && ws != self.current_workspace {
                        self.switch_workspace(ws)?;
                    }
                self.focus_client(window)?;
            }
        } else if event.type_ == atoms.net_current_desktop {
            let data = event.data.as_data32();
            let target_ws = data[0] + 1; // 0-indexed in EWMH -> 1-based workspace
            if (1..=9).contains(&target_ws) {
                self.switch_workspace(target_ws)?;
            }
        } else if event.type_ == atoms.net_close_window {
            let window = event.window;
            if self.client_state.contains(window) {
                self.kill_client(window)?;
            }
        } else if event.type_ == atoms.net_wm_state {
            let data = event.data.as_data32();
            let action_num = data[0];
            let prop1 = data[1];
            let prop2 = data[2];

            if let Some(action) = NetWmStateAction::from_u32(action_num) {
                let window = event.window;
                let is_fullscreen_target = prop1 == atoms.net_wm_state_fullscreen || prop2 == atoms.net_wm_state_fullscreen;
                let is_above_target = prop1 == atoms.net_wm_state_above || prop2 == atoms.net_wm_state_above;

                if is_fullscreen_target {
                    let current_fullscreen = self
                        .client_state
                        .get_client(window)
                        .map(|c| c.is_fullscreen())
                        .unwrap_or(false);
                    let new_fullscreen = match action {
                        NetWmStateAction::Remove => false,
                        NetWmStateAction::Add => true,
                        NetWmStateAction::Toggle => !current_fullscreen,
                    };
                    self.set_fullscreen(window, new_fullscreen)?;
                }

                if is_above_target {
                    let currently_floating = self.client_state.get_client(window).map(|c| c.floating).unwrap_or(false);
                    let new_floating = match action {
                        NetWmStateAction::Remove => false,
                        NetWmStateAction::Add => true,
                        NetWmStateAction::Toggle => !currently_floating,
                    };
                    if new_floating != currently_floating {
                        self.toggle_floating(window)?;
                    }
                }
            }
        }

        Ok(())
    }
}
