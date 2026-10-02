# Rusty Pinball

A terminal pinball game written in Rust, rendered entirely in braille-pixel vector graphics inside your terminal.

## Inspiration

Rusty Pinball is inspired by [Vector Pinball](https://github.com/dozingcat/VectorPinball) by Brian Nenninger — an open-source Android pinball game with a clean physics-driven design and eight great tables. Tables 1–8's layouts and game logic in Rusty Pinball are derived from Vector Pinball's JSON table definitions; table 9 is an original layout authored in the same format.

## Features

- Nine playable tables (Classic, Round and Round, Ringmaster, Party Land, and more)
- Real-time 2D physics via [Rapier](https://rapier.rs/)
- Braille vector graphics rendered with [drawille](https://github.com/asciimoo/drawille)
- Dot-matrix display (DMD) showing score, hi-score, and ball count
- Optional sound effects
- Runs entirely in the terminal — no GPU required

## Requirements

- A terminal with at least **50 rows × 90 columns**
- A monospace font with Unicode box-drawing and braille support (e.g. JetBrains Mono, Fira Code, Hack, or any Nerd Font)
- Linux / macOS (audio requires ALSA on Linux)

## Build & Run

```
git clone <repo-url>
cd rusty-pinball
cargo build --release
./target/release/rusty-pinball
```

Or build and run in one step:

```
cargo run --release
```

Running inside **tmux** is recommended for the best display experience.

## Controls

### Menu

| Key | Action |
|-----|--------|
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Select |
| `Q` | Quit |

### In Game

| Key | Action |
|-----|--------|
| `Space` | Launch ball |
| `Z` / `←` / Left Shift | Left flipper |
| `X` / `→` / Right Shift | Right flipper |
| `Enter` | Continue after ball lost / restart after game over |
| `Esc` | Return to main menu |

## Tips

- The game field is taller than the visible viewport — the camera follows the ball automatically.
- The flippers respond to letter keys (`Z`/`X`), arrow keys, and Left/Right Shift, so you can use whichever feels more natural. Shift on its own is only reported by terminals that support the Kitty keyboard protocol (kitty is one); elsewhere use `Z`/`X` or the arrows.
- Sound can be toggled on or off from the main menu without leaving it.
- For the smoothest experience, run in a dedicated tmux pane or a full-screen terminal window.

## Status against the original

Tables 1 to 8 reproduce Vector Pinball's layouts and generic behavior, but not each table's own rules (multiball, ball locks,
jackpots, lights): those live in per-table scripts we have not ported yet. [docs/ORIGINAL-PARITY.md](docs/ORIGINAL-PARITY.md) is
the checklist of what is done and what is missing, table by table.

## Tables

| # | Name |
|---|------|
| 1 | Classic |
| 2 | Round and Round |
| 3 | Ringmaster |
| 4 | Compound Interest |
| 5 | Eye of Horus |
| 6 | Orbiters |
| 7 | Ad Astra |
| 8 | Pool Party |
| 9 | Party Land |

## License

Rusty Pinball is released under the [GNU General Public License v2.0](LICENSE).

Tables 1–8 in `assets/tables/` are derived from
[Vector Pinball](https://github.com/dozingcat/VectorPinball) by Brian Nenninger,
also licensed under GPL-2.0. Table 9 (Party Land) is an original layout built
in Vector Pinball's JSON format, recreating the shot layout of Digital
Illusions CE's *Pinball Fantasies* (1992) table of the same name.
