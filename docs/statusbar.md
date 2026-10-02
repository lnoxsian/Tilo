# Built-in Status Bar

The tilo status bar is an integrated X11 window — not an external process. It occupies the top of the primary/active screen and is created at startup with a height of 24 pixels.

---

## Window Properties

| Property | Value |
|---|---|
| Window type | `override_redirect` X11 dock |
| EWMH type | `_NET_WM_WINDOW_TYPE_DOCK` |
| Position | Top of the active screen |
| Height | 24 pixels (initial; updated on monitor changes) |
| Strut | Published via `_NET_WM_STRUT` and `_NET_WM_STRUT_PARTIAL` |

The layout engine subtracts the bar's reserved area from the usable rectangle before computing tiled window geometries. This ensures tiled windows never overlap the bar.

The bar is stored as `Option<Statusbar>` inside `WindowManager`, not as a separate process.

---

## Layout and Sections

The bar is divided into three logical sections:

```
┌──────────────────────────────────────────────────────────────────────┐
│  [1] [●2] [3]  [B]  Firefox — GitHub           192.168.1.5  64% 2.1GB  37% ▶ 78%  14:32  │
│  ─── workspaces ──  ─── title ────────────────  ──────────── modules ──────────────────  │
└──────────────────────────────────────────────────────────────────────┘
```

**Left:** Workspace tags + layout symbol  
**Center:** Active window title  
**Right:** Status modules (IP, RAM, CPU, battery, clock)

---

## Default Modules

Modules are polled on individual intervals and joined with ` | ` separators (configurable via `bar_separator`):

| Module | Poll Interval | Data Source | Example Output |
|---|---|---|---|
| Workspace tags | On event | Internal state | `[1] [●2] [3]` |
| Layout symbol | On event | Internal state | `[B]` |
| Active title | On event | `WM_NAME` / `_NET_WM_NAME` | `Firefox — GitHub` |
| Local IP | 3 seconds | UDP socket route lookup | `192.168.1.5` |
| RAM | 2 seconds | `/proc/meminfo` | `2.1GB / 8GB` |
| CPU | 1 second | `/proc/stat` deltas | `37%` |
| Battery | 5 seconds | Linux power-supply sysfs | `▶ 78%` |
| Clock | 1 second | System time | `14:32` |

Module errors become cached error text rather than crashing the WM.

> [!NOTE]
> The `/proc`-based modules (CPU, RAM) are Linux-specific. Battery reads Linux power-supply information when available.

---

## Data Sources

### CPU (`/proc/stat`)

Reads the aggregate CPU line, computes idle vs. total jiffies delta between polls, reports usage percentage.

### RAM (`/proc/meminfo`)

Reads `MemTotal` and `MemAvailable`. Reports used memory (`MemTotal - MemAvailable`) and total.

### Local IP

Opens a UDP socket connected to `8.8.8.8:53` and reads the bound local address. This is a non-blocking probe that avoids DNS resolution.

### Battery

Reads `/sys/class/power_supply/BAT*/capacity` and `/sys/class/power_supply/BAT*/status`. Shows charging indicator (▶ charging, — discharging or full).

### Clock

Uses system time formatted as `HH:MM`.

---

## Workspace Tags

Workspace tags in the bar show:

| Tag style | Meaning |
|---|---|
| Dimmed / inactive | Workspace exists, no clients |
| Active indicator | Currently displayed workspace |
| Occupied indicator | Has clients but not currently shown |
| Urgent indicator | Has a window with urgency hint |

### Mouse Interaction

Requires `workspace_mouse true` (default):

| Click | Action |
|---|---|
| Left-click | Switch to that workspace |
| Right-click | Send focused window to that workspace |
| Scroll up | Previous workspace |
| Scroll down | Next workspace |

Scroll behavior requires `workspace_wheel_switch true` (default).

---

## Tabbed Layout Mode

When the active layout is `tabbed`, the bar replaces the title section with a tab strip:

```
┌─────────────────────────────────────────────────────────────────────┐
│  [1] [●2]  [T]  [1: Firefox] [2: Terminal] [3: Editor]  14:32      │
└─────────────────────────────────────────────────────────────────────┘
```

Each tab pill shows:
- Optional 1-based index (if `tab_show_index true`)
- Window title truncated to `tab_max_width` pixels
- Urgent styling if the window has urgency hints
- Focus styling for the active tab

### Tab Mouse Interaction

Requires `tab_mouse true` (default):

| Click | Action |
|---|---|
| Left-click tab | Focus that window |
| Middle-click tab | Close that window |
| Scroll up/down over tabs | Cycle tabs |

Scroll behavior requires `tab_wheel_switch true` (default).

### Tab Keyboard Shortcuts

| Binding | Action |
|---|---|
| `Mod+Tab` | Focus next tab |
| `Mod+Shift+Tab` | Focus previous tab |

---

## Rendering Pipeline

```
Module polling
     │
     ▼
Build frame state (text segments, colors, hit rectangles)
     │
     ▼
Hash check (skip full redraw if nothing changed)
     │
     ▼
Draw to off-screen X11 pixmap
     │
     ▼
Copy pixmap to visible bar window
```

The renderer uses an off-screen pixmap to eliminate flicker. A hash of the last rendered frame state avoids unnecessary redraws.

### Font Rendering

| Font specification | Renderer |
|---|---|
| `"Family Size"` or `monospace` | Pure Rust TrueType/OpenType (`fontdue`) |
| `pango:Family Size` | Legacy prefix alias (maps to `fontdue`) |
| `/path/to/font.ttf Size` | Direct font file loading (`fontdue`) |
| `fixed`, `6x13`, `9x15` | X11 core font |
| `core:...` | X11 core font (explicit) |
| XLFD name | X11 core font |

When a requested font is unavailable or if no font files exist on the host, tilo automatically falls back to standard X11 core fonts. Reload recreates the renderer when the effective font changes.

---

## Theming

The bar inherits from the global palette and can be overridden:

```
bar_bg #16161e
bar_fg #c0caf5
bar_active_bg #7aa2f7      # active workspace / focused tab background
bar_active_fg #15161e      # text on active background
bar_urgent_bg #f7768e      # urgent workspace / tab background
bar_inactive_fg #555566    # dimmed workspace text
bar_font "monospace 10"
```

See [themes.md](themes.md) for the full color system and all directive aliases.

---

## Adding a Module

To add a new status module:

1. Implement the `StatusModule` trait:
   - `update() -> ModuleUpdate` — fetches new data; returns `Changed`, `Unchanged`, or `Error`
   - `text() -> &str` — returns the current display string
   - `interval() -> Duration` — polling interval (keep it bounded and non-blocking)

2. Add the module to the default module list in `src/bar/modules.rs`.

3. Ensure `update()` never blocks the event loop. Use cached values from previous polls if the source is temporarily unavailable.

4. Write a unit test for the text formatter.
