//! Window Manager core and event loop.
//!
//! This module provides the central [`WindowManager`] struct and event dispatcher
//! which intercepts and processes all X11 events (MapRequest, ConfigureRequest, etc.),
//! manages client tracking, focus, window movement, and graceful window termination.

use crate::atoms::Atoms;
use crate::client::{Client, ClientState};
use crate::config::{load_config, load_config_or_default, Config, ReloadError};
use crate::ewmh::{
    init_ewmh, update_net_active_window, update_net_client_list, update_net_current_desktop,
    update_net_wm_state_above, update_net_wm_state_fullscreen, EwmhAtoms, NetWmStateAction,
};
use crate::floating::{calculate_default_floating_geometry, clamp_floating_geometry};
use crate::focus::{
    find_neighbor, set_window_border_color, set_window_border_width, BorderConfig, Direction,
    WorkspaceFocus,
};
use crate::fullscreen::{fullscreen_configure_aux, restore_configure_aux, Geometry};
use crate::monitor::{query_monitors, setup_randr_events, MonitorId, MonitorManager};
use crate::icccm::{
    get_wm_hints, get_wm_normal_hints, get_wm_state, send_wm_delete as icccm_send_wm_delete,
    set_wm_state as icccm_set_wm_state, IcccmAtoms, WmState,
};
use crate::input::InputManager;
use crate::keybind::{parse_modifier, Action, MOD_LOCK, MOD_SUPER};
use crate::layout::{apply_layout_with_border, calculate_master_stack, LayoutConfig, Rect};
use crate::rules::{apply_rules as apply_window_rules, RuleAction, WindowRule};
use crate::statusbar::{create_statusbar_window, Statusbar, StatusbarError};
use crate::workspace::{WorkspaceId, WorkspaceManager};
use crate::x11::{X11Context, X11Error};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Allow, AtomEnum, ButtonPressEvent, ButtonReleaseEvent, ChangeWindowAttributesAux, ClientMessageEvent,
    ConfigWindow, ConfigureRequestEvent, ConfigureWindowAux, ConnectionExt, EventMask, GrabMode, InputFocus,
    KeyPressEvent, MotionNotifyEvent, PropertyNotifyEvent, StackMode, Window,
};
use std::sync::atomic::{AtomicBool, Ordering};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

/// Global flag for SIGUSR1 signal (toggles launcher).
pub static LAUNCHER_SIGNAL: AtomicBool = AtomicBool::new(false);
/// Global flag for SIGHUP signal (reloads config).
pub static RELOAD_SIGNAL: AtomicBool = AtomicBool::new(false);

extern "C" fn sigusr1_handler(_: libc::c_int) {
    LAUNCHER_SIGNAL.store(true, Ordering::SeqCst);
}

extern "C" fn sighup_handler(_: libc::c_int) {
    RELOAD_SIGNAL.store(true, Ordering::SeqCst);
}

/// Registers POSIX signal handlers for runtime control.
pub fn setup_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGUSR1, sigusr1_handler as *const () as usize);
        libc::signal(libc::SIGHUP, sighup_handler as *const () as usize);
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }
}

/// Statistics on events processed by the window manager.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventStats {
    /// Number of MapRequest events handled.
    pub map_requests: usize,
    /// Number of ConfigureRequest events handled.
    pub configure_requests: usize,
    /// Number of DestroyNotify events handled.
    pub destroy_notifications: usize,
    /// Number of UnmapNotify events handled.
    pub unmap_notifications: usize,
    /// Number of PropertyNotify events handled.
    pub property_notifications: usize,
    /// Number of KeyPress events handled.
    pub key_presses: usize,
    /// Number of ButtonPress events handled.
    pub button_presses: usize,
    /// Number of ClientMessage events handled.
    pub client_messages: usize,
    /// Total events dispatched.
    pub total_events: usize,
}

/// The main Window Manager application state.
pub struct WindowManager {
    /// The X11 connection and state.
    pub context: X11Context,
    /// Client management state.
    pub client_state: ClientState,
    /// Indicates if the event loop should keep running.
    pub running: bool,
    /// Cumulative event dispatch statistics.
    pub stats: EventStats,
    /// Cached interned X11 atoms for ICCCM and EWMH.
    pub atoms: Atoms,
    /// Active tiling layout configuration (ratios, gaps, borders).
    pub layout_config: LayoutConfig,
    /// Window border and focus appearance configuration.
    pub border_config: BorderConfig,
    /// Workspace-specific focus state tracker.
    pub workspace_focus: WorkspaceFocus,
    /// Currently active workspace ID.
    pub current_workspace: u32,
    /// Keyboard input manager handling global key grabs and actions.
    pub input_manager: InputManager,
    /// Workspace manager tracking virtual desktops and client allocation.
    pub workspace_manager: WorkspaceManager,
    /// Configured window rules for matching window properties.
    pub rules: Vec<WindowRule>,
    /// Cached interned X11 atoms for EWMH.
    pub ewmh_atoms: EwmhAtoms,
    /// Window ID of the hidden child window used for `_NET_SUPPORTING_WM_CHECK`.
    pub supporting_window: Window,
    /// Multi-monitor manager tracking geometry and active workspaces per monitor.
    pub monitor_manager: MonitorManager,
    /// Parsed user configuration.
    pub config: Config,
    /// Built-in statusbar instance if enabled.
    pub statusbar: Option<Statusbar>,
    /// Built-in application launcher instance if initialized.
    pub launcher: Option<crate::launcher::Launcher>,
    /// Dedicated floating scratchpad state (Phase 23).
    pub scratchpad: crate::scratchpad::Scratchpad,
    /// Active mouse drag interaction state (Phase 24).
    pub mouse_action: crate::mouse::MouseAction,
    /// Registered mouse button bindings (Phase 24).
    pub mouse_bindings: Vec<crate::mouse::MouseBinding>,
}

impl WindowManager {
    /// Creates a new `WindowManager` wrapping the provided `X11Context` with default or loaded config.
    pub fn new(context: X11Context) -> Result<Self, X11Error> {
        let config = load_config_or_default();
        Self::new_with_config(context, config)
    }

    /// Creates a new `WindowManager` wrapping the provided `X11Context` and custom [`Config`].
    pub fn new_with_config(context: X11Context, config: Config) -> Result<Self, X11Error> {
        let atoms = Atoms::intern(&context.connection)?;
        let ewmh_atoms = EwmhAtoms::intern(&context.connection)?;
        let supporting_window = init_ewmh(
            &context.connection,
            context.root_window,
            &ewmh_atoms,
            context.screen_width,
            context.screen_height,
            9,
            1,
        )?;

        let mut border_config = BorderConfig::default();
        border_config.width = config.border_width;
        border_config.focused_color = config.focused_color;
        border_config.normal_color = config.normal_color;
        border_config.urgent_color = config.urgent_color;

        let mut layout_config = LayoutConfig::default();
        layout_config.border_width = config.border_width;
        layout_config.inner_gap = config.inner_gap;
        layout_config.outer_gap = config.outer_gap;
        layout_config.master_ratio = config.master_ratio;

        let mut input_manager = InputManager::new(config.bindings.clone());
        let _ = input_manager.setup(&context.connection, context.root_window);
        let workspace_manager = WorkspaceManager::new();

        let mut monitor_manager = MonitorManager::new(
            context.screen_width as u32,
            context.screen_height as u32,
        );
        let _ = setup_randr_events(&context.connection, context.root_window);
        if let Ok(randr_monitors) = query_monitors(
            &context.connection,
            context.root_window,
            context.screen_width as u32,
            context.screen_height as u32,
        ) {
            let _ = monitor_manager.reconcile_monitors(randr_monitors);
        }

        let mod_mask = parse_modifier(&config.mod_key).unwrap_or(MOD_SUPER);
        let mouse_bindings = crate::mouse::default_mouse_bindings(mod_mask);

        Ok(Self {
            context,
            client_state: ClientState::new(),
            running: true,
            stats: EventStats::default(),
            atoms,
            layout_config,
            border_config,
            workspace_focus: WorkspaceFocus::new(),
            current_workspace: 1,
            input_manager,
            workspace_manager,
            rules: Vec::new(),
            ewmh_atoms,
            supporting_window,
            monitor_manager,
            config,
            statusbar: None,
            launcher: Some(crate::launcher::Launcher::new()),
            scratchpad: crate::scratchpad::Scratchpad::default(),
            mouse_action: crate::mouse::MouseAction::None,
            mouse_bindings,
        })
    }

    /// Applies a new [`Config`] to the running window manager, updating borders,
    /// layouts, gaps, and registering the new keybindings.
    pub fn apply_config(&mut self, config: Config) -> Result<(), X11Error> {
        self.border_config.width = config.border_width;
        self.border_config.focused_color = config.focused_color;
        self.border_config.normal_color = config.normal_color;
        self.border_config.urgent_color = config.urgent_color;
        self.layout_config.border_width = config.border_width;
        self.layout_config.inner_gap = config.inner_gap;
        self.layout_config.outer_gap = config.outer_gap;
        self.layout_config.master_ratio = config.master_ratio;

        self.input_manager.re_register(
            &self.context.connection,
            self.context.root_window,
            config.bindings.clone(),
        )?;

        let mod_mask = parse_modifier(&config.mod_key).unwrap_or(MOD_SUPER);
        self.mouse_bindings = crate::mouse::default_mouse_bindings(mod_mask);
        let focused = self.client_state.focused_window();
        for &win in &self.client_state.all_clients() {
            let _ = crate::mouse::update_client_button_grabs(
                &self.context.connection,
                win,
                &self.mouse_bindings,
                self.input_manager.num_lock_mask,
                focused == Some(win),
            );
        }

        self.config = config;

        // Apply updated border width and colors to all existing client windows
        let focused = self.client_state.focused_window();
        for &win in &self.client_state.all_clients() {
            let is_fullscreen = self
                .client_state
                .get_client(win)
                .map(|c| c.fullscreen)
                .unwrap_or(false);
            if !is_fullscreen {
                let _ = set_window_border_width(
                    &self.context.connection,
                    win,
                    self.border_config.width,
                );
                let border_color = if focused == Some(win) {
                    self.border_config.focused_color
                } else {
                    self.border_config.normal_color
                };
                let _ = set_window_border_color(&self.context.connection, win, border_color);
            }
        }

        self.retile()?;
        self.context.flush()?;
        Ok(())
    }

    /// Reloads the configuration from disk while the window manager is running.
    ///
    /// If loading or parsing fails, the previous configuration is retained and
    /// an error is logged to stderr.
    pub fn reload_config(&mut self) -> Result<(), ReloadError> {
        let new_cfg = match load_config() {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("[ERROR tilo::wm] Failed to reload config: {}", err);
                return Err(ReloadError::ConfigInvalid(err));
            }
        };
        self.apply_config(new_cfg)?;
        println!("[INFO  tilo::wm] Configuration reloaded successfully.");
        Ok(())
    }

    /// Reloads configuration from a specific file path on disk.
    ///
    /// If loading or parsing fails, the previous configuration is retained and
    /// an error is logged to stderr.
    pub fn reload_config_from_path<P: AsRef<std::path::Path>>(
        &mut self,
        path: P,
    ) -> Result<(), ReloadError> {
        let new_cfg = match crate::config::load_config_from_path(path) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("[ERROR tilo::wm] Failed to reload config: {}", err);
                return Err(ReloadError::ConfigInvalid(err));
            }
        };
        self.apply_config(new_cfg)?;
        println!("[INFO  tilo::wm] Configuration reloaded successfully.");
        Ok(())
    }

    /// Enables the built-in statusbar at the top of the primary screen.
    ///
    /// Creates an `override_redirect` window, reserves screen space via struts,
    /// Enables the built-in statusbar at the top of the primary screen.
    ///
    /// Creates an `override_redirect` window, reserves screen space via struts,
    /// initializes the double-buffered renderer, and recalculates tiling layouts.
    pub fn enable_statusbar(&mut self, height: u32) -> Result<u32, StatusbarError> {
        let rect = Rect::new(0, 0, self.context.screen_width as u32, height);
        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let win_id = create_statusbar_window(&self.context.connection, screen, rect)?;
        let mut bar = Statusbar::default_with_window(win_id, rect);
        let occupied = self.workspace_manager.occupied_workspaces();
        bar.set_workspace(self.current_workspace, &occupied);

        // Initialize and attach double-buffered renderer
        let font_name = if !self.config.font.is_empty() {
            self.config.font.as_str()
        } else {
            "monospace"
        };
        if let Ok(renderer) = crate::render::Renderer::new(
            &self.context.connection,
            win_id,
            rect.width as u16,
            rect.height as u16,
            screen.root_depth,
            font_name,
        ) {
            bar.attach_renderer(renderer);
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

    /// Updates all statusbar modules and re-renders if metrics, text, or resolution changed.
    pub fn update_statusbar(&mut self) -> bool {
        let resized = self.check_screen_resize();
        let changed = if let Some(ref mut bar) = self.statusbar {
            bar.update_all()
        } else {
            false
        };
        if changed || resized {
            let _ = self.render_statusbar();
        }
        changed || resized
    }

    /// Redraws the statusbar UI using the attached renderer.
    pub fn render_statusbar(&mut self) -> Result<(), StatusbarError> {
        if let Some(ref bar) = self.statusbar {
            let current_ws = self.current_workspace;
            let occupied = self.workspace_manager.occupied_workspaces();
            let layout_symbol = match self.client_state.focused_window() {
                Some(w) if self.client_state.get_client(w).map(|c| c.floating).unwrap_or(false) => "><>",
                _ => "[]=",
            };
            let focused = self.client_state.focused_window();
            let title = focused.map(|w| self.get_window_title(w)).unwrap_or_default();
            bar.render(
                &self.context.connection,
                current_ws,
                &occupied,
                layout_symbol,
                &title,
            )?;
        }
        Ok(())
    }

    // -- Application Launcher (Phase 19, 20, 21) --

    /// Opens the application launcher overlay, centering it on the active monitor and grabbing keyboard.
    pub fn open_launcher(&mut self) -> Result<(), StatusbarError> {
        let active_mon = self.monitor_manager.active_monitor().clone();
        let width = crate::launcher::LAUNCHER_WIDTH;
        let height = crate::launcher::LAUNCHER_HEIGHT;
        let x = (active_mon.x + (active_mon.width as i32 - width as i32) / 2).max(active_mon.x) as i16;
        let y = (active_mon.y + (active_mon.height as i32 - height as i32) / 3).max(active_mon.y) as i16;
        let rect = Rect::new(x as i32, y as i32, width as u32, height as u32);

        let screen = &self.context.connection.setup().roots[self.context.screen_num];
        let font_name = if !self.config.font.is_empty() {
            self.config.font.as_str()
        } else {
            "monospace"
        };

        if self.launcher.is_none() {
            self.launcher = Some(crate::launcher::Launcher::new());
        }

        let launcher = self.launcher.as_mut().unwrap();
        launcher.rect = rect;

        if launcher.window_id == 0 {
            let win_id = crate::launcher::create_launcher_window(
                &self.context.connection,
                screen,
                rect,
                launcher.color_scheme.bg,
            )?;
            launcher.window_id = win_id;

            if let Ok(renderer) = crate::render::Renderer::new(
                &self.context.connection,
                win_id,
                width,
                height,
                screen.root_depth,
                font_name,
            ) {
                launcher.attach_renderer(renderer);
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
            ) {
                if let Ok(reply) = cookie.reply() {
                    if reply.status == x11rb::protocol::xproto::GrabStatus::SUCCESS {
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

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
        if let Some(ref mut launcher) = self.launcher {
            if launcher.visible {
                launcher.visible = false;
                launcher.query.clear();
                launcher.cursor_position = 0;
                launcher.selected_index = 0;
                let _ = self.context.connection.ungrab_keyboard(x11rb::CURRENT_TIME);
                let _ = self.context.connection.unmap_window(launcher.window_id);
                self.context.flush()?;

                // Restore focus to active client window or root
                if let Some(focused) = self.client_state.focused_window() {
                    let _ = self.focus_client(focused);
                } else {
                    let _ = self.context.connection.set_input_focus(
                        InputFocus::POINTER_ROOT,
                        self.context.root_window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.context.flush();
                }
            }
        }
        Ok(())
    }

    /// Toggles the visibility of the application launcher overlay.
    pub fn toggle_launcher(&mut self) -> Result<(), StatusbarError> {
        let is_visible = self.launcher.as_ref().map(|l| l.visible).unwrap_or(false);
        if is_visible {
            self.close_launcher()
        } else {
            self.open_launcher()
        }
    }

    /// Renders the launcher UI if visible.
    pub fn render_launcher(&mut self) -> Result<(), StatusbarError> {
        if let Some(ref mut launcher) = self.launcher {
            if launcher.visible {
                launcher.render(&self.context.connection)?;
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
                crate::launcher::LauncherAction::Execute => {
                    let cmd_to_spawn = if let Some(ref launcher) = self.launcher {
                        let results = launcher.filtered_results(8);
                        let trimmed = launcher.query.trim();
                        if trimmed.is_empty() {
                            results.get(launcher.selected_index).cloned()
                        } else if trimmed.contains(' ') {
                            // User typed arguments: e.g. "firefox --private-window"
                            // If first word matches selected result, use selected binary with the typed arguments
                            let first_word = trimmed.split_whitespace().next().unwrap_or("");
                            let args_part = &trimmed[first_word.len()..];
                            let binary = if let Some(sel) = results.get(launcher.selected_index) {
                                sel.as_str()
                            } else {
                                first_word
                            };
                            Some(format!("{}{}", binary, args_part))
                        } else if let Some(selected) = results.get(launcher.selected_index) {
                            Some(selected.clone())
                        } else {
                            Some(trimmed.to_string())
                        }
                    } else {
                        None
                    };

                    let _ = self.close_launcher();

                    if let Some(cmd) = cmd_to_spawn {
                        if let Err(e) = crate::launcher::spawn_app(&cmd) {
                            eprintln!("[WARN  tilo::launcher] {}", e);
                        }
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
        if let Some(win) = self.scratchpad.client {
            if !self.client_state.contains(win) {
                self.scratchpad.clear_client();
            }
        }

        if self.scratchpad.client.is_none() {
            // Spawn default terminal emulator as scratchpad
            let _ = crate::launcher::spawn_app(&self.config.terminal);
            return Ok(());
        }

        let win = self.scratchpad.client.unwrap();
        if self.scratchpad.is_visible {
            // Hide: unmap window
            let _ = self.context.connection.unmap_window(win);
            self.scratchpad.is_visible = false;
            if let Some(client) = self.client_state.get_client_mut(win) {
                client.mapped = false;
            }
            let _ = self.set_wm_state(win, WmState::Iconic);

            // Restore focus if scratchpad had focus
            if self.client_state.focused_window() == Some(win) {
                let focus_candidate = self
                    .workspace_manager
                    .get(self.current_workspace)
                    .and_then(|ws| ws.clients.iter().copied().find(|&w| w != win));

                if let Some(next_focus) = focus_candidate {
                    let _ = self.focus_client(next_focus);
                } else {
                    let _ = self.context.connection.set_input_focus(
                        x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                        self.context.root_window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.update_ewmh_active_window();
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

    /// Updates the root window's `_NET_ACTIVE_WINDOW` property.
    pub fn update_ewmh_active_window(&self) -> Result<(), X11Error> {
        let focused = self.client_state.focused_window();
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
            &clients,
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
    pub fn run_autostart(&self) -> Result<(), crate::autostart::AutostartError> {
        crate::autostart::run_autostart_commands(&self.config.autostart)
    }

    /// Starts the main event loop, blocking the thread and waiting for X11 events.
    pub fn run(&mut self) -> Result<(), X11Error> {
        setup_signal_handlers();
        let _ = self.run_autostart();
        self.context.flush()?;
        let mut last_status_update = std::time::Instant::now();
        while self.running {
            if LAUNCHER_SIGNAL.swap(false, Ordering::SeqCst) {
                let _ = self.toggle_launcher();
            }
            if RELOAD_SIGNAL.swap(false, Ordering::SeqCst) {
                let _ = self.reload_config();
            }

            let mut got_event = false;
            while let Some(event) = self.context.connection.poll_for_event()? {
                self.handle_event(event)?;
                got_event = true;
            }

            if last_status_update.elapsed() >= std::time::Duration::from_millis(250) {
                self.update_statusbar();
                last_status_update = std::time::Instant::now();
            }

            if got_event {
                self.context.flush()?;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        Ok(())
    }

    /// Polls and processes all currently buffered events from the X server without blocking.
    ///
    /// # Returns
    /// The number of events processed.
    pub fn process_pending_events(&mut self) -> Result<usize, X11Error> {
        let mut count = 0;
        while let Some(event) = self.context.connection.poll_for_event()? {
            self.handle_event(event)?;
            count += 1;
        }
        if count > 0 {
            self.context.flush()?;
        }
        Ok(count)
    }

    /// Signals the event loop to stop running.
    pub fn stop(&mut self) {
        self.running = false;
    }

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
                if let Some(ref launcher) = self.launcher {
                    if launcher.visible {
                        self.handle_launcher_key(&ev)?;
                        return Ok(());
                    }
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
                if ev.window == self.context.root_window || ev.event == self.context.root_window {
                    let _ = self.refresh_monitors();
                }
            }
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => {
                let _ = self.refresh_monitors();
            }
            Event::Expose(ev) => {
                if let Some(ref bar) = self.statusbar {
                    if ev.window == bar.window_id {
                        let _ = self.render_statusbar();
                    }
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
            _ => {}
        }
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
        let attrs = match self.context.connection.get_window_attributes(window)?.reply() {
            Ok(attrs) => attrs,
            Err(err) => {
                eprintln!(
                    "[WARN  tilo::wm] Failed to get attributes for window 0x{:x}: {:?}",
                    window, err
                );
                return Ok(false);
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
        let rule_action = self.evaluate_window_rules(window)?;
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

            // Map, retile, and focus client window on active workspace
            self.map_client(window)?;
            let _ = self.set_wm_state(window, WmState::Normal);
            self.retile()?;
            self.focus_client(window)?;
        } else {
            // Target is a background workspace: window remains unmapped until switched to
            client.mapped = false;
            self.workspace_manager.add_client(window, target_ws);
            self.client_state.add_client(window, client);
            if is_floating {
                let _ = update_net_wm_state_above(&self.context.connection, window, true, &self.ewmh_atoms);
            }
            if is_fullscreen {
                let _ = update_net_wm_state_fullscreen(&self.context.connection, window, true, &self.ewmh_atoms);
            }
            let _ = self.set_wm_state(window, WmState::Withdrawn);
        }

        let is_focused = self.client_state.focused_window() == Some(window);
        let _ = crate::mouse::update_client_button_grabs(
            &self.context.connection,
            window,
            &self.mouse_bindings,
            self.input_manager.num_lock_mask,
            is_focused,
        );
        let _ = self.update_ewmh_client_list();
        Ok(true)
    }

    /// Deregisters and unmanages a window, removing it from `client_state`.
    ///
    /// If the window was previously focused, input focus is transferred to the
    /// previous window on the workspace/focus stack or reverted to the root window.
    pub fn unmanage_window(&mut self, window: Window) -> Option<Client> {
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
        let _ = self.set_wm_state(window, WmState::Withdrawn);
        let _ = self.update_ewmh_client_list();
        if removed.is_some() {
            let _ = self.retile();
            // Restore focus to previous client on workspace, or global stack, or root
            if let Some(next_window) = next_ws_win.or_else(|| self.client_state.focused_window()) {
                let _ = self.focus_client(next_window);
            } else {
                let _ = self.context.connection.set_input_focus(
                    InputFocus::POINTER_ROOT,
                    self.context.root_window,
                    x11rb::CURRENT_TIME,
                );
                let _ = self.update_ewmh_active_window();
                if let Some(ref mut bar) = self.statusbar {
                    bar.set_title("");
                }
                let _ = self.render_statusbar();
            }
        }
        removed
    }

    /// Recalculates and applies the tiling layout to all visible managed clients on each monitor's active workspace.
    pub fn retile(&mut self) -> Result<(), X11Error> {
        let monitors = self.monitor_manager.monitors.clone();
        for monitor in &monitors {
            let mon_ws = monitor.workspace;
            let monitor_rect = monitor.rect();
            let usable_rect = if let Some(ref bar) = self.statusbar {
                if bar.rect.y as i32 <= monitor.y
                    && (bar.rect.y as i32 + bar.rect.height as i32) > monitor.y
                {
                    let offset = (bar.rect.y as i32 + bar.rect.height as i32) - monitor.y;
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

            let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(mon_ws);

            if !visible_tiled.is_empty() {
                let rects = calculate_master_stack(&visible_tiled, &usable_rect, &self.layout_config);
                apply_layout_with_border(
                    &self.context.connection,
                    &visible_tiled,
                    &rects,
                    self.layout_config.border_width,
                )?;
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
        let prev_focused = self.client_state.focused_window();

        // 1. Reset previous window's border to normal color
        if let Some(prev) = prev_focused {
            if prev != window && self.client_state.contains(prev) {
                let _ = set_window_border_color(
                    &self.context.connection,
                    prev,
                    self.border_config.normal_color,
                );
            }
        }

        // 2. Set new window's border to active focused color
        let _ = set_window_border_color(
            &self.context.connection,
            window,
            self.border_config.focused_color,
        );

        // 3. Set X11 input focus
        self.context.connection.set_input_focus(
            InputFocus::POINTER_ROOT,
            window,
            x11rb::CURRENT_TIME,
        )?;

        // 4. Raise window in stacking order
        let aux = ConfigureWindowAux::default().stack_mode(StackMode::ABOVE);
        let _ = self.context.connection.configure_window(window, &aux);

        // 5. Send WM_TAKE_FOCUS if supported by client (ICCCM)
        let _ = self.send_take_focus_if_supported(window);

        // 6. Update internal focus tracking
        self.client_state.focus(window);
        let ws = self
            .client_state
            .get_client(window)
            .map(|c| c.workspace)
            .unwrap_or(self.current_workspace);
        self.workspace_focus.focus(ws, window);

        // 7. Update EWMH _NET_ACTIVE_WINDOW
        let _ = self.update_ewmh_active_window();

        // 8. Update statusbar window title if active
        let title = self.get_window_title(window);
        if let Some(ref mut bar) = self.statusbar {
            bar.set_title(&title);
        }
        let _ = self.render_statusbar();

        // 9. Update mouse button grabs for click-to-focus management
        if let Some(prev) = prev_focused {
            if prev != window && self.client_state.contains(prev) {
                let _ = crate::mouse::update_client_button_grabs(
                    &self.context.connection,
                    prev,
                    &self.mouse_bindings,
                    self.input_manager.num_lock_mask,
                    false,
                );
            }
        }
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
        let current = self.client_state.focused_window();
        let current_ws = self.current_workspace;
        let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
        if visible_tiled.is_empty() {
            return Ok(None);
        }

        let screen_rect = Rect::new(
            0,
            0,
            self.context.screen_width as u32,
            self.context.screen_height as u32,
        );
        let rects = calculate_master_stack(&visible_tiled, &screen_rect, &self.layout_config);
        let pairs: Vec<(Window, Rect)> = visible_tiled
            .into_iter()
            .zip(rects.into_iter())
            .collect();

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

        if let Some(target_mon_id) = target_mon_id {
            if target_mon_id != self.monitor_manager.active_monitor_id {
                self.monitor_manager.active_monitor_id = target_mon_id;
                self.workspace_manager.current_workspace_id = target_id;
                self.current_workspace = target_id;
                let _ = self.update_ewmh_current_desktop();

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(target_id)
                    .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.focused_client))
                    .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.clients.first().copied()));

                if let Some(focus_win) = target_focus {
                    if self.client_state.contains(focus_win) {
                        self.focus_client(focus_win)?;
                    }
                } else {
                    let _ = self.context.connection.set_input_focus(
                        InputFocus::POINTER_ROOT,
                        self.context.root_window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.update_ewmh_active_window();
                }
                self.context.flush()?;
                return Ok(());
            }
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
        if let Some(scratch_win) = self.scratchpad.client {
            if self.scratchpad.is_visible {
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
        }

        self.monitor_manager.active_monitor_mut().workspace = target_id;
        self.workspace_manager.current_workspace_id = target_id;
        self.current_workspace = target_id;
        let _ = self.update_ewmh_current_desktop();
        if let Some(ref mut bar) = self.statusbar {
            let occupied = self.workspace_manager.occupied_workspaces();
            bar.set_workspace(target_id, &occupied);
        }
        let _ = self.render_statusbar();

        self.retile()?;

        let target_focus = self
            .workspace_focus
            .focused_window_on(target_id)
            .or_else(|| self.workspace_manager.get(target_id).and_then(|ws| ws.focused_client))
            .or_else(|| target_windows.first().copied());

        if let Some(focus_win) = target_focus {
            if self.client_state.contains(focus_win) {
                self.focus_client(focus_win)?;
            }
        } else {
            let _ = self.context.connection.set_input_focus(
                InputFocus::POINTER_ROOT,
                self.context.root_window,
                x11rb::CURRENT_TIME,
            );
            let _ = self.update_ewmh_active_window();
        }

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
        let next_focus = self.workspace_focus.remove_window(current_id, window);
        if let Some(next_win) = next_focus {
            if self.client_state.contains(next_win) {
                let _ = self.focus_client(next_win);
            }
        } else if let Some(&first) = self
            .workspace_manager
            .get(current_id)
            .and_then(|ws| ws.clients.first())
        {
            let _ = self.focus_client(first);
        } else {
            self.client_state.remove_focus(window);
            let _ = self.context.connection.set_input_focus(
                InputFocus::POINTER_ROOT,
                self.context.root_window,
                x11rb::CURRENT_TIME,
            );
            let _ = self.update_ewmh_active_window();
            if let Some(ref mut bar) = self.statusbar {
                bar.set_title("");
            }
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

    /// Adds a window rule to the window manager.
    pub fn add_rule(&mut self, rule: WindowRule) {
        self.rules.push(rule);
    }

    /// Replaces the configured window rules with a new set.
    pub fn set_rules(&mut self, rules: Vec<WindowRule>) {
        self.rules = rules;
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

    /// Focuses the monitor visually to the left of the active monitor.
    pub fn focus_monitor_left(&mut self) -> Result<(), X11Error> {
        if let Some(target_id) = self.monitor_manager.focus_monitor_left() {
            if let Some(mon) = self.monitor_manager.get_monitor(target_id) {
                let ws = mon.workspace;
                self.current_workspace = ws;
                self.workspace_manager.current_workspace_id = ws;
                let _ = self.update_ewmh_current_desktop();

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(ws)
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.focused_client))
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.clients.first().copied()));

                if let Some(w) = target_focus {
                    if self.client_state.contains(w) {
                        self.focus_client(w)?;
                    }
                } else {
                    let _ = self.context.connection.set_input_focus(
                        InputFocus::POINTER_ROOT,
                        self.context.root_window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.update_ewmh_active_window();
                }
            }
        }
        self.context.flush()?;
        Ok(())
    }

    /// Focuses the monitor visually to the right of the active monitor.
    pub fn focus_monitor_right(&mut self) -> Result<(), X11Error> {
        if let Some(target_id) = self.monitor_manager.focus_monitor_right() {
            if let Some(mon) = self.monitor_manager.get_monitor(target_id) {
                let ws = mon.workspace;
                self.current_workspace = ws;
                self.workspace_manager.current_workspace_id = ws;
                let _ = self.update_ewmh_current_desktop();

                let target_focus = self
                    .workspace_focus
                    .focused_window_on(ws)
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.focused_client))
                    .or_else(|| self.workspace_manager.get(ws).and_then(|w| w.clients.first().copied()));

                if let Some(w) = target_focus {
                    if self.client_state.contains(w) {
                        self.focus_client(w)?;
                    }
                } else {
                    let _ = self.context.connection.set_input_focus(
                        InputFocus::POINTER_ROOT,
                        self.context.root_window,
                        x11rb::CURRENT_TIME,
                    );
                    let _ = self.update_ewmh_active_window();
                }
            }
        }
        self.context.flush()?;
        Ok(())
    }

    /// Moves the currently focused client to the specified monitor.
    pub fn move_to_monitor(&mut self, target_monitor_id: MonitorId) -> Result<(), X11Error> {
        let win = match self.client_state.focused_window() {
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

        if let Some(client) = self.client_state.get_client(win) {
            if client.is_floating() {
                if let Some(target_mon) = self.monitor_manager.get_monitor(target_monitor_id) {
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
            }
        }

        let next_focus = self.workspace_focus.remove_window(current_ws, win);
        if let Some(next_win) = next_focus {
            if self.client_state.contains(next_win) {
                let _ = self.focus_client(next_win);
            }
        } else if let Some(&first) = self
            .workspace_manager
            .get(current_ws)
            .and_then(|ws| ws.clients.first())
        {
            let _ = self.focus_client(first);
        } else {
            let _ = self.context.connection.set_input_focus(
                InputFocus::POINTER_ROOT,
                self.context.root_window,
                x11rb::CURRENT_TIME,
            );
            let _ = self.update_ewmh_active_window();
        }

        self.retile()?;
        self.context.flush()?;
        Ok(())
    }

    /// Refreshes monitor geometry by querying XRandR and reconciles state.
    pub fn refresh_monitors(&mut self) -> Result<(), X11Error> {
        // 1. Query the live root window geometry to get the actual current resolution
        if let Ok(reply) = self.context.connection.get_geometry(self.context.root_window) {
            if let Ok(geom) = reply.reply() {
                self.context.screen_width = geom.width;
                self.context.screen_height = geom.height;
            }
        }

        // 2. Query updated monitors from XRandR (using the updated screen dimensions as fallback)
        let new_monitors = query_monitors(
            &self.context.connection,
            self.context.root_window,
            self.context.screen_width as u32,
            self.context.screen_height as u32,
        )?;

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
        if let Some(ref mut launcher) = self.launcher {
            if launcher.visible {
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
        }

        // 6. Clamp floating windows to new screen dimensions
        for client in self.client_state.clients.values_mut() {
            if client.floating {
                if let Some(geom) = client.floating_geometry {
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
        }

        self.retile()?;
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

    // -- Event Handlers --

    /// Handles a request from a client to map (show) its window.
    pub fn handle_map_request(&mut self, window: Window) -> Result<(), X11Error> {
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
        } else if let Some(client) = self.client_state.get_client_mut(event.window) {
            if client.floating {
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
        }
        Ok(())
    }

    /// Handles the notification that a window was destroyed.
    pub fn handle_destroy_notify(&mut self, window: Window) -> Result<(), X11Error> {
        self.unmanage_window(window);
        Ok(())
    }

    /// Handles the notification that a window was unmapped.
    pub fn handle_unmap_notify(&mut self, window: Window) -> Result<(), X11Error> {
        // If the window belongs to an inactive workspace, it was unmapped intentionally
        // by the window manager during a workspace switch. Keep it tracked on its workspace!
        if let Some(ws) = self.workspace_manager.find_workspace_for(window) {
            if ws != self.current_workspace {
                return Ok(());
            }
        }
        if self.scratchpad.client == Some(window) {
            self.scratchpad.is_visible = false;
        }
        let _ = self.unmap_client(window);
        Ok(())
    }

    /// Handles window property changes (e.g. WM_NAME, WM_HINTS, WM_NORMAL_HINTS).
    pub fn handle_property_notify(&mut self, event: &PropertyNotifyEvent) -> Result<(), X11Error> {
        self.stats.property_notifications += 1;

        if event.atom == AtomEnum::WM_HINTS.into() {
            if let Ok(Some(hints)) = get_wm_hints(&self.context.connection, event.window) {
                if hints.is_urgent {
                    let _ = set_window_border_color(
                        &self.context.connection,
                        event.window,
                        self.border_config.urgent_color,
                    );
                }
            }
        } else if event.atom == AtomEnum::WM_NORMAL_HINTS.into() {
            let _ = get_wm_normal_hints(&self.context.connection, event.window);
        } else if (event.atom == AtomEnum::WM_NAME.into() || event.atom == self.atoms.net_wm_name)
            && Some(event.window) == self.client_state.focused_window()
        {
            let title = self.get_window_title(event.window);
            if let Some(ref mut bar) = self.statusbar {
                bar.set_title(&title);
            }
            let _ = self.render_statusbar();
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
                if let Some(win) = self.client_state.focused_window() {
                    let _ = self.kill_client(win);
                }
            }
            Action::Focus(dir) => {
                let _ = self.focus_in_direction(dir, false)?;
            }
            Action::Move(dir) => {
                let _ = self.swap_in_direction(dir)?;
            }
            Action::Workspace(ws) => {
                let _ = self.switch_workspace(ws)?;
            }
            Action::MoveToWorkspace(ws) => {
                let win = self.client_state.focused_window()
                    .or_else(|| self.workspace_focus.focused_window_on(self.current_workspace))
                    .or_else(|| self.workspace_manager.get(self.current_workspace).and_then(|w| w.clients.first().copied()));
                if let Some(win) = win {
                    let _ = self.move_to_workspace(win, ws)?;
                }
            }
            Action::ToggleFloating => {
                if let Some(win) = self.client_state.focused_window() {
                    let _ = self.toggle_floating(win);
                }
            }
            Action::ToggleFullscreen => {
                if let Some(win) = self.client_state.focused_window() {
                    let _ = self.toggle_fullscreen(win);
                }
            }
            Action::FocusMonitorLeft => {
                let _ = self.focus_monitor_left()?;
            }
            Action::FocusMonitorRight => {
                let _ = self.focus_monitor_right()?;
            }
            Action::MoveToMonitor(mon_id) => {
                let _ = self.move_to_monitor(mon_id)?;
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
                if let Some(focused) = self.client_state.focused_window() {
                    self.set_scratchpad_client(focused);
                }
            }
            Action::Exit => {
                self.stop();
            }
        }
        Ok(())
    }

    /// Swaps the currently focused window with its spatial neighbor in the given direction.
    pub fn swap_in_direction(&mut self, direction: Direction) -> Result<Option<Window>, X11Error> {
        let current = match self.client_state.focused_window() {
            Some(w) => w,
            None => return Ok(None),
        };
        let current_ws = self.current_workspace;
        let visible_tiled = self.client_state.visible_tiled_clients_on_workspace(current_ws);
        if visible_tiled.len() < 2 {
            return Ok(None);
        }

        let screen_rect = Rect::new(
            0,
            0,
            self.context.screen_width as u32,
            self.context.screen_height as u32,
        );
        let rects = calculate_master_stack(&visible_tiled, &screen_rect, &self.layout_config);
        let pairs: Vec<(Window, Rect)> = visible_tiled
            .into_iter()
            .zip(rects.into_iter())
            .collect();

        if let Some(target) = find_neighbor(direction, Some(current), &pairs, false) {
            let idx_a = self.client_state.client_order.iter().position(|&w| w == current);
            let idx_b = self.client_state.client_order.iter().position(|&w| w == target);
            if let (Some(a), Some(b)) = (idx_a, idx_b) {
                self.client_state.client_order.swap(a, b);
                self.retile()?;
                self.focus_client(current)?;
                return Ok(Some(target));
            }
        }
        Ok(None)
    }

    /// Raises a window to the top of the stack.
    pub fn raise_window(&self, window: Window) -> Result<(), X11Error> {
        let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        self.context.connection.configure_window(window, &aux)?;
        self.context.flush()?;
        Ok(())
    }

    /// Handles mouse button press events on client windows.
    ///
    /// Supports Phase 24:
    /// - Mod + Left Click: focus and drag window to move.
    /// - Mod + Right Click: focus and drag window to resize.
    /// - Normal Left Click: focus clicked window and replay event to client.
    pub fn handle_button_press(&mut self, event: &ButtonPressEvent) -> Result<(), X11Error> {
        let window = if self.client_state.contains(event.child) {
            event.child
        } else if self.client_state.contains(event.event) {
            event.event
        } else {
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
                    x11rb::NONE,
                    event_time,
                );
                self.context.flush()?;
            }
            Some(crate::mouse::MouseActionType::Resize) => {
                let _ = self.focus_client(window);
                let _ = self.raise_window(window);

                if !is_floating {
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
                    x11rb::NONE,
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
                if let Some(client) = self.client_state.get_client_mut(window) {
                    if let Some(ref mut fg) = client.floating_geometry {
                        fg.width = new_w;
                        fg.height = new_h;
                    } else {
                        client.floating_geometry = Some(crate::floating::FloatingGeometry::new(0, 0, new_w, new_h));
                    }
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
                if let Some(ws) = self.workspace_manager.find_workspace_for(window) {
                    if ws != self.current_workspace {
                        self.switch_workspace(ws)?;
                    }
                }
                self.focus_client(window)?;
            }
        } else if event.type_ == atoms.net_current_desktop {
            let data = event.data.as_data32();
            let target_ws = data[0] + 1; // 0-indexed in EWMH -> 1-based workspace
            if target_ws >= 1 && target_ws <= 9 {
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

/// Reloads the configuration from disk while the window manager is running.
///
/// # Arguments
/// * `state` - The mutable window manager state containing the current configuration.
///
/// # Returns
/// * `Result<(), ReloadError>` - Success or failure reason.
pub fn reload_config(state: &mut WindowManager) -> Result<(), ReloadError> {
    state.reload_config()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_stats_default() {
        let stats = EventStats::default();
        assert_eq!(stats.total_events, 0);
        assert_eq!(stats.map_requests, 0);
        assert_eq!(stats.configure_requests, 0);
        assert_eq!(stats.destroy_notifications, 0);
    }

    #[test]
    fn test_wm_lifecycle_and_stop() {
        if std::env::var("DISPLAY").is_ok() {
            let ctx = X11Context::new().expect("Failed to connect to DISPLAY");
            let mut wm = WindowManager::new(ctx).expect("Failed to create WM");
            assert!(wm.running);
            assert_eq!(wm.stats.total_events, 0);

            wm.stop();
            assert!(!wm.running);
        }
    }

    #[test]
    fn test_wm_process_pending_events_empty() {
        if std::env::var("DISPLAY").is_ok() {
            let ctx = X11Context::new().expect("Failed to connect to DISPLAY");
            let mut wm = WindowManager::new(ctx).expect("Failed to create WM");
            let result = wm.process_pending_events();
            assert!(result.is_ok());
        }
    }
}
