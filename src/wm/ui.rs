//! Window Manager UI integration (Statusbar and Launcher).

use super::WindowManager;
use crate::bar::{Statusbar, StatusbarError};
use crate::layout::{LayoutType, Rect};
use crate::workspace::WorkspaceId;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, InputFocus, KeyPressEvent, StackMode,
};

impl WindowManager {
    pub fn enable_statusbar(&mut self, height: u32) -> Result<u32, StatusbarError> {
        if let Some(old_bar) = self.statusbar.take() {
            let _ = old_bar.destroy(&self.context.connection);
        }
        for old_sec in self.secondary_statusbars.drain(..) {
            let _ = old_sec.destroy(&self.context.connection);
        }

        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let bar_colors = self.config.bar_color_scheme();
        let font_name = self.config.bar_font();

        // 1. Primary monitor: full statusbar with all configured modules
        let primary_mon = self.monitor_manager.monitors.iter().find(|m| m.is_primary)
            .or_else(|| self.monitor_manager.monitors.first())
            .cloned();
        let (prim_x, prim_y, prim_w, prim_id, prim_ws) = if let Some(ref m) = primary_mon {
            (m.x, m.y, m.width, m.id, m.workspace)
        } else {
            (0, 0, self.context.screen_width as u32, 0, self.current_workspace)
        };
        let prim_rect = Rect::new(prim_x, prim_y, prim_w, height);
        let win_id = crate::bar::create_statusbar_window_with_color(&self.context.connection, screen, prim_rect, bar_colors.bg)?;
        let _ = self.cursor_manager.apply_window(&self.context.connection, win_id);
        let mut bar = Statusbar::with_modules(win_id, prim_rect, &self.config.bar_modules, bar_colors);
        bar.separator = self.config.bar_separator().to_string();
        bar.set_workspace_tags(&self.config.workspace_tags);
        let prim_tag = crate::workspace::workspace_to_tag(prim_ws);
        let occupied_prim = self.monitor_occupied_tags(prim_id);
        bar.set_workspace(prim_tag, &occupied_prim);

        // Initialize and attach double-buffered renderer
        match crate::render::Renderer::new(
            &self.context.connection,
            win_id,
            prim_rect.width as u16,
            prim_rect.height as u16,
            screen.root_depth,
            font_name,
        ) {
            Ok(renderer) => {
                bar.attach_renderer(renderer);
            }
            Err(err) => {
                eprintln!("[WARN  tilo::wm] Failed to initialize primary statusbar renderer: {}", err);
            }
        }
        let _ = bar.update_all();
        self.statusbar = Some(bar);

        // 2. Secondary monitors: statusbars with all features minus hardware metrics
        let monitors = self.monitor_manager.monitors.clone();
        for m in &monitors {
            if m.id == prim_id {
                continue;
            }
            if let Ok(sec_bar) = self.create_secondary_statusbar(m, height) {
                self.secondary_statusbars.push(sec_bar);
            }
        }

        let _ = self.render_statusbar();
        let _ = self.retile();
        Ok(win_id)
    }

    /// Creates and initializes a secondary statusbar for a non-primary monitor.
    pub fn create_secondary_statusbar(
        &self,
        mon: &crate::monitor::Monitor,
        height: u32,
    ) -> Result<Statusbar, StatusbarError> {
        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let bar_colors = self.config.bar_color_scheme();
        let font_name = self.config.bar_font();
        let sec_modules: Vec<crate::bar::modules::BarModuleType> = self
            .config
            .bar_modules
            .iter()
            .copied()
            .filter(|m| *m == crate::bar::modules::BarModuleType::Clock)
            .collect();

        let sec_rect = Rect::new(mon.x, mon.y, mon.width, height);
        let sec_win = crate::bar::create_statusbar_window_with_color(
            &self.context.connection,
            screen,
            sec_rect,
            bar_colors.bg,
        )?;
        let _ = self.cursor_manager.apply_window(&self.context.connection, sec_win);
        let mut sec_bar = Statusbar::with_modules(sec_win, sec_rect, &sec_modules, bar_colors);
        sec_bar.separator = self.config.bar_separator().to_string();
        sec_bar.set_workspace_tags(&self.config.workspace_tags);
        let sec_tag = crate::workspace::workspace_to_tag(mon.workspace);
        let occupied_sec = self.monitor_occupied_tags(mon.id);
        sec_bar.set_workspace(sec_tag, &occupied_sec);
        if let Ok(renderer) = crate::render::Renderer::new(
            &self.context.connection,
            sec_win,
            sec_rect.width as u16,
            sec_rect.height as u16,
            screen.root_depth,
            font_name,
        ) {
            sec_bar.attach_renderer(renderer);
        }
        let _ = sec_bar.update_all();
        Ok(sec_bar)
    }

    /// Disables and destroys all built-in statusbars, releasing reserved screen space.
    pub fn disable_statusbar(&mut self) -> Result<(), X11Error> {
        if let Some(bar) = self.statusbar.take() {
            bar.destroy(&self.context.connection)?;
        }
        for bar in self.secondary_statusbars.drain(..) {
            let _ = bar.destroy(&self.context.connection);
        }
        self.retile()?;
        Ok(())
    }

    /// Updates all statusbar modules and re-renders if metrics or text changed.
    pub fn update_statusbar(&mut self) -> bool {
        let mut changed = if let Some(ref mut bar) = self.statusbar {
            bar.update_all()
        } else {
            false
        };
        for bar in &mut self.secondary_statusbars {
            if bar.update_all() {
                changed = true;
            }
        }
        if changed {
            let _ = self.render_statusbar();
        }
        changed
    }

    /// Extracts window title, layout symbol, and tab info for a workspace.
    pub fn workspace_display_info(
        &self,
        ws: WorkspaceId,
    ) -> (String, &'static str, Option<Vec<crate::bar::tabs::TabInfo>>) {
        let title = self
            .workspace_focus
            .focused_window_on(ws)
            .and_then(|w| self.client_state.get_client(w))
            .and_then(|c| c.title.as_deref())
            .unwrap_or("")
            .to_string();
        let layout = self
            .workspace_manager
            .get(ws)
            .map(|w| w.layout)
            .unwrap_or(self.config.layout);
        let focused = self.workspace_focus.focused_window_on(ws);
        let layout_sym = match focused {
            Some(w) if self.client_state.get_client(w).map(|c| c.floating).unwrap_or(false) => "><>",
            _ => layout.symbol(),
        };
        let tabs = if layout == LayoutType::Tabbed {
            Some(self.get_workspace_tabs(ws))
        } else {
            None
        };
        (title, layout_sym, tabs)
    }

    /// Redraws all active statusbar UIs (primary and secondary) using their attached renderers.
    pub fn render_statusbar(&mut self) -> Result<(), StatusbarError> {
        let primary_mon = self.monitor_manager.monitors.iter().find(|m| m.is_primary)
            .or_else(|| self.monitor_manager.monitors.first())
            .cloned();
        let primary_id = primary_mon.as_ref().map(|m| m.id).unwrap_or(0);
        let primary_ws = primary_mon.as_ref().map(|m| m.workspace).unwrap_or(self.current_workspace);

        // 1. Render primary statusbar
        let (primary_title, primary_layout_sym, primary_tabs) = self.workspace_display_info(primary_ws);
        let primary_tag = crate::workspace::workspace_to_tag(primary_ws);
        let occupied_prim = self.monitor_occupied_tags(primary_id);

        if let Some(ref mut bar) = self.statusbar {
            bar.set_title(&primary_title);
            bar.set_workspace(primary_tag, &occupied_prim);
            if let Err(err) = bar.render(
                &self.context.connection,
                primary_tag,
                &occupied_prim,
                primary_layout_sym,
                &primary_title,
                primary_tabs.as_deref(),
                &self.config.tab_config,
            ) {
                eprintln!("[WARN  tilo::wm] Primary statusbar render failed: {}", err);
            }
        }

        // 2. Precompute info for secondary statusbars to avoid borrowing self while iterating iter_mut
        let monitors = self.monitor_manager.monitors.clone();
        let sec_info: Vec<_> = self.secondary_statusbars.iter().map(|bar| {
            let mon = monitors.iter().find(|m| {
                bar.rect.x >= m.x && (bar.rect.x as i32) < (m.x + m.width as i32)
            });
            let sec_mon_id = mon.map(|m| m.id).unwrap_or(1);
            let sec_ws = mon.map(|m| m.workspace).unwrap_or(11);
            let (sec_title, sec_layout_sym, sec_tabs) = self.workspace_display_info(sec_ws);
            let sec_tag = crate::workspace::workspace_to_tag(sec_ws);
            let occupied_sec = self.monitor_occupied_tags(sec_mon_id);

            (sec_title, sec_layout_sym, sec_tabs, sec_tag, occupied_sec)
        }).collect();

        for (bar, (sec_title, sec_layout_sym, sec_tabs, sec_tag, occupied_sec)) in
            self.secondary_statusbars.iter_mut().zip(sec_info.into_iter())
        {
            bar.set_title(&sec_title);
            bar.set_workspace(sec_tag, &occupied_sec);
            if let Err(err) = bar.render(
                &self.context.connection,
                sec_tag,
                &occupied_sec,
                sec_layout_sym,
                &sec_title,
                sec_tabs.as_deref(),
                &self.config.tab_config,
            ) {
                eprintln!("[WARN  tilo::wm] Secondary statusbar render failed: {}", err);
            }
        }

        Ok(())
    }

    // -- Application Launcher (Phase 19, 20, 21) --

    /// Opens the application launcher overlay in normal App mode.
    pub fn open_launcher(&mut self) -> Result<(), StatusbarError> {
        self.open_launcher_with_mode(crate::launcher::LauncherMode::App)
    }

    /// Opens the application launcher overlay in Exit/Power menu mode (Logout, Reboot, Poweroff).
    pub fn open_exit_launcher(&mut self) -> Result<(), StatusbarError> {
        let is_visible_exit = self.launcher.as_ref().map(|l| l.visible && l.mode == crate::launcher::LauncherMode::Exit).unwrap_or(false);
        if is_visible_exit {
            self.close_launcher()
        } else {
            self.open_launcher_with_mode(crate::launcher::LauncherMode::Exit)
        }
    }

    /// Opens the application launcher overlay with the given mode, centering it on the monitor containing the mouse pointer and grabbing keyboard.
    pub fn open_launcher_with_mode(&mut self, mode: crate::launcher::LauncherMode) -> Result<(), StatusbarError> {
        // Query pointer coordinates to target whichever monitor contains the mouse cursor
        let target_mon_id = if let Ok(cookie) = self.context.connection.query_pointer(self.context.root_window)
            && let Ok(reply) = cookie.reply()
        {
            self.monitor_manager.monitor_for_point(reply.root_x as i32, reply.root_y as i32).map(|m| m.id)
        } else {
            None
        };

        if let Some(mon_id) = target_mon_id {
            if mon_id != self.monitor_manager.active_monitor_id {
                let _ = self.activate_monitor(mon_id);
            }
        }

        let active_mon = self.monitor_manager.active_monitor().clone();
        let width = crate::launcher::LAUNCHER_WIDTH;
        let height = crate::launcher::LAUNCHER_HEIGHT;
        let x = (active_mon.x + (active_mon.width as i32 - width as i32) / 2).max(active_mon.x) as i16;
        let y = (active_mon.y + (active_mon.height as i32 - height as i32) / 3).max(active_mon.y) as i16;
        let rect = Rect::new(x as i32, y as i32, width as u32, height as u32);

        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let font_name = self.config.launcher_font();
        let launcher_colors = self.config.launcher_color_scheme();

        let launcher = self
            .launcher
            .get_or_insert_with(crate::launcher::Launcher::new);
        launcher.color_scheme = launcher_colors;
        launcher.border_color = Some(self.config.launcher_border_color());
        launcher.refresh_if_needed();
        launcher.rect = rect;

        if launcher.window_id == 0 {
            let win_id = crate::launcher::create_launcher_window(
                &self.context.connection,
                screen,
                rect,
                launcher.color_scheme.bg,
            )?;
            launcher.window_id = win_id;
            let _ = self.cursor_manager.apply_window(&self.context.connection, win_id);

            match crate::render::Renderer::new(
                &self.context.connection,
                win_id,
                width,
                height,
                screen.root_depth,
                font_name,
            ) {
                Ok(renderer) => {
                    launcher.attach_renderer(renderer);
                }
                Err(err) => {
                    eprintln!("[WARN  tilo::wm] Failed to initialize launcher renderer: {}", err);
                }
            }
        } else {
            // Reposition window if active monitor changed
            let _ = self.context.connection.configure_window(
                launcher.window_id,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(x as i32)
                    .y(y as i32)
                    .width(width as u32)
                    .height(height as u32),
            );
        }

        // Map and raise
        self.context.connection.map_window(launcher.window_id)?;
        self.context.connection.configure_window(
            launcher.window_id,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
        )?;

        // Set input focus directly to launcher window
        let _ = self.context.connection.set_input_focus(
            InputFocus::POINTER_ROOT,
            launcher.window_id,
            x11rb::CURRENT_TIME,
        );

        // Grab keyboard with small retries to ensure grab succeeds if another key grab was released
        for _ in 0..10 {
            if let Ok(cookie) = self.context.connection.grab_keyboard(
                false,
                launcher.window_id,
                x11rb::CURRENT_TIME,
                x11rb::protocol::xproto::GrabMode::ASYNC,
                x11rb::protocol::xproto::GrabMode::ASYNC,
            )
                && let Ok(reply) = cookie.reply()
                    && reply.status == x11rb::protocol::xproto::GrabStatus::SUCCESS {
                        break;
                    }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        launcher.mode = mode;
        launcher.visible = true;
        launcher.query.clear();
        launcher.cursor_position = 0;
        launcher.selected_index = 0;
        launcher.scroll_offset = 0;

        let _ = launcher.render(&self.context.connection);
        self.context.flush()?;
        Ok(())
    }

    /// Closes and unmaps the application launcher overlay, releasing keyboard grab.
    pub fn close_launcher(&mut self) -> Result<(), StatusbarError> {
        if let Some(ref mut launcher) = self.launcher
            && launcher.visible {
                launcher.visible = false;
                launcher.mode = crate::launcher::LauncherMode::App;
                launcher.query.clear();
                launcher.cursor_position = 0;
                launcher.selected_index = 0;
                launcher.scroll_offset = 0;
                let _ = self.context.connection.ungrab_keyboard(x11rb::CURRENT_TIME);
                let _ = launcher.destroy(&self.context.connection);
                self.context.flush()?;

                // Restore focus to active client window or clear focus
                if let Some(focused) = self.focused_window() {
                    let _ = self.focus_client(focused);
                } else {
                    let _ = self.clear_focus();
                }
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::malloc_trim(0);
                }
            }
        Ok(())
    }

    /// Ensures the launcher overlay maintains top priority above all open windows.
    pub fn ensure_launcher_on_top(&self) -> Result<(), X11Error> {
        if let Some(ref launcher) = self.launcher
            && launcher.visible
            && launcher.window_id != 0
        {
            let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
            let _ = self.context.connection.configure_window(launcher.window_id, &aux);
            let _ = self.context.flush();
        }
        Ok(())
    }

    /// Toggles the visibility of the application launcher overlay.
    pub fn toggle_launcher(&mut self) -> Result<(), StatusbarError> {
        let is_visible_app = self.launcher.as_ref().map(|l| l.visible && l.mode == crate::launcher::LauncherMode::App).unwrap_or(false);
        if is_visible_app {
            self.close_launcher()
        } else {
            self.open_launcher()
        }
    }

    /// Renders the launcher UI if visible.
    pub fn render_launcher(&mut self) -> Result<(), StatusbarError> {
        if let Some(ref mut launcher) = self.launcher
            && launcher.visible
            && let Err(err) = launcher.render(&self.context.connection)
        {
            eprintln!("[WARN  tilo::wm] Launcher render failed: {}", err);
            return Err(err);
        }
        Ok(())
    }

    /// Handles KeyPress events when the launcher overlay is active.
    pub fn handle_launcher_key(&mut self, event: &KeyPressEvent) -> Result<(), X11Error> {
        let mapping = match self.input_manager.mapping.as_ref() {
            Some(m) => m,
            None => return Ok(()),
        };

        if let Some(action) = crate::launcher::handle_launcher_keypress(event.detail, u16::from(event.state), mapping) {
            match action {
                crate::launcher::LauncherAction::Cancel => {
                    let _ = self.close_launcher();
                }
                crate::launcher::LauncherAction::Refresh => {
                    if let Some(ref mut launcher) = self.launcher {
                        launcher.refresh();
                    }
                    let _ = self.render_launcher();
                }
                crate::launcher::LauncherAction::Execute => {
                    let mut power_action: Option<String> = None;
                    let mut cmd_to_spawn: Option<String> = None;

                    if let Some(ref launcher) = self.launcher {
                        if launcher.mode == crate::launcher::LauncherMode::Exit {
                            let results = launcher.all_filtered_results();
                            let selected = results.get(launcher.selected_index).cloned()
                                .unwrap_or_else(|| launcher.query.trim().to_string());
                            power_action = Some(selected);
                        } else {
                            let results = launcher.all_filtered_results();
                            let trimmed = launcher.query.trim();
                            let candidate = if trimmed.is_empty() {
                                results.get(launcher.selected_index).map(|s| launcher.command_for(s))
                            } else if let Some(selected) = results.get(launcher.selected_index) {
                                let sel_cmd = launcher.command_for(selected);
                                if !trimmed.contains(' ') {
                                    Some(sel_cmd)
                                } else {
                                    let first_word = trimmed.split_whitespace().next().unwrap_or("");
                                    let sel_binary = sel_cmd.split_whitespace().next().unwrap_or(&sel_cmd);
                                    let sel_bin_name = std::path::Path::new(sel_binary)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(sel_binary);

                                    if trimmed.eq_ignore_ascii_case(selected) {
                                        Some(sel_cmd)
                                    } else if first_word.eq_ignore_ascii_case(selected)
                                        || first_word.eq_ignore_ascii_case(sel_binary)
                                        || first_word.eq_ignore_ascii_case(sel_bin_name)
                                    {
                                        let args_part = &trimmed[first_word.len()..];
                                        Some(format!("{}{}", sel_cmd, args_part))
                                    } else if trimmed.to_lowercase().starts_with(&selected.to_lowercase())
                                        && trimmed.len() > selected.len()
                                        && trimmed[selected.len()..].starts_with(' ')
                                    {
                                        let args_part = &trimmed[selected.len()..];
                                        Some(format!("{}{}", sel_cmd, args_part))
                                    } else {
                                        Some(sel_cmd)
                                    }
                                }
                            } else {
                                Some(trimmed.to_string())
                            };

                            if let Some(ref c) = candidate {
                                let c_lower = c.trim().to_lowercase();
                                if c_lower == "logout" || c_lower == "reboot" || c_lower == "poweroff" || c_lower == "shutdown" {
                                    power_action = Some(c.clone());
                                } else {
                                    cmd_to_spawn = candidate;
                                }
                            }
                        }
                    }

                    let _ = self.close_launcher();

                    if let Some(action) = power_action {
                        self.execute_power_action(&action);
                    } else if let Some(cmd) = cmd_to_spawn
                        && let Err(e) = crate::launcher::spawn_app(&cmd) {
                            eprintln!("[WARN  tilo::launcher] {}", e);
                        }
                }
                other => {
                    if let Some(ref mut launcher) = self.launcher {
                        crate::launcher::update_launcher_state(launcher, other);
                    }
                    let _ = self.render_launcher();
                }
            }
        }
        Ok(())
    }

    /// Executes a system power / exit action (Logout, Reboot, Poweroff).
    pub fn execute_power_action(&mut self, action_name: &str) {
        let lower = action_name.trim().to_lowercase();
        match lower.as_str() {
            "logout" | "exit" | "quit" => {
                let _ = self.perform_desktop_shutdown(None);
            }
            "reboot" | "restart" => {
                let _ = self.perform_desktop_shutdown(None);
                let _ = std::process::Command::new("systemctl")
                    .arg("reboot")
                    .spawn()
                    .or_else(|_| std::process::Command::new("reboot").spawn());
            }
            "poweroff" | "power off" | "shutdown" => {
                let _ = self.perform_desktop_shutdown(None);
                let _ = std::process::Command::new("systemctl")
                    .arg("poweroff")
                    .spawn()
                    .or_else(|_| std::process::Command::new("poweroff").spawn());
            }
            other => {
                eprintln!("[WARN  tilo::wm] Unknown power action: {}", other);
            }
        }
    }

}
