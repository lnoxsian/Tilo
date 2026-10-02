# ICCCM and EWMH Protocol Support

Tilo implements the ICCCM and EWMH subsets necessary for a standalone X11 WM that interoperates with standard X11 client applications.

---

## Supporting Window

At startup, tilo creates a hidden supporting window named `tilo` and:

- Points `_NET_SUPPORTING_WM_CHECK` on the root window to its ID.
- Points `_NET_SUPPORTING_WM_CHECK` on the supporting window to itself.
- Sets `_NET_WM_NAME` to `"tilo"` on the supporting window.

This tells EWMH-aware clients and pagers that tilo is an EWMH-compliant WM.

---

## EWMH Root Properties

Published on the root window at startup and updated as state changes:

| Atom | Meaning | Updated On |
|---|---|---|
| `_NET_SUPPORTED` | List of all EWMH atoms tilo supports | Startup |
| `_NET_CLIENT_LIST` | Ordered managed client window IDs | Window adoption / removal |
| `_NET_ACTIVE_WINDOW` | Focused client window ID, or `0` | Focus change |
| `_NET_CURRENT_DESKTOP` | Current workspace, zero-indexed | Workspace switch |
| `_NET_NUMBER_OF_DESKTOPS` | `9` at startup | Startup |
| `_NET_DESKTOP_NAMES` | Null-separated `"1\02\0...9"` | Startup |
| `_NET_DESKTOP_GEOMETRY` | Root width × height | Startup, RandR change |
| `_NET_WORKAREA` | Work area per desktop (excludes bar) | Startup, RandR change |
| `_NET_SUPPORTING_WM_CHECK` | Supporting window ID | Startup |
| `_NET_WM_NAME` | `"tilo"` on supporting window | Startup |

---

## Client Window Properties

### Read by Tilo

| Atom | Purpose |
|---|---|
| `WM_CLASS` | Instance and class for window rules matching |
| `WM_NAME` | Fallback window title |
| `_NET_WM_NAME` | Preferred window title (UTF-8) |
| `WM_HINTS` | Urgency hint, input flag |
| `WM_NORMAL_HINTS` | Size hints (min/max/increment) |
| `WM_PROTOCOLS` | Supported protocols (e.g., `WM_DELETE_WINDOW`) |
| `_NET_WM_WINDOW_TYPE` | Window type (dialog, dock, splash, etc.) |
| `_NET_WM_STATE` | Requested state (fullscreen, above, etc.) |

### Written by Tilo

| Atom | Value |
|---|---|
| `WM_STATE` | `NormalState`, `IconicState`, or `WithdrawnState` |
| `_NET_WM_STATE` | `_NET_WM_STATE_FULLSCREEN`, `_NET_WM_STATE_ABOVE` |

---

## ICCCM Adoption Rules

On startup, tilo queries the root tree and adopts all **pre-existing** top-level windows that pass these filters:

| Condition | Action |
|---|---|
| Window is the root | Skip |
| Window is the supporting window | Skip |
| Window is the bar or launcher | Skip |
| `override_redirect` flag set | Skip (menus, tooltips, etc.) |
| Unmapped (`map_state != IsViewable`) | Skip |
| All others | Adopt (manage) |

Newly mapped client windows follow the same rules in real time via `MapRequest` events.

---

## Client Lifecycle (ICCCM)

### Adoption

1. Check `override_redirect` — skip if set.
2. Select `PROPERTY_CHANGE`, `STRUCTURE_NOTIFY`, and `ENTER_WINDOW` events on the client.
3. Read `WM_CLASS`, `WM_NAME`, `_NET_WM_NAME`, `WM_HINTS`, `WM_NORMAL_HINTS`.
4. Evaluate window rules (workspace, floating, fullscreen).
5. Create a `Client` record, assign to workspace and monitor.
6. Apply borders.
7. Map if target workspace is active; otherwise leave unmapped.

### Window Close

Tilo prefers graceful shutdown via ICCCM:

1. Check `WM_PROTOCOLS` for `WM_DELETE_WINDOW`.
2. If supported, send a `WM_DELETE_WINDOW` `ClientMessage`.
3. If **not** supported, issue a forceful `KillClient` X11 request.

### Unmap vs. Withdrawal

- If a client unmaps itself **on the active workspace**, tilo treats it as withdrawal and removes it from management.
- Unmaps caused by workspace switching or tabbed-layout hiding remain tracked — the client is not destroyed.

---

## EWMH Client Messages

Tilo handles these incoming `ClientMessage` events:

| Message | Data | Result |
|---|---|---|
| `_NET_ACTIVE_WINDOW` | Window ID | Focus the requested managed client |
| `_NET_CURRENT_DESKTOP` | Desktop index | Switch to that workspace |
| `_NET_WM_STATE` | Action + atom(s) | Add, remove, or toggle fullscreen/above |
| `_NET_CLOSE_WINDOW` | — | Request client close (ICCCM protocol) |

### `_NET_WM_STATE` Actions

| Action value | Meaning |
|---|---|
| `0` | Remove the state |
| `1` | Add the state |
| `2` | Toggle the state |

Unsupported state atoms are ignored rather than being stored in internal state.

---

## Fullscreen Protocol

When a window enters fullscreen:

1. Previous geometry and border width are saved.
2. The window is moved and resized to fill the active monitor exactly.
3. The window is raised above all tiled and floating clients.
4. `_NET_WM_STATE_FULLSCREEN` is set on the window.

When exiting fullscreen:

1. The saved geometry and border width are restored.
2. The window is retiled or returns to its floating position.
3. `_NET_WM_STATE_FULLSCREEN` is removed.

---

## Bar and Strut

The built-in bar is an `override_redirect` dock window. It publishes:

| Property | Value |
|---|---|
| `_NET_WM_WINDOW_TYPE` | `_NET_WM_WINDOW_TYPE_DOCK` |
| `_NET_WM_STRUT` | Top reservation = bar height |
| `_NET_WM_STRUT_PARTIAL` | Full strut geometry (top, left x range) |

This allows EWMH-aware pagers and taskbars to correctly compute the work area.

---

## XRandR Events

On `RandrNotify`:

1. Fresh output/CRTC query is issued.
2. Tilo updates `MonitorManager`.
3. `_NET_DESKTOP_GEOMETRY` and `_NET_WORKAREA` are republished.
4. All visible workspaces are retiled.

---

## Diagnostic Properties

Useful for inspecting tilo's EWMH state:

```bash
xprop -root _NET_SUPPORTED
xprop -root _NET_CLIENT_LIST
xprop -root _NET_ACTIVE_WINDOW
xprop -root _NET_CURRENT_DESKTOP
xprop -id <window_id> WM_CLASS WM_NAME _NET_WM_STATE WM_STATE
```
