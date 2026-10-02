# Configuration

---

## Config File Location

| Condition | Path |
|---|---|
| `$XDG_CONFIG_HOME` is set | `$XDG_CONFIG_HOME/tilo/config` |
| `$XDG_CONFIG_HOME` is unset | `~/.config/tilo/config` |

If the file is absent on startup, tilo creates the parent directory and writes a built-in template, then parses it. The template contains example values and comments explaining every directive.

---

## Parser

The parser is line-oriented and whitespace-delimited:

- `#` starts a comment and is ignored until end of line, **except** when a `#RGB`, `#RRGGBB`, or `#RRGGBBAA` hex color token follows a directive on the same line.
- Quoted strings (`"like this"`) preserve internal spaces and allow multi-word font names.
- Numbers are parsed as `u32` integers or `f32` floats.
- Boolean values accept `true`/`false`, `1`/`0`, and `yes`/`no`.
- `$mod` in keybind definitions expands to the configured `mod_key`.

Parse errors report a line number and message. Invalid configurations during hot-reload keep the previous configuration intact.

---

## Defaults

| Setting | Default |
|---|---|
| Modifier | `Mod4` (Super / Win key) |
| Font | `monospace` |
| Terminal | `foot` |
| Layout | `bsp` |
| Border width | `2` px |
| Inner gap | `8` px |
| Outer gap | `8` px |
| Master ratio | `0.60` |
| Workspace click | enabled |
| Workspace wheel | enabled |
| Tab index display | disabled |
| Tab max width | `160` px |
| Tab click | enabled |
| Tab wheel | enabled |

---

## General Directives

### Modifier Key

```
mod_key Mod4
```

Sets the primary modifier. Also accepted as `mod` or `modifier`.

Available modifiers: `Shift`, `Control` / `Ctrl`, `Alt` / `Mod1`, `Mod2`, `Mod3`, `Mod4` / `Super` / `Win`, `Mod5`, `Lock`.

### Font

```
font "monospace 10"
font "JetBrainsMono Nerd Font 11"
font 6x13
```

The font applies to both the status bar and launcher unless overridden by `bar_font` or `launcher_font`. See [themes.md](themes.md) for font format details.

### Terminal

```
terminal foot
terminal alacritty
terminal "xterm -fa monospace -fs 12"
```

Command used by `bind $mod+Return spawn $terminal` in the default bindings.

### Default Layout

```
layout bsp
layout master
layout grid
layout tabbed
layout monocle
```

See [layouts.md](layouts.md) for all layout names. This sets the starting layout for every workspace.

### Border Width

```
border_width 2
```

Width of the X11 border around managed windows, in pixels.

### Gaps

```
inner_gap 8    # space between neighboring tiled windows
outer_gap 8    # space between windows and monitor edges
```

Both values are in pixels. Set to `0` to disable gaps.

### Master Ratio

```
master_ratio 0.60
```

Proportion of the usable area given to the master pane in master-oriented layouts. Must be between `0.0` and `1.0`. Default is `0.60`.

---

## Theme and Color Directives

See [themes.md](themes.md) for the full color system reference. Quick reference:

```
# Global palette
bg #1a1b26
fg #c0caf5
active_bg #7aa2f7
active_fg #15161e
urgent_bg #f7768e
urgent_fg #15161e
inactive_fg #555566

# Window borders
border_focused #ffffff
border_normal #444444
border_urgent #ff0000

# Status bar overrides (prefix: bar_)
bar_bg #16161e
bar_fg #c0caf5
bar_font "monospace 10"

# Launcher overrides (prefix: launcher_)
launcher_bg #1a1b26
launcher_fg #c0caf5
launcher_font "monospace 11"
```

---

## Keybindings

### Syntax

```
bind <Modifier+...+Key> <Action> [Arguments...]
```

`$mod` expands to the configured `mod_key`. The key name is case-insensitive and follows X11 keysym names (e.g. `Return`, `space`, `h`, `F1`, `Tab`, `grave`, `bracketleft`).

If **any** `bind` directive is present, it completely replaces the built-in default binding list. Start from the generated template to retain standard bindings.

### Available Actions

| Action | Arguments | Description |
|---|---|---|
| `spawn` | `<command>` | Launch via `sh -c "<command>"` |
| `close` | — | Close focused window (ICCCM WM_DELETE_WINDOW) |
| `focus` | `left`/`right`/`up`/`down` | Focus window in direction |
| `move` | `left`/`right`/`up`/`down` | Move focused window in direction |
| `workspace` | `1`–`9` | Switch to workspace |
| `move_to_workspace` | `1`–`9` | Send focused window to workspace |
| `toggle_floating` | — | Toggle floating state |
| `toggle_fullscreen` | — | Toggle fullscreen state |
| `launcher` | — | Open application launcher |
| `toggle_scratchpad` | — | Show/hide the scratchpad window |
| `set_scratchpad` | — | Designate focused window as scratchpad |
| `focus_monitor` | `left`/`right` | Focus adjacent monitor |
| `move_to_monitor` | `left`/`right` | Send window to adjacent monitor |
| `resize` | `left`/`right`/`up`/`down` | Resize floating window |
| `split` | `vertical`/`horizontal` | Preselect BSP split direction |
| `cycle_layout` | — | Cycle to next layout |
| `rotate_master` | — | Rotate master orientation |
| `layout` | `<layout_name>` | Switch to specific layout |
| `focus_tab` | `next`/`prev` | Focus next/previous tab |
| `close_tab` | — | Close focused tab |
| `reload` | — | Hot-reload configuration |
| `exit` | — | Quit tilo |
| `exit_menu` | — | Open exit/power menu in launcher |
| `quit` | — | Alias for `exit` |

### Default Bindings

```
bind $mod+Return       spawn foot
bind $mod+d            launcher
bind $mod+Shift+q      exit_menu
bind $mod+q            close
bind $mod+Shift+r      reload

# Focus
bind $mod+h            focus left
bind $mod+j            focus down
bind $mod+k            focus up
bind $mod+l            focus right
bind $mod+Left         focus left
bind $mod+Down         focus down
bind $mod+Up           focus up
bind $mod+Right        focus right

# Move
bind $mod+Shift+h      move left
bind $mod+Shift+j      move down
bind $mod+Shift+k      move up
bind $mod+Shift+l      move right

# Workspaces
bind $mod+1            workspace 1
bind $mod+2            workspace 2
bind $mod+3            workspace 3
bind $mod+4            workspace 4
bind $mod+5            workspace 5
bind $mod+6            workspace 6
bind $mod+7            workspace 7
bind $mod+8            workspace 8
bind $mod+9            workspace 9

bind $mod+Shift+1      move_to_workspace 1
bind $mod+Shift+2      move_to_workspace 2
bind $mod+Shift+3      move_to_workspace 3
bind $mod+Shift+4      move_to_workspace 4
bind $mod+Shift+5      move_to_workspace 5
bind $mod+Shift+6      move_to_workspace 6
bind $mod+Shift+7      move_to_workspace 7
bind $mod+Shift+8      move_to_workspace 8
bind $mod+Shift+9      move_to_workspace 9

# Layouts
bind $mod+Space        cycle_layout
bind $mod+r            rotate_master
bind $mod+Shift+Space  toggle_floating
bind $mod+f            toggle_fullscreen

# BSP
bind $mod+v            split vertical
bind $mod+b            split horizontal
bind $mod+Ctrl+Left    resize left
bind $mod+Ctrl+Right   resize right
bind $mod+Ctrl+Up      resize up
bind $mod+Ctrl+Down    resize down

# Scratchpad
bind $mod+grave        toggle_scratchpad
bind $mod+Shift+grave  set_scratchpad

# Tabs
bind $mod+Tab          focus_tab next
bind $mod+Shift+Tab    focus_tab prev

# Monitors
bind $mod+comma        focus_monitor left
bind $mod+period       focus_monitor right
bind $mod+Shift+comma  move_to_monitor left
bind $mod+Shift+period move_to_monitor right
```

---

## Status Bar and Tab Directives

```
workspace_mouse true           # click workspace tags to switch
workspace_wheel_switch true    # scroll over workspace tags to cycle

tab_show_index false           # prepend 1-based index to tab labels
tab_max_width 160              # maximum tab pill width in pixels
tab_mouse true                 # click tab pills to focus; middle-click to close
tab_wheel_switch true          # scroll over tabs to cycle
```

---

## Autostart Directives

```
autostart nm-applet
autostart dunst
autostart feh --bg-fill "/path/to/wallpaper.jpg"
autostart "picom --backend glx"
```

Commands are parsed into a program and argument vector. They are launched detached after initialization and never awaited. Failures are logged as warnings and do not stop remaining autostart commands.

> [!IMPORTANT]
> Autostart uses a direct program/argument vector — shell syntax is **not** available. Use `spawn sh -c "..."` in a keybinding instead if shell features are needed.

---

## Hot Reload

Reload the configuration without restarting:

```
bind $mod+Shift+r  reload
```

Or from a terminal:
```bash
kill -HUP "$(pidof tilo)"
```

On reload, tilo:

1. Parses the configuration file from disk.
2. If parsing fails, keeps the previous configuration (no partial application).
3. If parsing succeeds, applies:
   - Border colors and widths
   - Gap sizes and master ratio
   - Active layout (current and all workspaces)
   - Key grabs (releases old grabs, installs new ones)
   - Bar and launcher theme and font
   - Retiles all workspaces

Running clients are **not** closed or moved during reload.

---

## Window Rules

The rule engine matches windows before their first map and can assign workspace, floating, or fullscreen state.

Match criteria:
- `WM_CLASS` instance name (exact)
- `WM_CLASS` class name (exact)
- Window title (`WM_NAME` / `_NET_WM_NAME`, exact)

Rules are evaluated in order; later rules override earlier ones for the same field.

> [!NOTE]
> The rule directive is not yet exposed in the user config file. Rule behavior is currently set internally for testing. A documented `rule` directive is planned for a future release.

---

## Example Configuration

```
# ~/.config/tilo/config

mod_key Mod4
font "JetBrainsMono 10"
terminal alacritty
layout bsp
border_width 2
inner_gap 6
outer_gap 6
master_ratio 0.55

# Nord theme
bg #2e3440
fg #d8dee9
active_bg #88c0d0
active_fg #2e3440
urgent_bg #bf616a
urgent_fg #2e3440
inactive_fg #4c566a
border_focused #88c0d0
border_normal #3b4252
border_urgent #bf616a

# Statusbar
workspace_mouse true
workspace_wheel_switch true
tab_show_index true
tab_max_width 180
tab_mouse true

# Autostart
autostart nm-applet
autostart feh --bg-fill ~/wallpaper.png

# Keybindings
bind $mod+Return  spawn alacritty
bind $mod+d       launcher
bind $mod+q       close
bind $mod+h       focus left
bind $mod+j       focus down
bind $mod+k       focus up
bind $mod+l       focus right
bind $mod+Shift+h move left
bind $mod+Shift+j move down
bind $mod+Shift+k move up
bind $mod+Shift+l move right
bind $mod+1       workspace 1
bind $mod+2       workspace 2
bind $mod+3       workspace 3
bind $mod+4       workspace 4
bind $mod+5       workspace 5
bind $mod+6       workspace 6
bind $mod+7       workspace 7
bind $mod+8       workspace 8
bind $mod+9       workspace 9
bind $mod+Shift+1 move_to_workspace 1
bind $mod+Shift+2 move_to_workspace 2
bind $mod+Shift+3 move_to_workspace 3
bind $mod+Shift+4 move_to_workspace 4
bind $mod+Shift+5 move_to_workspace 5
bind $mod+Shift+6 move_to_workspace 6
bind $mod+Shift+7 move_to_workspace 7
bind $mod+Shift+8 move_to_workspace 8
bind $mod+Shift+9 move_to_workspace 9
bind $mod+Space   cycle_layout
bind $mod+r       rotate_master
bind $mod+Shift+Space toggle_floating
bind $mod+f       toggle_fullscreen
bind $mod+v       split vertical
bind $mod+b       split horizontal
bind $mod+Ctrl+Left  resize left
bind $mod+Ctrl+Right resize right
bind $mod+Ctrl+Up    resize up
bind $mod+Ctrl+Down  resize down
bind $mod+grave   toggle_scratchpad
bind $mod+Shift+grave set_scratchpad
bind $mod+Tab     focus_tab next
bind $mod+Shift+Tab focus_tab prev
bind $mod+comma   focus_monitor left
bind $mod+period  focus_monitor right
bind $mod+Shift+comma  move_to_monitor left
bind $mod+Shift+period move_to_monitor right
bind $mod+Shift+r reload
bind $mod+Shift+q exit_menu
```

---

## Safety

> [!CAUTION]
> Always test configuration changes in Xephyr before reloading a production display. An invalid `bind` can leave you without a way to close windows or exit the WM.

Use the Xephyr harness with your config:
```bash
./scripts/run-xephyr.sh --config ~/.config/tilo/config
```
