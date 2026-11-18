//! Window Manager core and event loop.
//!
//! This module provides the central [`WindowManager`] struct and event dispatcher
//! which intercepts and processes all X11 events (MapRequest, ConfigureRequest, etc.),
//! manages client tracking, focus, window movement, and graceful window termination.

pub mod client;
pub mod events;
pub mod ipc;
pub mod ui;
pub mod workspace;

use crate::atoms::Atoms;
use crate::client::ClientState;
use crate::config::{Config, ReloadError, load_config, load_config_or_default};
pub use crate::error::WmError;
use crate::error::is_connection_lost_error;
use crate::ewmh::{EwmhAtoms, init_ewmh};
use crate::focus::{
    BorderConfig, WorkspaceFocus, set_window_border_color, set_window_border_width,
};
use crate::input::InputManager;
use crate::keybind::{MOD_SUPER, parse_modifier};
use crate::layout::LayoutConfig;
use crate::monitor::{MonitorManager, query_monitors, setup_randr_events};
use crate::rules::WindowRule;
use crate::statusbar::Statusbar;
use crate::workspace::WorkspaceManager;
use crate::x11::{X11Context, X11Error};
use std::sync::atomic::{AtomicBool, Ordering};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ButtonIndex, ChangeWindowAttributesAux, ConnectionExt, EventMask, InputFocus, MapState,
    ModMask, Window,
};
use x11rb::wrapper::ConnectionExt as _;

/// Global flag for SIGUSR1 signal (toggles launcher).
pub static LAUNCHER_SIGNAL: AtomicBool = AtomicBool::new(false);
/// Global flag for SIGHUP signal (reloads config).
pub static RELOAD_SIGNAL: AtomicBool = AtomicBool::new(false);
/// Global flag for SIGINT/SIGTERM signal (cleanly stops event loop).
pub static TERMINATE_SIGNAL: AtomicBool = AtomicBool::new(false);

extern "C" fn sigusr1_handler(_: libc::c_int) {
    LAUNCHER_SIGNAL.store(true, Ordering::Relaxed);
}

extern "C" fn sighup_handler(_: libc::c_int) {
    RELOAD_SIGNAL.store(true, Ordering::Relaxed);
}

extern "C" fn sigterm_handler(_: libc::c_int) {
    TERMINATE_SIGNAL.store(true, Ordering::Relaxed);
}

/// Registers POSIX signal handlers for runtime control.
pub fn setup_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGUSR1, sigusr1_handler as *const () as usize);
        libc::signal(libc::SIGHUP, sighup_handler as *const () as usize);
        libc::signal(libc::SIGINT, sigterm_handler as *const () as usize);
        libc::signal(libc::SIGTERM, sigterm_handler as *const () as usize);
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
    /// Built-in statusbar instances on secondary monitors.
    pub secondary_statusbars: Vec<Statusbar>,
    /// Built-in application launcher instance if initialized.
    pub launcher: Option<crate::launcher::Launcher>,
    /// Dedicated floating scratchpad state (Phase 23).
    pub scratchpad: crate::scratchpad::Scratchpad,
    /// Active mouse drag interaction state (Phase 24).
    pub mouse_action: crate::mouse::MouseAction,
    /// Registered mouse button bindings (Phase 24).
    pub mouse_bindings: Vec<crate::mouse::MouseBinding>,
    /// Cursors for pointer interactions (normal arrow, move fleur, resize sizing).
    pub cursor_manager: crate::cursor::CursorManager,
    /// Background IPC server instance for runtime control.
    pub ipc_server: Option<crate::ipc::IpcServer>,
    /// Receiver for requests forwarded from IPC clients.
    pub ipc_rx: Option<std::sync::mpsc::Receiver<crate::ipc::IpcWmRequest>>,
    /// Read file descriptor of wake pipe to interrupt event loop poll on IPC requests.
    pub wake_pipe_read: Option<i32>,
    /// Write file descriptor of wake pipe.
    pub wake_pipe_write: Option<i32>,
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

        let border_config = BorderConfig {
            width: config.border_width,
            focused_color: config.focused_color,
            normal_color: config.normal_color,
            urgent_color: config.urgent_color,
        };

        let layout_config = LayoutConfig {
            border_width: config.border_width,
            inner_gap: config.inner_gap,
            outer_gap: config.outer_gap,
            master_ratio: config.master_ratio,
        };

        let mut input_manager = InputManager::new(config.bindings.clone());
        let _ = input_manager.setup(&context.connection, context.root_window);
        let mut workspace_manager = WorkspaceManager::new();
        for ws in workspace_manager.workspaces.values_mut() {
            ws.layout = config.layout;
        }

        let mut monitor_manager =
            MonitorManager::new(context.screen_width as u32, context.screen_height as u32);
        let _ = setup_randr_events(&context.connection, context.root_window);
        if let Ok(monitors) = query_monitors(
            &context.connection,
            context.root_window,
            context.screen_width as u32,
            context.screen_height as u32,
        ) {
            println!(
                "[INFO  tilo::wm] Detected {} monitor(s): {:?}",
                monitors.len(),
                monitors
                    .iter()
                    .map(|m| format!(
                        "id={} {}x{}+{}+{} ws={}",
                        m.id, m.width, m.height, m.x, m.y, m.workspace
                    ))
                    .collect::<Vec<_>>()
            );
            let _ = monitor_manager.reconcile_monitors(monitors);
        }

        let mod_mask = parse_modifier(&config.mod_key).unwrap_or(MOD_SUPER);
        let mouse_bindings = crate::mouse::default_mouse_bindings(mod_mask);
        let _ = crate::mouse::update_client_button_grabs(
            &context.connection,
            context.root_window,
            &mouse_bindings,
            input_manager.num_lock_mask,
            true,
        );

        let cursor_manager =
            crate::cursor::CursorManager::new(&context.connection, context.screen_num);
        let _ = cursor_manager.apply_root(&context.connection, context.root_window);

        let mut pipe_fds = [0i32; 2];
        let (wake_read, wake_write) = if unsafe {
            libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK)
        } == 0 {
            (Some(pipe_fds[0]), Some(pipe_fds[1]))
        } else {
            (None, None)
        };

        let (ipc_server, ipc_rx) = if let Some(w_fd) = wake_write {
            let (tx, rx) = std::sync::mpsc::channel();
            match crate::ipc::IpcServer::start(tx, w_fd) {
                Ok(srv) => (Some(srv), Some(rx)),
                Err(err) => {
                    eprintln!("[WARN  tilo::wm] Failed to start IPC server: {}", err);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

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
            rules: config.window_rules.clone(),
            ewmh_atoms,
            supporting_window,
            monitor_manager,
            statusbar: None,
            secondary_statusbars: Vec::new(),
            launcher: {
                let mut l = crate::launcher::Launcher::new();
                l.color_scheme = config.launcher_color_scheme();
                l.border_color = Some(config.launcher_border_color());
                Some(l)
            },
            scratchpad: crate::scratchpad::Scratchpad::default(),
            mouse_action: crate::mouse::MouseAction::None,
            mouse_bindings,
            cursor_manager,
            ipc_server,
            ipc_rx,
            wake_pipe_read: wake_read,
            wake_pipe_write: wake_write,
            config,
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
        self.workspace_manager.current_workspace_mut().layout = config.layout;
        self.rules = config.window_rules.clone();

        self.input_manager.re_register(
            &self.context.connection,
            self.context.root_window,
            config.bindings.clone(),
        )?;

        let mod_mask = parse_modifier(&config.mod_key).unwrap_or(MOD_SUPER);
        self.mouse_bindings = crate::mouse::default_mouse_bindings(mod_mask);
        let _ = crate::mouse::update_client_button_grabs(
            &self.context.connection,
            self.context.root_window,
            &self.mouse_bindings,
            self.input_manager.num_lock_mask,
            true,
        );
        let focused = self.focused_window();
        for &win in self.client_state.all_clients() {
            let _ = crate::mouse::update_client_button_grabs(
                &self.context.connection,
                win,
                &self.mouse_bindings,
                self.input_manager.num_lock_mask,
                focused == Some(win),
            );
        }

        let _old_bar_font = self.config.bar_font().to_string();
        let old_launcher_font = self.config.launcher_font().to_string();
        self.config = config;

        // Apply updated border width and colors to all existing client windows
        for client in self.client_state.clients.values_mut() {
            if !client.fullscreen {
                let _ = set_window_border_width(
                    &self.context.connection,
                    client.window,
                    self.border_config.width,
                );
                let border_color = if focused == Some(client.window) {
                    self.border_config.focused_color
                } else {
                    self.border_config.normal_color
                };
                let _ =
                    set_window_border_color(&self.context.connection, client.window, border_color);
                client.border_color = Some(border_color);
            }
        }

        // Update workspace names in WorkspaceManager
        for ws in 1..=9 {
            if let Some(w) = self.workspace_manager.get_mut(ws) {
                w.name = self.config.workspace_tag(ws).to_string();
            }
        }

        // Update _NET_DESKTOP_NAMES on root
        let mut ewmh_names = Vec::new();
        for ws in 1..=9 {
            ewmh_names.extend_from_slice(self.config.workspace_tag(ws).as_bytes());
            ewmh_names.push(0);
        }
        let _ = self.context.connection.change_property8(
            x11rb::protocol::xproto::PropMode::REPLACE,
            self.context.root_window,
            self.ewmh_atoms.net_desktop_names,
            self.ewmh_atoms.utf8_string,
            &ewmh_names,
        );

        // Apply updated theme and modules to statusbar if active or toggle
        if self.config.statusbar {
            let bar_height = self.statusbar.as_ref().map(|b| b.rect.height).unwrap_or(24);
            if let Err(err) = self.enable_statusbar(bar_height) {
                eprintln!(
                    "[WARN  tilo::wm] Failed to enable statusbar on config reload: {}",
                    err
                );
            }
        } else if self.statusbar.is_some() || !self.secondary_statusbars.is_empty() {
            let _ = self.disable_statusbar();
        }

        // Apply updated theme and position to launcher if initialized
        if let Some(ref mut launcher) = self.launcher {
            launcher.position = self.config.launcher_position;
            launcher.color_scheme = self.config.launcher_color_scheme();
            launcher.border_color = Some(self.config.launcher_border_color());
            launcher.show_lock = self.config.lock_command.is_some();
            launcher.invalidate_cache();
            if launcher.visible && launcher.window_id != 0 {
                let active_mon = self.monitor_manager.active_monitor().clone();
                let (width, height, x, y) = match self.config.launcher_position {
                    crate::launcher::LauncherPosition::Spotlight => {
                        let width = crate::launcher::LAUNCHER_WIDTH;
                        let height = crate::launcher::LAUNCHER_HEIGHT;
                        let x = (active_mon.x + (active_mon.width as i32 - width as i32) / 2)
                            .max(active_mon.x) as i16;
                        let y = (active_mon.y + (active_mon.height as i32 - height as i32) / 3)
                            .max(active_mon.y) as i16;
                        (width, height, x, y)
                    }
                    crate::launcher::LauncherPosition::Dmenu => {
                        let height = self
                            .statusbar
                            .as_ref()
                            .map(|b| b.rect.height as u16)
                            .unwrap_or(24);
                        let width = active_mon.width as u16;
                        let x = active_mon.x as i16;
                        let y = active_mon.y as i16;
                        (width, height, x, y)
                    }
                };
                launcher.rect =
                    crate::layout::Rect::new(x as i32, y as i32, width as u32, height as u32);
                let _ = self.context.connection.configure_window(
                    launcher.window_id,
                    &x11rb::protocol::xproto::ConfigureWindowAux::new()
                        .x(x as i32)
                        .y(y as i32)
                        .width(width as u32)
                        .height(height as u32),
                );
                let font_name = self.config.launcher_font();
                let screen = &self.context.connection.setup().roots[self.context.screen_num];
                if launcher.renderer.is_none() || font_name != old_launcher_font {
                    if let Ok(new_renderer) = crate::render::Renderer::new(
                        &self.context.connection,
                        launcher.window_id,
                        width,
                        height,
                        screen.root_depth,
                        font_name,
                    ) {
                        if let Some(old_renderer) = launcher.renderer.take() {
                            let _ = old_renderer.destroy(&self.context.connection);
                        }
                        launcher.attach_renderer(new_renderer);
                    }
                } else if let Some(ref mut renderer) = launcher.renderer {
                    let _ = renderer.resize(
                        &self.context.connection,
                        launcher.window_id,
                        width,
                        height,
                        screen.root_depth,
                    );
                }
                let _ = launcher.render(&self.context.connection);
            } else if !launcher.visible && launcher.window_id != 0 {
                let _ = launcher.destroy(&self.context.connection);
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
    pub fn run_autostart(&self) -> Result<(), crate::autostart::AutostartError> {
        crate::autostart::run_autostart_commands(&self.config.autostart)
    }

    /// Starts the main event loop, blocking the thread and waiting for X11 events.
    ///
    /// Implements Phase 29 Application State Recovery:
    /// - Recovers from client errors without panicking.
    /// - Recovers gracefully from non-fatal protocol errors.
    /// - Only terminates if the connection to the X11 server is lost or a shutdown is requested.
    pub fn run(&mut self) -> Result<(), WmError> {
        setup_signal_handlers();
        let _ = self.run_autostart();
        if let Err(err) = self.context.flush() {
            if is_connection_lost_error(&err) {
                return Err(WmError::ConnectionLost(err.to_string()));
            }
            eprintln!("[WARN  tilo::wm] Initial flush error: {}", err);
        }
        let mut last_status_update = std::time::Instant::now();
        use std::os::fd::AsRawFd;
        let conn_fd = self.context.connection.stream().as_raw_fd();

        while self.running {
            if TERMINATE_SIGNAL.swap(false, Ordering::Relaxed) {
                let _ = self.perform_desktop_shutdown(None);
                break;
            }
            if LAUNCHER_SIGNAL.swap(false, Ordering::Relaxed) {
                let _ = self.toggle_launcher();
            }
            if RELOAD_SIGNAL.swap(false, Ordering::Relaxed) {
                let _ = self.reload_config();
            }

            let mut had_activity = false;
            while let Some(event) = match self.context.connection.poll_for_event() {
                Ok(ev) => ev,
                Err(err) => return Err(WmError::ConnectionLost(err.to_string())),
            } {
                had_activity = true;
                if let Err(err) = self.handle_event(event) {
                    if is_connection_lost_error(&err) {
                        return Err(WmError::ConnectionLost(err.to_string()));
                    }
                    eprintln!(
                        "[WARN  tilo::wm] Recovered from event dispatch error: {}",
                        err
                    );
                }
            }

            // Periodic statusbar update if statusbar is active
            let timeout_ms = if self.statusbar.is_some() || !self.secondary_statusbars.is_empty() {
                let mut time_until_next = self
                    .statusbar
                    .as_ref()
                    .map(|b| b.time_until_next_update())
                    .unwrap_or(std::time::Duration::from_secs(1));
                for b in &self.secondary_statusbars {
                    time_until_next = time_until_next.min(b.time_until_next_update());
                }
                if time_until_next.is_zero()
                    || last_status_update.elapsed() >= std::time::Duration::from_millis(1000)
                {
                    if self.update_statusbar() {
                        had_activity = true;
                    }
                    last_status_update = std::time::Instant::now();
                }
                (time_until_next.as_millis() as i32).clamp(10, 1000)
            } else {
                250
            };

            // Only flush pending X11 requests when activity occurred
            if had_activity && let Err(err) = self.context.flush() {
                if is_connection_lost_error(&err) {
                    return Err(WmError::ConnectionLost(err.to_string()));
                }
                eprintln!("[WARN  tilo::wm] Flush error before poll: {}", err);
            }

            // Wait for incoming X11 events on the connection socket or IPC wake pipe,
            // waking immediately upon input or timing out when the next periodic statusbar update is due.
            let mut poll_fds = [
                libc::pollfd {
                    fd: conn_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake_pipe_read.unwrap_or(-1),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let nfds = if self.wake_pipe_read.is_some() { 2 } else { 1 };
            unsafe {
                libc::poll(poll_fds.as_mut_ptr(), nfds, timeout_ms);
            }

            if nfds == 2 && (poll_fds[1].revents & libc::POLLIN != 0) {
                let mut drain = [0u8; 64];
                while unsafe {
                    libc::read(
                        poll_fds[1].fd,
                        drain.as_mut_ptr() as *mut libc::c_void,
                        drain.len(),
                    )
                } > 0 {}
                let _ = self.process_pending_ipc_requests()?;
            }
        }
        if self.running {
            let _ = self.perform_desktop_shutdown(None);
        } else {
            let _ = self.perform_graceful_shutdown();
        }
        Ok(())
    }

    /// Processes all pending requests received over the IPC channel.
    pub fn process_pending_ipc_requests(&mut self) -> Result<usize, WmError> {
        let mut count = 0;
        let mut state_changed = false;

        while let Some(req) = self.ipc_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            count += 1;
            match req.request {
                crate::ipc::IpcRequest::Command(cmd_str) => {
                    let chains = crate::ipc::split_command_chain(&cmd_str);
                    let mut results = Vec::new();

                    for chain in chains {
                        let res = self.execute_ipc_command_line(&chain);
                        if res.success {
                            state_changed = true;
                        }
                        results.push(res);
                    }

                    let resp_json = if results.len() == 1 {
                        let r = &results[0];
                        if r.success {
                            "{\"success\":true}".to_string()
                        } else {
                            format!(
                                "{{\"success\":false,\"error\":{}}}",
                                crate::ipc::JsonValue::Str(r.error.clone().unwrap_or_default())
                                    .to_json_string()
                            )
                        }
                    } else {
                        let mut arr = crate::ipc::JsonValue::array();
                        for r in results {
                            let mut item = crate::ipc::JsonValue::object();
                            item.insert("success", crate::ipc::JsonValue::Bool(r.success));
                            if let Some(err) = r.error {
                                item.insert("error", crate::ipc::JsonValue::Str(err));
                            }
                            arr.push(item);
                        }
                        arr.to_json_string()
                    };
                    let _ = req.responder.send(resp_json);
                }
                crate::ipc::IpcRequest::GetTree => {
                    let tree_json = self.dump_tree_json();
                    let _ = req.responder.send(tree_json);
                }
                crate::ipc::IpcRequest::GetWorkspaces => {
                    let ws_json = self.dump_workspaces_json();
                    let _ = req.responder.send(ws_json);
                }
                crate::ipc::IpcRequest::GetOutputs => {
                    let out_json = self.dump_outputs_json();
                    let _ = req.responder.send(out_json);
                }
                crate::ipc::IpcRequest::GetMarks => {
                    let marks_json = self.dump_marks_json();
                    let _ = req.responder.send(marks_json);
                }
                crate::ipc::IpcRequest::GetVersion => {
                    let mut v = crate::ipc::JsonValue::object();
                    v.insert("name", crate::ipc::JsonValue::Str("tilo".to_string()));
                    v.insert(
                        "version",
                        crate::ipc::JsonValue::Str(env!("CARGO_PKG_VERSION").to_string()),
                    );
                    let _ = req.responder.send(v.to_json_string());
                }
                crate::ipc::IpcRequest::Subscribe(_) => {
                    let _ = req.responder.send("{\"success\":true}".to_string());
                }
            }
        }

        if state_changed {
            if let Err(e) = self.retile() {
                if is_connection_lost_error(&e) {
                    return Err(WmError::ConnectionLost(e.to_string()));
                }
            }
            let _ = self.update_ewmh_current_desktop();
            let _ = self.render_statusbar();
            let _ = self.context.flush();
        }

        Ok(count)
    }

    /// Polls and processes all currently buffered events from the X server without blocking.
    ///
    /// # Returns
    /// The number of events processed.
    pub fn process_pending_events(&mut self) -> Result<usize, WmError> {
        let mut count = 0;
        while let Some(event) = match self.context.connection.poll_for_event() {
            Ok(ev) => ev,
            Err(err) => return Err(WmError::ConnectionLost(err.to_string())),
        } {
            if let Err(err) = self.handle_event(event) {
                if is_connection_lost_error(&err) {
                    return Err(WmError::ConnectionLost(err.to_string()));
                }
                eprintln!(
                    "[WARN  tilo::wm] Recovered from event dispatch error: {}",
                    err
                );
            }
            count += 1;
        }
        if count > 0
            && let Err(err) = self.context.flush()
            && is_connection_lost_error(&err)
        {
            return Err(WmError::ConnectionLost(err.to_string()));
        }
        Ok(count)
    }

    /// Signals the event loop to stop running.
    pub fn stop(&mut self) {
        self.running = false;
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

/// Scans the X11 root window tree and manages pre-existing windows.
///
/// Follows Phase 31 of Plan.md:
/// 1. Queries root tree.
/// 2. Identifies existing top-level windows.
/// 3. Ignores override-redirect and unmapped windows.
/// 4. Registers valid clients via `manage_window`.
/// 5. Applies window rules (workspace assignment, floating, fullscreen).
/// 6. Assigns workspaces & configure geometries (unmap background workspace clients).
/// 7. Maps/configures clients on active workspace, set `WM_STATE`.
/// 8. Focuses the topmost active window.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `root` - The root window ID.
/// * `state` - The mutable WM state to populate with existing clients.
///
/// # Returns
/// Result indicating success.
pub fn manage_existing_windows(
    conn: &impl Connection,
    root: u32,
    state: &mut WindowManager,
) -> Result<(), WmError> {
    let tree = conn
        .query_tree(root)
        .map_err(|e| WmError::ConnectionLost(e.to_string()))?
        .reply()
        .map_err(|e| WmError::X11ProtocolError(crate::x11::X11Error::Reply(e)))?;

    for child in tree.children {
        if child == root || child == state.supporting_window {
            continue;
        }
        if state
            .statusbar
            .as_ref()
            .map(|b| b.window_id == child)
            .unwrap_or(false)
        {
            continue;
        }
        if state
            .launcher
            .as_ref()
            .map(|l| l.window_id == child)
            .unwrap_or(false)
        {
            continue;
        }

        let attrs = match conn.get_window_attributes(child) {
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

        if let Err(err) = state.manage_window(child) {
            if is_connection_lost_error(&err) {
                return Err(WmError::ConnectionLost(err.to_string()));
            }
            eprintln!(
                "[WARN  tilo::wm] Failed to manage existing window 0x{:x}: {:?}",
                child, err
            );
        }
    }

    let _ = state.retile();
    let _ = state.render_statusbar();
    let _ = state.context.flush();
    Ok(())
}

/// Executes the full desktop environment shutdown sequence.
///
/// Closes all running applications gracefully via ICCCM `WM_DELETE_WINDOW`
/// (or forcefully kills unresponsive clients), cleans up state, releases X11 resources,
/// and terminates the window manager.
///
/// # Arguments
/// * `_conn` - The X11 connection.
/// * `state` - The current WM state.
/// * `timeout` - Optional timeout duration to wait for applications to close.
pub fn perform_desktop_shutdown(
    _conn: &impl Connection,
    state: &mut WindowManager,
    timeout: Option<std::time::Duration>,
) -> Result<(), WmError> {
    state.perform_desktop_shutdown(timeout)
}

/// Executes the graceful shutdown sequence.
///
/// Follows Phase 32 of Plan.md:
/// 1. Stops launcher.
/// 2. Removes statusbar.
/// 3. Restores client borders (`border_width = 0`).
/// 4. Restores client geometry and ensures background workspace windows are mapped.
/// 5. Removes WM properties (`WM_STATE`, `_NET_WM_STATE`).
/// 6. Releases `WM_Sn` selection ownership and destroys supporting window.
/// 7. Releases root window event masks and grabs.
/// 8. Sets input focus to `PointerRoot`.
/// 9. Flushes X11 connection.
/// 10. Terminates main event loop.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `state` - The current WM state.
pub fn perform_graceful_shutdown(
    conn: &impl Connection,
    state: &mut WindowManager,
) -> Result<(), WmError> {
    // 1. Stop launcher
    if let Some(ref mut launcher) = state.launcher
        && launcher.window_id != 0
    {
        let _ = conn.unmap_window(launcher.window_id);
        let _ = conn.destroy_window(launcher.window_id);
        launcher.window_id = 0;
        launcher.visible = false;
    }

    // 2. Remove statusbar
    if let Some(bar) = state.statusbar.take() {
        let _ = bar.destroy(conn);
    }
    for bar in state.secondary_statusbars.drain(..) {
        let _ = bar.destroy(conn);
    }

    // 3 & 4. Iterate over all managed clients: restore borders, map background windows, remove properties
    let all_clients: Vec<Window> = state.client_state.all_clients().to_vec();
    for win in all_clients {
        let _ = set_window_border_width(conn, win, 0);
        let _ = conn.delete_property(win, state.atoms.wm_state);
        let _ = conn.delete_property(win, state.ewmh_atoms.net_wm_state);
        let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::NO_EVENT);
        let _ = conn.change_window_attributes(win, &aux);

        if state.scratchpad.client == Some(win) && !state.scratchpad.is_visible {
            let _ = conn.unmap_window(win);
        } else {
            let _ = conn.map_window(win);
        }
    }

    if state.scratchpad.client.is_some() {
        state.scratchpad.clear_client();
    }

    state.client_state = ClientState::new();
    for ws in state.workspace_manager.workspaces.values_mut() {
        ws.clients.clear();
        ws.focused_client = None;
    }

    // 5. Release WM_Sn selection ownership
    let sel_name = format!("WM_S{}", state.context.screen_num);
    if let Ok(cookie) = conn.intern_atom(false, sel_name.as_bytes())
        && let Ok(reply) = cookie.reply()
    {
        let _ = conn.set_selection_owner(x11rb::NONE, reply.atom, x11rb::CURRENT_TIME);
    }

    // Release EWMH properties and supporting window
    let root = state.context.root_window;
    let _ = conn.delete_property(root, state.ewmh_atoms.net_supporting_wm_check);
    let _ = conn.delete_property(root, state.ewmh_atoms.net_supported);
    let _ = conn.delete_property(root, state.ewmh_atoms.net_client_list);
    let _ = conn.delete_property(root, state.ewmh_atoms.net_active_window);
    let _ = conn.delete_property(root, state.ewmh_atoms.net_workarea);

    if state.supporting_window != 0 {
        let _ = conn.destroy_window(state.supporting_window);
        state.supporting_window = 0;
    }

    let _ = conn.ungrab_key(0, root, ModMask::ANY);
    let _ = conn.ungrab_button(ButtonIndex::ANY, root, ModMask::ANY);
    let aux = ChangeWindowAttributesAux::default().event_mask(EventMask::NO_EVENT);
    let _ = conn.change_window_attributes(root, &aux);
    let _ = state
        .context
        .connection
        .change_window_attributes(root, &aux);
    let _ = state.context.flush();

    let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, root, x11rb::CURRENT_TIME);
    let _ = conn.flush();

    if let Some(ref srv) = state.ipc_server {
        srv.broadcast_event("shutdown", "{\"event\":\"shutdown\",\"change\":\"exit\"}");
        srv.shutdown();
    }
    if let Some(r_fd) = state.wake_pipe_read.take() {
        unsafe {
            libc::close(r_fd);
        }
    }
    if let Some(w_fd) = state.wake_pipe_write.take() {
        unsafe {
            libc::close(w_fd);
        }
    }

    state.running = false;
    Ok(())
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
