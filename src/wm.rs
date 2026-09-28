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
use crate::keybind::Action;
use crate::layout::{apply_layout_with_border, calculate_master_stack, LayoutConfig, Rect};
use crate::rules::{apply_rules as apply_window_rules, RuleAction, WindowRule};
use crate::statusbar::{create_statusbar_window, Statusbar, StatusbarError};
use crate::workspace::{WorkspaceId, WorkspaceManager};
use crate::x11::{X11Context, X11Error};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ChangeWindowAttributesAux, ClientMessageEvent,
    ConfigWindow, ConfigureRequestEvent, ConfigureWindowAux, ConnectionExt, EventMask, InputFocus,
    KeyPressEvent, PropertyNotifyEvent, StackMode, Window,
};
use x11rb::protocol::Event;

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
        })
    }

    /// Applies a new [`Config`] to the running window manager, updating borders,
    /// layouts, gaps, and registering the new keybindings.
    pub fn apply_config(&mut self, config: Config) -> Result<(), X11Error> {
        self.border_config.width = config.border_width;
        self.layout_config.border_width = config.border_width;
        self.layout_config.inner_gap = config.inner_gap;
        self.layout_config.outer_gap = config.outer_gap;
        self.layout_config.master_ratio = config.master_ratio;

        self.input_manager.re_register(
            &self.context.connection,
            self.context.root_window,
            config.bindings.clone(),
        )?;

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
            "fixed"
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

    /// Starts the main event loop, blocking the thread and waiting for X11 events.
    pub fn run(&mut self) -> Result<(), X11Error> {
        self.context.flush()?;
        while self.running {
            let event = self.context.connection.wait_for_event()?;
            self.handle_event(event)?;

            // Drain any pending events in the queue to minimize latency and batch flushes
            while let Some(event) = self.context.connection.poll_for_event()? {
                self.handle_event(event)?;
            }

            self.context.flush()?;
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
                self.handle_key_press(&ev)?;
            }
            Event::ButtonPress(ev) => {
                self.stats.button_presses += 1;
                self.handle_button_press(&ev)?;
            }
            Event::ClientMessage(ev) => {
                self.stats.client_messages += 1;
                self.handle_client_message(&ev)?;
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
        let current_id = self.workspace_manager.current_id();
        if current_id == target_id {
            return Ok(());
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
            let _ = self.context.connection.set_input_focus(
                InputFocus::POINTER_ROOT,
                self.context.root_window,
                x11rb::CURRENT_TIME,
            );
        }

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
                if let Some(win) = self.client_state.focused_window() {
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

    /// Handles mouse button press.
    pub fn handle_button_press(&mut self, _event: &ButtonPressEvent) -> Result<(), X11Error> {
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
