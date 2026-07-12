# Changelog

All notable changes to Rusty Pinball are documented here.

## [Unreleased]

### Added
- Table 9, **Party Land**: an original table (not a Vector-Pinball port) recreating David's first pinball table, Party Land from Digital Illusions CE's *Pinball Fantasies* (1992), from an archived rule sheet — no schematic survives, so geometry is hand-designed to match the described shot layout. Built entirely from existing element types (walls, bumpers, flippers, drop targets, rollover groups) with no new physics; the four true "ball vanishes into a hole" captures (Left Sinkhole, Tunnel, Arcade, Hidden Entrance) are approximated as scoring kick-walls at the same spot rather than true capture/eject physics. Tunnel/Arcade/Hidden-Entrance each get a funnel mouth (matching the Left-Sinkhole idiom) and Cyclone/Skyride are proper two-rail channels (matching the orbits), rather than bare floating wall stubs. Letter-lighting (MAD/CRAZY/PARTY), timed super-modes, the Demon Head award ladder, Jackpot, and the bonus multiplier are deferred to a future pass
- Per-element JSON colors (`color`/`outerColor`/`ballcolor`) are now rendered instead of being silently dropped — bumpers, the ball, rollovers, drop targets, and plain (non-layered) walls all show their table-specified colors; tables that never set unusual colors render identically to before
- Spinners (`SpinnerElement`): virtual, non-colliding zones — a ball crossing one at or above `minActivationSpeed` sets it spinning, flashing on/off at a decaying frequency and scoring once per cycle until the frequency decays below `minCyclesPerSecond` and it goes idle; ported from Vector Pinball's tick-based model (table 8's `LeftOrbitSpinner`)
- Debug object-ID overlay (`I` key while playing): labels every wall segment, kick-wall, drop target, bumper, flipper, rollover, and spinner with its index in the table JSON's `elements` array (`N` for single elements, `N.i` for grouped sub-items like rollovers/drop targets), for cross-referencing mechanics against the source table
- Drop targets: per-target proximity hit detection, targets disable on contact, group-clear bonus (2 000 pts), groups reset when fully cleared
- Shooter-lane gate: horizontal wall across the lane that closes once the ball escapes upward, preventing it from rolling back into the dead zone
- Dead-zone detection: ball that rolls back before clearing the gate is automatically re-parked and the player can re-launch
- `park_ball()`: freezes ball at launch position with gravity off; reactivates ball-saver kick-walls for each new ball
- Drop targets rendered in yellow in the braille viewport
- `WallArcDef` now accepts `radius` (circular arc) as a fallback when `xradius`/`yradius` are absent — fixes table 2 loading
- First unit tests: regression coverage for spinner activation/scoring/decay and the table 8 load fix below (`cargo test`)

### Fixed
- Table 8 (Pool Party) failed to load entirely: its JSON uses `layer: -1` on a few purely-decorative, ball-ignoring elements (a Vector-Pinball convention for a physics world no ball ever occupies), but `layer` was typed as `u8`. Negative layers now map to an unreachable sentinel layer, so those elements build without colliding or rendering — matching their original decorative-only behavior — instead of failing to parse

### Changed
- Package/binary renamed from `rusty-pin` to `rusty-pinball` — the "rusti-pin" pun was short-lived; menu title art and window chrome were always "Rusty Pinball"
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
