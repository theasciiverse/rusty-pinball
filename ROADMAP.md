# Roadmap

Rusty Pinball is at 0.8.0 (2026-10-28): eight tables from Vector Pinball,
each with its full rules. 1.0.0 is when the polish below is in; new tables
come after 1.0. The detail behind each item is in
[`docs/ORIGINAL-PARITY.md`](docs/ORIGINAL-PARITY.md).

## 0.8.0: built

- [x] Eight tables, every one with its table script: centre pieces, moving
  parts, multiball, ball locks and jackpots
- [x] Ball savers (mercy ball, multiball saver), extra balls, the stuck-ball
  bump, unlimited balls
- [x] Game messages on the DMD, the original's sounds and score-driven music
- [x] Score animations and ball trails, both switchable

## 0.9.0: the original's app features

- [ ] The top 5 high scores and the last score per table (one high score
  per table now)
- [ ] Settings as the original has them: music and effects apart, game
  speed, zoom
- [ ] A help screen

## 1.0.0: the last details

- [ ] Inactive layers fade over 250 ms, as in the original (they switch at once)
- [ ] The variable plunger
- [ ] The ball's secondary colour spin dot, and its size from `ballradius`
- [ ] Sounds checked by ear against the original
- [ ] Every table played through and tuned

## After 1.0: more tables

- [ ] Table 9, Party Land, redone from scratch
- [ ] More tables of our own as they come along
