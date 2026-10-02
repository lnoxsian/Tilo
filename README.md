<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/svg/tilo_white_transparent.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/svg/tilo_dark_transparent.svg">
    <img alt="tilo logo" src="assets/svg/tilo_dark_transparent.svg" width="400">
  </picture>
</p>

<h1 align="center">tilo</h1>

<p align="center">
  <em>A minimal, fast, single-binary X11 tiling window manager written in Rust.</em>
</p>

```
X11
 │
 ▼
┌──────────────────────────────────┐
│              tilo                │
│                                  │
│  Window Manager                  │
│  Tiling Engine  (BSP, Master,    │
│                  Grid, Tabbed…)  │
│  Workspace Manager  (1–9)        │
│  Multi-Monitor  (XRandR)         │
│  Keybindings                     │
│  Integrated Launcher             │
│  Integrated Status Bar           │
│  ICCCM / EWMH                    │
└──────────────────────────────────┘
```

**Everything runs in one process. No compositor. No external bar. No external launcher.**

---

## Features

- Binary space partitioning, master-stack, grid, monocle, tabbed, and more layouts
- 9 virtual workspaces with multi-monitor XRandR support
- Integrated status bar (CPU, RAM, battery, clock, workspace tags, tabs)
- Integrated application launcher with fuzzy search and exit mode
- Floating and fullscreen window layers
- Dedicated scratchpad window
- Hot-reload configuration (`Mod+Shift+r` / `SIGHUP`)
- ICCCM and EWMH protocol compliance
- Pure Rust TrueType/OpenType font rendering with anti-aliasing via `fontdue` (zero C GUI dependencies)
- Dependency-minimal: `fontdue`, `x11rb`, `libc`, `thiserror`

---

## Quick Start

### Prerequisites

| Requirement | Notes |
|---|---|
| Rust stable (edition 2024) | Install via [rustup](https://rustup.rs/) |
| Cargo | Included with Rust |
| X11 development libraries | `libx11-dev`, `libxrandr-dev`, `libxcb-*` |
| TrueType / OpenType fonts | `fonts-dejavu-core` or `fonts-liberation` |
| pkg-config | For build-time library detection |
| GNU Make | Optional, wraps Cargo commands |

On Debian/Ubuntu:
```bash
sudo apt install libx11-dev libxrandr-dev libxcb1-dev \
    pkg-config build-essential fonts-dejavu-core
```

### Build

```bash
# Debug build
cargo build

# Optimized release build (small binary < 1 MB, LTO, stripped)
cargo build --release
```

Or via Make:
```bash
make          # debug
make release  # release
```

### Install

```bash
make install              # installs to /usr/local/bin/tilo
make install PREFIX=~/.local  # install to ~/.local/bin/tilo
```

### Run

Tilo is designed to be launched by a display manager or `~/.xinitrc`. It must be the **only** WM on the display — starting it inside an existing desktop session will fail cleanly.

```bash
# In ~/.xinitrc or equivalent
exec tilo
```

For development and testing, use the included Xephyr harness:
```bash
./scripts/run-xephyr.sh              # interactive debug session
./scripts/run-xephyr.sh --release    # release build
./scripts/run-xephyr.sh --demo all   # full feature showcase
```

---

## Configuration

Config file location: `$XDG_CONFIG_HOME/tilo/config` (defaults to `~/.config/tilo/config`).

A template is generated on first run. Key directives:

```
mod_key Mod4
font "monospace 10"
terminal foot
layout bsp
border_width 2
inner_gap 8
outer_gap 8
master_ratio 0.60

bg #1a1b26
fg #c0caf5
active_bg #7aa2f7

bind $mod+Return spawn foot
bind $mod+d launcher
bind $mod+Shift+r reload
bind $mod+q close
bind $mod+1 workspace 1
bind $mod+Space cycle_layout
```

→ See [`docs/config.md`](docs/config.md) for the full reference.

---

## Default Keybindings

| Key | Action |
|---|---|
| `Mod+Return` | Open terminal |
| `Mod+d` | Open application launcher |
| `Mod+q` | Close focused window |
| `Mod+h/j/k/l` | Focus left/down/up/right |
| `Mod+Shift+h/j/k/l` | Move window |
| `Mod+1`–`Mod+9` | Switch workspace |
| `Mod+Shift+1`–`9` | Move window to workspace |
| `Mod+Space` | Cycle layout |
| `Mod+r` | Rotate master orientation |
| `Mod+Shift+Space` | Toggle floating |
| `Mod+f` | Toggle fullscreen |
| `Mod+\`` | Toggle scratchpad |
| `Mod+Shift+\`` | Set scratchpad |
| `Mod+v` | Preselect vertical BSP split |
| `Mod+b` | Preselect horizontal BSP split |
| `Mod+Ctrl+Arrow` | Resize BSP pane |
| `Mod+Shift+r` | Reload configuration |
| `Mod+Shift+q` | Open exit menu |

---

## Documentation

| Document | Description |
|---|---|
| [`docs/config.md`](docs/config.md) | Configuration file format, all directives, keybindings |
| [`docs/building.md`](docs/building.md) | Build system, release process, version management |
| [`docs/layouts.md`](docs/layouts.md) | Layout engine, geometry contract, all layout types |
| [`docs/workspaces.md`](docs/workspaces.md) | Workspaces, multi-monitor, focus invariants |
| [`docs/statusbar.md`](docs/statusbar.md) | Built-in status bar, modules, rendering |
| [`docs/launcher.md`](docs/launcher.md) | Application launcher, fuzzy search, exit mode |
| [`docs/protocols.md`](docs/protocols.md) | ICCCM and EWMH protocol support |
| [`docs/x11.md`](docs/x11.md) | X11 integration, XRandR, cursors, diagnostics |
| [`docs/architecture.md`](docs/architecture.md) | Internal architecture, module map, runtime state |
| [`docs/testing.md`](docs/testing.md) | Test strategy, Xephyr harness, integration tests |
| [`docs/themes.md`](docs/themes.md) | Theme system, color palette, built-in presets |

---

## License

See [`LICENSE`](LICENSE).
