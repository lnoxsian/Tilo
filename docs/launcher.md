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

## Window

| Property | Value |
|---|---|
| Width | 560 pixels |
| Maximum visible results | 8 |
| Position | Centered on the active monitor |
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
| `Home` | Move cursor to beginning |
| `End` | Move cursor to end |
| `Left` | Move cursor left |
| `Right` | Move cursor right |
| `Up` / `Shift+Tab` | Select previous result |
| `Down` / `Tab` | Select next result |
| `Return` / `Enter` | Launch selected result |
| `Escape` | Close launcher |

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

At most 8 suggestions are rendered. The selection index is clamped to the filtered result count after each query change.

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
