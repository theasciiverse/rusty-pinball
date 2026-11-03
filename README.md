# Rusty Pinball

A terminal pinball game written in Rust, rendered entirely in braille-pixel vector graphics inside your terminal.

## Inspiration

Rusty Pinball is inspired by [Vector Pinball](https://github.com/dozingcat/Vector-Pinball) by Brian Nenninger, an open-source Android pinball game with a clean physics-driven design and eight great tables. Tables 1-8's layouts and game logic in Rusty Pinball are derived from Vector Pinball's JSON table definitions.

## Features

- Eight playable tables (Classic, Round and Round, Ringmaster, Pool Party, and more)
- Real-time 2D physics via [Rapier](https://rapier.rs/)
- Braille vector graphics rendered with [drawille](https://github.com/asciimoo/drawille)
- Dot-matrix display (DMD) showing score, hi-score, and ball count
- The original's sound design: score dings, rollover notes, message and launch sounds, and music that comes in as you score
- Runs entirely in the terminal, no GPU required

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

Settings, High scores and Help are on the menu below the tables. In those, `←` / `→` change a setting or the table, and `Esc` goes back.

### In Game

| Key | Action |
|-----|--------|
| `Space` | Launch ball; hold it to pull the plunger back (on terminals that report key releases); start again after game over |
| `Z` / `←` / Left Shift | Left flipper |
| `X` / `→` / Right Shift | Right flipper |
| `P` | Pause |
| `H` | Help and this table's rules |
| `Esc` / `Q` | Return to main menu |

## Tips

- With Zoom on (the default) the table fills the terminal's width and the view follows the ball up and down; with Zoom off the whole table is shown at once, which suits a wide terminal.
- The flippers respond to letter keys (`Z`/`X`), arrow keys, and Left/Right Shift, so you can use whichever feels more natural. Shift on its own is only reported by terminals that support the Kitty keyboard protocol (kitty is one); elsewhere use `Z`/`X` or the arrows.
- Sound effects, music, floating score animations, ball trails, zoom, game speed and unlimited balls (the game never ends; no high score is recorded and the score shows a `*`) are in Settings. The choices are remembered, and so are the five best scores and the last score on every table.
- For the smoothest experience, run in a dedicated tmux pane or a full-screen terminal window.

## Status against the original

Tables 1 to 8 carry Vector Pinball's layouts and each table's own rules, ported from its delegate class: multiball and ball locks
(every table that has them), jackpots, the moving parts (orbiting bumpers, the spinning triangle, planets, the star map), the
mini table and lock lane on table 7, lights and bonuses. The engine rules follow the original too (multiplier halving, mercy ball
and multiball saver, stuck-ball bump, floating scores, ball trails, the original's sounds). What is still different, and why, is
listed in [docs/ORIGINAL-PARITY.md](docs/ORIGINAL-PARITY.md); every change is in [CHANGELOG.md](CHANGELOG.md).

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

## License

Rusty Pinball is released under the [GNU General Public License v3.0](LICENCE.txt).

Tables 1-8 in `assets/tables/` are derived from
[Vector Pinball](https://github.com/dozingcat/Vector-Pinball) by Brian Nenninger,
also licensed under GPL-3.0.
