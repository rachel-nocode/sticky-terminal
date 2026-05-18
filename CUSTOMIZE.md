# Make it your own

Sticky Terminal is small on purpose. Here are the easy places to tweak it —
each one is a few lines, and you'll see the change next `cargo run`.

## Paper colours

All seven papers live in one `match` in [`src/sticky.rs`](src/sticky.rs) (around
line 49). Each row is `(body, header, edge highlight, dog-ear)` as RGB:

```rust
Self::Lemon => ((236, 228, 168), (226, 215, 138), (245, 239, 196), (223, 214, 150)),
```

- **Change a colour** — edit the RGB numbers.
- **Add a colour** — add a variant to the `PaperColor` enum, add it to `ALL`,
  and add a row to the `match`. The terminal palette is derived automatically.
- **Default paper** — `PaperColor::default()` (around line 29), currently `Lemon`.

## Window size

In [`src/main.rs`](src/main.rs) (lines 17–18):

```rust
.with_inner_size([340.0, 380.0])      // starting size
.with_min_inner_size([200.0, 64.0])   // smallest you can drag it
```

## Fonts

Drop a `.ttf` into [`assets/fonts/`](assets/fonts/) and point at it in
`install_fonts` in [`src/ui/mod.rs`](src/ui/mod.rs) (around line 59). Monospace
goes to the terminal, proportional goes to the UI.

## Keyboard shortcuts

The shortcut handlers are in `update()` in [`src/ui/mod.rs`](src/ui/mod.rs)
(around line 251). Copy a block and swap the `egui::Key` to add your own.

## The look of the note

In [`src/sticky.rs`](src/sticky.rs):

- `HEADER`, `RADIUS`, `DOGEAR`, `SHADOW_MARGIN` (top of the file) — proportions.
- The `egui::epaint::Shadow { offset, blur, spread, color }` (around line 157)
  — the drop shadow. Bump `blur` for a softer look.

## Where everything lives

| File | What it does |
|------|--------------|
| `src/main.rs` | Window flags, app entry point |
| `src/sticky.rs` | The paper chrome, dropdown menu, paper colours |
| `src/ui/mod.rs` | App state, the update loop, menu actions |
| `src/ui/pane.rs` | The terminal grid — cells, cursor, scrollback |
| `src/terminal/mod.rs` | The PTY shell + vt100 parser |
| `src/config.rs` | Saves your chosen paper colour |
| `src/theme.rs` | The terminal colour-palette struct |

Have fun. Break things. That's the point.
