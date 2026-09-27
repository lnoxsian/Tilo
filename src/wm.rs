//! Window Manager core and event loop.
//!
//! This module provides the central [`WindowManager`] struct and event dispatcher
//! which intercepts and processes all X11 events (MapRequest, ConfigureRequest, etc.),
//! manages client tracking, focus, window movement, and graceful window termination.

use crate::atoms::Atoms;
use crate::client::{Client, ClientState};
use crate::layout::{apply_layout_with_border, calculate_master_stack, LayoutConfig, Rect};
use crate::x11::{X11Context, X11Error};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ChangeWindowAttributesAux, ClientMessageData, ClientMessageEvent,
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
}

impl WindowManager {
    /// Creates a new `WindowManager` wrapping the provided `X11Context`.
    pub fn new(context: X11Context) -> Result<Self, X11Error> {
        let atoms = Atoms::intern(&context.connection)?;
        Ok(Self {
            context,
            client_state: ClientState::new(),
            running: true,
            stats: EventStats::default(),
            atoms,
            layout_config: LayoutConfig::default(),
        })
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
                self.stats.key_presses += 1;
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

        // Construct client state
        let mut client = Client::new(window);
        client.mapped = true;
        self.client_state.add_client(window, client);

        // Map, retile, and focus client window
        self.map_client(window)?;
        self.retile()?;
        self.focus_client(window)?;

        Ok(true)
    }

    /// Deregisters and unmanages a window, removing it from `client_state`.
    ///
    /// If the window was previously focused, input focus is transferred to the
    /// previous window on the focus stack or reverted to the root window.
    pub fn unmanage_window(&mut self, window: Window) -> Option<Client> {
        let removed = self.client_state.remove_client(window);
        if removed.is_some() {
            let _ = self.retile();
            // Restore focus to previous client or root
            if let Some(next_window) = self.client_state.focused_window() {
                let _ = self.focus_client(next_window);
            } else {
                let _ = self.context.connection.set_input_focus(
                    InputFocus::POINTER_ROOT,
                    self.context.root_window,
                    x11rb::CURRENT_TIME,
                );
            }
        }
        removed
    }

    /// Recalculates and applies the tiling layout to all visible managed clients.
    pub fn retile(&mut self) -> Result<(), X11Error> {
        let visible_tiled = self.client_state.visible_tiled_clients();
        if visible_tiled.is_empty() {
            return Ok(());
        }

        let screen_rect = Rect::new(
            0,
            0,
            self.context.screen_width as u32,
            self.context.screen_height as u32,
        );

        let rects = calculate_master_stack(&visible_tiled, &screen_rect, &self.layout_config);
        apply_layout_with_border(
            &self.context.connection,
            &visible_tiled,
            &rects,
            self.layout_config.border_width,
        )?;

        self.context.flush()?;
        Ok(())
    }

    /// Sets input focus to a specific client window and raises it.
    pub fn focus_client(&mut self, window: Window) -> Result<(), X11Error> {
        self.context.connection.set_input_focus(
            InputFocus::POINTER_ROOT,
            window,
            x11rb::CURRENT_TIME,
        )?;

        // Raise window in stacking order
        let aux = ConfigureWindowAux::default().stack_mode(StackMode::ABOVE);
        let _ = self.context.connection.configure_window(window, &aux);

        self.client_state.focus(window);
        self.context.flush()?;
        Ok(())
    }

    /// Maps a client window, making it visible on the screen.
    pub fn map_client(&mut self, window: Window) -> Result<(), X11Error> {
        self.context.connection.map_window(window)?;
        if let Some(client) = self.client_state.get_client_mut(window) {
            client.mapped = true;
        }
        self.context.flush()?;
        Ok(())
    }

    /// Unmaps a client window, hiding it from the screen.
    pub fn unmap_client(&mut self, window: Window) -> Result<(), X11Error> {
        self.context.connection.unmap_window(window)?;
        if let Some(client) = self.client_state.get_client_mut(window) {
            client.mapped = false;
        }
        self.context.flush()?;
        Ok(())
    }

    /// Safely requests a client to close via ICCCM `WM_DELETE_WINDOW`, or forcefully kills it.
    pub fn kill_client(&self, window: Window) -> Result<(), X11Error> {
        // Query WM_PROTOCOLS property to check for WM_DELETE_WINDOW support
        let reply = self
            .context
            .connection
            .get_property(
                false,
                window,
                self.atoms.wm_protocols,
                AtomEnum::ATOM,
                0,
                32,
            )?
            .reply();

        let mut supports_delete = false;
        if let Ok(prop) = reply {
            if let Some(atoms) = prop.value32() {
                for atom in atoms {
                    if atom == self.atoms.wm_delete_window {
                        supports_delete = true;
                        break;
                    }
                }
            }
        }

        if supports_delete {
            let data = [
                self.atoms.wm_delete_window,
                x11rb::CURRENT_TIME,
                0,
                0,
                0,
            ];
            let event = ClientMessageEvent {
                response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window,
                type_: self.atoms.wm_protocols,
                data: ClientMessageData::from(data),
            };
            self.context.connection.send_event(
                false,
                window,
                EventMask::NO_EVENT,
                event,
            )?;
        } else {
            self.context.connection.kill_client(window)?;
        }

        self.context.flush()?;
        Ok(())
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
        let _ = self.unmap_client(window);
        Ok(())
    }

    /// Handles window property changes (e.g. WM_NAME, WM_HINTS).
    pub fn handle_property_notify(&mut self, _event: &PropertyNotifyEvent) -> Result<(), X11Error> {
        Ok(())
    }

    /// Handles keyboard input.
    pub fn handle_key_press(&mut self, _event: &KeyPressEvent) -> Result<(), X11Error> {
        Ok(())
    }

    /// Handles mouse button press.
    pub fn handle_button_press(&mut self, _event: &ButtonPressEvent) -> Result<(), X11Error> {
        Ok(())
    }

    /// Handles client messages (e.g. _NET_WM_STATE, WM_PROTOCOLS).
    pub fn handle_client_message(&mut self, _event: &ClientMessageEvent) -> Result<(), X11Error> {
        Ok(())
    }
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
