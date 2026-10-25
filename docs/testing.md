# Testing Tilo

Tilo has a layered test strategy: from pure-logic unit tests to full integration tests running a real WM inside a nested Xephyr display.

---

## Test Layers

| Layer | Command | What It Tests |
|---|---|---|
| 1. Type check | `cargo check` | Compilation, feature wiring, type safety |
| 2. Unit tests | `cargo test` | Pure logic: layouts, config, keybinds, fuzzy, BSP, rules |
| 3. X11 integration | `cargo test` (gated) | X11-dependent paths compiled but not run without a display |
| 4. Xephyr WM session | `make test-xephyr` | Real WM running in an isolated nested display |
| 5. Automated integration | `make test-layouts` | Scripted WM + client interaction in Xephyr |
| 6. Interactive harness | `./scripts/run-xephyr.sh` | Manual feature exploration |

> [!IMPORTANT]
> Never use a real desktop display for WM integration testing. Tilo would displace the running WM. Always use Xephyr.

---

## Running Tests

### Unit and Pure-Logic Tests

```bash
cargo test
```

Covers:
- Configuration parsing and tokenization
- Key combo parsing and action matching
- All layout algorithms (zero, one, small, many clients)
- BSP tree insertion, removal, resize, and rotation
- Fuzzy match scoring and ranking
- Workspace state transitions
- Monitor reconciliation
- Window rules evaluation
- Renderer helper functions
- Status bar module formatting

### Single Test File

```bash
cargo test --test layout          # layout integration tests
cargo test --test config          # config parsing tests
cargo test --test floating        # floating client tests
cargo test --test statusbar       # statusbar module tests
cargo test --test launcher        # launcher search and discovery tests
cargo test --test tabbed          # tabbed layout + bar tab tests
cargo test --test workspace       # workspace management tests
cargo test --test focus           # focus and border state tests
cargo test --test ewmh            # EWMH protocol compliance tests
cargo test --test icccm           # ICCCM compliance tests
cargo test --test keyboard_input  # keybind parsing tests
cargo test --test mouse           # mouse grab and interaction tests
cargo test --test scratchpad      # scratchpad lifecycle tests
cargo test --test fullscreen      # fullscreen client tests
cargo test --test multi_monitor   # XRandR multi-monitor tests
cargo test --test decoration      # window border rendering tests
cargo test --test window_rules    # rule engine tests
cargo test --test autostart       # autostart command tests
cargo test --test shutdown        # graceful shutdown tests
cargo test --test recovery        # error recovery tests
cargo test --test existing_windows # adopt pre-existing windows at startup
cargo test --test config_reload   # hot reload tests
```

---

## Xephyr Test Harness

The [`scripts/run-xephyr.sh`](../scripts/run-xephyr.sh) script starts a nested Xephyr X server, builds and launches tilo inside it, and optionally runs layout demos or automated tests.

### Prerequisites

```bash
# Debian/Ubuntu
sudo apt install xserver-xephyr x11-apps xterm
```

### Basic Usage

```bash
./scripts/run-xephyr.sh             # debug build, interactive session
./scripts/run-xephyr.sh --release   # release build
./scripts/run-xephyr.sh --1080p     # 1920x1080 resolution
./scripts/run-xephyr.sh --demo all  # full showcase of all features
```

### Resolution Presets

| Flag | Resolution |
|---|---|
| `--1366`, `--1366x768` | 1366×768 (WXGA) |
| `--1080p`, `--1080` | 1920×1080 (Full HD) |
| `--720p`, `--720` | 1280×720 (HD, default) |
| `--480p`, `--480` | 854×480 (SD 16:9) |
| `--res 2560x1440` | Custom resolution |

### Demo Modes (`--demo <mode>`)

| Mode | What It Shows |
|---|---|
| `single` | Single-window monocle tiling |
| `master` | 2-window master/stack (55%/45%) |
| `stack` | 3-window master + vertical stack |
| `grid` | Evenly divided 2×2 grid |
| `tabbed` | Tabbed layout with statusbar tab pills |
| `bsp` | Binary space partitioning tree |
| `rotate` | Master-stack orientation rotation (5 positions) |
| `floating` | Tiled + floating window layer |
| `fullscreen` | Fullscreen mode toggle |
| `scratchpad` | Dedicated floating scratchpad |
| `workspaces` | Multi-workspace client distribution |
| `mouse` | Statusbar mouse and tab interactions |
| `theme` | Available theme presets |
| `all` | Comprehensive showcase (default) |
| `multi` | Multi-monitor dual-workspace showcase with separate screens |
| `tour` | Guided step-by-step interactive tour |

### Multi-Monitor Options

Simulate multiple physical monitors as separate Xephyr windows on your desktop:

```bash
./scripts/run-xephyr.sh --dual-monitor            # 2 separate monitor windows (720p each)
./scripts/run-xephyr.sh -m 2 -r 800x600           # 2 separate monitor windows (800x600 each)
./scripts/run-xephyr.sh -m 3 --480p               # 3 separate monitor windows (480p each)
./scripts/run-xephyr.sh --dual-monitor --demo multi # dual-monitor layout demo
```

### Layout Option

```bash
./scripts/run-xephyr.sh --layout master    # start with master layout
./scripts/run-xephyr.sh --layout bsp       # start with BSP (default)
./scripts/run-xephyr.sh --layout grid      # start with grid
```

### Theme Option

```bash
./scripts/run-xephyr.sh --theme nord          # Nord color scheme
./scripts/run-xephyr.sh --theme catppuccin    # Catppuccin Mocha
./scripts/run-xephyr.sh --theme gruvbox       # Gruvbox Dark
./scripts/run-xephyr.sh --theme dracula       # Dracula
./scripts/run-xephyr.sh --theme solarized     # Solarized Dark
./scripts/run-xephyr.sh --theme tokyo-night   # Tokyo Night (default)
```

### Custom Configuration

```bash
./scripts/run-xephyr.sh --config /path/to/myconfig
```

### Non-Interactive Mode

```bash
./scripts/run-xephyr.sh --non-interactive    # no interactive prompt
./scripts/run-xephyr.sh --demo all --non-interactive
```

### Automated Integration Tests

```bash
make test-layouts
# or directly:
./scripts/run-xephyr.sh --automated-tests
```

This runs a fixed set of cargo integration tests inside Xephyr:

```
layout, floating, statusbar, launcher, tabbed, scratchpad, shutdown, config, mouse
```

### Make Targets

```bash
make test-xephyr            # debug build + interactive Xephyr
make test-xephyr-release    # release build + interactive Xephyr
make test-layouts           # automated Xephyr integration suite
make test-layout LAYOUT=bsp # Xephyr layout demo with a specific layout
```

---

## Interactive Prompt Commands

When running in interactive mode, the harness provides a live prompt:

```
tilo> demo bsp           # Switch to BSP demo
tilo> demo tabbed        # Switch to tabbed demo
tilo> layout master      # Hot-reload layout to master
tilo> theme nord         # Hot-reload theme to Nord
tilo> spawn xclock       # Launch xclock into Xephyr
tilo> spawn xterm        # Launch xterm into Xephyr
tilo> clean              # Close all demo windows
tilo> reload             # Send SIGHUP to tilo (hot reload)
tilo> kill               # Send SIGTERM to tilo
tilo> exit / quit / q    # Exit the harness
```

---

## Writing New Tests

### Unit Test Pattern

```rust
// In tests/layout.rs or src/layout/mod.rs
#[test]
fn test_grid_zero_clients() {
    let rects = calculate_layout(LayoutType::Grid, &[], Rect::new(0, 0, 1280, 720), &LayoutConfig::default());
    assert!(rects.is_empty());
}

#[test]
fn test_grid_single_client() {
    let rects = calculate_layout(LayoutType::Grid, &[42], Rect::new(0, 0, 1280, 720), &LayoutConfig::default());
    assert_eq!(rects.len(), 1);
}
```

### Integration Test Pattern (Xephyr)

Integration tests connect to `$DISPLAY`, rely on an already-running Xephyr + tilo session, and are gated so they do not run without a display:

```rust
// In tests/layout.rs
#[test]
#[cfg(feature = "x11_integration")]
fn test_tiling_three_clients() {
    // connect to $DISPLAY, spawn clients, verify geometry...
}
```

---

## Test Coverage Areas

| Area | Unit Tests | Integration Tests |
|---|---|---|
| Configuration parser | ✓ | ✓ (config_reload) |
| Keybind parsing | ✓ | ✓ (keyboard_input) |
| Layout algorithms | ✓ | ✓ (layout) |
| BSP tree operations | ✓ | — |
| Fuzzy matching | ✓ | — |
| Workspace state | ✓ | ✓ (workspace) |
| Monitor reconciliation | ✓ | ✓ (multi_monitor) |
| Window rules | ✓ | ✓ (window_rules) |
| Floating clients | ✓ | ✓ (floating) |
| Fullscreen clients | ✓ | ✓ (fullscreen) |
| Tabbed layout | ✓ | ✓ (tabbed) |
| Scratchpad | ✓ | ✓ (scratchpad) |
| ICCCM compliance | — | ✓ (icccm) |
| EWMH compliance | — | ✓ (ewmh) |
| Status bar modules | ✓ | ✓ (statusbar) |
| Launcher discovery | ✓ | ✓ (launcher) |
| Mouse interaction | — | ✓ (mouse) |
| Graceful shutdown | — | ✓ (shutdown) |
| Error recovery | — | ✓ (recovery) |
| Decoration / borders | ✓ | ✓ (decoration) |
| Autostart | — | ✓ (autostart) |

---

## Common Failure Modes

| Failure | Cause | Fix |
|---|---|---|
| `another WM running` | Xephyr already has a WM | Kill existing tilo process on that display |
| `DISPLAY not set` | No X server available | Use `./scripts/run-xephyr.sh` instead of running manually |
| Font errors in tests | Missing fonts | Install `xfonts-base` or use `--no-default-features` build |
| Xephyr not found | Not installed | `sudo apt install xserver-xephyr` |
| Test timeout | Slow machine or resource contention | Increase sleep timings in harness |
