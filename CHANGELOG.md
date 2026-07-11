# Changelog

All notable changes to Rusty Pin are documented here.

## [Unreleased]

### Added
- Debug object-ID overlay (`I` key while playing): labels every wall segment, kick-wall, drop target, bumper, flipper, and rollover with its index in the table JSON's `elements` array (`N` for single elements, `N.i` for grouped sub-items like rollovers/drop targets), for cross-referencing mechanics against the source table
- Drop targets: per-target proximity hit detection, targets disable on contact, group-clear bonus (2 000 pts), groups reset when fully cleared
- Shooter-lane gate: horizontal wall across the lane that closes once the ball escapes upward, preventing it from rolling back into the dead zone
- Dead-zone detection: ball that rolls back before clearing the gate is automatically re-parked and the player can re-launch
- `park_ball()`: freezes ball at launch position with gravity off; reactivates ball-saver kick-walls for each new ball
- Drop targets rendered in yellow in the braille viewport
- `WallArcDef` now accepts `radius` (circular arc) as a fallback when `xradius`/`yradius` are absent — fixes table 2 loading

### Changed
- `launch_ball()` reuses the existing ball body rather than destroying and recreating it each launch
- Launch jitter: ±4 % Y-velocity variation plus a small lateral nudge, seeded from nanosecond wall-clock time (LCG)
- `is_ball_drained()` now ignores a ball sitting in the shooter-lane dead zone
- Active kick-walls only are included in `FieldSnapshot`; inactive (retracted) walls are hidden from the renderer
- Version bumped to 0.2.0

---

## [0.1.1] — 2026-06-06

### Added
- Proximity-based slingshot impulse: `WallElement` entries with a `kick` JSON field fire an impulse when the ball comes within range, with a debounce timer to prevent double-firing
- `retract_when_hit` kick-walls (ball-savers): deactivate permanently on first contact
- CCD (continuous collision detection) on the ball body to prevent tunnelling through thin walls
- Colored braille renderer (`ColoredFrame`): walls gray, kick-walls green, bumpers blue / lit yellow, flippers cyan, ball white, background dark gray
- UI groups same-color braille characters into `Span`s for per-character color output; menu items padded to equal width and centered as a block
- Flipper hold-tick decay: flippers stay raised for a few frames after the last key event, bridging the key-release gap on terminals without Kitty keyboard protocol
- Kitty keyboard protocol push/pop for accurate key-release detection where supported
- `Q` as an alternate in-game quit key
- Sound toggle actually pauses/resumes background music via `AudioManager::set_enabled()`

---

## [0.1.0] — 2026-06-06

### Added
- Initial release: Rust TUI pinball with Rapier2d physics and braille vector graphics
- Ball physics at 1× real time (2 steps/frame, dt = 1/60 s, 30 fps target)
- Flippers: Z / Left = left, X / Right = right; pivot positions and angles match Vector-Pinball table 1
- Bumpers with scoring (500 / 500 / 500 / 1 000 pts) and lit-flash animation
- Camera scrolls to follow the ball vertically
- Drain detection with BALL LOST / GAME OVER screens
- Full game loop: launch → play → BALL LOST → Enter to continue → GAME OVER
- DMD overlay shows score, balls remaining, and game phase
- Start menu with table selection (8 tables) and sound toggle
- 8 tables loaded from Vector-Pinball JSON assets (`assets/tables/table1–8.json`)
- Background music and sound effects via `rodio` / `kira`
- GPL-3.0 licence
