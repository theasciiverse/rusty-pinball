# Vector Pinball original vs rusty-pinball: engine and app parity audit

> **Provenance and status.** Detailed notes from the original-parity audit of 2026-10-02, written by a research
> agent that read the original Java source and grepped our Rust source. Line numbers are into the code as it was
> when the audit ran (commit `fa27092` on branch `rapier-0.36`) and will drift. The master document,
> [../ORIGINAL-PARITY.md](../ORIGINAL-PARITY.md), is the authoritative summary; it records which claims below were
> verified, corrected or since fixed. Where this file disagrees with the master document, the master wins.
>
> **Corrections and fixes since this was written**
> - Drop targets from `wallStart`: verified for **table 3 only** (4 banks), now FIXED (commit `b6f264b`). The "22 groups on tables 3 to 8" figure below was NOT reproduced: tables 4 to 8 in our repo carry explicit `positions` and load correctly; a test now guards that every bank on every table has targets.
> - Four launches with `numballs` 3: verified, FIXED (commit `de4c8bc`).
> - `ignoreBall` on walls, arcs and paths: verified (table 5 centre lines, table 7 guides, table 8 pieces), FIXED (commit `da9595b`); they are now bodiless and hidden until table scripts can show them.
> - Launch barrier, drop bank reset delay, ball saver re-arm by bank, rotating bumpers (table 2) and rotating triangle (table 5): implemented after the audit started; rows below that say MISSING for these are out of date.
> - Friction: now split by surface (see the master document); rows about the contact model are out of date.
>

Original root: `asciiverse/references/Vector-Pinball/app/src/main/java/com/dozingcatsoftware/` (abbreviated `VP/` = `vectorpinball/`, `B/` = `bouncy/`).
Rust root: `asciiverse/games/rusty-pinball/src` (abbreviated `RS/`).
Statuses were determined by reading and grepping the Rust source. Three claims were also confirmed by running throwaway tests in a COPY of the repo under the scratchpad (repos untouched): the extra-ball count (4 launches with numballs=3), and the zero drop targets on tables 3 to 8 wallStart groups. Everything else is read/grep only.

Context that explains many gaps: the Rust port has NO delegate mechanism. There is no `Field.Delegate` dispatch (gameStarted, ballLost, gameEnded, tick, processCollision, flippersActivated, allDropTargetsInGroupHit, allRolloversInGroupActivated, spinnerActivated, ballInSensorRange, isFieldActive). Table logic is hard coded in `RS/physics/mod.rs` by element id (`saver_for_drop_bank`, `LAUNCH_BARRIER_ID`, `"TriangleWall*"`) and `RS/physics/movers.rs` (Field2Delegate bumpers, Field5Delegate triangle). The physics world also holds exactly ONE ball (`ball_handle: Option<RigidBodyHandle>`), so every multi-ball feature is structurally absent.

## Game rules and state

| # | Feature | What the original does | Original ref | Status in Rust |
|---|---|---|---|---|
| 1 | Ball count | `numballs` default 3; `GameState.startNewGame` sets ballNumber=1; `doNextBall` increments ballNumber while `ballNumber < totalBalls` else game ends. So 3 balls = 3 launches. | VP/model/FieldLayout.java:76, GameState.java:24,34 | PARTIAL, BUG. `game.rs` `GameState::sync` decrements `balls_remaining` only if it is >0, else GameOver, so with numballs=3 the player gets 4 launches (verified by a test run: LAUNCHES=4). DMD icons (`ui/dmd.rs ball_indicator`) hard code MAX=3. No ballNumber concept ("Ball N" message). |
| 2 | Unlimited balls | Menu checkbox; `startGameWithUnlimitedBalls`; ballNumber keeps incrementing, never game over; score shown with "*" suffix; ScoreView draws a vanishing series of 5 balls (radius x0.8 each); high scores NOT recorded. | Field.java:201-220, GameState.java:34-48, B/ScoreView.java:151, BouncyActivity.java:655 | MISSING |
| 3 | Extra balls | `addExtraBall` ++extraBalls; on ball loss `doNextBall` consumes one instead of advancing ballNumber; message "Shoot Again" (1500 ms, delayed 1500 ms). Used by Field5Delegate. | Field.java:849,465-491; GameState.java:41-43 | MISSING |
| 4 | Score multiplier model | `double scoreMultiplier` (fractional; table 4 adds 0.01 per bumper), unbounded; `addScore` = `Math.round(points * mult)`; `incrementScoreMultiplier` +1; `incrementAndDisplayScoreMultiplier(ms)` also shows "%dx Multiplier" message. Displayed as "Nx" or "%.2fx" when >1. | GameState.java:57,494; Field.java:901-906; B/ScoreView.java:170-176 | PARTIAL. `game.rs` has an integer `multiplier` clamped to 1..5 (`MAX_MULTIPLIER`), integer score math, only raised by the Asciiverse-only `multiplierStep` rollover extension. No fractional multiplier, no cap-free growth, no setScoreMultiplier hook, no message. DMD shows `xN` integer only. |
| 5 | Multiplier behavior on ball loss | `MultiplierBehavior` enum: REMOVE (reset to 1), HOLD, ROUND_HALF_DOWN (default: `max(1, floor(m/2))`). Applied in `doNextBall`. | GameState.java:7-12,34-41 | MISSING. Rust resets to 1 every ball (`game.rs sync`), i.e. always REMOVE. |
| 6 | Preserve multiplier increase | If the ball is lost within `preserveMultiplierIncreaseDuration` (default 7500 ms game time) of a multiplier increase, the increase is undone, the halving applied, then re-added for the next ball. Property `preserveMultiplierIncreaseDuration`. | Field.java:451-477; FieldLayout.java:38,75 | MISSING (not parsed, not implemented) |
| 7 | Scoring | Every contact with an element having `score != 0` adds `score * multiplier`, plays a ding, and spawns a floating score animation at the ball position. Rollovers score on state change; spinner scores per cycle. Contact is recorded at `endContact`. | Field.java:675-701, 718-735 | PARTIAL. Scoring is by per-step proximity checks in `physics/mod.rs step` (bumpers, kick walls, drop targets), pushed to `score_events`. Differences: bumper default score is 100 and kick default 8.0 when absent (original 0/0); plain Walls/Arcs/Paths with `score` never score (no tables currently use this); extra `DROP_CLEAR_BONUS` 2000 for any cleared drop bank is invented (original gives bonuses only via delegates); rollover group completion adds the group `score` again as a bonus (invented). |
| 8 | Score animations | Floating digits (7-seg shapes) at ball pos, 2 s lifetime, rise 3.0 units, scale 1.0 to 0.5, fade alpha in last 50 %, digit w 0.45 h 0.75 spacing 0.675, x clamped to [0.1, width-0.1], layer 1000. Colour by awarded value: >=10000 0xFF0080, >2000 0xFF4000, >500 0xFFFF00, else 0x80FF80. Spinner limits animations to one per 250 ms (silent adds in between). Preference `showScoreAnimations` default true. | model/ScoreAnimation.java:22-110, Field.java:298-325, SpinnerElement.java:31,119-127, DigitShapes.java | MISSING |
| 9 | Game messages | `showGameMessage(text, ms, playSound)`: one message at a time, real-time duration, replaces the score line in ScoreView, plays message sound by default. Engine messages: ball saved (1500 mercy / 1000 multiball), Shoot Again (1500 after 1500 delay), "Ball N" (1500 after 1500 delay), Game Over (2500), "Bump!" (1000), multiplier message. Table delegates add many more (see strings.xml). | Field.java:750-770,404,419,482-490,647,839; model/GameMessage.java | MISSING. DMD only shows a phase string (`PRESS SPACE`, `BALL LOST  SPACE`, `GAME OVER  SPACE`, `SUPER Ns`). No timed messages. |
| 10 | Stuck ball detection | Each tick records ball positions; if all balls have speed^2 <= 0.01 and moved <= 0.1 for `STUCK_BALL_NANOS` = 10 s game time (about 5 s real) with no flipper engaged, show "Bump!" and apply impulse (+-1, 1.5) to every ball (random x sign), reset timer. | Field.java:70,793-847 | MISSING (grep for stuck/bump finds nothing) |
| 11 | Ball lost: kill wall | `kill` Wall: on contact `field.removeBall(ball)`. Tables place `kill` at y=0.1 spanning x 0.1..18.7 (not the shooter lane). | WallElement.java:133-148 | PARTIAL. Kill walls are skipped (`add_wall` returns None for kill) and replaced by a global `drain_y = 1.0` test with a shooter-lane x-range exemption (`is_ball_drained`). Roughly equivalent for single ball. |
| 12 | removeBall / doBallLost flow | After last ball removed: mercy ball check, else `doBallLost`: records `lostBallWallTimeMillis`, resets usedMercyBall, extra ball / multiplier handling, schedules next-ball message, or `endGame`, then `delegate.ballLost`. | Field.java:425-496 | PARTIAL. `GameState::sync` + `app.rs tick` handle drain: phase BallLost/GameOver, `park_ball()`. No delegate hook, no scheduling. |
| 13 | Mercy ball (ball saver, early drain) | If the only ball is lost within `mercyBallDuration` (default 20000 ms game time) of launch and not yet used this ball, the ball is relaunched, message "Ball Saved" (1500 ms, message sound). `usedMercyBall` flag. | Field.java:72,395-406,428; FieldLayout.java:33,73 | MISSING (not parsed, not implemented) |
| 14 | Multiball saver | If a ball is lost while others remain and within `multiballSaverDuration` (default 30000 ms) of multiball start, a replacement is launched ("Ball Saved" 1000 ms), at most one per 1000 ns window. | Field.java:408-422,436-438; FieldLayout.java:35,74 | MISSING (no multiball at all) |
| 15 | Multiball support | `balls` list; `createBall`, `launchBall`, `removeBallWithoutBallLoss`, `updateBallLaunchTimes` (separate `ballStart` and `multiballStart` clocks); every element tick iterates all balls. Used by tables 3,4,5,6,7,8 (launchBall x23, createBall in table 7, removeBallWithoutBallLoss in 4,5,7). | Field.java:327-377,446-449 | MISSING (single `ball_handle`) |
| 16 | Launch | Ball created at `launchPosition` with `launchVelocity` plus `rand()*launchVelocityRandomDelta` added to each component (additive, [0,delta]); plays ball sound; `launchBallIfNeeded`: if no ball in play OR a ball sits in `launchDeadZone` it is removed and a new one launched. Touch ignored for 750 ms after a ball loss (`END_BALL_DELAY_MS`). | Field.java:353-393, FieldLayout.java:176-189, B/FieldViewManager.java:144-150 | PARTIAL. `PhysicsWorld::launch_ball` launches from the parked ball. Random delta: y is `vy + frac*dy` (ok) but x is `vx + (frac-0.5)*max(dx,0.1)` (centred, min 0.1; original additive [0,dx]). Dead zone handled differently (`is_ball_in_dead_zone` then `park_ball`). No 750 ms post-loss input lockout. `input.rs plunger_charge()` exists but is unused (no variable plunger). |
| 17 | Game start/end | `startGame` resets timers, `gameStarted` delegate; `endGame` plays start sound, destroys balls, `setGameInProgress(false)`, message "Game Over" 2500 ms, `gameEnded` delegate. | Field.java:201-220,640-649 | PARTIAL (phase GameOver; no sounds/delegates/message; new game via Space in `app.rs handle_play_key`). |
| 18 | Time ratio and game speed | `FieldDriver` ticks `field.tick(frameNanos * getTargetTimeRatio(), 4 iterations)`, `getTargetTimeRatio = gameSpeedMultiplier * layout.targetTimeRatio` (no default: asFloat gives 0 if missing; every table sets 2.25/2.3). Game speed pref: slow 0.85, normal 1.0, fast 1.2. Adaptive frame rate manager {144..30 fps}; inactive field sleeps 250 ms/frame. | Field.java:949-955, model/FieldDriver.java:36-60, B/BouncyActivity.java:571-575 | PARTIAL. `PhysicsWorld::step_frame` uses `frame_secs * time_ratio / 8` substeps at a fixed 33 ms frame (`app.rs FRAME_MILLIS`); `TableDef::time_ratio()` default 2.3. No game-speed preference. Frame-rate manager NOT APPLICABLE (terminal fixed tick). |
| 19 | Scheduled actions | `scheduleAction(ms, Runnable)` priority queue on game time, processed each tick. Used by rollover reset, drop target reset, tables 4,5,6,7, ball-lost messages. | Field.java:129-138,269-280,342-347 | MISSING as a general facility. Only hard-coded timers exist: drop-bank `reset_timer`, tick-counted flash timers. Rollover `reset` is never scheduled (see JSON section). |
| 20 | Game state flags | `GameState`: gameInProgress, paused, ballNumber, extraBalls, totalBalls, unlimitedBalls, score, multiplier, behavior. `isGameRunning = inProgress && !paused`. | GameState.java:14-30 | PARTIAL (`Phase` enum, `paused` lives in `app.rs`). |
| 21 | Per-ball properties | `Ball`: layer (own Box2D world), primary colour (layoutball `ballcolor`, default red 255,0,0), secondary colour (`secondaryBallColor`, default 176,176,176) drawn as a small circle at radius/2 rotated by body angle (shows spin) with radius/4, mostRecentSensorId, setPrimaryColor/SecondaryColor (tables 5,6 recolour the ball), density scaled `0.25/r^2` so mass is constant for any radius, bullet body. | model/Ball.java:20-60 (create/density 47-55), :90 (drawTrails), :161 (setPrimaryColor), :181 (moveToLayer); FieldLayout.java:65-66,78-80 | PARTIAL. `ball_layer` + `ball_groups_for` exist; `ballcolor` parsed as `[u8;3]` (no RGBA) and rendered; `secondaryBallColor`, spin dot, setPrimaryColor, mostRecentSensorId missing; ball density is fixed 1.0 (mass varies with `ballradius`; tables 6 and 7 use 0.46); ball drawn at fixed `field_width*0.025` radius in `renderer/mod.rs`, ignoring actual `ballradius`. |
| 22 | Ball trails | Per-ball ring of 20 previous positions stamped each tick; if `showBallTrails` (default true), up to 25 interpolated circles every 12 ms of game time, radius shrinking linearly, colour alpha `0.3*sizeFraction`. | model/Ball.java:61 (tick), :90 (drawTrails); Field.java:89 | MISSING |
| 23 | Layers (worlds) | One Box2D `World` per integer `layer`; element lives in its layer's world; ball moves between worlds via `moveToLayer`; ball only collides with elements of ITS layer (layer 0 elements do not touch a ball on layer 1). `hasBallAtLayer` drives inactive-layer colour fade (250 ms). Rollovers require `ball.layer == element.layer`. | model/WorldLayers.java, Ball.java (moveToLayer ~181), FieldElement.java:127-141, RolloverGroupElement.java:101 | PARTIAL, DIVERGENT. Rust uses rapier collision-group bits. `layer: None` is "universal, collides on all layers" (`definition.rs` doc comment, `add_wall`), which differs from the original where an element with no layer is layer 0 only. Only Wall/Arc/Path layers are applied. Bumpers, flippers, drop targets, rollovers, sensors, spinners have no layer filtering (their `layer` is not even parsed). Negative layers mapped to inert sentinel 31 (ok). Table 7 has layer-1 flippers (x2), bumpers (x3), drop targets (x2), layer-4 rollovers: all behave as layer 0 in Rust. |
| 24 | Draw order | Sort by layer, then shapes(1) < elements(2) < balls(3) < bumpers(4). | Field.java:551-591 | PARTIAL (`renderer/mod.rs` fixed paint order: walls, kick walls, drop targets, bumpers, rollovers, spinners, flippers, ball; no layer sort). |
| 25 | Shapes | `Field.setShapes(List<Shape>)`: extra Line/Circle (solid or outline) shapes with optional inactive-layer colour, used by table 7 projection map. | Field.java:310-314, model/Shape.java | MISSING |
| 26 | Collision/flipper engine rules | Flipper engagement: `setFlippersEngaged` plays flipper sound only if a flipper newly engaged, calls every element's `flippersActivated(activated list)` and the delegate. Left/right/all variants. | Field.java:600-634 | PARTIAL. Rust applies engagement every tick from `InputState`; rollover cycling uses the `*_just_pressed` edge (`step`), flipper sound plays on any key Press while Playing (`app.rs`). No generic element hook. |
| 27 | Box2D contact model | Ball: circle density 1/(adjusted), friction 0.3, restitution 0.6; walls: thin 0.1 thick box, restitution 0 unless wall `restitution`; bumpers static circles friction 0.3 restitution 0.6; flippers dynamic density 5, bullet, revolute joint, maxMotorTorque 1000, thickness 0.24, top-of-swing speed limit 0.5 within 0.05 rad. 10/10 iterations, 4 steps per frame. | elements/Box2DFactory.java:17-90, FlipperElement.java:78-122,163-173, Ball.java createBody | IMPLEMENTED approximately in `physics/mod.rs` (`BALL_RESTITUTION`, `FLIPPER_*` constants, `add_flipper`); friction intentionally lower on guides (documented in comments). Bumpers and kick walls use proximity detection plus manual impulses rather than contact callbacks. |
| 28 | Pause / resume | `GameState.paused`; pause on focus loss, Back button, P key, touching the score view; menu offers Resume / End Game. | B/BouncyActivity.java:382-426,459-485 | PARTIAL: `P` pause overlay in `app.rs`/`ui/mod.rs`. No end-game-from-pause (Esc leaves to menu and abandons the game), no resume-from-menu, no auto-pause on focus loss (N/A in terminal). |
| 29 | Debug message | `getDebugMessage` (disabled in production) | Field.java:863 | NOT APPLICABLE |

## Element JSON properties

Legend: P = parsed by `RS/table/definition.rs`; U = used by physics/rendering (grepped). "None in tables" means no original table JSON uses it, so the practical impact is nil today.

### Table level (FieldLayout)

| Property | Default / meaning | Orig ref | P | U | Notes |
|---|---|---|---|---|---|
| `width` | 20 | FieldLayout.java:69 | yes | yes | |
| `height` | 30 | :70 | yes | yes | |
| `gravity` | 4.0 | :71 | yes | yes | `gravity_magnitude()` |
| `targetTimeRatio` | no default (0) | :72 | yes | yes | Rust default 2.3 |
| `numballs` | 3 | :76 | yes | PARTIAL | off by one (4 launches), see rules #1 |
| `ballradius` | 0.5 | :77 | yes | PARTIAL | physics yes; render ignores it; density not rescaled |
| `ballcolor` | RGB(255,0,0) | :78 | yes (RGB only) | yes | alpha/4 element form would fail to parse |
| `secondaryBallColor` | RGB(176,176,176) | :79 | NO | no | spin dot missing |
| `launchPosition` | list | :81 | yes | yes | |
| `launchVelocity` | list | :82 | yes | yes | |
| `launchVelocityRandomDelta` | [dx,dy] additive | :83, :180-187 | yes | PARTIAL | x handled differently |
| `launchDeadZone` | rect | :84 | yes | PARTIAL | used to park ball, not to remove/relaunch |
| `mercyBallDuration` | 20000 ms | :73 | NO | no | |
| `multiballSaverDuration` | 30000 ms | :74 | NO | no | |
| `preserveMultiplierIncreaseDuration` | 7500 ms | :75 | NO | no | |
| `delegate` | class name | :213 | yes (string) | PARTIAL | only matched for `Field2Delegate` in `movers.rs` |
| `script` | text for delegates | :217 | NO | no | none in tables |
| `variables` | map read via `getValueWithKey` | :41,100; Field.java:965 | yes | PARTIAL | only table 2 (and 3) variables; used by `movers.rs` for Field2 only |
| `name`, `version` | metadata | n/a | yes | no | |

### FieldElement (common to all)

| Property | Meaning | Orig ref | P (where) | U |
|---|---|---|---|---|
| `class` | element type; unknown class throws | FieldElement.java:54-67 | yes (serde tag); unknown silently skipped | yes |
| `id` | lookup key `getFieldElementById`, used by delegates | :23,92 | most defs; `RolloverDef` n/a | PARTIAL: used only for labels and hard coded ids |
| `score` | awarded on every collision/state change | :24,102 | Wall, Bumper, Rollover(+per item), DropTarget, Spinner; NOT on Arc/Path/Flipper/Sensor | PARTIAL (see rules #7) |
| `color` | RGB or RGBA | :25,94 | all visible defs as raw JSON Value | PARTIAL: rendered for walls, arcs, paths, bumpers, rollovers, drop targets, ball; IGNORED for flippers, spinner, kick walls (fixed constants in `renderer/mod.rs`); no alpha, no `setNewColor` override, no flash inversion |
| `layer` | Box2D world index | :26,105 | Wall, Arc, Path only | Wall/Arc/Path only (see rules #23) |
| `inactiveLayerColor` | colour blended in as ball leaves layer (250 ms fade) | :27,97,127-141,257-262 | Arc, Path (field parsed but not used); NOT Wall, Bumper, Flipper, Rollover, DropTarget | NO (renderer hides inactive layers outright and uses fixed `COL_LAYER`) |

### WallElement (`VP/elements/WallElement.java`)

| Property | Default | Meaning | P | U |
|---|---|---|---|---|
| `position` [x1,y1,x2,y2] | required | thin wall 0.1 thick | yes | yes |
| `restitution` | 0 | wall bounce | yes | yes |
| `kick` | 0 | impulse of magnitude kick along normal toward ball, 100 ms flash; only if >0.01 | yes | yes (proximity model in `step`, interior normal toward table centre, not toward ball side) |
| `kill` | false | ball lost | yes | PARTIAL (global drain line, not the wall) |
| `retractWhenHit` | false | wall removed when hit (ball savers) | yes | yes (`retract_when_hit`, `retract_pending`) |
| `disabled` | false | starts retracted; delegates call `setRetracted(false)` | yes | PARTIAL: disabled walls are skipped entirely at build time except the id `LaunchBarrier`, so table 2 `LeftTubeBarrier` and table 8 `LaunchGate` never exist |
| `ignoreBall` | false | drawn but no body | NO | NO. `WallDef` has no field, so decorative ignoreBall walls get SOLID bodies: table 5 has 12 `CenterLine_*` walls (inside the centre triangle), table 8 one wall, plus table 7 three `WallPathElement` guides and table 8 arcs on negative layers (those are inert via layer 31) |
| `layer`, `inactiveLayerColor`, `color`, `score`, `id` | | | layer, color, score, id yes; `inactiveLayerColor` no | partially |

### WallArcElement (`WallArcElement.java:52-95`)

| Property | Default | Meaning | P | U |
|---|---|---|---|---|
| `center` | required | | yes | yes |
| `radius` | none | circle shortcut; else `xradius`,`yradius` (default 0) | yes | yes (Rust default 1.0 if absent) |
| `minangle`/`maxangle` | 0 | degrees | yes | yes |
| `segments` | 5 | line segments | yes | PARTIAL: Rust default is 16 when absent (all tables specify it) |
| `ignoreBall` | false | no bodies | NO | NO (solid) |
| `color`, `layer`, `inactiveLayerColor` | | | yes/yes/parsed | colour and layer used; inactive colour not |

### WallPathElement (`WallPathElement.java:31-60`)

`positions` (yes, used), `ignoreBall` (NO, so table 7 LeftLoopGuide/RightLoopGuide/LockAndJackpotGuide are solid), `color`/`layer` (yes), `inactiveLayerColor` (parsed, unused).

### BumperElement (`BumperElement.java:44-63`)

| Property | Default | Meaning | P | U |
|---|---|---|---|---|
| `position` | required | | yes | yes |
| `radius` | 0 (no body if 0) | | yes | yes |
| `kick` | 0 | impulse = kick * unit(ball - bumper centre); needs >0.01; 100 ms colour flash | yes | yes, but default is 8.0 when absent (bug-risk; all tables set it) |
| `outerRadius` | 0 | non-colliding outer ring | yes | yes (rendered) |
| `outerColor` | RGBA(0,0,255,128) | | yes | yes |
| `inactiveLayerOuterColor` | none | | NO | NO |
| `layer`, `inactiveLayerColor` | | table 7 uses on 3 bumpers | NO | NO |
| `color` | blue | | yes | yes; flash shows fixed yellow `COL_BUMPER_LIT`, original inverts the colour |
| `score` | 0 | | yes | default 100 in Rust |

### FlipperElement (`FlipperElement.java:59-122`)

| Property | Default | Meaning | P | U |
|---|---|---|---|---|
| `position` | | pivot | yes | yes |
| `length` | 0 | negative = pivots at right end (right flipper) | yes | yes |
| `minangle`/`maxangle` | 0 | degrees (mirrored for negative length) | yes | yes |
| `upspeed`/`downspeed` | 0 | motor rad/s | yes | yes |
| `layer` | 0 | table 7 has 2 flippers on layer 1 (mini field) | NO | NO |
| `color` | green | | NO | NO (renderer uses cyan constant) |
| `inactiveLayerColor` | | | NO | NO |
| `id` | | table 8 | yes | label only |

### RolloverGroupElement (`RolloverGroupElement.java:60-90`)

| Property | Default | Meaning | P | U |
|---|---|---|---|---|
| `rollovers` | required | list | yes | yes |
| `radius` | 0 | group default; per-rollover override | yes | yes, BUT activation test is `dist <= rollover.radius + ball_radius` vs original `dist <= radius` (ball centre only); Rust default radius 0.4 |
| per-rollover `position`,`radius`,`color`,`score` | group value | | yes | yes |
| per-rollover `reset` | group value | | NO | NO |
| `score` | 0 | per activation (and on toggle off) | yes | yes |
| `color` | green | | yes | yes |
| `cycleOnFlipper` | false | rotate lit states on flipper activation, right if any right flipper activated | yes | yes (`just_pressed` edge, right wins) |
| `toggleOff` | false | re-hit toggles off, scores and sounds again; table 4 uses | NO | NO |
| `ignoreBall` | false | skip ball detection (delegate-driven lights) | yes | yes |
| `reset` (seconds) | 0 | after activation, auto-deactivate that rollover after N game seconds via `scheduleAction`; used by tables 1 (`RampRollovers` 5), 3 (ramp rollovers 3), 8 (`LaneRollovers` 0, outlanes 1) | parsed in `RolloverGroupDef.reset` | NO (`build_rollover_group` never reads it) |
| `layer` | 0 | ball must be on same layer; tables 7 (layer 4 lock rollovers) and 8 (-1) | NO | NO |
| `inactiveLayerColor` | | | NO | NO |
| Rust-only `multiplierStep` | | Asciiverse extension | yes | yes |

Behaviour difference: Rust lights a rollover once, never unlights it except by cycling or by its own group-complete flash. Original fires `allRolloversInGroupActivated` to the delegate, which decides what happens. Rust invented the flash-then-reset-then-bonus (`tick_completion_flash`, `ROLLOVER_FLASH_TICKS`).

### DropTargetGroupElement (`DropTargetGroupElement.java:67-130`)

| Property | Meaning | P | U |
|---|---|---|---|
| `positions` [[x1,y1,x2,y2]...] | explicit targets | yes | yes |
| relative form: `wallStart`, `wallEnd`, `gapFromWall`, `startDistanceAlongWall`, `targetWidth`, `gapBetweenTargets`, `numTargets` | generate targets along a wall | NO | NO. VERIFIED: groups using this form load with 0 targets in Rust. That is 22 groups: table 3 (all 4), table 4 (all 3), table 5 (all 6), table 6 (all 3), table 7 (all 4), table 8 (SpecialTarget, RightBankTargets, EightBallTarget). Consequently on tables 3 to 7 the ball savers can never be re-armed (`saver_for_drop_bank`), and no multiball/ramp-bank targets exist. Only tables 1, 2, 9 and table 8 `InlineTargets` have working drop targets. |
| `reset` (s) | targets return after N seconds once all are down; 0 or absent = never | yes | yes (`reset_delay`, `reset_timer`) |
| `score` | per hit | yes | yes |
| `color` | green | yes | yes |
| `layer`, `inactiveLayerColor` | table 7 mini-field groups on layer 1 | NO | NO |

Hit detection: original uses real contacts (`bodyHit.setActive(false)`); Rust uses distance `< ball_radius + 0.05` each step.

### SensorElement (`SensorElement.java:37-107`)

| Property | Meaning | P | U |
|---|---|---|---|
| `rect` [xmin,ymin,xmax,ymax] | monitored area (corners normalised) | yes (not normalised) | yes |
| `ballLayer` | move ball to this layer when inside | yes | yes (`layer_sensors`) |
| `ballLayerFrom` | only fire if ball is on this layer | yes | yes |
| `recordBallTimes` | track entry timestamps (`ballEntryTimeNanos`); not used by any current delegate | NO | NO |
| (always) | calls `delegate.ballInSensorRange` every tick for each ball inside; sets `ball.mostRecentSensorId` | n/a | NO. Only the ids `LaunchBarrierSensor` and `LaunchBarrierRetract` are special-cased in `from_table`. Every other table sensor (about 10 to 19 per table) does nothing except layer switching. |

### SpinnerElement (`SpinnerElement.java:48-63`)

`position`, `radius`, `minActivationSpeed` (4), `baseCyclesPerSecond` (8), `minCyclesPerSecond` (1.5), `speedDecayRate` (0.9), `score`, `color` (224,224,224): all parsed. Used: yes (`SpinnerInfo`, `step`). Missing: `layer` (not parsed), `color` (parsed, snapshot ignores; fixed `COL_SPINNER*`), `spinnerActivated` delegate call, 250 ms score-animation throttle. Rust default `radius` 0.5 when absent.

## Element behaviors and setters

Counts after each setter are the number of call sites in the original Field1..8 delegates (so you can weigh importance); delegates are covered by others, but these are ENGINE APIs the Rust port needs to expose to be able to port them.

| Behavior / API | What the original does | Orig ref | Status in Rust |
|---|---|---|---|
| Flash on hit | `flashForNanos(100_000_000)` on bumper hit and kick wall hit; while `flashCounter>0` colour is bitwise inverted (alpha kept); counter decremented in `tick`. | FieldElement.java:128,154,261; BumperElement.java:90-97; WallElement.java:133-148 | PARTIAL: bumpers show a yellow "lit" state for `BUMPER_LIT_TICKS` (6*4 substeps); no colour inversion; kick walls never flash; drop targets, flippers do not flash. |
| `setNewColor(Integer)` (x15 in tables 3,5,7) | override base colour at runtime; `null` restores JSON colour | FieldElement.java:242 | MISSING |
| `setNewOuterColor` (table 3) | bumper outer ring colour override | BumperElement.java:110 | MISSING |
| `setRetracted(bool)` / `isRetracted` (x about 60) | toggles `body.setActive`; retracted wall is neither solid nor drawn | WallElement.java:91 | PARTIAL: only for kick walls with `retractWhenHit` via `restore_ball_saver`; the `LaunchBarrier` and TriangleWall have bespoke code; no generic API. |
| `setVisible(bool)` (walls x12; rollover groups) | hide drawing without affecting physics | WallElement.java:~87, RolloverGroupElement.java:291 | MISSING |
| `retractWhenHit` | retract on contact | WallElement.java:133 | IMPLEMENTED (kick walls only; a retractWhenHit wall WITHOUT kick, e.g. table 5 `ExtraBallBarrier_*` if they have no kick, is registered as a plain solid wall: PARTIAL) |
| `ignoreBall` on Wall/Arc/Path | drawn but no bodies | WallElement.java:~74, WallArcElement.java:83, WallPathElement.java | MISSING (see JSON section; makes decorative walls solid) |
| `setStartAndDirection` / `setStartAndAngle` (table 5 x3) | move a wall keeping its length | WallElement.java:~150-160 | PARTIAL: only in `movers.rs` for table 5 triangle |
| `BumperElement.setCenter` / `getCenter` (tables 2, 5) | teleports the static body | BumperElement.java:102 | PARTIAL: `movers.rs` for table 2 only (`advance_rotating_groups` sets body positions) |
| Bumper `kick` impulse | `kick * unit(ball - centre)` applied as linear impulse | BumperElement.java:78-96 | IMPLEMENTED (proximity trigger, debounce 12 substeps) |
| Wall `kick` | impulse of magnitude kick perpendicular, oriented toward the ball's side by dot product | WallElement.java:110-130 | PARTIAL: direction is a precomputed "toward table centre" normal (`interior_normal`), plus a hand applied bounce before the kick. |
| Flipper tick clamp | when motor speed >0.5 and within 0.05 rad of top, speed forced to 0.5 | FlipperElement.java:163-172 | IMPLEMENTED (`FLIPPER_TOP_SLOWDOWN_*`) |
| Flipper draw clamp | draws a single line clamped to [lower, upper] | FlipperElement.java:188 | IMPLEMENTED approximately (draws from body transform) |
| `RolloverGroupElement.activateFirstUnactivatedRollover` (tables 1, 8) | | line 124 | MISSING |
| `setAllRolloversActivated(bool)` (x about 25) | | line 272 | MISSING (only internal `lit=false` reset) |
| `setRolloverActiveAtIndex`, `isRolloverActiveAtIndex`, `allRolloversActive`, `numberOfRollovers` (x about 25) | | lines ~100-122 | MISSING as API |
| `setRolloverCenterAtIndex`, `setRolloverRadiusAtIndex`, `setRolloverColorAtIndex` (tables 2, 4, 5, 6, 8) | move/resize/recolour individual rollovers (e.g. orbiting planets) | lines 157-175 | MISSING |
| `setIgnoreBall` on rollover groups (x about 12) | turns group's ball sensitivity on/off | line 283 | MISSING (flag is static) |
| `cycleRollovers(toRight)` | rotate states, wrapping | line 257 | IMPLEMENTED internally (`step`) |
| Rollover reset delay | `scheduleAction(reset*1000, deactivate)` | line ~209 | MISSING |
| Rollover toggleOff | | line ~214 | MISSING |
| Rollover sound | `audioPlayer.playRollover()` on each state change | line ~203 | MISSING (no sound) |
| Rollover draw | inactive = outlined circle, active = filled; per-rollover colour else group colour | line 295 | IMPLEMENTED (`draw_rollover`) |
| Rollover ball layer check | ball.layer must equal group layer | RolloverGroupElement.java:101 | MISSING |
| `DropTargetGroupElement.allTargetsHit`, `makeAllTargetsVisible` (table 8 x4) | | lines ~120,144 | MISSING as API; auto reset implemented |
| Drop target group `allDropTargetsInGroupHit` notification | | line 133 | MISSING (hard coded saver re-arm only) |
| Sensor `ballInSensorRange` notification and `mostRecentSensorId` (tables 6, 7) | | SensorElement.java:73-95 | MISSING |
| Sensor `ballLayer` moves ball between worlds | `Ball.moveToLayer` recreates the body in the target world, keeping position, velocity, angle, angular velocity | Ball.java:181; SensorElement.java:77-80 | IMPLEMENTED (collision-group swap in `step`) |
| Sensor `recordBallTimes` | | SensorElement.java:108 | MISSING |
| Spinner | activation >= minActivationSpeed, cycles/s = (speed/minSpeed)*base, decay 0.9 per cycle, stops below 1.5; scores every cycle; invisible/visible alternation; delegate `spinnerActivated` | SpinnerElement.java:92-135 | IMPLEMENTED (`step` spinner block) except delegate call and animation throttle |
| `Ball.setPrimaryColor/SecondaryColor` (tables 5, 6) | recolour ball at runtime | Ball.java ~165 | MISSING |
| `Field.setShapes` (table 7) | | Field.java:310 | MISSING |
| `Field.getFieldElementById` | generic element lookup | Field.java:226 | MISSING (ids are matched ad hoc by string) |
| `Field.getValueWithKey` (table 2 x4) | read `variables` | Field.java:965 | PARTIAL (`TableDef::variable`, used by `movers.rs`) |
| `Field.addScoreWithAnimation(points,x,y)` (x about 15) | score with multiplier plus animation at position | Field.java:693-701 | PARTIAL: `score_events` queue exists; no position, no animation |
| `Field.showGameMessage` (x about 55) | | Field.java:750 | MISSING |
| `Field.scheduleAction` (x about 12) | | Field.java:342 | MISSING |
| `Field.incrementAndDisplayScoreMultiplier` (x7), `setScoreMultiplier` (table 4) | | Field.java:897-906 | PARTIAL (`multiplier_events`, integer only) |
| `Field.addExtraBall` (table 5) | | Field.java:849 | MISSING |
| `Field.createBall`/`launchBall`/`removeBallWithoutBallLoss`/`updateBallLaunchTimes`/`playBallLaunchSound` (tables 1 to 8) | | Field.java:327-377,446 | MISSING (single ball; `launch_ball` only) |
| `Delegate.isFieldActive` | keep ticking when no ball | Field.java:511-519 | NOT APPLICABLE (Rust always ticks) |
| Launch barrier | the original is just a `disabled` wall that delegates `setRetracted(false)` when the ball passes a sensor | tables 1 to 7 | IMPLEMENTED in a table-agnostic way keyed on ids `LaunchBarrier`, `LaunchBarrierSensor`, `LaunchBarrierRetract` (`LaunchBarrierInfo`). Table 8 `LaunchGate` and table 2 `LeftTubeBarrier` are not handled. |
| Ball saver re-arm | delegates `setRetracted(false)` on `allDropTargetsInGroupHit` | Field1/3/4/5/6/7 delegates | IMPLEMENTED via `saver_for_drop_bank` but broken on tables 3 to 7 by the wallStart gap (no drop targets exist to clear). |

## App and UI features

| # | Feature | What the original does | Orig ref | Status in Rust |
|---|---|---|---|---|
| 1 | Table select | Prev/next buttons (and flipper keys) cycle tables 1..N, wrap-around; last table is remembered (`initialLevel`) | B/BouncyActivity.java:861-873,428-456,762-774 | PARTIAL: menu list of 9 tables (`ui/menu.rs`); selection not remembered between runs; table 9 is Rust-only. |
| 2 | Start / resume / end / quit | Start Game, Resume Game, End Game, Quit buttons; touch anywhere starts a game, launches a ball, works flippers; 1000 ms start guard after a game ends | BouncyActivity.java:120-122,777-831 | PARTIAL: Space launches/restarts; Esc/Q abandons to menu; no resume from menu; no post game-over 1 s guard. |
| 3 | High scores | Top 5 per table (comma separated string `highScores.N`), plus `lastScore.N`; saved when the game ENDS (only if it beats lowest or fewer than 5); not for unlimited balls; High scores panel (list, "No scores for this table", last score); idle ScoreView cycles "Touch to start" / last score / each high score every 3500 ms | BouncyActivity.java:106-112,641-668,684-760,890-915; B/ScoreView.java:41-47,186-245 | PARTIAL: one high score per table in `persist.rs` (`high_scores: [u64; 9]`, XDG config `rusty-pinball/state.json`), updated and saved on every ball loss (`app.rs tick`), shown in menu and DMD (`HI SCORE`). No top 5, no last score, no cycling display, no high score screen. |
| 4 | Preferences screen | sound, music, haptic (default false), independentFlippers (true), zoom (true), gameSpeed slow/normal/fast, showBallTrails (true), showScoreAnimations (true), useOpenGL (true), lineWidth, showFPS (false) | res/xml/preferences.xml; BouncyActivity.java:578-625 | PARTIAL: only a single Sound toggle in the menu (`ToggleSound`) which also gates music (`AudioManager::set_enabled`). No separate music toggle, game speed, trails, score animations, zoom, FPS, line width. Haptic, independentFlippers, OpenGL, line width are NOT APPLICABLE to a terminal. |
| 5 | Camera zoom | While exactly one ball is in play, zoom eases (smoothstep over 1 s game time) from 1.0 to 1.5 and centres on the ball (x and y), clamped to table bounds; zooms out over 1 s after the last ball is lost, centred on its last position; table centred if the view is larger than the table. Preference `zoom`. | Field.java:80-84,772-791; B/FieldViewManager.java:94-128 | PARTIAL: `app.rs tick` smooths a vertical scroll `camera_y` toward the ball (target = ball y - 40 % of field height); full table width always shown; no scale zoom, no horizontal tracking, no zoom out on ball loss. |
| 6 | Ball trails | see rules #22 | | MISSING |
| 7 | Element flashing | see element behaviours (100 ms inversion) | | PARTIAL (bumper yellow) |
| 8 | Score animations | see rules #8 | | MISSING |
| 9 | Score view | Top strip: score (or game message), remaining/used ball icons (3 circles; ball leaves display when launched), multiplier text, optional FPS, "*" for unlimited | B/ScoreView.java:83-178 | PARTIAL: DMD bar (`ui/dmd.rs`): SCORE, HI SCORE, BALLS icons (filled=remaining, not hiding the launched ball), status, multiplier `xN`, Rust-only AWARD/JACKPOT/letters/SUPER lines. No messages, no unlimited marker. |
| 10 | Rendering | Vector lines/circles/arcs with width scaled to screen (default min(w,h)/216); OpenGL ES 2/1 or Canvas; arcs drawn as true arcs; filled and framed circles with alpha; elements in layer order; per-element colours; inverse flash; default colours wall (64,64,160), flipper/rollover/drop target green, bumper blue, spinner light grey | B/GL20Renderer.java, CanvasFieldView.java, FieldViewManager.java:56-75 | PARTIAL: braille renderer (`renderer/mod.rs`); no alpha, no inversion; flipper cyan, kick wall green, spinner and layer colours fixed constants; arcs drawn as segments (acceptable in terminal). |
| 11 | Sound effects | `VPSoundpool`: score ding (random of 6 bumper oggs, never same twice in a row, vol 0.5) on every scoring contact; launch `andBounce2.ogg` (vol 1); flipper `flipper1.ogg` (only when a flipper newly engages); message `message2.ogg` (vol 0.66) on every game message with sound; start `startup1.ogg` (vol 0.5) at game start AND game end; rollover `rolloverE.ogg` (vol 0.3 s) played as up to 3 simultaneous distinct random pitches from {0.794, 0.891, 1.0, 1.189, 1.335, 1.587} on each rollover change. Sound pref and music pref separate. | B/VPSoundpool.java:35-215, Field.java:687,616,641 | PARTIAL: `audio/mod.rs`: bumper dings cycle in fixed order (not random/non repeating) and only for bumper hits (not for kick walls/drop targets/spinner contacts that have a score); flipper sound on any Press during Playing; launch uses `ball1.ogg` (not `andBounce2.ogg`, though that file is present in assets); ball drain plays `message2.ogg` (the original plays it for messages, not drains); NO rollover sound, NO startup sound, NO message sound; sound toggle also pauses music. |
| 12 | Music | `drumbassloop.ogg` starts after every 20th scoring hit (`cScore%20==0`) while not playing; `androidpad.ogg` plays at score count 10 then every 10 + 42k hits (`andrModAmt` grows by 42 each time); both reset/seek to 0 at game start and on music toggle off; paused on pause. | VPSoundpool.java:132-155,117-130 | PARTIAL: `drumbassloop.ogg` loops continuously at volume 0.35 from app start; `androidpad.ogg` asset present but unused; no score-driven triggering; not paused on game pause. |
| 13 | Haptics | vibrate on every sound | VPSoundpool.java:127-135 | NOT APPLICABLE (terminal) |
| 14 | Input | Touch: left/right half independent flippers (default) or all; 50 ms minimum flipper hold for edge touches; keys: Z, D-pad left, L1 (left), slash, D-pad right, R1 (right), Space/Enter/D-pad centre (all); P pauses; Back pauses; flipper keys switch tables in the menu | FieldViewManager.java:157-302 | PARTIAL: Z/X/arrows/Shift for left/right (no all-flippers key; Space is launch only); P pause; decay-based hold (`FLIPPER_HOLD_TICKS` = 3) to bridge terminals without key release. Touch and gamepad NOT APPLICABLE. No Enter or Space as "both flippers". |
| 15 | Launch gating | Touch/key launches only when no ball in play (or ball stuck in dead zone); ignored for 750 ms after ball loss | FieldViewManager.java:144-150 | PARTIAL (Space on WaitingToLaunch/BallLost; no lockout) |
| 16 | About/Help screen | App credits, instructions, per-table rules text (`tableN_rules`), preference descriptions, credits (Table 8 by Jack Morgan, audio by pdx) | B/AboutActivity.java:20-60; res/values/strings.xml | MISSING (no in-game help or rules text; README only) |
| 17 | Localisation | strings for de, eo, es, fr, id, in, nl, pl, tr | res/values-*/ | MISSING (English only; fine to defer) |
| 18 | Frame rate manager, power-save, OpenGL/Canvas switch, fullscreen/insets, tilt `OrientationListener` (disabled) | | B/BouncyActivity.java, util/FrameRateManager.java, OrientationListener.java | NOT APPLICABLE |
| 19 | Background/pause lifecycle | pause on losing window focus; return to paused menu rather than auto-resume | BouncyActivity.java:387-407 | NOT APPLICABLE (terminal has no focus events here); P covers manual pause |
| 20 | Table loading | tables bundled as assets, numbered, `FieldLayoutReader.getNumberOfLevels` | B/FieldLayoutReader.java | PARTIAL: `table::load_table` reads `assets/tables/tableN.json` via a relative path at runtime (works only when cwd is the project root); tables 1 to 8 are byte identical in element content to the originals except `ballradius` on tables 6 (0.5 to 0.46) and 7 (0.49 to 0.46). |

## Extra findings (bugs spotted while auditing)

1. Four launches per game with `numballs: 3` (verified by test run in a scratch copy).
2. Drop target groups using `wallStart` load with zero targets on tables 3 to 8 (verified by test run); breaks ball saver re-arming and every bank-driven feature there.
3. `ignoreBall` is not parsed on Wall/Arc/Path, so decorative geometry is solid (table 5 centre lines, table 7 loop guides, table 8 one wall).
4. `layer: None` is treated as universal rather than layer 0, and bumpers/flippers/drop targets/rollovers ignore layers; table 7's mini-field is therefore not isolated.
5. Rollover `reset` and `toggleOff` ignored (tables 1, 3, 4, 8).
6. Rollover trigger radius is `radius + ball_radius`; original is `radius` against the ball centre.
7. `RolloverGroupDef.reset` is parsed but dead; `SensorDef` ignores `recordBallTimes`.
8. Defaults differ: bumper `score` 100 and `kick` 8.0 (original 0/0), arc `segments` 16 (original 5), arc radii default 1.0 (original 0).
9. Table 8 `LaunchGate` and table 2 `LeftTubeBarrier` (disabled walls) never exist.
10. `input.rs plunger_charge()` is dead code.

### Summary of the biggest gaps

Ranked by how much of the original gameplay they affect:

1. Drop-target `wallStart` form unparsed: 22 groups on tables 3 to 8 have no targets, so ball savers never re-arm and all bank-driven table objectives are dead (verified).
2. No delegate/event layer or generic element API (setRetracted, setVisible, setNewColor, rollover setters, scheduleAction, showGameMessage, sensors `ballInSensorRange`, `allRolloversInGroupActivated`, `allDropTargetsInGroupHit`, `processCollision`): tables 3 to 8 rule logic (multiball, jackpots, colour changes, planets, constellations, pool balls, bonuses) cannot run; only hard-coded id hacks exist.
3. Single ball only: no multiball, ball lock, `createBall`/`launchBall`/`removeBallWithoutBallLoss`, multiball saver; this blocks the headline mechanic of tables 1 to 8.
4. Ball and game flow: 4 balls instead of 3, no extra ball, no mercy ball (20 s early-drain save), no unlimited balls mode, no stuck-ball bump, no multiplier carry rules (ROUND_HALF_DOWN, preserve window), integer multiplier capped at 5 instead of fractional/unbounded.
5. Layer system semantics: layer-0-only default, layer filtering for bumpers/flippers/drop targets/rollovers/spinners/sensors, inactive-layer colour fade; table 7 mini-field and table 6/8 ramp elements misbehave.
6. `ignoreBall` ignored on walls/arcs/paths: decorative geometry becomes solid (tables 5, 7, 8).
7. Rollover semantics: `reset` timers, `toggleOff`, ball-centre radius test, per-layer check, no rollover sound; invented completion flash/bonus.
8. Game messages and score animations (the main feedback channel for rules, e.g. "Ball Saved", "Multiball!", jackpots) are absent; DMD shows phase strings only.
9. Audio parity: no rollover/message/startup sounds, bumper-only dings, music not score-driven, `androidpad` unused, separate music toggle and game start/end jingles missing.
10. Presentation and settings: no ball trails, no spin dot/secondary ball colour, no flash inversion, no true zoom, no game speed, no top-5 high scores/last score screen, no help/rules screen, no remembered table, single sound toggle only.
