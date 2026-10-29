# Original parity: rusty-pinball vs Vector Pinball

Rusty Pinball ports the table layouts of [Vector Pinball](https://github.com/dozingcat/Vector-Pinball) (tables 1 to 8)
(an own table 9, Party Land, was removed on 2026-10-13). This document is the checklist of what the original does
that we do not yet do, so nothing gets forgotten. It exists because we kept finding unported behavior one piece at
a time (rotating bumpers on table 2, the table 5 triangle, the table 4 lights); an audit on 2026-10-02 went through
the whole original and this is the result.

Status as of the table-scripts commit on branch `rapier-0.36` (audit baseline: `da9595b`). Update this file when something lands.

- Detailed notes with original file and line references: [`parity/engine-and-app.md`](parity/engine-and-app.md),
  [`parity/tables-3-and-6.md`](parity/tables-3-and-6.md), [`parity/tables-7-and-8.md`](parity/tables-7-and-8.md).
  Tables 1, 2, 4 and 5 are covered here.
- Original source: `references/Vector-Pinball` in the Asciiverse checkout (`app/src/main/java/com/dozingcatsoftware/`;
  table logic is in `vectorpinball/fields/Field1Delegate.java` to `Field8Delegate.java`, JSON in `app/src/main/assets/tables/`).

## 1. The one structural fact

In the original, each table is a JSON layout **plus a Java "delegate"** that holds that table's rules: scoring,
multiball, ball locks, jackpots, moving parts, lights. The port now has the same split: the generic element behaviour lives in
`src/physics/mod.rs`, and each table's delegate is a Rust script (`src/physics/scripts.rs`, with the moving parts in
`src/physics/movers.rs` and the star map in `src/physics/stars.rs`), chosen by the JSON's `delegate` name. The physics world
holds a list of balls, so multiball, ball locks and per-ball features work.

**State on 2026-10-03:** tables 1 to 8 have every delegate rule ported, rounds 1 and 2 are complete (see the trackers below), and
the engine rules of the original's `Field` and `GameState` are ported. What is left is listed in section 6.

## 2. Capabilities (what the tables needed) and where they stand

These were the building blocks; the status column is current.

| ID | Capability | What it is in the original | Status |
|---|---|---|---|
| C1 | Table script layer | Delegate callbacks: `gameStarted`, `ballLost`, `tick`, `processCollision`, `allDropTargetsInGroupHit`, `allRolloversInGroupActivated`, `ballInSensorRange`, `spinnerActivated`, plus element lookup by id | DONE (`TableScript` trait: bumper hit, wall hit, drop target and bank events, spinner, rollover group completed, sensors, timers, `tick`, `game_started`, `ball_lost`) |
| C2 | Multi-ball | A list of balls; `createBall`, `launchBall`, `removeBallWithoutBallLoss`; "balls <= 1" means multiball ended; multiball saver | DONE |
| C3 | Scheduled actions | `scheduleAction(ms, fn)` on game time | DONE (`PhysicsWorld::schedule`, token callbacks) |
| C4 | Game messages | Timed text with a message sound | DONE |
| C5 | Element control | `setRetracted`, `setVisible`, `setNewColor`, rollover setters, drop target make-visible, spinner score | DONE |
| C6 | Sensors by id | `ballInSensorRange` with each ball's most recent sensor id | DONE |
| C7 | Per-ball state | Runtime ball colours, per-ball forces | DONE (colours, forces; the spin dot is cut out of the ball, see C14) |
| C8 | Full layer model | Layers as separate collision worlds | DONE (an element with no layer is layer 0; a ball only touches and triggers elements of its own layer; pieces of an idle layer are drawn faintly in `inactiveLayerColor`, lifted to a minimum brightness so a terminal shows them, and hidden when the table gives none; a layer fades between the two over 0.25 s of game time as a ball arrives or leaves, as in `FieldElement.tick`) |
| C9 | Shape renderer | `setShapes`: coloured circles and lines each frame (table 7 star map) | DONE |
| C10 | Multiplier and rollover rules | Fractional unbounded multiplier, carry-over rules, original rollover scoring | DONE (every table has a script) |
| C11 | Score animations | Floating score text at the ball, coloured by value | DONE |
| C12 | Ball and game flow | Extra ball, mercy ball, multiball saver, stuck-ball bump, unlimited balls, "Ball N" | DONE |
| C13 | Audio parity | Rollover, message and start sounds, non-repeating dings, score-driven music | DONE (unverified by ear) |
| C14 | App features | Ball trails, spin dot, zoom, game speed, top 5 high scores plus last score, help screen | DONE in 0.9.0. Top 5 scores plus the last per table, recorded when a game ends (or is left for the menu, as the original's End Game), with the original's after-game cycle on the DMD and a High scores screen. Settings: sound effects and music apart, game speed (0.85, 1, 1.2 on the time ratio), zoom. Zoom is adapted to a terminal: on (default) the table fills the width and the view follows the ball, as before; off shows the whole table, which suits wide terminals; the original's 1.5x ease-in is not done. The help screen gives the controls and each table's rules from the original's strings. The spin dot is cut out of the ball as a gap (a braille cell has one colour, so a grey dot would not show), drawn once the ball is at least 3 dots across; the ball is drawn at the table's `ballradius` |

### Which table needs what

`X` = needed for that table's rules; `(x)` = nice to have.

| Table | C1 | C2 | C3 | C4 | C5 | C6 | C7 | C8 | C9 | C10 | C11 | C12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 Classic | X | X | X | X | X | | | | | X | (x) | |
| 2 Round and Round | X | X | X | X | X | | | | | X | (x) | |
| 3 Ringmaster | X | X | X | X | X | | | | | X | X | |
| 4 Compound Interest | X | X | X | X | X | X | | | | X (fractional) | (x) | |
| 5 Eye of Horus | X | X | X | X | X | X | X | | | X | X | X (extra ball) |
| 6 Orbiters | X | X | X | X | X | X | X | (x) | | X | X | |
| 7 Ad Astra | X | X | X | X | X | X | | X | X | X | X | |
| 8 Pool Party | X | | X | X | X | X | | | | X | X | |

Reading it: **C1, C3, C4 and C5 are needed by every table, and C2 by seven of eight.** That is why they come first.

## 3. Plan

Agreed with David: **round 1 is every table's moving and centre-piece functionality; multiball is round 2, for all
tables together.**

### Round 1: centre pieces and moving parts

Shared building blocks first (each small, each tested), then the tables:

1. **C6 sensors by id** with each ball's most recent sensor id (ramp and orbit direction detection).
2. **C7 ball colors** (per ball, set at runtime, drawn) and a per-ball force hook (table 6's gravity wells).
3. **C5 element control**: runtime color override and show/hide for elements, movable rollover circles, set ignoreBall.
4. **C10 fractional multiplier** (table 4 adds 0.01 per bumper) and the original rollover scoring where a script owns it.
5. **C9 shape renderer** (table 7 only).

Then, roughly in this order: table 6 (planets, gravity, planet colors, ramps), table 4 (status grid, percent multiplier,
ramp walls), table 5 (colored-ball diagram and lines, ramp bonuses), table 3 (bumper bonus and indicator rings), table 8
(inserts, rack bonus, gates), table 7 (star map).

Round 1 leaves out only what needs more than one ball: ball locks, multiball launches, jackpots, the multiball saver.

### Round 1 status

| Table | Centre pieces and moving parts | Status |
|---|---|---|
| 2 | Orbiting bumpers | DONE |
| 5 | Rotating triangle, colored-ball diagram and lines, ramp colors and bonuses, extra-ball area | DONE (pyramid, multiball, triangle reversal too) |
| 6 | Orbiting planets, planet gravity, planet colors and activation, ramp detectors | DONE (multiball and jackpot too) |
| 4 | Status grid lit by bumpers, percent multiplier, ramp walls | DONE (locks, grid rows, multiball, jackpot too) |
| 3 | Bumper bonus, heat meter, indicator rings | DONE (multiball and its flashing rings: round 2) |
| 8 | Insert lights, rack bonus, gates, skill shot | DONE (whole table, with the billiard-ball decoration drawn) |
| 7 | Star-map field, constellations, loop shots, guide arrows | DONE (ball locks, multiball, jackpot and the layered mini-table too) |

### Round 2: multiball for all tables

The multi-ball core and the script layer exist (tables 1 and 2 already have multiball). Round 2 adds, per table: tables 3, 4,
5, 6, 7 multiball and locks, extra ball (table 5), the multiball saver, mercy ball, and the jackpots.

### Round 2 status

| Table | Multiball and what hangs off it | Status |
|---|---|---|
| 1 Classic | Extra-ball lights, ramp multiball | DONE |
| 2 Round and Round | Extra-ball lights, centre bumper multiball | DONE |
| 3 Ringmaster | Ready ring, ramp starts it, flashing rings, double ramps, reset | DONE |
| 6 Orbiters | All planets on, jackpot | DONE |
| 4 Compound Interest | Ball locks, status grid rows, multiball, jackpot | DONE |
| 5 Eye of Horus | Pyramid, multiball (four colored balls), jackpot, extra-ball use | DONE |
| 7 Ad Astra | Ball lock lane, layers, multiball, jackpot | DONE |

### Later

C11 score animations, C12 game flow extras (mercy ball, stuck-ball bump, unlimited balls), C13 audio parity, C14 app features.

## 4. Done so far (so it is not re-audited)

Landed on `rapier-0.36` (all with tests):

- Rapier 0.22 to 0.36; sim time at each table's `targetTimeRatio` (about 2.3x) in 8 substeps; original gravity, slingshot kicks and
  launch speeds restored.
- Ball savers and slingshots bounce then kick (the original applies the kick after the collision step).
- Contact model: restitution matches (ball and bumpers 0.6, others 0, max-combine). Friction does **not** match on the guides: the
  original's 0.245 made only about 42% of launches reach the top of the lane under Rapier, so lane and wall friction is 0.05 while
  flippers and bumpers keep the original's ball-contact friction.
- Flipper-cycled rollovers step once per press, in the original's direction; Left/Right Shift work the flippers.
- Table 2 orbiting bumpers; table 5 rotating triangle (`src/physics/movers.rs`).
- Table scripts, scheduled actions, game messages, and working multiball on tables 1 and 2 (`scripts.rs`).
- Round 1 (centre pieces) complete for tables 2 to 8, see the tracker above. Mercy ball (20 s) and multiball saver (30 s). Rollovers hit when the ball's centre is inside the radius, as in the original.
- Multi-ball core in the physics world (see C2): extra balls, silent drain while others remain, ball lock removal without loss, every element reacting to every ball, all balls drawn.
- Launch barrier by sensors (tables 1, 2, 3, 5, 6, 7); drop banks reset after their `reset` delay (none = stays down);
  ball savers re-armed by clearing the matching bank, **not** on every new ball.
- Found by this audit and fixed: table 3's four drop banks had no targets (`wallStart` form unparsed); a 3-ball game gave 4 balls;
  `ignoreBall` walls, arcs and paths were solid (table 5's twelve centre lines, table 7's loop guides, table 8 pieces).

## 5. Corrections to the audit agents' claims

The three detail reports were written by research agents. Spot-checked against the code:

- **Confirmed:** table 3 bumpers have JSON score 0 and the script supplies 500 plus a bonus; our rollover groups add the group score
  again on completion (the original pays per rollover only); a 3-ball game gave 4 balls; `ignoreBall` was unparsed on walls.
- **Wrong:** "22 drop-target groups on tables 3 to 8 load with zero targets". Only table 3's 4 banks did. Tables 4 to 8 in our repo
  have explicit `positions` and load fine. A test (`every_drop_bank_on_every_table_has_targets`) now guards this.
- **Not yet verified by hand:** the layer-model claims (C8), rollover trigger radius (`radius + ball radius` vs ball centre), rollover
  `reset` and `toggleOff` being ignored, `LeftGate`/`RightGate` on table 8 being permanent kick walls, the arc `segments` and bumper
  default differences. They are plausible and are listed as open items.

## 6. Other engine and app gaps worth remembering

Full list in [`parity/engine-and-app.md`](parity/engine-and-app.md) (older, partly out of date: this section wins).

**Done since the audit:** the sounds (a ding on every scoring contact, the three note rollover chord, message, launch and startup sounds, no drain sound, music that comes in after 10 and 20 scoring hits; assets are the original's); game messages (timed, one at a time, on the DMD); extra balls ("Shoot Again"); "Ball N" and "Game Over"
messages; mercy ball and multiball saver; the score multiplier is fractional and unbounded, halves (rounds down, never below 1)
when a ball is lost, and keeps an increase made in the last 7.5 s of game time, as in the original; the stuck-ball "Bump!"
(10 s of game time, no flipper held); bumper defaults (score 0, kick 0, as the original); additive launch jitter (independent per
component, none for a zero delta); layers as separate worlds; table scripts for every table; floating score animations
(the multiplied points rise 3 units over 2 s, coloured by value, fading in the second half); ball trails (20 frames, dimmed and
shrinking towards the tail); unlimited balls and menu switches for score animations and trails; table 8's billiard ball decoration (drawn dimmed, not solid).

**Still open, roughly in order of how much a player would notice:**

- A table without a script would still get the invented drop-bank clear bonus (2000) and rollover completion bonus; no table is without one since table 9 was removed.
- A plunger, which the original does not have: where the terminal reports key releases, holding Space pulls it back and the release launches at 0.85 to 1.15 of the table's launch speed; a tap launches at the table's own speed, as everywhere else.
- **Documented exception:** tables 6 and 7 use `ballradius` 0.46 (original 0.5 and 0.49), set in commit `81c7c1a` so the ball does not stick at the
  launch ramps. Checked on 2026-10-03 by 40 random launches per table with the original radii: only 5 of 40 balls (table 6) and 0 of 40
  (table 7) entered a ramp or loop, against 17 and 3 with 0.46, so the exception stays. Every other table JSON is identical to the original.
- Ball density not rescaled for radius.

## 7. Per-table notes

Tables 3 and 6, and 7 and 8, are in the detail files. Tables 1, 2, 4 and 5 follow.

### Table 1: Classic (`Field1Delegate`)

| Behavior | Status |
|---|---|
| Any rollover group completes: group clears and the score multiplier goes up one (message "Nx Multiplier", 1500 ms) | DONE (script) |
| `ExtraBallRollovers` (ignoreBall lights) advance one light each time any drop bank clears | DONE (script), with "Shoot Ramp for Multiball" |
| `RampRollovers` completes while all extra-ball lights are lit: multiball | DONE (script) |
| Multiball: message, both ball savers restored, balls launched at +1000 ms and +3500 ms while fewer than 3 are in play | DONE. Not done: the multiball saver (a replacement ball for an early drain within 30 s), the "Ball Saved" message |
| `DropTargetLeftSave` / `RightSave` re-arm the savers with a message | DONE |
| Launch barrier closes behind the ball, opens for a new ball | DONE |
| Ball savers, slingshots, bumpers, flippers, spinner-free layout | DONE |

### Table 2: Round and Round (`Field2Delegate`)

| Behavior | Status |
|---|---|
| Two groups of orbiting bumpers (4 around the centre bumper at 0.75 rad/s, 2 around (9.35, 13) at -0.5 rad/s) | DONE (`movers.rs`) |
| `tick` and `isFieldActive`: motion continues with no ball | DONE |
| Drop banks `DropTargetLeft` / `Right` re-arm their savers; message | DONE |
| Banks advance the `ExtraBallRollovers` columns (left bank column 0, top-left column 1, right bank column 2, in steps of 3) | DONE (script), with "Shoot Red Bumper" |
| Hit the red `CenterBumper1` with all `ExtraBallRollovers` lit: multiball (savers restored, balls at +1000 and +3500 ms, lights cleared) | DONE (script) |
| Rollover group completes: clears and multiplier +1 | DONE (script) |
| Launch barrier | DONE |
| `LeftTubeBarrier` and `LeftTubeSensor` | Nothing to do: the delegate never reacts to that sensor, so the barrier stays disabled in the original too |
| `kill` wall at the bottom | Handled by our drain line |
| Drop banks reset after 2 s | DONE |

### Table 4: Compound Interest (`Field4Delegate`)

The centre 6x3 grid is the multiball status display, not something the ball lights.

| Behavior | Status |
|---|---|
| Six color columns `Rollovers.<Color>` x 3 rows, all `ignoreBall` in normal play. Each colored bumper (`Bumper.Red` and so on) lights the next light in its column (row = balls locked) | DONE (row = balls locked) |
| Whole row lit across all six colors: the matching lock rollover on the right (`LockedBallRollover1..3`) activates; message "Ball Lock Ready" / "Multiball Ready" | MISSING |
| Roll into a lock rollover: the ball is removed without loss, a kicker un-retracts, "Ball N Locked" | MISSING (C2: `removeBallWithoutBallLoss`) |
| Third lock: multiball (savers restored; `ignoreBall` turns off so the ball lights the grid; balls launched at +1000 and +3500 ms) | MISSING |
| Light all 18 in multiball: jackpot `jackpot * 1000` points (25 000 first, +5 000 each time) and multiplier up by `jackpot` percent; grid clears | MISSING |
| Every bumper hit adds `bumperMultiplierIncrease` percent to the score multiplier (starts 1; +1 per top drop bank clear; +2 per `FlipperRollovers` completion; reset each ball) | DONE (fractional multiplier shown as `x1.37`) |
| Ramp logic: `TopRampSensor`, `RightRampSensor`, `AfterRampKickerSensor`, `TopRampBlocker`, `RightRampKicker` toggle walls so a ball takes the ramp | DONE |
| Launch barrier (retract sensor only; also closed by `TopRampSensor`) | DONE |
| Save banks and top bank | DONE |

### Table 5: Eye of Horus (`Field5Delegate`)

| Behavior | Status |
|---|---|
| Rotating triangle (3 walls, side 2, centre (10, 18.86), 1 rad/s; -2 in multiball) | DONE (kick follows the walls; -2 in multiball) |
| `TriangleCenter` rollover: when multiball is READY, hitting a triangle wall retracts it, and the centre rollover then starts multiball | MISSING |
| Four ball colors (blue, red, yellow, green). Ramps assign the ball's color (sensor pairs `RampSensor_*` with direction check); a multiball launches balls in unused colors | DONE (multiball launches balls in unused colors) |
| 12 centre lights (4 colors x 3) that only a ball of that color can light; 12 connecting lines (`CenterLine_*`) drawn when both end lights are lit | DONE |
| All 12 lines drawn: multiball becomes READY ("shoot the pyramid"); during multiball it pays a jackpot of 50 000 x count and resets the diagram | MISSING |
| Ramp score 5000, +10% per color from its drop bank ("`DropTargets_<Color>Ramp`"), doubled when more than one ball is in play | DONE |
| Extra ball: `ExtraBallRollover` and four `ExtraBallBarrier_*` walls; award an extra ball | DONE (the original also shows "Shoot Again" when the extra ball is used) |
| Multiball sequence: triangle walls restored, speed -2, savers restored, balls at +1000, +4000, +7000, +10000 ms, ACTIVE after the last; ends when one ball remains | MISSING |
| `FlipperRollovers` completion: multiplier +1 | DONE |
| Launch barrier, ball savers, save banks | DONE |
| Ball radius 0.4, launch speed 20.99 with tiny jitter | DATA ok |

## 8. How to keep this current

- When a capability or table feature lands, change its row here and in the matrix, and note the commit.
- When you touch an element property, check `parity/engine-and-app.md` section "Element JSON properties" for what the original reads.
- New findings go in section 5 (claims) or 6 (gaps), with how they were verified.
