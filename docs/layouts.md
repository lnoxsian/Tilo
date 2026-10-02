# Layout Engine

Tilo includes a built-in layout engine with multiple tiling strategies. All layouts are pure functions that receive window IDs and a usable screen rectangle, then return per-window rectangles.

---

## Geometry Contract

Before calculation, the bar's reserved strip is subtracted from the usable screen area. Layouts then receive:

| Input | Type | Description |
|---|---|---|
| `clients` | Ordered `Vec<Window>` | Tiled client IDs in order |
| `usable_rect` | `Rect { x, y, w, h }` | Screen area after bar reservation |
| `config` | `LayoutConfig` | Gap, ratio, and border settings |

| `LayoutConfig` field | Description |
|---|---|
| `master_ratio` | Master pane share (0.0–1.0) |
| `inner_gap` | Pixels between neighboring windows |
| `outer_gap` | Pixels between windows and monitor edges |
| `border_width` | X11 border subtracted from content geometry |

### Edge Cases

- Empty client list → no rectangles returned
- Single client → receives the full usable area after outer gaps and border adjustment
- Saturating arithmetic prevents underflow on very small displays
- Pixel remainders from integer division go to the last row or column

---

## Layout Types

| Name | Config aliases | Symbol | Algorithm |
|---|---|---|---|
| `bsp` | `bsp`, `binary` | `[B]` | Recursive binary space partition tree |
| `master` | `master`, `master-left`, `tall` | `[]=` | Master left, stack right |
| `master-right` | `master-right` | `=[]` | Master right, stack left |
| `master-top` | `master-top` | `TT=` | Master top, stack below |
| `master-bottom` | `master-bottom` | `=TT` | Master bottom, stack above |
| `center-master` | `center-master`, `centered` | `\|M\|` | Master centered, stacks left and right |
| `grid` | `grid` | `[G]` | Near-square rows and columns |
| `vertical` | `vertical`, `columns` | `\|\|\|` | Equal side-by-side columns |
| `horizontal` | `horizontal`, `rows` | `===` | Equal horizontal rows |
| `monocle` | `monocle`, `max` | `[M]` | All clients share full area; focused is raised |
| `tabbed` | `tabbed`, `tabs` | `[T]` | Full area; inactive tabs are unmapped |
| `stacking` | `stacking` | `[S]` | Monocle geometry, traditional stacking order |

---

## Layout Details

### BSP — Binary Space Partitioning

Each workspace owns a persistent BSP tree. Windows are leaves; splits are interior nodes.

**Insertion:** A new window replaces a target leaf with a split node containing the old and new clients. The split can be vertical or horizontal.

**Preselection:**
- `Mod+v` — preselect vertical split for the next window
- `Mod+b` — preselect horizontal split for the next window

**Resize:** `Mod+Ctrl+Left/Right/Up/Down` adjusts the split ratio of the focused pane by approximately `0.05`. Ratios are clamped to `0.05..=0.95`.

**Move:** Swaps two leaf nodes in the tree.

**Removal:** Collapses the parent node into its sibling.

```
Example BSP tree for 3 windows:
┌──────────┬──────┐
│          │  B   │
│    A     ├──────┤
│          │  C   │
└──────────┴──────┘

Tree:
  split(vertical)
  ├── A (leaf)
  └── split(horizontal)
      ├── B (leaf)
      └── C (leaf)
```

### Master-Stack Layouts

The first tiled client is the master. Remaining clients share the stack area.

```
master-left (default)     master-right
┌───────────┬─────┐       ┌─────┬───────────┐
│           │  B  │       │  B  │           │
│     A     ├─────┤       ├─────┤     A     │
│ (master)  │  C  │       │  C  │ (master)  │
└───────────┴─────┘       └─────┴───────────┘

master-top                master-bottom
┌─────────────────┐       ┌──────┬──────┐
│       A         │       │  B   │  C   │
├──────┬──────────┤       ├──────┴──────┤
│  B   │    C     │       │      A      │
└──────┴──────────┘       └─────────────┘

center-master
┌─────┬──────────┬─────┐
│     │          │     │
│  B  │    A     │  C  │
│     │ (master) │     │
└─────┴──────────┴─────┘
```

### Grid

Divides the usable area into a near-square grid. The number of columns is `ceil(sqrt(n))`, with rows computed accordingly. Remaining cells in the last row expand to fill width.

```
4 windows (2×2):        6 windows (3×2):
┌──────┬──────┐         ┌────┬────┬────┐
│  A   │  B   │         │ A  │ B  │ C  │
├──────┼──────┤         ├────┼────┼────┤
│  C   │  D   │         │ D  │ E  │ F  │
└──────┴──────┘         └────┴────┴────┘
```

### Vertical / Columns

Equal side-by-side columns:

```
┌────┬────┬────┐
│ A  │ B  │ C  │
│    │    │    │
└────┴────┴────┘
```

### Horizontal / Rows

Equal horizontal rows:

```
┌────────────────┐
│       A        │
├────────────────┤
│       B        │
├────────────────┤
│       C        │
└────────────────┘
```

### Monocle

All clients occupy the same full usable rectangle. The focused client is raised to the top of the stacking order.

```
┌─────────────────┐
│                 │
│   A / B / C     │  ← all share this rect
│  (focus raised) │
└─────────────────┘
```

### Tabbed

Same geometry as monocle, but only the focused tiled client is mapped. Other tabs remain tracked and are unmapped/iconified. The status bar renders a tab strip with clickable pill buttons.

Leaving tabbed layout re-maps all tracked tiled clients.

### Stacking

Uses monocle geometry but applies normal stacking order. Primarily for compatibility.

---

## Cycle and Rotation

### Cycle Layout (`Mod+Space`)

```
bsp → master-left → master-right → master-top → master-bottom
    → center-master → vertical → horizontal → monocle → tabbed
    → grid → bsp (wraps)
```

### Rotate Master (`Mod+r`)

Cycles through master orientation variants:

```
master-left → master-right → master-top → master-bottom
     ↑                                          ↓
     └──────────── center-master ←──────────────┘
```

`rotate_master` leaves `grid`, `monocle`, `tabbed`, `stacking`, and `bsp` unchanged. On `vertical` / `horizontal`, it toggles between them.

---

## Client Layers

Windows are organized into three stacking layers (bottom to top):

1. **Tiled** — calculated by the active layout
2. **Floating** — retain `FloatingGeometry`; stacked above tiled
3. **Fullscreen** — fill the active monitor; raised above floating

A client is never in more than one visible layer. Moving between layers triggers a retile.

---

## Tabbed Bar Integration

In tabbed layout, the status bar renders tab pills:

- Each tab shows the window title (truncated to `tab_max_width` pixels)
- Optionally prefixed with a 1-based index (`tab_show_index true`)
- Urgent windows get urgent styling
- Left-click: focus that tab
- Middle-click: close that tab
- Scroll wheel: cycle tabs
- `Mod+Tab` / `Mod+Shift+Tab`: next/previous tab

---

## Adding a Layout

To extend tilo with a new layout:

1. Add the `LayoutType` variant in `src/layout/mod.rs`
2. Add parser aliases in the `FromStr` implementation
3. Add a display name and symbol string
4. Write a pure rectangle function in a new `src/layout/mylayout.rs`
5. Route through `calculate_layout` in `src/layout/mod.rs`
6. Add WM visibility behavior in `src/wm/workspace.rs` if needed (e.g., tabbed unmapping)
7. Add unit tests for zero, one, small, and many clients
8. Add it to the `cycle_layout` sequence
