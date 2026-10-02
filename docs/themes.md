# Themes and Colors

Tilo uses a unified palette system where global colors cascade to the status bar and launcher, with optional per-component overrides.

---

## Color Format

Colors are specified as CSS-style hex values with an optional alpha channel:

| Format | Example | Description |
|---|---|---|
| `#RGB` | `#abc` | 3-digit shorthand |
| `#RRGGBB` | `#7aa2f7` | 6-digit (standard) |
| `#RRGGBBAA` | `#7aa2f7ff` | 8-digit with alpha |

> [!NOTE]
> X11 stores colors as 32-bit ARGB integers. Alpha is accepted in config but X11 window borders and bar rendering do not composite transparency.

---

## Global Palette

The global palette sets the base color scheme for all UI components. All values can be overridden per-component (see below).

```
# Background colors
bg #1a1b26          # main background
active_bg #7aa2f7   # selected / highlighted background
urgent_bg #f7768e   # urgent / alert background

# Foreground (text) colors
fg #c0caf5          # main foreground
active_fg #15161e   # selected foreground (text on active_bg)
urgent_fg #15161e   # urgent foreground (text on urgent_bg)
inactive_fg #555566 # dimmed / inactive text

# Window border colors
border_focused #ffffff   # focused window border
border_normal #444444    # unfocused window border
border_urgent #ff0000    # urgent window border
```

---

## Component Overrides

### Status Bar

Override any global value for the bar specifically by prefixing with `bar_`:

```
bar_bg #16161e
bar_fg #c0caf5
bar_active_bg #7aa2f7
bar_active_fg #15161e
bar_urgent_bg #f7768e
bar_urgent_fg #ffffff
bar_inactive_fg #3b3b52
bar_border #7aa2f7
bar_font "JetBrainsMono Nerd Font 10"
```

### Launcher

Override for the application launcher by prefixing with `launcher_`:

```
launcher_bg #1a1b26
launcher_fg #c0caf5
launcher_active_bg #7aa2f7
launcher_active_fg #15161e
launcher_urgent_bg #f7768e
launcher_urgent_fg #ffffff
launcher_inactive_fg #555566
launcher_border #7aa2f7
launcher_font "monospace 11"
```

Omitting any override causes it to inherit from the global palette.

---

## Built-in Theme Presets

The Xephyr harness ships six ready-made theme presets. Apply them with:

```bash
./scripts/run-xephyr.sh --theme <preset>
```

Or switch live in the interactive prompt:
```
tilo> theme nord
```

### Tokyo Night (default)

```
bg      #1a1b26    fg      #c0caf5
active  #7aa2f7    urgent  #f7768e
inactive #555566
border_focused #7aa2f7    border_normal #24283b
```

### Catppuccin Mocha

```
bg      #1e1e2e    fg      #cdd6f4
active  #cba6f7    urgent  #f38ba8
inactive #585b70
border_focused #cba6f7    border_normal #313244
```

### Nord

```
bg      #2e3440    fg      #d8dee9
active  #88c0d0    urgent  #bf616a
inactive #4c566a
border_focused #88c0d0    border_normal #3b4252
```

### Gruvbox Dark

```
bg      #282828    fg      #ebdbb2
active  #d79921    urgent  #cc241d
inactive #665c54
border_focused #d79921    border_normal #3c3836
```

### Dracula

```
bg      #282a36    fg      #f8f8f2
active  #bd93f9    urgent  #ff5555
inactive #6272a4
border_focused #bd93f9    border_normal #44475a
```

### Solarized Dark

```
bg      #002b36    fg      #839496
active  #268bd2    urgent  #dc322f
inactive #586e75
border_focused #268bd2    border_normal #073642
```

---

## Applying a Theme

### In Config File

Add the color directives directly to `~/.config/tilo/config`:

```
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
```

Then hot-reload with `Mod+Shift+r` or:

```bash
kill -HUP "$(pidof tilo)"
```

### Via Xephyr Harness

The harness generates a session config file from the selected preset and sends `SIGHUP` to apply it live.

---

## Hot Reload Behavior

When tilo reloads the configuration:

1. Theme colors are parsed from the new config.
2. All window borders are redrawn with the new `border_focused` / `border_normal` / `border_urgent` colors.
3. The status bar renderer is recreated if the effective font changed.
4. Bar and launcher color schemes update immediately.
5. If the config is invalid, the previous theme is kept (no partial application).

---

## Font Selection

Font rendering uses a 100% pure Rust dual-engine pipeline:
- TrueType/OpenType anti-aliased font rendering via `fontdue`.
- Core X11 font rendering for explicit `core:...` specifications or automatic fallback.

| Specification | Renderer Used |
|---|---|
| `monospace` | Pure Rust TrueType/OpenType (`fontdue`, default size 10pt) |
| `"JetBrainsMono 11"` | Pure Rust TrueType/OpenType (`fontdue`) |
| `pango:Noto Sans 10` | Legacy prefix alias (maps to `fontdue`) |
| `fixed` | X11 core font |
| `6x13`, `9x15` | X11 core bitmap font |
| `core:fixed` | Explicit X11 core font |
| XLFD (`-*-fixed-*-*-*-*-13-*`) | X11 core font |

When a requested font is unavailable or if no font files exist on the host, tilo automatically falls back to standard X11 core fonts (`fixed`, `6x13`).

---

## Directive Aliases

Several directives accept multiple names for compatibility:

| Primary Directive | Aliases |
|---|---|
| `bg` | `background`, `color_bg` |
| `fg` | `foreground`, `color_fg`, `font_color` |
| `active_bg` | `accent_color`, `color_active_bg` |
| `active_fg` | `color_active_fg` |
| `urgent_bg` | `color_urgent_bg` |
| `urgent_fg` | `color_urgent_fg` |
| `inactive_fg` | `color_inactive_fg` |
| `border_normal` | `normal_color`, `unfocused_color`, `border_color`, `color_border` |
| `border_focused` | `focused_color` |
| `border_urgent` | `urgent_color` |
| `bar_bg` | `statusbar_bg` |
| `bar_fg` | `statusbar_fg`, `bar_font_color`, `statusbar_font_color` |
| `bar_font` | `statusbar_font` |
| `launcher_active_bg` | `launcher_selected_bg` |
| `launcher_active_fg` | `launcher_selected_fg` |
| `launcher_border` | `launcher_border_color` |
