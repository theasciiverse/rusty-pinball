# Roadmap

Rusty Pinball is at 0.9.0 (2026-10-29): eight tables from Vector Pinball,
each with its full rules, and the original's app features. 1.0.0 is when
the last checks below are done; new tables come after 1.0. The detail behind each item is in
[`docs/ORIGINAL-PARITY.md`](docs/ORIGINAL-PARITY.md).

## 0.8.0: built

- [x] Eight tables, every one with its table script: centre pieces, moving
  parts, multiball, ball locks and jackpots
- [x] Ball savers (mercy ball, multiball saver), extra balls, the stuck-ball
  bump, unlimited balls
- [x] Game messages on the DMD, the original's sounds and score-driven music
- [x] Score animations and ball trails, both switchable

## 0.9.0: built

- [x] The top 5 high scores and the last score per table, a High scores
  screen, and the original's after-game cycle on the DMD
- [x] Settings as the original has them: music and effects apart, game
  speed, zoom (on follows the ball, off shows the whole table)
- [x] A help screen with the controls and each table's rules, also in game (H)
- [x] Inactive layers fade over a quarter second of game time, as in the original
- [x] The plunger: hold Space to pull it back, where the terminal reports key releases
- [x] The ball's spin dot, and its size from `ballradius`

## 1.0.0: the last details

- [ ] Sounds checked by ear against the original
- [ ] Every table played through and tuned

## After 1.0: more tables

- [ ] Table 9, Party Land, redone from scratch
- [ ] More tables of our own as they come along
