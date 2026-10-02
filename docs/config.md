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
| `terminal` | Default terminal emulator command | `foot` | `terminal alacritty` |
| `font` | Default font for UI elements | `monospace` | `font JetBrainsMono Nerd Font 10` |
| `layout` | Initial default layout (`bsp`, `master`, `grid`, `tabbed`, `monocle`) | `bsp` | `layout bsp` |
| `border_width` | Window border width in pixels | `2` | `border_width 2` |
| `inner_gap` | Gap between adjacent windows in pixels | `8` | `inner_gap 8` |
| `outer_gap` | Gap between windows and screen edges | `8` | `outer_gap 8` |
| `master_ratio` | Master window proportion (0.0 to 1.0) | `0.60` | `master_ratio 0.55` |

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
