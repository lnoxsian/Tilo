//! Window Manager UI integration (Statusbar and Launcher).

use super::WindowManager;
use crate::bar::{Statusbar, StatusbarError};
use crate::layout::{LayoutType, Rect};
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, InputFocus, KeyPressEvent, StackMode,
};

impl WindowManager {
    pub fn enable_statusbar(&mut self, height: u32) -> Result<u32, StatusbarError> {
        let rect = Rect::new(0, 0, self.context.screen_width as u32, height);
        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let bar_colors = self.config.bar_color_scheme();
        let win_id = crate::bar::create_statusbar_window_with_color(&self.context.connection, screen, rect, bar_colors.bg)?;
        let _ = self.cursor_manager.apply_window(&self.context.connection, win_id);
        let mut bar = Statusbar::default_with_window(win_id, rect);
        bar.color_scheme = bar_colors;
        bar.separator = self.config.bar_separator().to_string();
        let occupied = self.workspace_manager.occupied_workspaces();
        bar.set_workspace(self.current_workspace, &occupied);

        // Initialize and attach double-buffered renderer
        let font_name = self.config.bar_font();
        match crate::render::Renderer::new(
            &self.context.connection,
            win_id,
            rect.width as u16,
            rect.height as u16,
            screen.root_depth,
            font_name,
        ) {
            Ok(renderer) => {
                bar.attach_renderer(renderer);
            }
            Err(err) => {
                eprintln!("[WARN  tilo::wm] Failed to initialize statusbar renderer: {}", err);
            }
        }
        let _ = bar.update_all();

        self.statusbar = Some(bar);
        let _ = self.render_statusbar();
        let _ = self.retile();
        Ok(win_id)
    }

    /// Disables and destroys the built-in statusbar, releasing reserved screen space.
    pub fn disable_statusbar(&mut self) -> Result<(), X11Error> {
        if let Some(bar) = self.statusbar.take() {
            bar.destroy(&self.context.connection)?;
            self.retile()?;
        }
        Ok(())
    }

    /// Updates all statusbar modules and re-renders if metrics or text changed.
    pub fn update_statusbar(&mut self) -> bool {
        let changed = if let Some(ref mut bar) = self.statusbar {
            bar.update_all()
        } else {
            false
        };
        if changed {
            let _ = self.render_statusbar();
        }
        changed
    }

    /// Redraws the statusbar UI using the attached renderer.
    pub fn render_statusbar(&mut self) -> Result<(), StatusbarError> {
        let current_ws = self.current_workspace;
        let occupied = self.workspace_manager.occupied_workspaces();
        let focused = self.focused_window();
        let ws_layout = self.workspace_manager.get(current_ws).map(|ws| ws.layout).unwrap_or_default();
        let layout_symbol = match focused {
            Some(w) if self.client_state.get_client(w).map(|c| c.floating).unwrap_or(false) => "><>",
            _ => ws_layout.symbol(),
        };
        let title = focused
            .and_then(|w| self.client_state.get_client(w).and_then(|c| c.title.clone()))
            .unwrap_or_else(|| focused.map(|w| self.get_window_title(w)).unwrap_or_default());
        let tabs = if ws_layout == LayoutType::Tabbed {
            Some(self.get_workspace_tabs(current_ws))
        } else {
            None
        };
        if let Some(ref mut bar) = self.statusbar {
            bar.set_title(&title);
            if let Err(err) = bar.render(
                &self.context.connection,
                current_ws,
                &occupied,
                layout_symbol,
                &title,
                tabs.as_deref(),
                &self.config.tab_config,
            ) {
                eprintln!("[WARN  tilo::wm] Statusbar render failed: {}", err);
                return Err(err);
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

    /// Opens the application launcher overlay with the given mode, centering it on the active monitor and grabbing keyboard.
    pub fn open_launcher_with_mode(&mut self, mode: crate::launcher::LauncherMode) -> Result<(), StatusbarError> {
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
            && launcher.visible {
                if let Err(err) = launcher.render(&self.context.connection) {
                    eprintln!("[WARN  tilo::wm] Launcher render failed: {}", err);
                    return Err(err);
                }
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
                            let results = launcher.filtered_results(8);
                            let selected = results.get(launcher.selected_index).cloned()
                                .unwrap_or_else(|| launcher.query.trim().to_string());
                            power_action = Some(selected);
                        } else {
                            let results = launcher.filtered_results(8);
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
                let _ = self.perform_graceful_shutdown();
            }
            "reboot" | "restart" => {
                let _ = std::process::Command::new("systemctl")
                    .arg("reboot")
                    .spawn()
                    .or_else(|_| std::process::Command::new("reboot").spawn());
            }
            "poweroff" | "power off" | "shutdown" => {
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
