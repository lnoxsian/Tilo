# Integrated Application Launcher

Tilo ships with a built-in fuzzy application launcher. It runs as an overlay window managed directly by the WM — no `dmenu`, `rofi`, or external process needed.

---

## Opening the Launcher

| Method | Action |
|---|---|
| `Mod+d` | Open application launcher (App mode) |
| `SIGUSR1` | Toggle launcher (open if closed, close if open) |
| `Mod+Shift+q` | Open exit/power menu (Exit mode) |

```bash
# Toggle from a terminal or script
kill -USR1 "$(pidof tilo)"
```

---

## Window & Styles

Tilo supports two distinct launcher styles via the `launcher_position` configuration directive:

| Style | Directive | Description |
|---|---|---|
| **Spotlight** (Default) | `launcher_position spotlight` | Centered floating window (560×256 px) with query prompt and vertical suggestions list. |
| **Dmenu** | `launcher_position dmenu` | Single-line horizontal menu overlaying directly over the top status bar across the full width of the monitor (dwm dmenu style) with prompt pill and `<` / `>` pagination. |

Aliases: `launcher_style spotlight | dmenu`.

### Window Properties (Spotlight)

| Property | Value |
|---|---|
| Width | 560 pixels |
| Maximum visible results | 8 (vertical) |
| Position | Centered on the active monitor |
| Input ownership | Grabs keyboard while visible |

### Window Properties (Dmenu)

| Property | Value |
|---|---|
| Width | Full width of active monitor |
| Height | Matches statusbar height (default 24 pixels) |
| Position | Overlays top of active monitor (directly over status bar) |
| Layout | Seamless borderless bar with prompt `> `, search input, and candidate pills with `<` and `>` overflow indicators |
| Input ownership | Grabs keyboard while visible |

While visible, the launcher owns all keyboard input. Normal WM keybindings resume after the launcher closes.

---

## Modes

### App Mode

Searches `$PATH` for executable files and symlinks. Supports fuzzy matching on binary names and supports typed arguments (e.g., `firefox --private-window`).

### Exit Mode

Shows three fixed power actions:

| Label | Aliases | Action |
|---|---|---|
| `Logout` | `exit`, `quit` | Exit tilo |
| `Reboot` | `restart` | System reboot |
| `Poweroff` | `shutdown` | System shutdown |

---

## Navigation

| Key | Action |
|---|---|
| Type | Insert characters into the query |
| `Backspace` | Delete character before cursor |
| `Delete` | Delete character after cursor |
| `Home` / `Ctrl+A` | Move cursor to beginning |
| `End` / `Ctrl+E` | Move cursor to end |
| `Left` / `Ctrl+B` | Move cursor left (or explore previous binary like dmenu when query is empty) |
| `Right` / `Ctrl+F` | Move cursor right (or explore next binary like dmenu when query is empty) |
| `Up` / `Shift+Tab` / `Ctrl+P` | Select previous result (scrolls through all binaries) |
| `Down` / `Ctrl+N` | Select next result (scrolls through all binaries) |
| `PageUp` | Scroll up by 8 results |
| `PageDown` | Scroll down by 8 results |
| `Tab` | Auto-complete selected suggestion |
| `Return` / `Enter` | Launch selected result |
| `Escape` / `Ctrl+C` | Close launcher |

The query cursor tracks Unicode character positions (not byte offsets), so multi-byte characters work correctly.

---

## Discovery and Caching

### Discovery

On first open, the launcher scans every non-empty directory in `$PATH`:
- Collects regular files and symlinks with the executable bit set.
- Deduplicates by name and sorts alphabetically.
- Desktop entry files (`.desktop`) are not scanned for results but parsing helpers exist for library/test use.

### Incremental Cache

The launcher caches scan results per-directory, keyed by the directory's modification time and the previous `$PATH` value.

On subsequent opens:
- Only **changed** directories are rescanned.
- Cached results from unchanged directories are reused.
- The result lists are merged.

This makes reopening the launcher fast even on systems with large `$PATH` lists.

---

## Fuzzy Matching and Ranking

Ranking uses multiple scoring tiers (higher is better):

| Rank | Match Type |
|---|---|
| 1 | Case-insensitive exact match |
| 2 | Case-insensitive prefix match |
| 3 | Fuzzy subsequence with bonus scoring |
| 4 | Case-insensitive substring match |

**Fuzzy score bonuses:**
- First character match
- Word-boundary match (after `-`, `_`, space, `.`)
- Consecutive character sequence

**Argument handling:** Queries with arguments are matched using both the full query and just the first token. So `firefox --private-window` matches `firefox` in the suggestions.

At most 8 suggestions are visible in the overlay window at a time. The list dynamically scrolls so you can explore all binaries in `$PATH` using arrow keys (or PageUp/PageDown), exactly like dmenu.

---

## Launching

A selected App result is launched using the full process path with the typed arguments. The launcher does **not** invoke a shell.

A configured `spawn` keybinding **does** use `sh -c`, so shell syntax is available there.

> [!CAUTION]
> Exit mode actions (`Reboot`, `Poweroff`) affect the entire session. Only bind `exit_menu` in trusted configurations.

---

## Theming

The launcher inherits global palette colors and can be overridden with `launcher_` prefixed directives:

```
launcher_position dmenu      # or spotlight (default)
launcher_bg #1a1b26
launcher_fg #c0caf5
launcher_active_bg #7aa2f7   # highlighted selection background
launcher_active_fg #15161e   # text on selection background
launcher_urgent_bg #f7768e
launcher_inactive_fg #555566
launcher_border #7aa2f7      # overlay border color
launcher_font "monospace 11"
```

Font and color reloads update a visible launcher immediately on hot-reload.

---

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---|---|---|
| Empty result list | Empty `$PATH`, missing dirs | Check `echo $PATH`; verify directories exist |
| Missing expected binary | No executable bit | `chmod +x /path/to/binary` |
| Launcher doesn't open | `Mod+d` not bound | Check config keybinding or use `kill -USR1` |
| Font rendering broken | Missing font | Install the font or change `launcher_font` |
| Stale results | Directory not rescanned | Close and reopen; directory mtime must change |
