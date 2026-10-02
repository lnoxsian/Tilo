 # Tilo Project Plan and Delivery Guide

This is the implementation-backed plan for Tilo, a single-process Linux/X11 window manager. The project includes tiling, floating and fullscreen clients, workspaces, XRandR monitors, keyboard and mouse input, an integrated bar and launcher, hot reload, scratchpad behavior, and ICCCM/EWMH support.

## Boundary

Tilo is a single binary. It does not provide Wayland, a compositor, wallpaper management, notifications, a terminal emulator, a file manager, or a plugin/scripting runtime. Those remain external programs.

## Build Prerequisites

Install Rust stable with edition 2024 support, Cargo, GNU Make, an X11 development environment, and `pkg-config`. TrueType/OpenType fonts (e.g. `fonts-dejavu-core`) are used for UI rendering. Xephyr and ordinary X11 test clients are required for isolated integration testing.

Dependencies are 100% pure Rust: `fontdue`, `x11rb`, `libc`, and `thiserror`. The default release profile uses `opt-level = "z"`, LTO, one codegen unit, aborting panics, and symbol stripping.

## Build and Test

```bash
cargo check
cargo build
cargo test
cargo build --release

make
make release
make test
make test-xephyr
make test-xephyr-release
make test-layouts
make clean
make install
```

Version helpers validate or update `VERSION` and `Cargo.toml`: `make version`, `make version-check`, `make bump-patch`, `make bump-minor`, `make bump-major`, and `make set-version V=x.y.z`.

## Test Layers

1. `cargo check` verifies compilation and feature wiring.
2. `cargo test` covers pure logic and conditional X11 tests.
3. `make test-xephyr` runs the real WM in a nested display.
4. `make test-layouts` runs automated Xephyr layout integration tests.
5. `scripts/run-xephyr.sh` demos launcher, themes, mouse, tabs, workspaces, scratchpad, floating, fullscreen, and reload.

Never take over a normal desktop display for tests; it already has a WM in most sessions.

## Xephyr Harness

The harness supports `1366x768`, `1920x1080`, `1280x720`, `854x480`, and arbitrary `WIDTHxHEIGHT`, plus `--demo`, `--layout`, `--theme`, `--config`, `--release`, `--non-interactive`, and `--automated-tests`.

```bash
./scripts/run-xephyr.sh --res 1280x720 --demo all
./scripts/run-xephyr.sh --layout bsp --non-interactive
./scripts/run-xephyr.sh --theme nord --demo theme
```

## Extension Checklist

Protocol behavior belongs in `src/x11/` and an event/action handler; client state in `src/client/`; geometry in `src/layout/`; workspace/output behavior in `src/workspace/` or `src/monitor/`; key syntax in `src/input/keybind.rs` and config action mapping; visible UI in `src/bar/`, `src/launcher/`, or `src/render/`; user settings in `Config`, its parser, and the generated template.

For every state mutation, update related X11 properties, focus/border state, bar hit rectangles, and retile behavior. Add a regression test at the narrowest layer that proves the change.

## Risk Areas

X11 behavior depends on server extensions and client compliance. `/proc` metrics are Linux-specific. Direct autostart and shell-backed `spawn` have different quoting semantics. Reload must preserve the old config on every parse or grab failure. Monitor hotplug can invalidate geometry while clients move. Rendering depends on available fonts and feature flags. These concerns require Xephyr coverage, not weaker unit tests.
# Minimal Rust X11 Tiling Window Manager

## Project Overview

Build a very small, fast, dependency-light tiling window manager for Linux/X11, written entirely in Rust.

The project should provide the core workflow of an i3-like window manager while remaining significantly simpler:

* X11 only
* Single binary
* Single process
* No compositor
* No external panel/statusbar
* No external launcher
* No desktop-environment framework
* No GTK/Qt
* No GPU rendering
* No tabs/splits UI outside the window-management tree
* Minimal runtime memory usage
* Minimal dependency tree
* Configurable keybindings
* Integrated application launcher
* Integrated status bar
* Workspaces
* Tiling and floating windows
* Multi-monitor support
* ICCCM/EWMH compatibility

The implementation should prioritize:

1. Correctness
2. Simplicity
3. Low memory usage
4. Low binary size
5. Minimal dependencies
6. Maintainability

Do not implement features merely because they exist in i3. Only implement features that fit the project's minimal philosophy.

---

# 1. Project Goals

## 1.1 Primary Goal

Create a standalone X11 window manager that can replace a basic i3/dwm-style setup for users who want:

```text
X11
  │
  ▼
┌──────────────────────────────┐
│            tilo               │
│                              │
│  Window Manager              │
│  Tiling Engine               │
│  Workspace Manager           │
│  Keybindings                 │
│  Launcher                    │
│  Statusbar                   │
│  Configuration               │
└──────────────────────────────┘
```

Everything should run inside one process.

---

# 2. Non-Goals

The first implementation must NOT include:

* Wayland
* GPU rendering
* OpenGL
* Vulkan
* GTK
* Qt
* Electron
* Tauri
* Desktop widgets
* Notification daemon
* Wallpaper manager
* Compositor
* File manager
* Terminal emulator
* Network manager
* Audio manager
* GUI configuration editor
* Plugin system
* Lua scripting
* Python scripting
* JavaScript runtime
* External launcher dependency
* External statusbar dependency

These can be considered only after the core WM is stable.

---

# 3. Technology Stack

## 3.1 Language

Rust stable.

Target:

```text
Linux
X11
x86_64
aarch64
```

The implementation should avoid unsafe Rust wherever practical.

---

# 4. Dependency Philosophy

Keep the dependency tree extremely small.

Primary dependency:

```toml
x11rb = "..."
```

Additional dependencies should only be introduced when there is a concrete requirement that cannot reasonably be implemented internally.

Avoid large GUI frameworks.

Potential dependencies for later phases:

```text
x11rb
fontconfig/font rendering library
```

Before adding any dependency:

1. Determine whether X11 already provides the functionality.
2. Determine whether Rust standard library can provide it.
3. Determine whether a small internal implementation is practical.
4. Only then add the dependency.

Run dependency audits regularly.

---

# 5. Repository Structure

Use the following structure:

```text
tilo/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── plan.md
├── docs/
│   ├── architecture.md
│   ├── x11.md
│   ├── layouts.md
│   ├── workspaces.md
│   ├── config.md
│   ├── launcher.md
│   ├── statusbar.md
│   └── protocols.md
├── src/
│   ├── main.rs
│   ├── wm.rs
│   ├── x11.rs
│   ├── client.rs
│   ├── monitor.rs
│   ├── workspace.rs
│   ├── container.rs
│   ├── layout.rs
│   ├── focus.rs
│   ├── input.rs
│   ├── keybind.rs
│   ├── config.rs
│   ├── launcher.rs
│   ├── statusbar.rs
│   ├── ewmh.rs
│   ├── atoms.rs
│   ├── events.rs
│   ├── spawn.rs
│   └── util.rs
└── tests/
    ├── config.rs
    ├── layout.rs
    └── workspace.rs
```

Keep modules small and single-purpose.

---

# Phase 0 — Project Bootstrap

## Objective

Create the Rust project and establish the minimal build environment.

## Tasks

Initialize:

```bash
cargo init --bin
```

Configure:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Add only the X11 dependency.

Create a minimal `main.rs`.

Verify:

```bash
cargo check
cargo build
cargo test
cargo build --release
```

Measure:

```bash
ls -lh target/release/tilo
```

Record the initial binary size.

## Deliverable

A compiling Rust executable that starts and exits cleanly.

---

# Phase 1 — X11 Connection

## Objective

Establish a reliable connection to the X server.

## Implement

Create:

```text
x11.rs
```

Responsibilities:

* Open X connection
* Obtain screen information
* Obtain root window
* Obtain visual information
* Expose X11 connection to WM
* Centralize X11 calls

### Error Handling

Define an error enum to capture connection and protocol errors:

```rust
use thiserror::Error;
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};

/// Represents errors that can occur during X11 initialization and operation.
#[derive(Debug, Error)]
pub enum X11Error {
    #[error("Failed to connect to X server: {0}")]
    Connect(#[from] ConnectError),
    #[error("X11 connection lost or invalid: {0}")]
    Connection(#[from] ConnectionError),
    #[error("Failed to get a reply from the X server: {0}")]
    Reply(#[from] ReplyError),
    #[error("Failed to execute an X11 request: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),
    #[error("Screen {0} is not available")]
    InvalidScreen(usize),
}
```

### Struct Definitions

Create:

```rust
/// Manages the underlying X11 connection and root screen details.
///
/// This struct holds the connection trait object, allowing the WM to send
/// requests to the X server. It also caches the initial screen dimensions
/// and root window ID.
pub struct X11Context {
    /// The active X11 connection (typically a `RustConnection` in x11rb).
    pub connection: x11rb::rust_connection::RustConnection,
    /// The screen number the WM is managing.
    pub screen_num: usize,
    /// The root window ID of the primary screen.
    pub root_window: x11rb::protocol::xproto::Window,
    /// The initial width of the screen in pixels.
    pub screen_width: u16,
    /// The initial height of the screen in pixels.
    pub screen_height: u16,
}
```

### Function Signatures

```rust
impl X11Context {
    /// Initializes a new connection to the X11 server.
    ///
    /// It attempts to connect to the server specified by the `DISPLAY` environment
    /// variable (or the default display if omitted). It extracts the root window and
    /// screen dimensions from the connection setup.
    ///
    /// # Returns
    /// A configured `X11Context` on success, or an `X11Error` on failure.
    pub fn new() -> Result<Self, X11Error>;

    /// Gets a reference to the active screen structure.
    ///
    /// # Returns
    /// An `x11rb::protocol::xproto::Screen` instance containing detailed
    /// visual and dimension information.
    pub fn screen(&self) -> &x11rb::protocol::xproto::Screen;
    
    /// Flushes the output buffer, ensuring all pending requests are sent to the X server.
    pub fn flush(&self) -> Result<(), X11Error>;
}
```

### Implementation Notes
- **x11rb Crate**: Use `x11rb` instead of raw C FFI (like `xlib` or `xcb`) for a safer, more idiomatic Rust experience.
- **Connection establishment**: Use `x11rb::connect(None)` to connect to the default display.
- **Screen retrieval**: The screen setup is obtained via `self.connection.setup().roots[self.screen_num]`.

## Validation

Running:

```bash
tilo
```

must successfully connect to:

```text
$DISPLAY
```

and print minimal diagnostic information when debug logging is enabled.

Example output:
```
[INFO  tilo::x11] Connected to X11 server.
[INFO  tilo::x11] Managing root window: 0x5a1
[INFO  tilo::x11] Screen dimensions: 1920x1080
```

---

# Phase 2 — Become the Window Manager

## Objective

Take ownership of the X11 root window.

The WM must correctly select the required root window events.

Implement handling for:

```text
SubstructureRedirect
SubstructureNotify
StructureNotify
PropertyChange
KeyPress
ButtonPress
PointerMotion
```

Handle the case where another WM already owns the display.

If another WM is running:

```text
exit cleanly
```

Do not crash.

### Function Signatures

```rust
use x11rb::protocol::xproto::{EventMask, Window};

impl X11Context {
    /// Claims the Window Manager role by registering for substructure redirection
    /// on the root window.
    ///
    /// If another window manager is already running, the X server will return an
    /// `Access` error. This function intercepts that error to exit gracefully.
    ///
    /// # Returns
    /// `Ok(())` if ownership is successfully claimed. Returns an `X11Error` if
    /// another WM is active or a protocol error occurs.
    pub fn become_wm(&self) -> Result<(), X11Error>;
}
```

### Algorithm Pseudocode

```text
Function become_wm(context):
    1. Define `event_mask` containing:
       - SubstructureRedirect
       - SubstructureNotify
       - StructureNotify
       - PropertyChange
       - KeyPress
       - ButtonPress
       - PointerMotion
    
    2. Send `ChangeWindowAttributes` request to `context.root_window` with `event_mask`.
    
    3. Synchronize with the X server (e.g., using `check()` or flushing).
    
    4. If the request fails with an `Access` error:
       a. Log a clear message: "Another window manager is already running."
       b. Return an error causing a graceful exit.
    
    5. Otherwise, return success.
```

### Implementation Notes
- **Event Masks**: In X11, only one client can register for `SubstructureRedirect` on a specific window (the root window) at a time. This is the defining characteristic of a Window Manager.
- **Error catching**: Since x11rb requests are asynchronous by default, make sure to call `.check()` on the cookie returned by `change_window_attributes` so you can synchronously catch the `Access` error before continuing.

## Deliverable

The program successfully becomes the X11 window manager.

---

# Phase 3 — X11 Event Loop

## Objective

Create the central event-processing loop.

Architecture:

```text
main()
  │
  ▼
WM::run()
  │
  ▼
event loop
  │
  ├── MapRequest
  ├── ConfigureRequest
  ├── DestroyNotify
  ├── UnmapNotify
  ├── PropertyNotify
  ├── KeyPress
  ├── ButtonPress
  └── ClientMessage
```

Create a centralized event dispatcher.

Avoid putting business logic directly inside `main.rs`.

### Struct Definitions

```rust
/// The main Window Manager application state.
pub struct WindowManager {
    /// The X11 connection and state.
    pub context: X11Context,
    /// Client management state (added in Phase 4).
    pub client_state: ClientState,
    /// Indicates if the event loop should keep running.
    pub running: bool,
}
```

### Function Signatures

```rust
use x11rb::protocol::Event;

impl WindowManager {
    /// Starts the main event loop, blocking the thread and waiting for X11 events.
    ///
    /// # Returns
    /// `Ok(())` when the loop exits normally (e.g., `self.running` becomes false).
    /// Returns `X11Error` if a fatal protocol error occurs.
    pub fn run(&mut self) -> Result<(), X11Error>;

    /// Dispatches a single X11 event to the appropriate handler method.
    ///
    /// # Arguments
    /// * `event` - The parsed X11 event from x11rb.
    pub fn handle_event(&mut self, event: Event) -> Result<(), X11Error>;
    
    // -- Event Handlers --
    
    /// Handles a request from a client to map (show) its window.
    fn handle_map_request(&mut self, window: Window) -> Result<(), X11Error>;
    
    /// Handles a request from a client to change its size, position, or stacking order.
    fn handle_configure_request(&mut self, event: &x11rb::protocol::xproto::ConfigureRequestEvent) -> Result<(), X11Error>;
    
    /// Handles the notification that a window was destroyed.
    fn handle_destroy_notify(&mut self, window: Window) -> Result<(), X11Error>;
    
    /// Handles keyboard input.
    fn handle_key_press(&mut self, event: &x11rb::protocol::xproto::KeyPressEvent) -> Result<(), X11Error>;
}
```

### Algorithm Pseudocode

```text
Function run(wm):
    1. Flush pending requests to the X server.
    2. Loop while `wm.running` is true:
        a. Wait for the next event: `event = wm.context.connection.wait_for_event()`
        b. If an error occurs during waiting, log it and possibly exit.
        c. Match the event type in `handle_event(event)`.
        d. After handling the event, call `wm.context.flush()`.
```

### Implementation Notes
- **Event Fetching**: `x11rb::Connection::wait_for_event()` blocks until an event is available. This prevents high CPU usage.
- **ClientMessage**: These events are essential for ICCCM and EWMH compliance (e.g., closing windows, changing workspaces).

---

# Phase 4 — Client Management

## Objective

Track every managed X11 window.

Create:

```rust
use x11rb::protocol::xproto::Window;

/// Represents a single tracked application window.
#[derive(Debug, Clone, Default)]
pub struct Client {
    /// The X11 Window ID.
    pub window: Window,
    /// The ID of the workspace this client is currently on.
    pub workspace: u32,
    /// The ID of the monitor this client is currently displayed on.
    pub monitor: u32,
    /// True if the window is floating (exempt from tiling).
    pub floating: bool,
    /// True if the window is in fullscreen mode.
    pub fullscreen: bool,
    /// True if the window is currently mapped (visible) to the screen.
    pub mapped: bool,
    /// True if the window currently has input focus.
    pub focused: bool,
}

/// Central registry for tracking and interacting with all managed clients.
pub struct ClientState {
    /// A map binding X11 Window IDs to `Client` data structures.
    pub clients: std::collections::HashMap<Window, Client>,
    /// An ordered list of Window IDs for focus history/stacking.
    pub focus_stack: Vec<Window>,
}
```

Implement:

* Add client
* Remove client
* Find client
* Focus client
* Map client
* Unmap client
* Kill client
* Move client
* Resize client

Maintain:

```text
Window -> Client
```

for fast lookup.

### Function Signatures

```rust
impl ClientState {
    /// Creates a new, empty client state registry.
    pub fn new() -> Self;

    /// Registers a new window to be managed by the window manager.
    ///
    /// # Arguments
    /// * `window` - The X11 window ID.
    /// * `client` - The initial state of the client.
    pub fn add_client(&mut self, window: Window, client: Client);

    /// Deregisters and removes a window from the window manager's state.
    ///
    /// # Arguments
    /// * `window` - The X11 window ID to remove.
    ///
    /// # Returns
    /// The removed `Client` instance, or `None` if it was not tracked.
    pub fn remove_client(&mut self, window: Window) -> Option<Client>;

    /// Retrieves a reference to a tracked client.
    pub fn get_client(&self, window: Window) -> Option<&Client>;

    /// Retrieves a mutable reference to a tracked client.
    pub fn get_client_mut(&mut self, window: Window) -> Option<&mut Client>;
}

impl WindowManager {
    /// Sets input focus to a specific client window.
    /// Updates borders and internal tracking.
    pub fn focus_client(&mut self, window: Window) -> Result<(), X11Error>;

    /// Maps a client window, making it visible on the screen.
    pub fn map_client(&self, window: Window) -> Result<(), X11Error>;

    /// Unmaps a client window, hiding it from the screen.
    pub fn unmap_client(&self, window: Window) -> Result<(), X11Error>;

    /// Safely requests a client to close via ICCCM protocols, or forcefully kills it.
    pub fn kill_client(&self, window: Window) -> Result<(), X11Error>;

    /// Moves and resizes a client window.
    pub fn move_resize_client(&self, window: Window, x: i32, y: i32, w: u32, h: u32) -> Result<(), X11Error>;
}
```

### Algorithm Pseudocode: Add Client

```text
Function add_client(window):
    1. Check if `window` is already in `ClientState.clients`. If yes, return.
    2. Query window attributes (e.g., is it an override_redirect window?).
    3. If it's an override_redirect window (like a popup or menu), ignore it.
    4. Construct a new `Client` object with default/inferred values.
    5. Insert into `ClientState.clients`.
    6. Push `window` to the front of `ClientState.focus_stack`.
    7. Trigger `map_client` to make it visible.
    8. Trigger tiling layout update.
    9. Trigger `focus_client` on the new window.
```

### Implementation Notes
- **HashMap Choice**: Using `std::collections::HashMap<Window, Client>` is ideal for maintaining the `Window -> Client` relationship, offering O(1) lookups.
- **Focus Stack**: Maintaining a `Vec<Window>` as a focus stack allows you to easily fallback to the previously focused window if the current one is closed.
- **Override Redirect**: Windows like tooltips or context menus have the `override_redirect` flag set. The window manager MUST ignore these windows and not attempt to manage them.
- **Graceful Kill**: To kill a client gracefully, send a `WM_DELETE_WINDOW` `ClientMessage`. If the window doesn't support the protocol (check `WM_PROTOCOLS` property), use the forceful `KillClient` X11 request.

---

# Phase 5 — Basic Tiling

## Objective

Implement the first usable tiling layout.

Start with a simple master/stack layout.

Example:

```text
┌──────────────────┬───────────────┐
│                  │               │
│                  │     app 2     │
│                  │               │
│      master      ├───────────────┤
│                  │               │
│                  │     app 3     │
│                  │               │
└──────────────────┴───────────────┘
```

### Data Structures

```rust
use x11rb::protocol::xproto::Window;

/// Represents a 2D rectangle in screen space, typically used for window geometries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// The X coordinate of the top-left corner.
    pub x: i32,
    /// The Y coordinate of the top-left corner.
    pub y: i32,
    /// The width of the rectangle in pixels.
    pub width: u32,
    /// The height of the rectangle in pixels.
    pub height: u32,
}

/// Configuration settings for the layout engine.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    /// Percentage of screen width dedicated to the master pane (0.0 to 1.0).
    pub master_ratio: f32,
    /// Number of pixels between adjacent windows.
    pub inner_gap: u32,
    /// Number of pixels between windows and the screen edge.
    pub outer_gap: u32,
    /// Width of the window borders in pixels.
    pub border_width: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            master_ratio: 0.55,
            inner_gap: 8,
            outer_gap: 12,
            border_width: 2,
        }
    }
}
```

### Functions & Logic

```rust
/// Calculates the bounding rectangles for a master/stack layout.
/// 
/// # Arguments
/// * `clients` - A slice of managed window IDs to layout.
/// * `screen_rect` - The usable screen area (excluding docks/bars).
/// * `config` - The layout configuration parameters (gaps, ratios).
/// 
/// # Returns
/// A vector of rectangles corresponding to each client in the same order.
pub fn calculate_master_stack(
    clients: &[Window],
    screen_rect: &Rect,
    config: &LayoutConfig,
) -> Vec<Rect>;

/// Applies the computed layout geometries to the actual X11 windows.
/// 
/// # Arguments
/// * `conn` - The X11 connection.
/// * `clients` - The windows to configure.
/// * `rects` - The computed geometry for each window.
/// 
/// # Returns
/// `Result<(), TiloError>` - Returns an error if an X11 configure request fails.
pub fn apply_layout<C: x11rb::connection::Connection>(
    conn: &C,
    clients: &[Window],
    rects: &[Rect],
) -> Result<(), TiloError>;
```

### Algorithm: Master/Stack with Gaps

```text
function calculate_master_stack(clients, screen, config):
    if clients is empty: return []
    
    n = clients.length
    rects = []
    
    // Calculate total usable area after outer gaps
    usable_x = screen.x + config.outer_gap
    usable_y = screen.y + config.outer_gap
    usable_w = screen.width - (2 * config.outer_gap)
    usable_h = screen.height - (2 * config.outer_gap)
    
    if n == 1:
        // Only one window, takes up the entire usable area
        rects.push(Rect(usable_x, usable_y, usable_w, usable_h))
        return rects
        
    // Calculate master dimensions
    master_w = usable_w * config.master_ratio - (config.inner_gap / 2)
    rects.push(Rect(usable_x, usable_y, master_w, usable_h))
    
    // Calculate stack dimensions
    stack_x = usable_x + master_w + config.inner_gap
    stack_w = usable_w - master_w - config.inner_gap
    stack_h = (usable_h - (config.inner_gap * (n - 2))) / (n - 1)
    
    for i in 1 to n-1:
        current_y = usable_y + (i - 1) * (stack_h + config.inner_gap)
        rects.push(Rect(stack_x, current_y, stack_w, stack_h))
        
    // Adjust dimensions to account for X11 borders inside the config logic
    for rect in rects:
        rect.width -= 2 * config.border_width
        rect.height -= 2 * config.border_width
        
    return rects
```

### Error Handling & Implementation Notes
*   **Error Types:** `TiloError::X11Error` when `ConfigureWindow` fails. This can happen if a window is destroyed between layout calculation and application. Log the error but continue running.
*   **Implementation Note:** X11 window borders (`border_width`) are *added* to the width and height by the X server. The layout algorithm must subtract `2 * border_width` from the calculated width and height to ensure windows fit the precise gaps.
*   Do not implement a complex container tree yet. A simple vector of clients per workspace is sufficient for this phase.

---

# Phase 6 — Focus Management

## Objective

Implement predictable keyboard-driven focus. Maintain one focused client per workspace.

### Data Structures

```rust
/// Direction for spatial focus and movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Represents the focus state of a given workspace.
#[derive(Debug, Clone, Default)]
pub struct FocusState {
    /// The currently focused window, if any.
    pub focused_window: Option<Window>,
    /// A history of focused windows to restore focus when a window closes.
    pub focus_history: Vec<Window>,
}
```

### Functions & Logic

```rust
/// Focuses a specific X11 window.
/// 
/// This updates internal state, sets X11 input focus, raises the window, 
/// and updates border colors.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The window to focus.
pub fn focus_client<C: x11rb::connection::Connection>(
    conn: &C,
    window: Window,
) -> Result<(), TiloError>;

/// Focuses the next window in a given spatial direction.
/// 
/// # Arguments
/// * `direction` - The direction to look for the next window.
/// * `clients` - Current visible windows and their geometries.
pub fn focus_direction(
    direction: Direction,
    clients: &[(Window, Rect)],
) -> Result<(), TiloError>;
```

### Focus Algorithm Updates

When focus changes:
1.  **Update internal state:** Push previous focus to history, set new focus.
2.  **Set X11 input focus:** Use `SetInputFocus` with `RevertTo::POINTER_ROOT`.
3.  **Raise window:** Use `ConfigureWindow` with `StackMode::ABOVE` (especially for floating windows, though tiling handles overlaps).
4.  **Update border:** Change border pixel color (active vs inactive hex values).
5.  **Update statusbar:** Send a signal (e.g., EWMH properties or IPC) to the statusbar.

### Error Handling & Implementation Notes
*   **Edge Cases:** If focusing `Left` from the master pane, and no window exists to the left, focus should wrap around or simply do nothing.
*   **X11 Gotcha:** Some apps (e.g., older Java apps) have strict `WM_HINTS` regarding input focus. Ensure you handle the `WM_TAKE_FOCUS` protocol if supported by the client, otherwise fall back to standard `SetInputFocus`.

---

# Phase 7 — Keyboard Input

## Objective

Implement configurable global keybindings.

Create:

```text
keybind.rs
input.rs
```

### Data Structures

```rust
use x11rb::protocol::xproto::ModMask;
use x11rb::protocol::xproto::Keysym;

/// High-level actions that can be triggered by keybindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Spawn an external command.
    Spawn(String),
    /// Close the currently focused window.
    Close,
    /// Move focus in a specific spatial direction.
    Focus(Direction),
    /// Move the currently focused window in a specific direction.
    Move(Direction),
    /// Switch to a different workspace by ID.
    Workspace(u32),
    /// Move the currently focused window to a different workspace.
    MoveToWorkspace(u32),
    /// Toggle floating state for the focused window.
    ToggleFloating,
    /// Toggle fullscreen state for the focused window.
    ToggleFullscreen,
    /// Reload the window manager configuration.
    Reload,
    /// Exit the window manager gracefully.
    Exit,
}

/// Represents a single keybinding configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBind {
    /// Modifier mask (e.g., Mod4, Shift).
    pub modifiers: u16,
    /// The specific keysym (e.g., Return, Q, j, k).
    pub keysym: Keysym,
    /// The action to execute when this keybind is pressed.
    pub action: Action,
}
```

### Functions & Logic

```rust
/// Grabs all registered keybindings globally on the X server.
/// 
/// This tells the X server to send KeyPress events to the window manager
/// whenever these specific combinations are pressed, regardless of which
/// window has focus.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `root` - The root window.
/// * `bindings` - The list of configured keybindings.
pub fn grab_keys<C: x11rb::connection::Connection>(
    conn: &C,
    root: Window,
    bindings: &[KeyBind],
) -> Result<(), TiloError>;

/// Translates a raw X11 KeyPress event into an actionable command.
/// 
/// # Arguments
/// * `event` - The raw X11 KeyPress event.
/// * `bindings` - The list of configured keybindings.
/// 
/// # Returns
/// An `Option<Action>` representing the command to run, if a match is found.
pub fn handle_key_press(
    event: &x11rb::protocol::xproto::KeyPressEvent,
    bindings: &[KeyBind],
) -> Option<Action>;
```

### Keybinding Registration Flow

1.  **Map Keysyms to Keycodes:** X11 key grabs require *keycodes* (hardware specific), but configurations use *keysyms* (logical symbols like "Return"). You must query the X server's keyboard mapping.
2.  **Handle NumLock/CapsLock:** X11 modifiers are strict. If NumLock is on, it adds a modifier mask. To ensure `Mod4 + Return` works regardless of NumLock, you must grab the key combination 4 times: (Modifiers), (Modifiers + NumLock), (Modifiers + CapsLock), (Modifiers + NumLock + CapsLock).
3.  **Execute Grab:** Send `GrabKey` requests for the root window using `GrabMode::ASYNC`.

### Example Configuration Snippet

```rust
vec![
    KeyBind {
        modifiers: ModMask::M4.into(),
        keysym: x11rb::protocol::x11::keysyms::XK_Return,
        action: Action::Spawn("alacritty".to_string()),
    },
    KeyBind {
        modifiers: ModMask::M4.into(),
        keysym: x11rb::protocol::x11::keysyms::XK_q,
        action: Action::Close,
    },
    KeyBind {
        modifiers: ModMask::M4.into(),
        keysym: x11rb::protocol::x11::keysyms::XK_j,
        action: Action::Focus(Direction::Down),
    },
]
```

Do not directly associate keycodes with shell commands.

### Error Handling & Implementation Notes
*   **Errors:** `TiloError::GrabFailed` if another application (or another WM) already has a grab on a specific key combination.
*   **Result Usage:** `grab_keys` should return a `Result`. If a grab fails, log a warning but do not crash the entire window manager unless the failure is critical (e.g. unable to communicate with X11).

---

# Phase 8 — Workspace System

## Objective

Implement i3-style workspaces.

Default:

```text
1
2
3
4
5
6
7
8
9
```

### Structures and Enums

```rust
/// Represents a unique identifier for a workspace.
pub type WorkspaceId = u32;

/// Defines the state and layout of a single workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Unique identifier for this workspace (e.g., 1-9).
    pub id: WorkspaceId,
    /// The name of the workspace, typically matching the ID as a string, but configurable.
    pub name: String,
    /// Ordered list of client windows currently managed in this workspace.
    pub clients: Vec<x11rb::protocol::xproto::Window>,
    /// The currently focused window in this workspace, if any.
    pub focused_client: Option<x11rb::protocol::xproto::Window>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            id: 1,
            name: "1".to_string(),
            clients: Vec::new(),
            focused_client: None,
        }
    }
}
```

### Functions and Signatures

```rust
/// Switches the currently active workspace to the specified target.
///
/// If the workspace does not exist, it will be implicitly created.
/// 
/// # Arguments
/// * `target` - The ID of the workspace to switch to.
/// 
/// # Returns
/// * `Result<(), WmError>` - Returns an error if X11 communication fails.
pub fn switch_workspace(target: WorkspaceId) -> Result<(), WmError>;

/// Moves the currently focused window to the target workspace.
///
/// # Arguments
/// * `window` - The X11 Window ID to move.
/// * `target` - The ID of the target workspace.
///
/// # Returns
/// * `Result<(), WmError>` - Returns an error if the window is invalid or X11 fails.
pub fn move_to_workspace(window: x11rb::protocol::xproto::Window, target: WorkspaceId) -> Result<(), WmError>;

/// Checks if a workspace with the given ID currently exists (is active or has clients).
///
/// # Arguments
/// * `id` - The WorkspaceId to check.
///
/// # Returns
/// * `bool` - True if the workspace is known and active/populated.
pub fn workspace_exists(id: WorkspaceId) -> bool;

/// Returns the ID of the currently active workspace.
///
/// # Returns
/// * `WorkspaceId` - The ID of the current workspace.
pub fn current_workspace() -> WorkspaceId;
```

### Algorithm: Workspace Switching

```text
Function switch_workspace(target_id):
  1. If target_id == current_workspace_id, return (no-op).
  2. Let current_ws = get_workspace(current_workspace_id).
  3. Let target_ws = get_or_create_workspace(target_id).
  4. For each window in current_ws.clients:
       Unmap window (hide from screen).
  5. For each window in target_ws.clients:
       Map window (show on screen).
  6. Update WM state variable to reflect current_workspace_id = target_id.
  7. If target_ws has a focused_client:
       Set X11 Input Focus to focused_client.
     Else:
       Set X11 Input Focus to root window.
  8. Recalculate and apply tiling layout for target_ws.
  9. Update EWMH properties (_NET_CURRENT_DESKTOP).
```

Switching workspace must:

1. Hide current workspace windows.
2. Show target workspace windows.
3. Restore previous focus where possible.
4. Recalculate layout.
5. Update statusbar.

### Implementation Notes
- **Edge cases:** Unmapping a window generates an `UnmapNotify` event. The WM must ignore `UnmapNotify` events that are generated by the WM itself during a workspace switch, otherwise it might mistakenly assume the client closed the window. Maintain a state flag or check if the window belongs to a hidden workspace when processing `UnmapNotify`.
- **Error Handling:** Returning `WmError::X11Error` on connection failures.

---

# Phase 9 — Floating Windows

## Objective

Allow selected windows to bypass tiling.

### Structures and Enums

```rust
/// Represents the saved geometry of a floating window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FloatingGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Indicates whether a client is tiled or floating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientLayoutState {
    /// Window is managed by the tiling algorithm.
    Tiled,
    /// Window is floating and placed freely. Contains its last known geometry.
    Floating(FloatingGeometry),
}
```

### Functions and Signatures

```rust
/// Toggles the floating state of a specific window.
///
/// If the window is currently tiled, it will be made floating, restoring its previous 
/// floating geometry or using a default centered geometry. If it is floating, it will
/// be tiled.
///
/// # Arguments
/// * `window` - The X11 Window ID.
///
/// # Returns
/// * `Result<(), WmError>` - Returns error on X11 failure or invalid window.
pub fn toggle_floating(window: x11rb::protocol::xproto::Window) -> Result<(), WmError>;

/// Moves a floating window to a new absolute position.
///
/// # Arguments
/// * `window` - The floating window.
/// * `x` - New X coordinate.
/// * `y` - New Y coordinate.
pub fn move_floating(window: x11rb::protocol::xproto::Window, x: i32, y: i32) -> Result<(), WmError>;

/// Resizes a floating window to new dimensions.
///
/// # Arguments
/// * `window` - The floating window.
/// * `width` - New width in pixels.
/// * `height` - New height in pixels.
pub fn resize_floating(window: x11rb::protocol::xproto::Window, width: u32, height: u32) -> Result<(), WmError>;
```

Floating windows should retain:

```text
x
y
width
height
```

### Implementation Notes
- When `toggle_floating` switches a window to `Tiled`, the WM must immediately trigger a layout recalculation for the current workspace.
- When moving or resizing, bounds checking can optionally be implemented to prevent windows from being dragged completely off-screen, though not strictly required for a minimal WM.
- Floating windows should generally be kept above tiled windows. Consider using `ConfigureWindow` to manipulate the stacking order (Z-index).

Do not implement a full floating layout manager.

---

# Phase 10 — Window Rules

## Objective

Allow configuration-based window behavior.

Example:

```text
rule class="Firefox" workspace=2
rule class="Pavucontrol" floating=true
rule class="Gimp" floating=true
```

Rules should be evaluated when a new client is mapped.

Initial supported properties:

```text
class
instance
title
workspace
floating
fullscreen
```

### Structures and Enums

```rust
/// Defines the property to match against for a window rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCondition {
    /// Matches the window's WM_CLASS (class part).
    Class(String),
    /// Matches the window's WM_CLASS (instance part).
    Instance(String),
    /// Matches the window's WM_NAME or _NET_WM_NAME (title).
    Title(String),
}

/// Defines the action to take when a rule matches a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleAction {
    /// If Some, assigns the window to the specified workspace ID instead of the active one.
    pub target_workspace: Option<WorkspaceId>,
    /// If Some(true), spawns the window in a floating state.
    pub floating: Option<bool>,
    /// If Some(true), spawns the window in fullscreen.
    pub fullscreen: Option<bool>,
}

/// A complete rule mapping a condition to an action.
#[derive(Debug, Clone)]
pub struct WindowRule {
    pub condition: RuleCondition,
    pub action: RuleAction,
}
```

### Functions and Signatures

```rust
/// Evaluates window rules for a newly mapped client and applies the resulting actions.
///
/// This should be called during the `MapRequest` handler, before the window is mapped
/// and assigned to a workspace.
///
/// # Arguments
/// * `window` - The newly requested window.
/// * `rules` - A slice of configured `WindowRule` objects.
///
/// # Returns
/// * `Result<RuleAction, WmError>` - Returns a combined `RuleAction` containing all applied overrides.
pub fn apply_rules(
    window: x11rb::protocol::xproto::Window,
    rules: &[WindowRule]
) -> Result<RuleAction, WmError>;
```

### Algorithm: Rule Matching
1. Extract the `WM_CLASS` (both instance and class strings) and `WM_NAME` from the window using X11 property requests.
2. Initialize a default `RuleAction` (all fields `None`).
3. Iterate through `rules`.
4. If `rule.condition` matches the extracted properties (exact string match):
   - Update the default `RuleAction` fields with the `rule.action` fields (if `Some`).
5. Use the resulting `RuleAction` to determine whether to put the window on a background workspace, make it float, etc., *before* finalizing the client struct.

Keep matching simple.

---

# Phase 11 — ICCCM Support

## Objective

Implement basic ICCCM compatibility.

Handle:

* WM_PROTOCOLS
* WM_DELETE_WINDOW
* WM_TAKE_FOCUS
* WM_NORMAL_HINTS
* WM_HINTS
* WM_STATE

### Structures and Enums

```rust
/// Represents the WM_STATE property values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum WmState {
    /// Window is withdrawn (hidden).
    Withdrawn = 0,
    /// Window is in its normal state (mapped).
    Normal = 1,
    /// Window is minimized (iconified).
    Iconic = 3,
}

/// Custom struct caching interned X11 atoms for ICCCM protocols.
#[derive(Debug, Clone, Copy)]
pub struct IcccmAtoms {
    pub wm_protocols: x11rb::protocol::xproto::Atom,
    pub wm_delete_window: x11rb::protocol::xproto::Atom,
    pub wm_take_focus: x11rb::protocol::xproto::Atom,
    pub wm_state: x11rb::protocol::xproto::Atom,
}
```

### Functions and Signatures

```rust
/// Attempts to close a client window gracefully via WM_DELETE_WINDOW.
///
/// If the client does not support WM_DELETE_WINDOW in its WM_PROTOCOLS,
/// it falls back to forcibly killing the client via `KillClient`.
///
/// # Arguments
/// * `window` - The target window to close.
/// * `atoms` - Interned atoms needed to check protocols and send messages.
///
/// # Returns
/// * `Result<(), WmError>`
pub fn send_wm_delete(
    window: x11rb::protocol::xproto::Window,
    atoms: &IcccmAtoms,
) -> Result<(), WmError>;

/// Updates the WM_STATE property on a client window.
///
/// # Arguments
/// * `window` - The window to update.
/// * `state` - The new `WmState`.
/// * `atoms` - Interned atoms containing `wm_state`.
pub fn set_wm_state(
    window: x11rb::protocol::xproto::Window,
    state: WmState,
    atoms: &IcccmAtoms,
) -> Result<(), WmError>;
```

### Algorithm: Graceful Shutdown
1. Query the `WM_PROTOCOLS` property of `window`.
2. Check if the returned list of atoms contains the `WM_DELETE_WINDOW` atom.
3. If true, construct a `ClientMessageEvent`:
   - `format`: 32
   - `window`: `window`
   - `type_`: `wm_protocols` atom
   - `data`: `[wm_delete_window_atom, timestamp, 0, 0, 0]`
4. Send the event to `window` using `SendEvent`.
5. If false, invoke the `KillClient` X11 request on `window`.

Correctly request graceful application shutdown using:

```text
WM_DELETE_WINDOW
```

Fallback to:

```text
XKillClient
```

only when appropriate.

---

# Phase 12 — EWMH Support

## Objective

Implement the EWMH properties required for normal desktop compatibility.

Implement:

```text
_NET_SUPPORTED
_NET_CLIENT_LIST
_NET_ACTIVE_WINDOW
_NET_CURRENT_DESKTOP
_NET_NUMBER_OF_DESKTOPS
_NET_DESKTOP_NAMES
_NET_DESKTOP_GEOMETRY
_NET_WORKAREA
_NET_SUPPORTING_WM_CHECK
```

Support important client messages:

```text
_NET_ACTIVE_WINDOW
_NET_CURRENT_DESKTOP
_NET_WM_STATE
_NET_CLOSE_WINDOW
```

Support states:

```text
_NET_WM_STATE_FULLSCREEN
_NET_WM_STATE_ABOVE
```

### Structures and Enums

```rust
/// Custom struct caching interned X11 atoms for EWMH.
#[derive(Debug, Clone)]
pub struct EwmhAtoms {
    pub net_supported: x11rb::protocol::xproto::Atom,
    pub net_client_list: x11rb::protocol::xproto::Atom,
    pub net_active_window: x11rb::protocol::xproto::Atom,
    pub net_current_desktop: x11rb::protocol::xproto::Atom,
    pub net_number_of_desktops: x11rb::protocol::xproto::Atom,
    pub net_desktop_names: x11rb::protocol::xproto::Atom,
    pub net_wm_state: x11rb::protocol::xproto::Atom,
    pub net_wm_state_fullscreen: x11rb::protocol::xproto::Atom,
    pub net_wm_state_above: x11rb::protocol::xproto::Atom,
    pub net_close_window: x11rb::protocol::xproto::Atom,
}
```

### Functions and Signatures

```rust
/// Updates the root window's _NET_ACTIVE_WINDOW property.
///
/// # Arguments
/// * `window` - The currently focused window, or `None` if focus is lost/on root.
/// * `atoms` - The EWMH atoms struct.
pub fn update_net_active_window(
    window: Option<x11rb::protocol::xproto::Window>,
    atoms: &EwmhAtoms,
) -> Result<(), WmError>;

/// Updates the root window's _NET_CLIENT_LIST property.
///
/// This should be called whenever a window is managed or unmanaged.
///
/// # Arguments
/// * `clients` - Slice containing all currently managed window IDs across all workspaces.
/// * `atoms` - The EWMH atoms struct.
pub fn update_net_client_list(
    clients: &[x11rb::protocol::xproto::Window],
    atoms: &EwmhAtoms,
) -> Result<(), WmError>;

/// Dispatches an incoming ClientMessage event related to EWMH.
///
/// # Arguments
/// * `msg` - The parsed ClientMessageEvent from X11.
/// * `atoms` - The EWMH atoms struct.
pub fn handle_client_message(
    msg: x11rb::protocol::xproto::ClientMessageEvent,
    atoms: &EwmhAtoms,
) -> Result<(), WmError>;
```

### Implementation Notes
- **_NET_SUPPORTING_WM_CHECK:** The WM must create a hidden child window of the root window, set `_NET_SUPPORTING_WM_CHECK` on the root window to point to this child window, and also set `_NET_SUPPORTING_WM_CHECK` on the child window pointing to itself. It must also set `_NET_WM_NAME` on the child window to "tilo".
- **_NET_WM_STATE:** Handling `_NET_WM_STATE` via ClientMessage requires checking `data.data32[0]`. According to the spec, `0` = Remove/Drop, `1` = Add/Set, `2` = Toggle. Ensure you parse these actions correctly when handling `_NET_WM_STATE_FULLSCREEN`.
- **Properties vs ClientMessages:** Some EWMH atoms are both properties stored on the root window and message types sent by clients (e.g., `_NET_CURRENT_DESKTOP`). Differentiate context clearly.

Only implement properties that are actually needed.

---

# Phase 13 — Fullscreen

## Objective

Implement fullscreen windows.

When fullscreen:

```text
border = 0
x = monitor.x
y = monitor.y
width = monitor.width
height = monitor.height
```

Save the previous geometry.

Toggling fullscreen restores previous state.

Update:

```text
_NET_WM_STATE
```

accordingly.

### Data Structures & Types

```rust
/// Represents the geometry of a window or monitor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Geometry {
    /// The X coordinate of the top-left corner.
    pub x: i32,
    /// The Y coordinate of the top-left corner.
    pub y: i32,
    /// The width of the element in pixels.
    pub width: u32,
    /// The height of the element in pixels.
    pub height: u32,
}

/// The fullscreen state of a client window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FullscreenState {
    /// The window is not fullscreen.
    None,
    /// The window is fullscreen. Stores the previous geometry to restore upon un-fullscreening.
    Fullscreen {
        previous_geometry: Geometry,
        previous_border_width: u32,
    },
}
```

### Functions

```rust
/// Toggles the fullscreen state of the currently focused window.
///
/// If the window is currently windowed, it saves its geometry, sets its geometry
/// to match the monitor, removes borders, and updates X11 properties.
/// If it is fullscreen, it restores the saved geometry and borders.
///
/// # Arguments
/// * `conn` - The active X11 connection.
/// * `client_id` - The X11 Window ID of the client to toggle.
/// * `monitor` - The monitor the client resides on, used for fullscreen dimensions.
///
/// # Returns
/// * `Result<(), WmError>` - Success or an error communicating with X11.
pub fn toggle_fullscreen(
    conn: &impl Connection,
    client_id: Window,
    monitor: &Monitor,
) -> Result<(), WmError>;

/// Updates the `_NET_WM_STATE` property for a window to add or remove `_NET_WM_STATE_FULLSCREEN`.
///
/// # Arguments
/// * `conn` - The active X11 connection.
/// * `client_id` - The X11 Window ID of the client.
/// * `is_fullscreen` - Whether to add (true) or remove (false) the property.
pub fn update_net_wm_state_fullscreen(
    conn: &impl Connection,
    client_id: Window,
    is_fullscreen: bool,
) -> Result<(), WmError>;
```

### Fullscreen Geometry Save/Restore Logic

**Algorithm:**
1. Check current `FullscreenState` of the client.
2. If `FullscreenState::None`:
   - Save client's current `x, y, width, height` and `border_width` into `FullscreenState::Fullscreen`.
   - Update client geometry: `x = monitor.x`, `y = monitor.y`, `width = monitor.width`, `height = monitor.height`.
   - Set client border width to `0`.
   - Send `ConfigureWindow` request to apply changes.
   - Call `update_net_wm_state_fullscreen(conn, client, true)`.
   - Raise window to the top of the stack (`StackMode::ABOVE`).
3. If `FullscreenState::Fullscreen`:
   - Retrieve `previous_geometry` and `previous_border_width`.
   - Update client geometry to the saved values.
   - Set client border width to `previous_border_width`.
   - Send `ConfigureWindow` request to apply changes.
   - Call `update_net_wm_state_fullscreen(conn, client, false)`.
   - Update internal state to `FullscreenState::None`.
   - Trigger layout recalculation to ensure everything fits.

### Error Handling & Strategy

- **Errors:** `WmError::X11Error` when setting geometry or properties fails.
- **Recovery:** If an X11 request fails during fullscreen toggle, revert the internal `FullscreenState` back to its previous state to prevent desync between WM memory and X server. Return the `WmError` up the stack to be logged.

### Implementation Notes & Edge Cases

- Remember to handle `ClientMessage` events for `_NET_WM_STATE`. Some applications (like video players or browsers) will send a client message to request fullscreen, rather than the user hitting a keybind.
- If a fullscreen window is closed, ensure `FullscreenState` is cleaned up.
- When moving a fullscreen window to another monitor, it should either un-fullscreen or adopt the new monitor's geometry. The latter is preferred in multi-monitor setups.
- Dialogs/popups spawned by a fullscreen app should appear on top of the fullscreen window.

---

# Phase 14 — Multi-Monitor Support

## Objective

Support multiple X11 monitors.

Use XRandR information through X11.

### Data Structures & Types

```rust
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
```

### Functions

```rust
/// Focuses the monitor visually to the left of the active monitor.
pub fn focus_monitor_left(state: &mut WmState) -> Result<(), WmError>;

/// Focuses the monitor visually to the right of the active monitor.
pub fn focus_monitor_right(state: &mut WmState) -> Result<(), WmError>;

/// Moves the currently focused client to the specified monitor.
pub fn move_to_monitor(state: &mut WmState, target_monitor_id: MonitorId) -> Result<(), WmError>;

/// Refreshes monitor geometry by querying XRandR and updates the WM state.
pub fn refresh_monitors(conn: &impl Connection, state: &mut WmState) -> Result<(), WmError>;
```

Each monitor should have an independent active workspace.

### Monitor Detection and Hotplug Algorithm

**Algorithm:**
1. **Initialization:**
   - Query XRandR version to ensure it is supported.
   - Select for XRandR events on the root window (`RRScreenChangeNotify`, `RROutputChangeNotify`).
2. **Querying:**
   - Call `get_screen_resources_current`.
   - Iterate over returned outputs.
   - For each output, call `get_output_info` and `get_crtc_info`.
   - Filter out disconnected outputs or those without a valid CRTC.
   - Build a list of `Monitor` structs from the CRTC geometries.
3. **Hotplug Handling (`RRScreenChangeNotify` received):**
   - Detect change.
   - Fetch the new list of connected monitors.
   - **Reconciliation:**
     - Identify removed monitors: Move all workspaces (and their clients) from removed monitors to the primary or closest surviving monitor.
     - Identify new monitors: Assign them an unused workspace.
     - Identify changed monitors: Update `x`, `y`, `width`, `height`.
   - Rebuild monitor geometry state.
   - Recalculate layouts for all visible workspaces based on new geometries.
   - Keep clients alive (do not unmap/destroy them, just reconfigure their windows).

When monitor configuration changes:

1. Detect change.
2. Rebuild monitor geometry.
3. Recalculate layouts.
4. Keep clients alive.

### Error Handling & Strategy

- **Errors:** `WmError::RandrNotSupported`, `WmError::X11Error` when querying CRTC/Output info.
- **Result Usage:** `refresh_monitors` returns `Result<(), WmError>`. If it fails on startup, the WM should fallback to querying basic X11 screen geometry (single monitor mode). If it fails during a hotplug event, log the error and retain the previous monitor state.

### Implementation Notes & Edge Cases

- XRandR coordinate space is a single large virtual screen. Monitors are just viewports into this space. Make sure window bounds clamping accounts for overlapping monitors (if the user configures them that way) or gaps between monitors.
- When an output is turned off via `xrandr`, it loses its CRTC. Handle empty CRTCs gracefully.

---

# Phase 15 — Configuration System

## Objective

Create a tiny custom configuration format.

Default:

```text
~/.config/tilo/config
```

### Config File Grammar & Syntax Specification

The syntax is line-oriented and whitespace-delimited.
- **Comments:** Lines starting with `#` are ignored.
- **Empty lines:** Ignored.
- **Directives:**
  - `mod <Key>` (e.g., `mod Mod4`)
  - `font <String...>` (e.g., `font JetBrainsMono Nerd Font 10`)
  - `border_width <u32>`
  - `inner_gap <u32>`
  - `outer_gap <u32>`
  - `master_ratio <f32>`
  - `terminal <String>`
  - `bind <KeyCombo> <Action> [Args...]`

Example:

```text
mod Mod4

font JetBrainsMono Nerd Font 10

border_width 2
inner_gap 8
outer_gap 8
master_ratio 0.60

terminal foot

bind Mod4+Return spawn foot
bind Mod4+Space launcher
bind Mod4+q close

bind Mod4+h focus left
bind Mod4+j focus down
bind Mod4+k focus up
bind Mod4+l focus right

bind Mod4+Shift+h move left
bind Mod4+Shift+j move down
bind Mod4+Shift+k move up
bind Mod4+Shift+l move right

bind Mod4+1 workspace 1
bind Mod4+2 workspace 2

bind Mod4+Shift+1 move_workspace 1
bind Mod4+Shift+2 move_workspace 2
```

Do not use TOML/YAML/JSON unless there is a compelling reason.

### Data Structures & Types

```rust
/// Tokens produced by the lexical analyzer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A string literal or identifier.
    Word(String),
    /// A numeric integer value.
    Number(u32),
    /// A floating-point numeric value.
    Float(f32),
    /// The end of a line.
    Newline,
    /// End of the configuration file.
    Eof,
}

/// Represents a parsed user configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// The modifier key used for WM actions (e.g., "Mod4" for Super).
    pub mod_key: String,
    /// Font specification string (e.g. "monospace 10", "DejaVu Sans Mono 11").
    pub font: String,
    /// Width of window borders in pixels.
    pub border_width: u32,
    /// Gap between windows in pixels.
    pub inner_gap: u32,
    /// Gap between windows and screen edges in pixels.
    pub outer_gap: u32,
    /// The proportion of the screen given to the master area (0.0 to 1.0).
    pub master_ratio: f32,
    /// The default terminal emulator command.
    pub terminal: String,
    /// A list of defined keybindings.
    pub bindings: Vec<KeyBind>,
}
```

### Functions

```rust
/// Tokenizes the raw string content of a configuration file.
///
/// # Arguments
/// * `content` - The raw string content of the configuration file.
///
/// # Returns
/// A vector of `Token`s, or an error if invalid syntax is encountered.
pub fn tokenize(content: &str) -> Result<Vec<Token>, ConfigError>;

/// Parses a list of tokens into a `Config` struct.
///
/// # Arguments
/// * `tokens` - The tokens generated by the tokenizer.
///
/// # Returns
/// A fully constructed `Config` or a `ConfigError` detailing what failed.
pub fn parse(tokens: &[Token]) -> Result<Config, ConfigError>;

/// Loads and parses the configuration file from disk.
///
/// Automatically resolves the path to `~/.config/tilo/config`.
///
/// # Returns
/// The parsed `Config`.
pub fn load_config() -> Result<Config, ConfigError>;
```

### Tokenizer/Parser Algorithm

**Tokenizer (`tokenize`):**
1. Read the input string character by character.
2. Skip whitespace spaces (but not newlines).
3. If `#` is encountered, ignore all characters until `\n`.
4. If `\n` is encountered, emit `Token::Newline`.
5. Accumulate alphanumeric characters and punctuation into a string buffer.
6. Try to parse the buffer as an `f32` or `u32`. If successful, emit `Token::Float` or `Token::Number`. Otherwise, emit `Token::Word`.
7. End with `Token::Eof`.

**Parser (`parse`):**
1. Iterate over tokens line by line (split by `Token::Newline`).
2. Match the first `Token::Word` in the line (the directive).
3. Parse arguments based on the directive:
   - `bind`: Expect `<Word(Keys)> <Word(Action)> [Args...]`. Parse `Keys` by splitting on `+` to extract modifiers and keysyms.
   - `border_width`: Expect `<Number>`.
   - `master_ratio`: Expect `<Float>`.
   - `font`: Collect all remaining `Word` tokens on the line into a single string.
4. If a line is malformed, construct a `ConfigError::ParseError` with line number context.
5. Accumulate valid directives into the `Config` struct.

### Error Handling

```rust
/// Errors that can occur during configuration loading and parsing.
#[derive(Debug)]
pub enum ConfigError {
    /// File not found or could not be read.
    IoError(std::io::Error),
    /// Tokenization or parsing failed at the given line number, with a descriptive message.
    ParseError { line: usize, message: String },
    /// An unknown action or modifier was specified.
    InvalidValue { line: usize, value: String },
}
```

Invalid lines should produce useful errors without crashing.

### Implementation Notes & Edge Cases

- Some font names have spaces (e.g., `JetBrainsMono Nerd Font 10`). The parser must consume all tokens up to `Token::Newline` for the `font` directive and join them with spaces.
- Same for `spawn`: `bind Mod4+Return spawn foot -a my_term` should collect `foot -a my_term` as a single `Action::Spawn` string.

---

# Phase 16 — Configuration Reload

## Objective

Reload configuration without restarting the WM.

Binding:

```text
Mod4 + Shift + R
```

### Functions

```rust
/// Reloads the configuration from disk while the WM is running.
///
/// This will parse the file, validate it, update the active WM state,
/// ungrab old keybindings, and grab the new ones.
///
/// # Arguments
/// * `conn` - The active X11 connection.
/// * `state` - The mutable WM state containing the current config.
///
/// # Returns
/// * `Result<(), ReloadError>` - Success or failure reason.
pub fn reload_config(conn: &impl Connection, state: &mut WmState) -> Result<(), ReloadError>;

/// Un-registers all current keybindings and registers the new ones with the X server.
///
/// # Arguments
/// * `conn` - The active X11 connection.
/// * `bindings` - The list of new parsed keybindings.
/// * `root` - The root window ID.
pub fn re_register_keybindings(
    conn: &impl Connection,
    bindings: &[KeyBind],
    root: Window,
) -> Result<(), WmError>;
```

### Configuration Reload Process Algorithm

```text
read config
    ↓
parse
    ↓
validate
    ↓
replace active config
    ↓
re-register keybindings
    ↓
redraw statusbar
```

**Detailed Process:**
1. **Read config:** Call `load_config()`.
2. **Parse & Validate:** If `load_config()` returns an `Err(ConfigError)`, catch it, log it via standard error, and **abort the reload**.
3. **Replace active config:**
   - Take the successfully parsed `Config`.
   - Safely swap it with `state.config`.
4. **Re-register keybindings:**
   - Call `UngrabKey` on the root window with `GRAB_ANY` to clear all old bindings.
   - Iterate through `state.config.bindings`.
   - Translate string modifiers and keysyms to X11 KeyCodes.
   - Call `GrabKey` for each combination (remember to grab variations for NumLock/CapsLock).
5. **Redraw & Re-layout:**
   - If borders, gaps, or fonts changed, update existing clients.
   - Trigger a re-layout of all visible workspaces.
   - Redraw the statusbar if fonts or colors were updated.

If parsing fails, retain the old configuration.

Never leave the WM without a valid configuration.

### Error Types

```rust
/// Errors that can occur during a hot configuration reload.
#[derive(Debug)]
pub enum ReloadError {
    /// The configuration file was invalid. The previous configuration was kept.
    ConfigInvalid(ConfigError),
    /// Failed to update X11 state (e.g., grabbing keys).
    X11Error(WmError),
}
```

### Implementation Notes & Edge Cases

- **Key Grabbing Edge Case:** Un-grabbing and re-grabbing keys might fail if another application holds a grab. Always check the result of `GrabKey`.
- **Client Borders:** When reloading `border_width` or `colors`, iterate over all currently managed clients and send `ConfigureWindow` and `ChangeWindowAttributes` so the new style applies immediately without needing to open a new window.

---

# Phase 17 — Built-in Statusbar

## Objective

Implement a lightweight X11 statusbar directly inside the WM.

Create a statusbar window using:

```text
override_redirect
```

The statusbar should occupy the configured work area.

Example:

```text
┌──────────────────────────────────────────────────────────┐
│ 1:term  2:web  3:code                  RAM 31%  19:42   │
└──────────────────────────────────────────────────────────┘
```

Initial modules:

```text
workspace
layout
window title
RAM
CPU
clock
```

Avoid external commands for every update.

Use internal Linux interfaces where practical:

```text
/proc/stat
/proc/meminfo
/sys/class/power_supply
```

Update modules on a timer.

### Statusbar Module System & Design

The statusbar consists of multiple modules that update independently based on their own intervals.

```rust
/// Core struct representing the built-in statusbar.
#[derive(Debug)]
pub struct Statusbar {
    /// The X11 window ID for the statusbar window.
    pub window_id: u32,
    /// The geometric bounds of the statusbar.
    pub rect: Rect,
    /// The collection of active modules to display.
    pub modules: Vec<Box<dyn StatusModule>>,
    /// Active foreground and background colors.
    pub color_scheme: ColorScheme,
}

/// Represents the data returned by a module after an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleUpdate {
    /// The module's text has changed; UI needs to be re-rendered.
    Changed(String),
    /// No change in module text since last tick.
    Unchanged,
    /// The module encountered an error (e.g., failed to read /proc).
    Error(String),
}

/// Trait defining the behavior of a statusbar module.
pub trait StatusModule: std::fmt::Debug {
    /// Returns the name of the module for debugging.
    fn name(&self) -> &'static str;
    
    /// The interval in milliseconds at which this module should be polled.
    fn update_interval(&self) -> u64;
    
    /// Called periodically to fetch new data.
    fn update(&mut self) -> Result<ModuleUpdate, TiloError>;
    
    /// Returns the current cached text to be drawn.
    fn current_text(&self) -> &str;
}
```

```rust
/// Creates and maps the statusbar window using the X11 override_redirect attribute.
///
/// # Arguments
/// * `conn` - The X11 connection handle.
/// * `screen` - The root screen information.
/// * `rect` - The target geometry of the statusbar.
///
/// # Returns
/// The newly created window ID, or a `TiloError`.
pub fn create_statusbar_window(conn: &impl Connection, screen: &Screen, rect: Rect) -> Result<u32, TiloError>;
```

### Error Handling Strategy
Errors reading from `/proc` or X11 mapping failures are encapsulated in `TiloError::StatusbarError`. If a specific module fails to read (e.g. `RAM` module cannot parse `/proc/meminfo`), it returns `ModuleUpdate::Error(msg)` and gracefully shows an error state string (e.g., `RAM: ERR`) without crashing the window manager.

### Implementation Notes & Edge Cases
* **X11 Gotcha**: Always set the `_NET_WM_WINDOW_TYPE` to `_NET_WM_WINDOW_TYPE_DOCK`.
* **X11 Gotcha**: Set `_NET_WM_STRUT_PARTIAL` to reserve space at the top or bottom of the screen so tiled windows do not overlap the statusbar.
* **Timers**: Use an event loop timer (e.g., via `epoll` or the event framework) to trigger module `update()` calls according to their `update_interval()`.

---

# Phase 18 — Statusbar Renderer

## Objective

Implement simple text rendering.

Requirements:

* UTF-8
* configurable font
* foreground color
* background color
* workspace highlighting
* active window indication

Do not introduce a GUI framework.

Keep rendering lightweight.

If proper modern font rendering is required, introduce the smallest practical font-rendering dependency at this stage.

For modern UTF-8 support without heavy UI frameworks, using `x11rb` alongside a pure Rust TrueType/OpenType rasterizer (`fontdue`) is implemented. For proper anti-aliasing and Unicode, `fontdue` renders into a software framebuffer with alpha blending; X11 core fonts (`ImageText8`) can be used for absolute minimalism (< 2 MB RAM).

```rust
/// Defines the color scheme for the statusbar or launcher.
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorScheme {
    /// Background color of the bar (e.g., "#282c34").
    pub bg: u32,
    /// Default foreground text color (e.g., "#abb2bf").
    pub fg: u32,
    /// Background color for the currently active workspace.
    pub active_bg: u32,
    /// Foreground text color for the currently active workspace.
    pub active_fg: u32,
}

/// Responsible for rendering text and shapes to the statusbar window.
#[derive(Debug)]
pub struct Renderer {
    /// The X11 graphics context used for drawing.
    pub gc: u32,
    /// The loaded font identifier (or Xft font reference).
    pub font_id: u32,
    /// Reusable double-buffer pixmap to prevent flickering.
    pub pixmap: u32,
}

impl Renderer {
    /// Loads an X11 font by name.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `font_name` - The logical font name (e.g., "-misc-fixed-medium-r-*-*-13-*-*-*-*-*-*-*").
    ///
    /// # Returns
    /// A font ID, or a `TiloError::FontLoadError` if the font cannot be found.
    pub fn load_font(conn: &impl Connection, font_name: &str) -> Result<u32, TiloError>;

    /// Draws a string of text at the specified coordinates.
    ///
    /// # Arguments
    /// * `conn` - The X11 connection handle.
    /// * `drawable` - The target window or pixmap ID.
    /// * `x` - The starting X coordinate.
    /// * `y` - The starting Y (baseline) coordinate.
    /// * `text` - The UTF-8 string to draw.
    /// * `color` - The text color in RGB format.
    pub fn draw_text(&self, conn: &impl Connection, drawable: u32, x: i16, y: i16, text: &str, color: u32) -> Result<(), TiloError>;
}
```

### Implementation Notes & Edge Cases
* **Flickering**: Always draw text and background shapes to a `Pixmap` first (double buffering), then use `CopyArea` to blast the completed image to the statusbar window in one frame.
* **Font Baseline**: Remember that X11 text drawing coordinates typically refer to the *baseline* of the text, not the top-left corner of the bounding box.

---

# Phase 19 — Built-in Application Launcher

## Objective

Implement an integrated launcher.

Binding:

```text
Mod4 + Space
```

Launcher:

```text
┌─────────────────────────────────────┐
│ > fire                              │
├─────────────────────────────────────┤
│ Firefox                             │
│ Firefox Developer Edition           │
└─────────────────────────────────────┘
```

## Launcher responsibilities

1. Scan `$PATH`.
2. Discover executable programs.
3. Cache executable names.
4. Display launcher window.
5. Accept keyboard input.
6. Filter results.
7. Launch selected program.
8. Close launcher.

Do not execute shell commands through:

```text
sh -c
```

when unnecessary.

Use direct process spawning.

### $PATH Scanning and Caching Algorithm

To ensure the launcher opens instantly, `$PATH` scanning should be performed asynchronously or cached upon WM startup, only invalidating if the user requests a refresh.

```rust
/// Core configuration and state for the application launcher.
#[derive(Debug)]
pub struct Launcher {
    /// X11 window ID for the launcher overlay.
    pub window_id: u32,
    /// Cached list of executable paths/names.
    pub executable_cache: Vec<String>,
    /// Current query typed by the user.
    pub query: String,
    /// Currently selected index in the filtered results.
    pub selected_index: usize,
}

/// Scans the system `$PATH` for executable files and caches them.
///
/// # Returns
/// A sorted vector of unique executable names.
///
/// # Errors
/// Returns `TiloError::PathScanError` if the PATH variable cannot be read.
pub fn scan_path() -> Result<Vec<String>, TiloError>;

/// Spawns the selected application as a detached child process.
///
/// # Arguments
/// * `app_name` - The executable name or full path to launch.
///
/// # Returns
/// The Process ID (PID) of the spawned application, or an error.
pub fn spawn_app(app_name: &str) -> Result<u32, TiloError>;
```

### Error Types
* `TiloError::SpawnError(std::io::Error)`: Returned if `spawn_app` fails. Should be caught by the event loop and optionally drawn as an error message on the statusbar.

---

# Phase 20 — Launcher Filtering

## Objective

Implement simple fuzzy matching.

Example:

```text
query: ff
```

should find:

```text
firefox
```

Start with a simple scoring algorithm.

Do not introduce a fuzzy-search crate unless needed.

### Fuzzy Matching Scoring Algorithm

A robust yet simple fuzzy match algorithm awards points for consecutive matches and matching at the start of words, while penalizing for large gaps between matched characters.

**Pseudocode:**
```text
function fuzzy_score(query, target):
    if query is empty: return 0
    
    query_idx = 0
    score = 0
    consecutive_matches = 0
    
    for each char in target:
        if query[query_idx] == char:
            score += 10
            score += consecutive_matches * 5
            
            // Bonus if it's the first character of the target
            if is_first_char: score += 15
            
            consecutive_matches += 1
            query_idx += 1
            
            if query_idx == query.length:
                return score
        else:
            consecutive_matches = 0
            score -= 1 // slight penalty for gaps
            
    // If we didn't match the whole query
    return 0 (or null)
```

### Rust Implementation

```rust
/// Computes a fuzzy match score between a user query and a target string.
///
/// # Arguments
/// * `query` - The user's input string.
/// * `target` - The executable name being evaluated.
///
/// # Returns
/// `Some(score)` if the query fuzzy-matches the target, `None` otherwise.
/// Higher scores represent better matches.
pub fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let query_chars: Vec<char> = query.chars().collect();
    let target_chars: Vec<char> = target.chars().collect();
    
    if query_chars.is_empty() {
        return Some(0);
    }
    
    let mut q_idx = 0;
    let mut score = 0;
    let mut consecutive = 0;
    
    for (t_idx, &t_c) in target_chars.iter().enumerate() {
        if q_idx < query_chars.len() && t_c.eq_ignore_ascii_case(&query_chars[q_idx]) {
            score += 10;
            score += consecutive * 5;
            if t_idx == 0 {
                score += 15; // Starting character bonus
            }
            consecutive += 1;
            q_idx += 1;
        } else {
            consecutive = 0;
            score -= 1; // Gap penalty
        }
    }
    
    if q_idx == query_chars.len() {
        Some(score)
    } else {
        None
    }
}
```

---

# Phase 21 — Launcher Rendering

Implement:

```text
input field
result list
selection highlight
keyboard navigation
```

Support:

```text
Arrow Up
Arrow Down
Enter
Escape
Backspace
Ctrl+W
Ctrl+A
Ctrl+U
```

The launcher must remain lightweight and responsive.

### Launcher Keyboard Event State Machine

Managing the launcher safely requires a well-defined state machine to interpret X11 key press events into actionable intents.

```rust
/// Represents the parsed action from a user keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherAction {
    /// Move selection up (Arrow Up, Ctrl+P)
    SelectPrevious,
    /// Move selection down (Arrow Down, Ctrl+N)
    SelectNext,
    /// Execute the currently selected item (Enter)
    Execute,
    /// Close the launcher without executing (Escape)
    Cancel,
    /// Delete the character immediately before the cursor (Backspace)
    DeleteChar,
    /// Delete the word before the cursor (Ctrl+W)
    DeleteWord,
    /// Delete all text before the cursor (Ctrl+U)
    DeleteLine,
    /// Move cursor to start of line (Ctrl+A)
    MoveToStart,
    /// Insert a character into the query.
    InsertChar(char),
}

/// Processes raw X11 KeyPress events and translates them to a `LauncherAction`.
///
/// # Arguments
/// * `keycode` - The physical keycode from X11.
/// * `modifiers` - Active modifiers (Shift, Control, Alt/Mod1, Mod4).
/// * `keysym_map` - Reference to the keyboard mapping to translate keycodes to characters.
///
/// # Returns
/// A semantic action `LauncherAction` to update the launcher state, or `None` if unhandled.
pub fn handle_launcher_keypress(keycode: u8, modifiers: u16, keysym_map: &KeysymMap) -> Option<LauncherAction>;

/// Applies a `LauncherAction` to the current launcher state, updating the UI query and selection.
///
/// # Arguments
/// * `launcher` - A mutable reference to the `Launcher` state.
/// * `action` - The parsed user action.
pub fn update_launcher_state(launcher: &mut Launcher, action: LauncherAction);
```

### Implementation Notes & Edge Cases
* **Keyboard Grab**: When the launcher is active, you must grab the keyboard via `GrabKeyboard` so that all inputs are routed directly to the launcher, overriding global WM bindings.
* **Selection Wrap-around**: Consider implementing wrap-around for `SelectNext` and `SelectPrevious` to improve UX (e.g., pressing Up on the first item jumps to the bottom).
* **Result Truncation**: Only draw the top `N` results (e.g., 10) to prevent the launcher window from overflowing vertically off-screen.

---

# Phase 22 — Startup / Autostart

## Objective

Provide optional startup commands.

Configuration:

```text
autostart "nm-applet"
autostart "dunst"
```

Spawn each command after the WM has initialized.

Avoid a shell where possible.

### Implementation Details

#### Structs & Enums

```rust
/// Represents a single autostart command configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutostartCmd {
    /// The binary name or path to execute.
    pub program: String,
    /// The arguments to pass to the program.
    pub args: Vec<String>,
}

/// Errors that can occur during the autostart phase.
#[derive(Debug)]
pub enum AutostartError {
    /// Failed to spawn the process.
    SpawnFailed(String, std::io::Error),
}
```

#### Functions

```rust
/// Executes all configured autostart commands.
///
/// Iterates over the provided list of commands and spawns each one in the background.
/// Note that this function does not block waiting for the commands to finish.
///
/// # Arguments
/// * `cmds` - A slice of autostart commands to execute.
///
/// # Returns
/// A `Result` which is `Ok` if all spawn attempts were processed (even if some failed
/// to launch, which are typically logged rather than causing a hard error).
pub fn run_autostart_commands(cmds: &[AutostartCmd]) -> Result<(), AutostartError>;
```

#### Error Handling Strategy
Autostart commands might fail to spawn (e.g., binary not found). Such errors should be logged as warnings, but they should **not** crash the window manager. The WM must continue running even if secondary tools fail to load.

#### Implementation Notes
- Use `std::process::Command` to spawn commands.
- Call `Command::spawn()` to run in the background.
- Ensure that the window manager ignores `SIGCHLD` or explicitly reaps zombie processes so that short-lived autostart programs do not leave zombies behind.

---

# Phase 23 — Scratchpad

## Objective

Implement one simple scratchpad workspace.

Binding:

```text
Mod4 + grave
```

Behavior:

```text
hidden
  ↓
toggle
  ↓
visible floating window
  ↓
toggle
  ↓
hidden
```

The scratchpad should not complicate the normal workspace model.

### Implementation Details

#### Structs & Enums

```rust
/// State of the scratchpad window.
#[derive(Debug, Clone, Default)]
pub struct Scratchpad {
    /// The X11 window ID of the current scratchpad client, if any.
    pub client: Option<u32>,
    /// Whether the scratchpad is currently visible.
    pub is_visible: bool,
}
```

#### Functions

```rust
/// Toggles the visibility of the scratchpad window.
///
/// If the scratchpad is hidden, it is moved to the current workspace, centered,
/// and mapped. If it is visible, it is unmapped and hidden.
///
/// # Arguments
/// * `scratchpad` - Mutable reference to the scratchpad state.
/// * `conn` - The X11 connection.
/// * `current_workspace` - The ID of the currently active workspace.
///
/// # Returns
/// `Result<(), X11Error>` indicating success or failure of X11 calls.
pub fn toggle_scratchpad(
    scratchpad: &mut Scratchpad,
    conn: &impl Connection,
    current_workspace: u32,
) -> Result<(), X11Error>;

/// Assigns a specific window to be the scratchpad.
///
/// # Arguments
/// * `scratchpad` - Mutable reference to the scratchpad state.
/// * `window_id` - The X11 window ID to track.
pub fn set_scratchpad_client(scratchpad: &mut Scratchpad, window_id: u32);
```

#### Algorithm (Toggle)
1. Check if `scratchpad.client` is `Some(window_id)`. If not, do nothing or spawn the scratchpad terminal.
2. If `scratchpad.is_visible` is `true`:
   - Send an X11 `UnmapWindow` request for the `window_id`.
   - Update `scratchpad.is_visible = false`.
3. If `scratchpad.is_visible` is `false`:
   - Calculate screen center coordinates.
   - Resize and move the `window_id` to the center (floating state).
   - Send an X11 `MapWindow` request.
   - Raise the window to the top of the stack.
   - Set X input focus to the `window_id`.
   - Update `scratchpad.is_visible = true`.

#### Implementation Notes
- The scratchpad window should ideally be excluded from standard tiling algorithms. Keep it as a permanent floating window.
- If the scratchpad client closes itself, remember to clear `scratchpad.client` on the `DestroyNotify` event.

---

# Phase 24 — Mouse Support

## Objective

Provide optional mouse interaction.

Support:

```text
Mod + Left Click
    focus/move

Mod + Right Click
    resize
```

Do not make mouse interaction mandatory.

Keyboard-first operation remains the primary design.

### Implementation Details

#### Structs & Enums

```rust
/// The current mouse interaction state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseAction {
    /// No action is currently in progress.
    None,
    /// Dragging a window to move it.
    Moving { window: u32, start_x: i32, start_y: i32, start_win_x: i32, start_win_y: i32 },
    /// Dragging a window to resize it.
    Resizing { window: u32, start_x: i32, start_y: i32, start_width: u32, start_height: u32 },
}

/// Represents a mouse button binding.
#[derive(Debug, Clone)]
pub struct MouseBinding {
    pub modifiers: u16,
    pub button: u8,
    pub action_type: MouseActionType,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseActionType {
    Move,
    Resize,
}
```

#### Functions

```rust
/// Handles a mouse button press event.
///
/// # Arguments
/// * `event` - The X11 ButtonPress event data.
/// * `action_state` - Mutable reference to the current mouse action state.
pub fn handle_button_press(
    event: &ButtonPressEvent,
    action_state: &mut MouseAction,
) -> Result<(), X11Error>;

/// Handles pointer motion events during a drag operation.
///
/// # Arguments
/// * `event` - The X11 MotionNotify event data.
/// * `action_state` - The current mouse action state.
/// * `conn` - The X11 connection to apply move/resize operations.
pub fn handle_motion_notify(
    event: &MotionNotifyEvent,
    action_state: &MouseAction,
    conn: &impl Connection,
) -> Result<(), X11Error>;
```

#### Algorithm (Mouse Drag)
1. On `ButtonPress` on a window frame/client:
   - Identify if modifiers match Move or Resize binding.
   - If Move: Set `MouseAction::Moving` saving initial pointer and window geometry.
   - If Resize: Set `MouseAction::Resizing` saving initial pointer and window geometry.
   - Grab the pointer (`GrabPointer`) to ensure we get all `MotionNotify` and `ButtonRelease` events.
2. On `MotionNotify`:
   - If `MouseAction::Moving`: Calculate delta (current pointer - start pointer). Add delta to `start_win_x/y`. Call `ConfigureWindow` to update position.
   - If `MouseAction::Resizing`: Calculate delta. Add delta to `start_width/height` (clamped to minimum dimensions). Call `ConfigureWindow` to update size.
3. On `ButtonRelease`:
   - Ungrab pointer (`UngrabPointer`).
   - Reset `MouseAction` to `None`.
   - Optional: Snap window into tiling grid if dropped in a tile area, or keep floating.

---

# Phase 25 — Window Decoration

## Objective

Implement minimal window borders.

Configuration:

```text
border_width 2

focused_color #ffffff
normal_color #444444
urgent_color #ff0000
```

No titlebars by default.

The decoration should only be:

```text
X11 border
```

This keeps rendering and memory usage minimal.

### Implementation Details

#### Structs & Enums

```rust
/// Configuration for window borders.
#[derive(Debug, Clone)]
pub struct BorderConfig {
    pub width: u32,
    pub focused_color: u32,
    pub normal_color: u32,
    pub urgent_color: u32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            width: 2,
            focused_color: 0xffffff,
            normal_color: 0x444444,
            urgent_color: 0xff0000,
        }
    }
}
```

#### Functions

```rust
/// Updates the border color of a specific window.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `color` - The RGB color value (e.g., 0xFFFFFF).
pub fn set_window_border_color(conn: &impl Connection, window: u32, color: u32) -> Result<(), X11Error>;

/// Applies the border width configuration to a window.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `width` - The border width in pixels.
pub fn set_window_border_width(conn: &impl Connection, window: u32, width: u32) -> Result<(), X11Error>;
```

#### Implementation Notes
- The X11 core protocol supports setting a border width and pixel color directly on the window attributes via `ChangeWindowAttributes`.
- Colors must be allocated or passed as a direct pixel value depending on the visual depth (usually TrueColor 24-bit).
- Border width takes up actual geometry space in X11; a 800x600 window with 2px borders is 804x604 on screen. Adjust tiling algorithms to account for `border_width * 2` when calculating dimensions.

---

# Phase 26 — Urgent Windows

Implement basic urgency handling.

Read:

```text
WM_HINTS
```

Track urgent clients.

Display urgency in the statusbar.

Optional urgent border color.

### Implementation Details

#### Structs & Enums

```rust
/// Flags parsed from WM_HINTS
#[derive(Debug, Clone, Default)]
pub struct WmHints {
    pub is_urgent: bool,
}
```

#### Functions

```rust
/// Checks if a window has the urgency hint set.
///
/// Reads the `WM_HINTS` property from the window and checks the `URGENCY_HINT` flag.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
///
/// # Returns
/// `Ok(true)` if urgent, `Ok(false)` otherwise, or `X11Error` on failure.
pub fn check_urgency_hint(conn: &impl Connection, window: u32) -> Result<bool, X11Error>;

/// Updates the visual state of a window based on its urgency.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `window` - The target X11 window ID.
/// * `is_urgent` - Whether the window is urgent.
/// * `config` - Border configuration for colors.
pub fn update_urgent_visuals(conn: &impl Connection, window: u32, is_urgent: bool, config: &BorderConfig) -> Result<(), X11Error>;
```

#### Algorithm (Urgency)
1. On `PropertyNotify` for `WM_HINTS`:
   - Fetch the property.
   - If `URGENCY_HINT` is set, mark client internal state as `urgent = true`.
   - Call `set_window_border_color` with `urgent_color`.
   - Trigger a statusbar refresh to highlight the workspace.
2. When the urgent window receives focus:
   - Clear the `urgent` flag internally.
   - Set border color back to `focused_color`.
   - Update `WM_HINTS` on the client to remove the `URGENCY_HINT` flag.

---

# Phase 27 — Layout Improvements

Once the basic master/stack layout is stable, add:

```text
master/stack
vertical
horizontal
monocle
```

Configuration:

```text
layout master
layout monocle
layout vertical
```

Add:

```text
cycle_layout
```

Do not implement every i3 layout feature.

## Layout Comparison with i3

The following table documents which layouts are implemented versus which are missing compared to i3:

| Layout                         | i3? | tilo? | Description |
|:-------------------------------|:---:|:-----:|:------------|
| **MasterStack** (`layout master`)   | Yes | ✅ Done  | First window is master (left `master_ratio` of screen); remaining windows stack vertically on the right. |
| **Vertical** (`layout vertical`)    | Yes | ✅ Done  | Divide screen width evenly by N. Windows placed side-by-side. |
| **Horizontal** (`layout horizontal`)| Yes | ✅ Done  | Divide screen height evenly by N. Windows stacked one above the other. |
| **Monocle** (`layout monocle`)      | Yes | ✅ Done  | All windows take full screen; only focused window is raised. |
| **Floating** (`toggle floating`)    | Yes | ✅ Done  | Windows float freely at arbitrary positions, stacked above tiled windows. |
| **Tabbed** (`layout tabbed`)        | Yes | ❌ Missing | All windows in the container share the full body area, with a **horizontal tab bar** rendered across the top showing client titles. The focused tab's window body is visible; other windows are hidden behind it. |
| **Stacked** (`layout stacking`)     | Yes | ❌ Missing | Window title bars are stacked **vertically** in an accordion-style header list. The focused window expands to fill the remaining body area below the stacked title headers. |

### Notes on Missing Layouts

**Tabbed Layout** (`layout tabbed`):
- Renders a horizontal row of tab buttons at the top of the container, one per managed window.
- Each tab displays the window's `WM_NAME` or `_NET_WM_NAME` title.
- The active tab's window body fills the area below the tab bar.
- Switching tabs changes focus and remaps/unmaps the windows accordingly.
- Requires X11 rendering of a tab decoration strip (or a separate drawable window per tab).

**Stacked Layout** (`layout stacking`):
- Renders a vertical list of title headers, one per managed window, in a column on one side.
- The focused window occupies all remaining screen space after the headers are drawn.
- Similar to Tabbed but oriented vertically for the title list rather than horizontally.
- Often used for a side-panel workflow where context switching is frequent.

Both layouts require a decoration/title rendering pass and are deferred to a later phase
after Window Decoration (Phase 25) is implemented.

### Implementation Details

#### Structs & Enums

```rust
/// Available layout types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutType {
    /// Traditional master and stack tiling.
    MasterStack,
    /// Evenly divide space vertically.
    Vertical,
    /// Evenly divide space horizontally.
    Horizontal,
    /// One window takes up the whole screen.
    Monocle,
    /// All windows share the full area with a horizontal tab bar at the top.
    /// Requires window decoration rendering. Deferred to post-Phase 25.
    Tabbed,
    /// Window title bars stacked vertically; focused window takes remaining body.
    /// Requires window decoration rendering. Deferred to post-Phase 25.
    Stacked,
}

impl Default for LayoutType {
    fn default() -> Self {
        Self::MasterStack
    }
}
```

#### Functions

```rust
/// Applies the chosen layout algorithm to the list of visible clients.
///
/// # Arguments
/// * `layout` - The layout type to apply.
/// * `clients` - List of window IDs to tile.
/// * `area` - The available screen geometry (excluding bars).
/// * `master_ratio` - The ratio of screen width given to the master area.
/// * `conn` - The X11 connection to apply positions.
pub fn apply_layout(
    layout: LayoutType,
    clients: &[u32],
    area: &Rect,
    master_ratio: f32,
    conn: &impl Connection,
) -> Result<(), X11Error>;

/// Cycles to the next layout in the list.
///
/// # Arguments
/// * `current` - The current layout.
/// # Returns
/// The next layout type.
pub fn cycle_layout(current: LayoutType) -> LayoutType;
```

#### Algorithms

- **Master/Stack**:
  - First window is master. It takes left `master_ratio` of width.
  - Remaining windows are stacked vertically in the right `1.0 - master_ratio` width.
- **Vertical**:
  - Divide `area.width` evenly by the number of `clients`.
  - Place windows side by side, each taking `1/N` width and full height.
- **Horizontal**:
  - Divide `area.height` evenly by the number of `clients`.
  - Place windows one above the other, each taking full width and `1/N` height.
- **Monocle**:
  - All windows take up the full `area`.
  - Only the focused window is raised to the top (or others are unmapped/pushed back).

---

# Phase 28 — Resize Controls

Implement:

```text
increase_master
decrease_master
```

Example:

```text
Mod4 + Ctrl + H
Mod4 + Ctrl + L
```

Allow configuration of step size:

```text
master_step 0.05
```

Clamp values:

```text
0.10 <= master_ratio <= 0.90
```

### Implementation Details

#### Structs & Enums

```rust
/// Workspace-specific layout state.
#[derive(Debug, Clone)]
pub struct LayoutState {
    pub layout_type: LayoutType,
    pub master_ratio: f32,
    pub master_step: f32,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            layout_type: LayoutType::default(),
            master_ratio: 0.5,
            master_step: 0.05,
        }
    }
}
```

#### Functions

```rust
/// Adjusts the master ratio by a specific delta, clamping to valid bounds.
///
/// # Arguments
/// * `state` - Mutable reference to the layout state.
/// * `delta` - The amount to add (can be negative).
pub fn adjust_master_ratio(state: &mut LayoutState, delta: f32);
```

#### Algorithm (Resize)
1. On `increase_master` binding: call `adjust_master_ratio(&mut state, state.master_step)`.
2. On `decrease_master` binding: call `adjust_master_ratio(&mut state, -state.master_step)`.
3. Inside `adjust_master_ratio`:
   - `state.master_ratio = (state.master_ratio + delta).clamp(0.10, 0.90);`
4. Trigger a re-layout of the current workspace via `apply_layout`.

---

# Phase 29 — Application State Recovery

The WM must gracefully handle:

* Applications closing unexpectedly
* Windows being unmapped
* Workspace changes during client destruction
* Monitor changes
* Launcher crashes
* Invalid configuration
* X11 errors
* Missing `$DISPLAY`

No panic should terminate the WM during normal X11 operation.

### Error Handling Strategy
- Use Rust's `Result` type extensively for all X11 operations.
- **Never** use `.unwrap()` or `.expect()` on X11 API calls, especially those targeting client windows, as clients can disconnect asynchronously at any time (e.g., `BadWindow`).
- Group operations logically: if an operation on a client fails because it disconnected, remove the client from the internal state and re-layout the workspace, then continue.

#### Error Types

```rust
/// Core application error types.
#[derive(Debug)]
pub enum WmError {
    /// X server connection lost or unavailable.
    ConnectionLost(String),
    /// Recoverable X11 protocol error (e.g. BadWindow).
    X11ProtocolError(X11Error),
    /// Configuration parse error.
    ConfigError(String),
}
```

#### Implementation Notes
- If X11 connection drops (`ConnectionLost`), logging and exiting gracefully is the only option.
- Validate configurations on load; if invalid, fallback to `Default::default()` rather than crashing.

---

# Phase 30 — X11 Error Handling

Centralize X11 error handling.

Distinguish:

```text
fatal X11 errors
recoverable X11 errors
client errors
```

Do not blindly unwrap X11 operations.

The WM should remain alive when a client disappears between operations.

### Implementation Details

#### Structs & Enums

```rust
/// Classification of X11 errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X11ErrorSeverity {
    /// Errors like BadWindow, BadDrawable — usually client disconnected. Recoverable.
    Recoverable,
    /// Errors like BadAlloc, BadLength — severe logic or server issues. Fatal.
    Fatal,
}

/// A parsed X11 error.
#[derive(Debug, Clone)]
pub struct ParsedX11Error {
    pub error_code: u8,
    pub severity: X11ErrorSeverity,
    pub description: String,
}
```

#### Functions

```rust
/// Custom X11 error handler that classifies and handles errors.
///
/// Parses the error and decides whether to panic (fatal) or log and continue (recoverable).
pub fn handle_x11_error(err: X11Error) -> Result<(), WmError>;
```

#### Strategy
- Map error codes to severities.
- Recoverable: `BadWindow`, `BadMatch`, `BadValue`. When intercepted, log a trace and return `Ok(())` for the specific handler so the main event loop continues.
- Fatal: `BadAlloc`, connection IO errors. These bubble up and initiate Phase 32 (Graceful Shutdown).

---

# Phase 31 — Startup Existing Windows

When starting after X11 already has windows:

1. Query root tree.
2. Identify existing top-level windows.
3. Ignore override-redirect windows.
4. Register valid clients.
5. Apply rules.
6. Assign workspaces.
7. Map/configure clients.
8. Focus the appropriate window.

This is required for reliable session startup.

### Implementation Details

#### Functions

```rust
/// Scans the X11 root window tree and manages pre-existing windows.
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
    state: &mut WmState,
) -> Result<(), WmError>;
```

#### Algorithm (Adopting Windows)
1. Call `QueryTree` on the root window.
2. For each child window returned:
   - Call `GetWindowAttributes`.
   - If `override_redirect` is true, or `map_state` is `IsUnmapped`, ignore the window.
   - Otherwise, call the normal `manage_client(window_id)` routine.
3. `manage_client` will:
   - Read window properties (`WM_CLASS`, `WM_NAME`).
   - Apply window rules (e.g. assigning to a specific workspace).
   - Set border width/color.
   - Subscribe to events on the window (`PropertyChange`, `StructureNotify`).
4. Re-layout all workspaces that received new clients.

---

# Phase 32 — Graceful Shutdown

Implement:

```text
Mod4 + Shift + Q
```

Before exiting:

1. Stop launcher.
2. Remove statusbar.
3. Restore client borders.
4. Restore client geometry where appropriate.
5. Release X11 selections.
6. Flush X11 connection.
7. Exit cleanly.

Never leave the X server in a broken WM-owned state.

### Implementation Details

#### Functions

```rust
/// Executes the graceful shutdown sequence.
///
/// Cleans up state, releases X11 resources, and unmanages all clients.
///
/// # Arguments
/// * `conn` - The X11 connection.
/// * `state` - The current WM state.
pub fn perform_graceful_shutdown(
    conn: &impl Connection,
    state: &mut WmState,
) -> Result<(), WmError>;
```

#### Algorithm (Shutdown Sequence)
1. Iterate over all managed clients:
   - Remove custom borders: `set_window_border_width(conn, client, 0)`.
   - Remove WM-specific properties or state (e.g. removing `WM_STATE` property).
   - Unmap scratchpads if they shouldn't persist.
2. Terminate owned child processes (statusbar, launcher) by sending `SIGTERM` and waiting briefly.
3. Release `WM_Sn` selection ownership so another WM can take over.
4. Call `SetInputFocus` to `PointerRoot`.
5. Call `conn.flush()`.
6. Terminate the main event loop and exit `main()` with `0`.

#### Implementation Notes
- Careful not to unmap normal application windows, otherwise X server restart is required to see them again without a WM. Just strip the borders and leave them visible.

---

# Phase 33 — Performance Optimization

Measure:

```text
binary size
RSS
startup time
CPU idle usage
CPU during window operations
launcher startup time
statusbar update cost
```

Target:

```text
Binary:
< 1 MB preferred

RSS:
as low as practical

Idle CPU:
near 0%

Processes:
1 WM process
```

Use:

```bash
size target/release/tilo
strip target/release/tilo
/usr/bin/time -v target/release/tilo
```

Avoid premature optimization before functionality works.

---

# Phase 34 — Dependency Audit

Run:

```bash
cargo tree
```

Document every dependency.

The final project should have a very small dependency graph.

Remove:

* unused crates
* redundant utilities
* unnecessary abstractions
* debug dependencies
* heavyweight libraries

---

# Phase 35 — Security Review

Check:

* command execution
* configuration parsing
* launcher input
* X11 property parsing
* integer conversions
* window IDs
* malformed client properties
* invalid configuration values
* filesystem paths

Never treat X11 client properties as trusted input.

---

# Phase 36 — Testing

Create unit tests for:

```text
configuration parser
keybinding parser
layout calculations
workspace operations
fuzzy matching
window rule matching
```

Example layout test:

```text
input:
screen = 1920x1080
clients = 3
master_ratio = 0.60
gap = 8

output:
master rectangle
stack rectangle 1
stack rectangle 2
```

Verify there are:

* no overlapping tiled windows
* no negative dimensions
* no windows outside the work area
* correct gaps
* correct workspace assignment

---

# Phase 37 — Integration Testing

Create an isolated X11 test environment using:

```text
Xephyr
```

Test:

```text
startup
window creation
window destruction
workspace switching
fullscreen
floating
launcher
statusbar
multi-monitor
configuration reload
```

Do not use a production desktop as the primary test environment.

---

# Phase 38 — Compatibility Testing

Test common applications:

```text
xterm
foot
alacritty
Firefox
Chromium
GTK applications
Qt applications
terminal TUIs
file managers
```

Verify:

```text
focus
keyboard input
resize
fullscreen
close
minimize-like behavior
clipboard interaction
```

The WM should not attempt to implement clipboard functionality itself.

---

# Phase 39 — Documentation

Write:

```text
README.md
docs/architecture.md
docs/config.md
docs/keybindings.md
docs/layouts.md
docs/x11.md
docs/protocols.md
docs/launcher.md
docs/statusbar.md
```

README should contain:

* Features
* Screenshots
* Requirements
* Build instructions
* Installation
* Configuration
* Keybindings
* Troubleshooting

---

# Phase 40 — Packaging

Support installation to:

```text
/usr/bin/tilo
```

Provide:

```text
Makefile
```

with:

```text
make
make release
make install
make uninstall
make clean
make test
```

Provide an XSession file:

```text
/usr/share/xsessions/tilo.desktop
```

Example:

```ini
[Desktop Entry]
Name=Tilo
Comment=Minimal Rust X11 Tiling Window Manager
Exec=tilo
Type=Application
```

---

# Phase 41 — Final Binary Optimization

Build with:

```bash
cargo build --release
```

Verify:

```text
LTO enabled
panic=abort
strip enabled
debug information removed
```

Measure final binary.

Compare:

```text
debug
release
stripped release
```

Document final size.

---

# Phase 42 — Final Feature Audit

Before declaring version 1.0, verify:

## Window management

* [ ] Tiling
* [ ] Floating
* [ ] Fullscreen
* [ ] Focus
* [ ] Move
* [ ] Resize
* [ ] Close
* [ ] Window rules

## Workspaces

* [ ] 1–9
* [ ] Switch workspace
* [ ] Move window
* [ ] Workspace persistence
* [ ] Multi-monitor workspaces

## Input

* [ ] Keyboard bindings
* [ ] Mouse bindings
* [ ] Modifier support
* [ ] Key grab management

## X11

* [ ] ICCCM
* [ ] EWMH
* [ ] XRandR
* [ ] WM_PROTOCOLS
* [ ] Existing-window management
* [ ] Graceful shutdown

## UI

* [ ] Statusbar
* [ ] Launcher
* [ ] Borders
* [ ] Font rendering
* [ ] Workspace indicators

## Configuration

* [ ] Config parser
* [ ] Keybindings
* [ ] Colors
* [ ] Layout
* [ ] Rules
* [ ] Autostart
* [ ] Reload

## Quality

* [ ] Unit tests
* [ ] Integration tests
* [ ] Xephyr testing
* [ ] Dependency audit
* [ ] Binary size measurement
* [ ] Memory measurement
* [ ] Documentation

---

# Development Rules

## Rule 1 — Keep It Small

Every feature must justify its implementation and dependencies.

Do not add abstractions simply because they may be useful later.

## Rule 2 — One Process

The normal WM environment should require only:

```text
tilo
```

for window management, launching applications, and displaying the statusbar.

## Rule 3 — No Shell Dependency

When launching applications:

```text
Command::new(...)
```

should be preferred over:

```text
sh -c ...
```

where possible.

## Rule 4 — Keyboard First

Everything important must be accessible from the keyboard.

## Rule 5 — No GPU

The WM must not require:

```text
OpenGL
Vulkan
GPU acceleration
```

## Rule 6 — X11 Only

Do not introduce Wayland abstractions into the initial architecture.

## Rule 7 — Incremental Implementation

Every phase must leave the project in a working state.

Do not implement several untested subsystems simultaneously.

## Rule 8 — Avoid Feature Creep

If a feature isn't required for the core WM workflow, postpone it.

---

# Recommended Milestones

## Milestone 0

```text
Rust project
X11 connection
```

## Milestone 1

```text
Basic working tiling WM
```

Includes:

* X11 event loop
* client management
* tiling
* focus
* keyboard bindings
* workspaces

## Milestone 2

```text
Usable daily-driver WM
```

Includes:

* floating
* fullscreen
* EWMH
* ICCCM
* configuration
* multi-monitor
* rules

## Milestone 3

```text
Integrated desktop workflow
```

Includes:

* launcher
* statusbar
* autostart
* scratchpad
* mouse support

## Milestone 4

```text
Production release
```

Includes:

* compatibility
* testing
* optimization
* documentation
* packaging

---

# Suggested Default Keybindings

```text
Mod4 + Return
    Spawn terminal

Mod4 + Space
    Launcher

Mod4 + Q
    Close window

Mod4 + Shift + Q
    Exit WM

Mod4 + Shift + R
    Reload config

Mod4 + H
    Focus left

Mod4 + J
    Focus down

Mod4 + K
    Focus up

Mod4 + L
    Focus right

Mod4 + Shift + H
    Move left

Mod4 + Shift + J
    Move down

Mod4 + Shift + K
    Move up

Mod4 + Shift + L
    Move right

Mod4 + F
    Fullscreen

Mod4 + Shift + Space
    Toggle floating

Mod4 + 1..9
    Switch workspace

Mod4 + Shift + 1..9
    Move window to workspace
```

---

# Definition of Done

Version 1.0 is complete when the following workflow works reliably:

```text
start X11
   │
   ▼
start tilo
   │
   ├── existing windows are managed
   │
   ├── new windows automatically tile
   │
   ├── keyboard navigation works
   │
   ├── workspaces work
   │
   ├── floating works
   │
   ├── fullscreen works
   │
   ├── launcher works
   │
   ├── statusbar works
   │
   ├── configuration reload works
   │
   └── multiple monitors work
```

The final result should be a small, fast, single-binary Rust X11 window manager that provides the core workflow expected from an i3-style tiling WM without requiring a collection of separate desktop utilities.
