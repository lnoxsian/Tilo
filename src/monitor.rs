//! Multi-monitor management using X11 XRandR.
//!
//! Implements multi-monitor support according to Phase 14 of Plan.md:
//! - Monitor detection and dynamic geometry tracking via XRandR.
//! - Independent active workspace per monitor.
//! - Directional monitor focus switching (`focus_monitor_left`, `focus_monitor_right`).
//! - Moving client windows to another monitor (`move_to_monitor`).
//! - Hotplug detection and geometry reconciliation (`refresh_monitors`).

use crate::layout::Rect;
use crate::workspace::WorkspaceId;
use crate::x11::X11Error;
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::Window;

/// A unique identifier for a monitor.
pub type MonitorId = u32;

/// Represents a physical or virtual screen output (Monitor) managed by XRandR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// Unique internal ID for the monitor.
    pub id: MonitorId,
    /// X coordinate of the monitor within the X screen.
    pub x: i32,
    /// Y coordinate of the monitor within the X screen.
    pub y: i32,
    /// Width of the monitor in pixels.
    pub width: u32,
    /// Height of the monitor in pixels.
    pub height: u32,
    /// The ID of the currently active workspace on this monitor.
    pub workspace: WorkspaceId,
    /// Indicator if this is the primary monitor.
    pub is_primary: bool,
}

impl Monitor {
    /// Creates a new `Monitor` with the given parameters.
    pub const fn new(
        id: MonitorId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        workspace: WorkspaceId,
        is_primary: bool,
    ) -> Self {
        Self {
            id,
            x,
            y,
            width,
            height,
            workspace,
            is_primary,
        }
    }

    /// Returns the bounding [`Rect`] geometry of the monitor.
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    /// Returns `true` if this monitor contains the given point `(px, py)`.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + self.width as i32
            && py >= self.y
            && py < self.y + self.height as i32
    }
}

/// Central manager for all detected physical and virtual monitors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorManager {
    /// List of tracked monitors, sorted by X coordinate.
    pub monitors: Vec<Monitor>,
    /// Currently focused monitor ID.
    pub active_monitor_id: MonitorId,
}

impl MonitorManager {
    /// Creates a `MonitorManager` initialized with a single primary monitor.
    pub fn new(fallback_width: u32, fallback_height: u32) -> Self {
        let primary = Monitor::new(0, 0, 0, fallback_width, fallback_height, 1, true);
        Self {
            monitors: vec![primary],
            active_monitor_id: 0,
        }
    }

    /// Returns a reference to the currently active monitor.
    pub fn active_monitor(&self) -> &Monitor {
        self.get_monitor(self.active_monitor_id)
            .or_else(|| self.monitors.first())
            .expect("At least one monitor must exist")
    }

    /// Returns a mutable reference to the currently active monitor.
    pub fn active_monitor_mut(&mut self) -> &mut Monitor {
        let id = self.active_monitor_id;
        if let Some(pos) = self.monitors.iter().position(|m| m.id == id) {
            &mut self.monitors[pos]
        } else {
            self.monitors.first_mut().expect("At least one monitor must exist")
        }
    }

    /// Retrieves a monitor by its unique [`MonitorId`].
    pub fn get_monitor(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.id == id)
    }

    /// Retrieves a mutable reference to a monitor by its unique [`MonitorId`].
    pub fn get_monitor_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.monitors.iter_mut().find(|m| m.id == id)
    }

    /// Finds which monitor hosts the given workspace ID.
    pub fn monitor_for_workspace(&self, ws: WorkspaceId) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.workspace == ws)
    }

    /// Finds which monitor contains the given screen coordinates.
    pub fn monitor_for_point(&self, px: i32, py: i32) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.contains_point(px, py))
    }

    /// Focuses the monitor visually to the left of the currently active monitor.
    ///
    /// If multiple monitors exist to the left, chooses the nearest one.
    /// Returns the newly focused [`MonitorId`], or `None` if no monitor is to the left.
    pub fn focus_monitor_left(&mut self) -> Option<MonitorId> {
        let cur = self.active_monitor();
        let cur_x = cur.x;
        let cur_id = cur.id;

        // Find candidate monitors strictly to the left (x + width <= cur_x or x < cur_x)
        let mut candidates: Vec<&Monitor> = self
            .monitors
            .iter()
            .filter(|m| m.id != cur_id && m.x < cur_x)
            .collect();

        // Sort descending by X (closest to cur_x first)
        candidates.sort_by_key(|m| std::cmp::Reverse(m.x));

        if let Some(target) = candidates.first() {
            let target_id = target.id;
            self.active_monitor_id = target_id;
            Some(target_id)
        } else {
            None
        }
    }

    /// Focuses the monitor visually to the right of the currently active monitor.
    ///
    /// If multiple monitors exist to the right, chooses the nearest one.
    /// Returns the newly focused [`MonitorId`], or `None` if no monitor is to the right.
    pub fn focus_monitor_right(&mut self) -> Option<MonitorId> {
        let cur = self.active_monitor();
        let cur_x = cur.x;
        let cur_id = cur.id;

        // Find candidate monitors strictly to the right (x > cur_x)
        let mut candidates: Vec<&Monitor> = self
            .monitors
            .iter()
            .filter(|m| m.id != cur_id && m.x > cur_x)
            .collect();

        // Sort ascending by X (closest to cur_x first)
        candidates.sort_by_key(|m| m.x);

        if let Some(target) = candidates.first() {
            let target_id = target.id;
            self.active_monitor_id = target_id;
            Some(target_id)
        } else {
            None
        }
    }

    /// Reconciles newly detected monitors with current monitor state.
    ///
    /// Preserves existing workspace assignments where possible and assigns
    /// new unused workspaces to newly discovered monitors.
    ///
    /// Returns a list of `(removed_monitor_id, old_workspace_id)` for monitors
    /// that were disconnected, allowing the window manager to migrate clients.
    pub fn reconcile_monitors(&mut self, mut new_monitors: Vec<Monitor>) -> Vec<(MonitorId, WorkspaceId)> {
        if new_monitors.is_empty() {
            return Vec::new();
        }

        // Sort new monitors by X coordinate
        new_monitors.sort_by_key(|m| (m.x, m.y));

        let mut disconnected = Vec::new();
        let old_monitors = std::mem::take(&mut self.monitors);

        // Track used workspaces
        let mut assigned_workspaces = Vec::new();

        // Reconcile: match by ID or position
        for new_m in &mut new_monitors {
            if let Some(old_m) = old_monitors.iter().find(|o| o.id == new_m.id || (o.x == new_m.x && o.y == new_m.y)) {
                new_m.workspace = old_m.workspace;
                assigned_workspaces.push(old_m.workspace);
            }
        }

        // Find surviving removed monitors
        for old_m in &old_monitors {
            if !new_monitors.iter().any(|n| n.id == old_m.id) {
                disconnected.push((old_m.id, old_m.workspace));
            }
        }

        // Assign unused workspaces to brand new monitors
        let mut next_ws = 1;
        for new_m in &mut new_monitors {
            if new_m.workspace == 0 || assigned_workspaces.iter().filter(|&&w| w == new_m.workspace).count() > 1 {
                while assigned_workspaces.contains(&next_ws) && next_ws <= 9 {
                    next_ws += 1;
                }
                new_m.workspace = next_ws;
                assigned_workspaces.push(next_ws);
                next_ws += 1;
            }
        }

        // Ensure active_monitor_id is valid
        if !new_monitors.iter().any(|m| m.id == self.active_monitor_id) {
            self.active_monitor_id = new_monitors.first().map(|m| m.id).unwrap_or(0);
        }

        self.monitors = new_monitors;
        disconnected
    }
}

/// Sets up XRandR event selection on the root window.
pub fn setup_randr_events(
    conn: &impl Connection,
    root: Window,
) -> Result<(), X11Error> {
    // Check if XRandR extension is present
    let version = conn.randr_query_version(1, 4)?.reply()?;
    if version.major_version < 1 {
        return Err(X11Error::RandrNotSupported);
    }

    let mask = randr::NotifyMask::SCREEN_CHANGE
        | randr::NotifyMask::OUTPUT_CHANGE
        | randr::NotifyMask::CRTC_CHANGE;
    conn.randr_select_input(root, mask)?;
    Ok(())
}

/// Queries XRandR for connected physical outputs and builds a list of [`Monitor`]s.
///
/// Falls back to querying root screen geometry as a single monitor if XRandR
/// is not supported or returns no active CRTCs.
pub fn query_monitors(
    conn: &impl Connection,
    root: Window,
    fallback_width: u32,
    fallback_height: u32,
) -> Result<Vec<Monitor>, X11Error> {
    let version_reply = conn.randr_query_version(1, 4);
    if version_reply.is_err() {
        return Ok(vec![Monitor::new(
            0,
            0,
            0,
            fallback_width,
            fallback_height,
            1,
            true,
        )]);
    }

    let primary_output = conn
        .randr_get_output_primary(root)?
        .reply()
        .map(|r| r.output)
        .unwrap_or(0);

    let (outputs, config_timestamp) = match conn.randr_get_screen_resources_current(root)?.reply() {
        Ok(res) => (res.outputs, res.config_timestamp),
        Err(_) => match conn.randr_get_screen_resources(root)?.reply() {
            Ok(res) => (res.outputs, res.config_timestamp),
            Err(_) => {
                return Ok(vec![Monitor::new(
                    0,
                    0,
                    0,
                    fallback_width,
                    fallback_height,
                    1,
                    true,
                )])
            }
        },
    };

    let mut monitors = Vec::new();
    let mut seen_crtcs = Vec::new();
    let mut next_ws = 1;

    for (idx, &output) in outputs.iter().enumerate() {
        let output_info = match conn.randr_get_output_info(output, config_timestamp)?.reply() {
            Ok(info) => info,
            Err(_) => continue,
        };

        if output_info.connection != randr::Connection::CONNECTED || output_info.crtc == 0 {
            continue;
        }

        // Avoid duplicate monitors for cloned displays on the same CRTC
        if seen_crtcs.contains(&output_info.crtc) {
            continue;
        }
        seen_crtcs.push(output_info.crtc);

        let crtc_info = match conn.randr_get_crtc_info(output_info.crtc, config_timestamp)?.reply() {
            Ok(info) => info,
            Err(_) => continue,
        };

        if crtc_info.width == 0 || crtc_info.height == 0 {
            continue;
        }

        let is_primary = output == primary_output || (primary_output == 0 && monitors.is_empty());
        let monitor = Monitor::new(
            idx as MonitorId,
            i32::from(crtc_info.x),
            i32::from(crtc_info.y),
            u32::from(crtc_info.width),
            u32::from(crtc_info.height),
            next_ws,
            is_primary,
        );
        monitors.push(monitor);
        next_ws += 1;
    }

    if monitors.is_empty() {
        monitors.push(Monitor::new(
            0,
            0,
            0,
            fallback_width,
            fallback_height,
            1,
            true,
        ));
    }

    // Sort by X coordinate
    monitors.sort_by_key(|m| (m.x, m.y));

    Ok(monitors)
}

/// Focuses the monitor visually to the left of the active monitor.
pub fn focus_monitor_left(wm: &mut crate::wm::WindowManager) -> Result<(), X11Error> {
    wm.focus_monitor_left()
}

/// Focuses the monitor visually to the right of the active monitor.
pub fn focus_monitor_right(wm: &mut crate::wm::WindowManager) -> Result<(), X11Error> {
    wm.focus_monitor_right()
}

/// Moves the currently focused client to the specified monitor.
pub fn move_to_monitor(
    wm: &mut crate::wm::WindowManager,
    target_monitor_id: MonitorId,
) -> Result<(), X11Error> {
    wm.move_to_monitor(target_monitor_id)
}

/// Refreshes monitor geometry by querying XRandR and updates the WM state.
pub fn refresh_monitors(
    _conn: &impl Connection,
    wm: &mut crate::wm::WindowManager,
) -> Result<(), X11Error> {
    wm.refresh_monitors()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_geometry_and_point_containment() {
        let mon = Monitor::new(0, 1000, 0, 1920, 1080, 2, false);
        assert_eq!(mon.rect(), Rect::new(1000, 0, 1920, 1080));
        assert!(mon.contains_point(1000, 0));
        assert!(mon.contains_point(2919, 1079));
        assert!(!mon.contains_point(999, 0));
        assert!(!mon.contains_point(2920, 1080));
    }

    #[test]
    fn test_monitor_manager_focus_left_right() {
        let mut mm = MonitorManager {
            monitors: vec![
                Monitor::new(0, 0, 0, 1920, 1080, 1, true),
                Monitor::new(1, 1920, 0, 1920, 1080, 2, false),
                Monitor::new(2, 3840, 0, 2560, 1440, 3, false),
            ],
            active_monitor_id: 0,
        };

        assert_eq!(mm.focus_monitor_left(), None);
        assert_eq!(mm.focus_monitor_right(), Some(1));
        assert_eq!(mm.active_monitor_id, 1);

        assert_eq!(mm.focus_monitor_right(), Some(2));
        assert_eq!(mm.active_monitor_id, 2);
        assert_eq!(mm.focus_monitor_right(), None);

        assert_eq!(mm.focus_monitor_left(), Some(1));
        assert_eq!(mm.active_monitor_id, 1);

        assert_eq!(mm.focus_monitor_left(), Some(0));
        assert_eq!(mm.active_monitor_id, 0);
    }

    #[test]
    fn test_monitor_manager_reconciliation() {
        let mut mm = MonitorManager::new(1920, 1080);
        assert_eq!(mm.monitors.len(), 1);

        // Simulate connecting a second monitor
        let new_monitors = vec![
            Monitor::new(0, 0, 0, 1920, 1080, 1, true),
            Monitor::new(1, 1920, 0, 1920, 1080, 0, false),
        ];

        let disconnected = mm.reconcile_monitors(new_monitors);
        assert!(disconnected.is_empty());
        assert_eq!(mm.monitors.len(), 2);
        assert_eq!(mm.monitors[0].workspace, 1);
        assert_eq!(mm.monitors[1].workspace, 2);

        // Simulate disconnecting the second monitor
        let single_mon = vec![Monitor::new(0, 0, 0, 1920, 1080, 1, true)];
        let disconnected = mm.reconcile_monitors(single_mon);
        assert_eq!(disconnected, vec![(1, 2)]);
        assert_eq!(mm.monitors.len(), 1);
    }
}
