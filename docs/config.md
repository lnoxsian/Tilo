# Tilo Window Manager Configuration

`tilo` is configured through a simple, line-oriented, whitespace-delimited configuration file located at:

```text
~/.config/tilo/config
```
(or `$XDG_CONFIG_HOME/tilo/config` if `XDG_CONFIG_HOME` is set).

---

## Automatic Initial Configuration Generation

When `tilo` is launched and no configuration file is found at `~/.config/tilo/config` (or `$XDG_CONFIG_HOME/tilo/config`):
- `tilo` automatically creates the destination directory structure (e.g. `~/.config/tilo/`).
- `tilo` writes a complete, fully-commented default configuration file populated with default modifier keys, theme palette colors, layout options, status bar behaviors, and the full standard keybinding map.
- `tilo` loads and applies this freshly generated configuration immediately upon startup.

---

## Hot Configuration Reload

`tilo` supports live, hot reloading of its configuration without restarting the window manager or closing running client windows.

### Methods to Reload Configuration:

1. **Default Keybinding:**
   Press **`Mod4 + Shift + r`** (or `Super + Shift + r`).
   This triggers an immediate reload of the configuration file from disk.

2. **Custom Config Keybinding:**
   If defining custom `bind` directives in `~/.config/tilo/config`, you can map any key combination to the `reload` action:
   ```text
   bind Mod4+Shift+r reload
   # or using the $mod variable:
   bind $mod+Shift+r reload
   ```

3. **Unix SIGHUP Signal:**
   Send a `SIGHUP` signal to the running `tilo` process:
   ```bash
   pkill -HUP tilo
   # or
   kill -HUP <tilo-pid>
   ```

4. **In Xephyr Test Harness (`./scripts/run-xephyr.sh`):**
   Type `reload` in the interactive harness prompt:
   ```text
   tilo-xephyr [1366x768 :1 bsp]> reload
   ```

### Safe Atomic Reloading:
When reloading:
- Borders, gaps, layouts, master ratios, fonts, themes, status bar elements, and keybindings update dynamically on existing windows and monitors.
- If the updated configuration has a syntax error or invalid value, the reload fails safely and logs a warning to stderr, keeping the previous valid configuration active and intact.

---

## Configuration Directives

### General Directives

| Directive | Description | Default | Example |
| :--- | :--- | :--- | :--- |
| `mod_key` (or `mod`) | Primary modifier key for window manager bindings | `Mod4` | `mod_key Mod4` |
| `terminal` | Default terminal emulator command | `alacritty` | `terminal alacritty` |
| `font` | Default font for UI elements (status bar, launcher) | `pango:monospace 10` | `font pango:monospace 10` |
| `layout` | Initial default layout (`bsp`, `master`, `grid`, `tabbed`, `monocle`) | `bsp` | `layout bsp` |
| `border_width` | Window border width in pixels | `2` | `border_width 2` |
| `inner_gap` | Gap between adjacent windows in pixels | `8` | `inner_gap 8` |
| `outer_gap` | Gap between windows and screen edges | `8` | `outer_gap 8` |
| `master_ratio` | Master window proportion (0.0 to 1.0) | `0.60` | `master_ratio 0.55` |

---

## Font Configuration & Dual-Engine Architecture

`tilo` features a high-performance **dual-engine** font and text rendering pipeline:

1. **Anti-Aliased Pango/Cairo Engine:**
   - Full support for modern TrueType / OpenType fonts, anti-aliased subpixel rendering, and Unicode / Nerd Font icons.
   - Configured using i3-compatible syntax (`pango:...` or standard font family names).
   - Bounded metrics caching and lazy surface management (the launcher's surface is freed on close to minimize resident memory).

2. **Zero-Allocation Core X11 Font Engine:**
   - Direct server-side bitmap rasterization using X11 core font protocols (`poly_text8`, `image_text8`, and `poly_fill_rectangle`).
   - Completely eliminates client-side image surfaces and font cache mapping, maintaining a resident memory footprint of **< 2.5 MB**.
   - Activated automatically by specifying core font names (`fixed`, `6x13`, `9x15`) or using the `core:` prefix (e.g., `font core:fixed`).

### Supported Font Syntaxes:

| Syntax Type | Engine | Memory Profile | Example |
| :--- | :--- | :--- | :--- |
| Explicit Pango | Pango/Cairo | Full anti-aliasing | `font pango:monospace 10` |
| Font Family & Size | Pango/Cairo | Full anti-aliasing | `font JetBrainsMono Nerd Font 10` |
| Core X11 Font | Core Engine | Ultra-low RAM (< 2.5 MB) | `font fixed` or `font 6x13` |
| Explicit Core Prefix | Core Engine | Ultra-low RAM (< 2.5 MB) | `font core:fixed` |

### Building for Ultra-Minimal Memory Footprint:
For embedded devices or minimalist setups desiring the absolute lowest memory footprint without linking C GUI libraries (`libcairo`, `libpango`, `libharfbuzz`, `libfontconfig`):
```bash
cargo build --release --no-default-features
```
In this mode, `tilo` links only standard system libraries (`libc` and X11/xcb), consuming only **~2.5 MB VmRSS** and **~212 kB private heap/stack memory**.

---

## Unified Theme & Colors

`tilo` features a centralized theme palette that applies consistently across window borders, status bar items, and the application launcher.

### Global Theme Palette:
```text
bg #1a1b26           # Window manager / bar / launcher background
fg #c0caf5           # Primary text foreground
active_bg #7aa2f7    # Active workspace, selected launcher item, layout badge
active_fg #15161e    # Text on active backgrounds
urgent_bg #f7768e    # Urgent workspace or window indicator
urgent_fg #15161e    # Text on urgent backgrounds
inactive_fg #555566  # Dimmed text (empty workspaces, inactive items)
```

### Window Border Colors:
```text
border_focused #ffffff
border_normal #444444
border_urgent #ff0000
```

### Component-Specific Overrides (Optional):
You can override colors specifically for the status bar or the launcher without affecting other UI elements:
```text
# Status bar overrides:
bar_bg #1a1b26
bar_fg #c0caf5
bar_active_bg #7aa2f7
bar_active_fg #15161e
bar_border #7aa2f7

# Launcher overrides:
launcher_bg #1a1b26
launcher_fg #c0caf5
launcher_selected_bg #7aa2f7
launcher_selected_fg #15161e
launcher_border #7aa2f7
```

### Keybinding Syntax (`bind`)

Format:
```text
bind <KeyCombo> <Action> [Args...]
```

Variables:
- `$mod` is automatically replaced with the configured `mod_key`.

#### Example Keybindings:
```text
# Window Management & Control
bind $mod+Shift+r           reload
bind $mod+q                 close
bind $mod+Shift+q           exit
bind $mod+Return            spawn alacritty
bind $mod+d                 launcher

# Layout & Tiling
bind $mod+space             cycle_layout
bind $mod+Shift+space       toggle_floating
bind $mod+f                 toggle_fullscreen
bind $mod+r                 rotate_master

# Spatial Navigation (Vim keys)
bind $mod+h                 focus left
bind $mod+j                 focus down
bind $mod+k                 focus up
bind $mod+l                 focus right

# Spatial Navigation (Arrow keys)
bind $mod+Left              focus left
bind $mod+Down              focus down
bind $mod+Up                focus up
bind $mod+Right             focus right

# Window Moving
bind $mod+Shift+h           move left
bind $mod+Shift+j           move down
bind $mod+Shift+k           move up
bind $mod+Shift+l           move right

# Workspaces (1-9)
bind $mod+1                 workspace 1
bind $mod+Shift+1           move_to_workspace 1
bind $mod+2                 workspace 2
bind $mod+Shift+2           move_to_workspace 2

# Scratchpad
bind $mod+grave             toggle_scratchpad
bind $mod+Shift+grave       set_scratchpad
```

### Autostart Directives

Commands that execute when `tilo` initializes:
```text
autostart picom --daemon
autostart feh --bg-scale ~/wallpaper.jpg
```
