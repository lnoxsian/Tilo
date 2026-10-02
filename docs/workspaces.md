# Workspaces and Monitors

---

## Workspace Model

Tilo creates workspaces **1 through 9** at startup. Each workspace is independent and owns:

- An ordered list of tiled client window IDs
- The currently focused client on that workspace
- A BSP tree (used when the active layout is `bsp`)
- The active layout type

Additional workspace IDs are created on demand when a window rule targets one or a monitor needs one. The current workspace starts at `1`.

---

## Switching Workspaces

### Keyboard

| Action | Binding |
|---|---|
| Switch to workspace N | `Mod+1` through `Mod+9` |
| Move focused window to workspace N | `Mod+Shift+1` through `Mod+Shift+9` |

### Status Bar (Mouse)

| Interaction | Result |
|---|---|
| Left-click workspace tag | Switch to that workspace |
| Right-click workspace tag | Send focused window to that workspace |
| Scroll up/down over tags | Cycle workspaces |

Mouse interactions require `workspace_mouse true` and `workspace_wheel_switch true` (both enabled by default).

### When Switching

On switching from workspace A to workspace B:

1. All clients on workspace A are unmapped (hidden).
2. All clients on workspace B are mapped (shown).
3. Focus is restored to the previously focused client on B, or cleared if B is empty.
4. `WM_STATE` is updated on all affected clients.
5. EWMH `_NET_CURRENT_DESKTOP` and `_NET_ACTIVE_WINDOW` are updated.
6. The status bar workspace tags refresh.
7. B's layout is applied (retile).

---

## Moving Clients

```
Mod+Shift+N      Move focused client to workspace N
```

A client moved to an **inactive workspace** stays unmapped until that workspace is shown. Moving to the current workspace is a no-op.

New clients target the active monitor's workspace unless a window rule supplies a workspace override.

---

## Window Rules

The rule engine matches windows **before their first map** and assigns workspace, floating, or fullscreen state.

Rules match against:

| Property | Source |
|---|---|
| Instance name | First part of `WM_CLASS` |
| Class name | Second part of `WM_CLASS` |
| Window title | `WM_NAME` or `_NET_WM_NAME` |

All matches are **exact string comparisons** (not regex or glob).

Rules are evaluated in order; later rules override earlier values for the same field. Multiple fields can be set by a single rule.

> [!NOTE]
> The `rule` directive is not yet exposed in the user configuration file. Rule support is present internally and is planned for a future release.

---

## Monitor Model

`MonitorManager` tracks each connected XRandR output:

| Field | Description |
|---|---|
| ID | XRandR output ID |
| Position (`x`, `y`) | Top-left corner of the monitor |
| Dimensions (`w`, `h`) | Size of the monitor in pixels |
| Primary flag | Whether this is the primary output |
| Active workspace | Which workspace is currently displayed |

Monitors are sorted by `(x, y)` coordinates (left-to-right, top-to-bottom).

Different monitors can display **different workspaces** simultaneously. Each monitor owns its own workspace independently.

---

## Multi-Monitor Navigation

### Focus

| Binding | Action |
|---|---|
| `Mod+comma` | Focus monitor to the left |
| `Mod+period` | Focus monitor to the right |

Direction is determined by X coordinate: "left" means the nearest monitor with a smaller X origin.

### Moving Windows

| Binding | Action |
|---|---|
| `Mod+Shift+comma` | Send focused window to monitor on the left |
| `Mod+Shift+period` | Send focused window to monitor on the right |

Moving a client to another monitor updates its monitor and workspace association, then retiles both monitors.

---

## XRandR Hotplug Reconciliation

When a `RandrNotify` event is received (monitor connected/disconnected or resolution changed):

1. A fresh XRandR output and CRTC query is issued.
2. Cloned outputs sharing a CRTC are collapsed into one logical monitor.
3. **Existing monitors** are matched by XRandR ID or position and **retain their workspace assignments**.
4. **New monitors** receive unused workspace IDs, preferring IDs 1–9 in order.
5. **Disconnected monitors**: their workspaces are returned to the pool. Clients on those workspaces become reachable on the first surviving monitor.

If XRandR is unavailable or returns no active CRTC, the root screen is used as a single fallback monitor.

---

## Focus Invariants

- At most one tracked client is focused globally at any time.
- A focused client belongs to the **current workspace** and must be in the client registry.
- When the focused client disappears (destroyed or moved away):
  1. Tilo selects the best remaining candidate on the current workspace (most recently focused).
  2. If no candidate exists, focus is cleared.
  3. Borders, `_NET_ACTIVE_WINDOW`, and the bar are updated.
- Switching workspaces restores the last focused client on the destination workspace.
- On focus change:
  - The previously focused window gets the `border_normal` color.
  - The newly focused window gets the `border_focused` color.
  - Urgent windows get `border_urgent` regardless of focus state.

---

## EWMH Desktop Integration

Tilo publishes the following EWMH properties for workspace-aware applications (taskbars, docks, pagers):

| Property | Value |
|---|---|
| `_NET_NUMBER_OF_DESKTOPS` | 9 (fixed at startup) |
| `_NET_DESKTOP_NAMES` | Null-separated `"1\02\03\0..."` |
| `_NET_CURRENT_DESKTOP` | Zero-indexed current workspace |
| `_NET_DESKTOP_GEOMETRY` | Root window width × height |
| `_NET_WORKAREA` | Work area excluding the bar |
| `_NET_CLIENT_LIST` | Ordered list of managed window IDs |
| `_NET_ACTIVE_WINDOW` | Focused window, or `0` |

These are updated on every workspace switch, focus change, window adoption, and window removal.
