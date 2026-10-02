# Tilo Architecture

Tilo is a single-process, reparenting-free X11 window manager written in Rust. It owns the root-window substructure redirect, receives X11 events, tracks client windows, computes geometry, and applies the resulting state directly through `x11rb`.

There is no compositor, external bar, external launcher, scripting runtime, or plugin system.

---

## Architectural Priorities

1. **Explicit state** — all runtime state is owned by `WindowManager` and its sub-structures; no global mutable state.
2. **Small protocol boundaries** — X11 interactions are centralized in `src/x11/`.
3. **Pure layout calculations** — layout functions are pure: input IDs + rect → output rects. No X11 calls inside layout code.
4. **Recoverable client errors** — per-window X11 failures are logged; only fatal connection loss stops the event loop.
5. **Non-blocking event loop** — filesystem and process work is kept out of the hot path except for bounded launcher refreshes.

---

## Process Startup

```
main()
  │
  ├── X11Context::new()              connect to $DISPLAY, cache root + screen
  ├── X11Context::become_wm()        select SUBSTRUCTURE_REDIRECT on root
  ├── WindowManager::new()           initialize all sub-systems
  ├── wm.enable_statusbar(24)        create bar window (non-fatal on failure)
  ├── wm.manage_existing_windows()   adopt pre-mapped windows (non-fatal on failure)
  ├── libc::malloc_trim(0)           (Linux only) return unused heap to OS
  └── wm.run()                       enter event loop
```

Initialization failures are fatal (cannot manage a display without a connection, root redirect, or usable state). Bar, monitor, and pre-existing-client failures are warnings.

---

## Module Map

| Area | Source path | Responsibility |
|---|---|---|
| Entry point | `src/main.rs` | Startup, fatal error reporting |
| WM coordinator | `src/wm/mod.rs` | `WindowManager`, shared state, event loop, reload, shutdown |
| Event dispatch | `src/wm/events.rs` | Routes X11 events to WM operations |
| Client lifecycle | `src/wm/client.rs` | Adoption, mapping, unmapping, destruction, scratchpad lifecycle |
| Layout/focus control | `src/wm/workspace.rs` | Retiling, focus, workspace and monitor transitions |
| WM UI | `src/wm/ui.rs` | Bar and launcher UI orchestration |
| Client records | `src/client/mod.rs` | Client struct, tiled/floating/fullscreen/urgent state |
| Floating geometry | `src/client/floating.rs` | Floating window position tracking |
| Fullscreen | `src/client/fullscreen.rs` | Fullscreen save/restore |
| Window decorations | `src/client/decoration.rs` | Border color application |
| Window rules | `src/client/rules.rs` | Rule engine: match and apply workspace/float/fullscreen |
| Scratchpad | `src/client/scratchpad.rs` | Dedicated floating scratchpad window |
| Layout algorithms | `src/layout/mod.rs` | Layout dispatch and `LayoutType` enum |
| BSP layout | `src/layout/bsp.rs` | Binary space partition tree |
| Grid layout | `src/layout/grid.rs` | Near-square grid |
| Master-stack | `src/layout/master_stack.rs` | All master/stack orientations + center-master |
| Monocle | `src/layout/monocle.rs` | Full-area monocle |
| Tabbed | `src/layout/tabbed.rs` | Tabbed (monocle + bar tab strip) |
| Virtual desktops | `src/workspace/mod.rs` | Workspace collection and ordering |
| Focus tracking | `src/workspace/focus.rs` | Focus metadata and history |
| Physical outputs | `src/monitor/mod.rs` | XRandR discovery and hotplug reconciliation |
| Input | `src/input/mod.rs` | InputManager, key grab lifecycle |
| Key parsing | `src/input/keybind.rs` | Key combo parsing, Action enum |
| Mouse | `src/input/mouse.rs` | Pointer grab and drag state |
| Status bar | `src/bar/mod.rs` | Bar window, modules, rendering |
| Bar modules | `src/bar/modules.rs` | CPU, RAM, IP, battery, clock modules |
| Tab rendering | `src/bar/tabs.rs` | Tab strip for tabbed layout |
| Workspace tags | `src/bar/tags.rs` | Workspace tag rendering + hit rectangles |
| Application launcher | `src/launcher/mod.rs` | Launcher overlay lifecycle |
| Desktop scanner | `src/launcher/scanner.rs` | `$PATH` scanning and incremental cache |
| Fuzzy match | `src/launcher/fuzzy.rs` | Scoring and ranking |
| Renderer | `src/render/mod.rs` | Double-buffered pixmap renderer, pure Rust `fontdue` & core-font paths |
| X11 connection | `src/x11/mod.rs` | `X11Context`, `X11Error` |
| Atom cache | `src/x11/atoms.rs` | Intern and cache all used atoms |
| EWMH | `src/x11/ewmh.rs` | EWMH root properties and client messages |
| ICCCM | `src/x11/icccm.rs` | WM_STATE, WM_PROTOCOLS, WM_HINTS |
| Cursor | `src/x11/cursor.rs` | Cursor loading |
| X11 errors | `src/x11/error.rs` | `X11Error` variants and conversions |
| Autostart | `src/autostart.rs` | Detached process spawning |
| Config | `src/config/mod.rs` | Parser, tokenizer, `Config` struct, hot reload |
| Theme | `src/config/theme.rs` | `ThemeConfig`, color scheme resolution |

---

## Runtime State Graph

```
WindowManager
├── X11Context                 connection, root window, screen
├── ClientState                Window → Client registry + focus/order lists
├── WorkspaceManager           workspace ID → Workspace
│   └── Workspace               clients + focus + BSP tree + layout
├── MonitorManager             outputs and workspace assignment
├── InputManager               key grabs and Action matching
├── Statusbar (Option)         modules, renderer, hit rectangles
├── Launcher (Option)          overlay, cache, query state, renderer
├── Scratchpad                 one dedicated floating client
└── Config                     borders, gaps, theme, bindings, autostart
```

The `ClientState` registry and workspace client lists both reference window IDs. The registry is authoritative for client properties; a workspace owns ordering and layout membership. Removing a client removes it from both structures and from the BSP tree.

---

## Event Loop

`WindowManager::run()` installs signal handlers, runs autostart commands, flushes setup state, and polls X11 events.

Signal flags are checked before each event batch:

| Signal | Effect |
|---|---|
| `SIGUSR1` | Toggle launcher |
| `SIGHUP` | Reload configuration |
| `SIGINT` | Graceful shutdown |
| `SIGTERM` | Graceful shutdown |
| `SIGCHLD` | Ignored — prevents zombie children |

`handle_event` routes all X11 event types:
- `MapRequest` / `ConfigureRequest` — client adoption and geometry negotiation
- `UnmapNotify` / `DestroyNotify` — withdrawal and removal
- `PropertyNotify` — title, urgency, and hints tracking
- `KeyPress` / `ButtonPress` / `MotionNotify` / `ButtonRelease` — input
- `ClientMessage` — EWMH protocol messages
- `RandrNotify` — monitor hotplug
- `ExposeEvent` — bar and launcher redraw

Fatal connection loss returns `WmError::ConnectionLost`. Client-specific failures are logged where recovery is safe.

---

## Client Lifecycle

```
MapRequest received
  │
  ├── override_redirect? → skip (not managed)
  ├── Select events on client window
  ├── Read WM_CLASS, WM_NAME, WM_HINTS, WM_NORMAL_HINTS
  ├── Evaluate window rules (workspace, floating, fullscreen)
  ├── Create Client record
  ├── Assign workspace and monitor
  ├── Apply border (color + width)
  ├── Active workspace? → Map window
  └── Inactive workspace? → Leave unmapped, track in workspace

Focus change
  ├── Update X11 input focus (SetInputFocus)
  ├── Raise client (ConfigureWindow StackMode)
  ├── Change border color (focused → focused_color, old → normal_color)
  ├── Update _NET_ACTIVE_WINDOW
  └── Refresh bar

Removal (DestroyNotify / voluntary UnmapNotify on active workspace)
  ├── Remove from ClientState registry
  ├── Remove from focus history
  ├── Remove from workspace client list
  ├── Collapse BSP tree node
  ├── Select best remaining focus candidate
  └── Retile
```

---

## Layout Boundary

Layout functions are pure:

```
calculate_layout(
    layout_type: LayoutType,
    clients: &[Window],
    usable_rect: Rect,
    config: &LayoutConfig,
) -> Vec<(Window, Rect)>
```

`apply_layout_with_border` is the shared helper that converts those rectangles into `ConfigureWindow` requests. Floating and fullscreen clients bypass tiled rectangle calculation. The bar's reserved area is subtracted before layout runs.

---

## Rendering Boundary

The bar and launcher render into an X11 pixmap and copy it to the visible window via `CopyArea`. A hash of the last frame state avoids a full redraw when nothing changed.

Font rendering uses a 100% pure Rust dual-engine pipeline: TrueType/OpenType font specifications use `fontdue` and an in-memory software framebuffer with SIMD rasterization; core-font specifications (`fixed`, `6x13`, XLFD) or host fallback use X11 core text drawing. Zero C GUI libraries are required.

---

## Error and Recovery Policy

| Scenario | Policy |
|---|---|
| Config parse error during reload | Keep previous configuration |
| Failed child spawn (autostart) | Log warning; continue remaining autostart commands |
| XRandR unavailable | Fall back to root screen as one monitor |
| Request for vanished window | Ignore or log; not process-fatal |
| Fatal connection loss | Clean up, exit code 1 |
| Graceful shutdown | Restore clients, destroy WM windows, exit code 0 |

---

## Testing Strategy

| Layer | Coverage |
|---|---|
| Unit tests | Config parsing, key/action parsing, layouts, BSP, fuzzy matching, workspace state, monitor reconciliation, rules, renderer helpers, status modules |
| Integration tests | X11 connection, client adoption, ICCCM/EWMH, floating, fullscreen, tabbed, scratchpad, multi-monitor, mouse, keyboard, shutdown, recovery |
| Xephyr harness | Interactive and automated real-WM testing in a nested display |

X11 integration tests require an X server; see [testing.md](testing.md) and the repository `Makefile`.
