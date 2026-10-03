// Per-table rules, ported from the original Vector Pinball's delegate classes
// (fields/Field1Delegate.java and so on). A script reacts to events the physics
// world reports (a bumper was hit, a drop bank cleared, a rollover group was
// completed by a ball, a scheduled timer fired) and acts on the world through
// its helper methods: show a message, light rollovers, launch another ball,
// re-arm a ball saver, raise the multiplier.
//
// Scripts are plain Rust, selected by the table's `delegate` name. A table
// with no script plays with the generic element behavior only.

use std::f32::consts::TAU;

use rapier2d::prelude::Vector;

use super::PhysicsWorld;
use crate::table::TableDef;

pub(super) trait TableScript {
    /// When true, rollover groups completed by a ball are the script's business:
    /// the world does not flash, pay a completion bonus or reset them on its own
    /// (the original leaves all of that to the delegate).
    fn handles_rollovers(&self) -> bool { false }

    /// Every step, before events: for motion and forces (table 5, table 6).
    fn tick(&mut self, _w: &mut PhysicsWorld, _dt: f32) {}
    /// `ball` is the index of the ball that caused the event.
    /// Ids of plain walls this script raises and lowers with `set_wall_retracted`
    /// (kick walls and the launch barrier need no listing).
    fn scripted_walls(&self) -> &'static [&'static str] { &[] }
    /// A new game's world has just been built.
    fn game_started(&mut self, _w: &mut PhysicsWorld) {}
    /// The ball in play was lost (before the next one is parked).
    fn ball_lost(&mut self, _w: &mut PhysicsWorld) {}
    fn bumper_hit(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str) {}
    fn drop_bank_cleared(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str) {}
    /// A ball lit the last rollover of a group (not called for script-lit groups).
    fn rollover_group_completed(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str) {}
    /// A ball is inside the sensor `id` (called every step while it is). `previous`
    /// is the sensor it was in before, so a script can tell which way it came.
    fn sensor(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str, _previous: Option<&str>) {}
    /// A timer from `PhysicsWorld::schedule` fired.
    fn scheduled(&mut self, _w: &mut PhysicsWorld, _token: u32) {}
}

pub(super) fn script_for(table: &TableDef) -> Option<Box<dyn TableScript>> {
    match table.delegate.as_deref() {
        Some("Field1Delegate") => Some(Box::new(Classic)),
        Some("Field2Delegate") => Some(Box::new(RoundAndRound)),
        Some("Field4Delegate") => Some(Box::new(CompoundInterest { bumper_multiplier_increase: 1 })),
        Some("Field5Delegate") => Some(Box::new(EyeOfHorus { ramp_bonus: [0; 4] })),
        Some("Field6Delegate") => Some(Box::new(Orbiters::new(clock_seed()))),
        _ => None,
    }
}

/// Timer token: launch another ball if fewer than three are in play.
const LAUNCH_MULTIBALL_BALL: u32 = 1;

/// Multiball start shared by tables 1 and 2: message, both ball savers back,
/// and balls launched at 1000 ms and 3500 ms of game time while fewer than
/// three are in play (original: `startMultiball`).
fn start_multiball(w: &mut PhysicsWorld) {
    w.show_message("Multiball!", 2000);
    w.restore_ball_saver("BallSaver-left");
    w.restore_ball_saver("BallSaver-right");
    w.schedule(1000, LAUNCH_MULTIBALL_BALL);
    w.schedule(3500, LAUNCH_MULTIBALL_BALL);
}

fn launch_multiball_ball(w: &mut PhysicsWorld) {
    if w.ball_count() < 3 {
        w.launch_extra_ball();
    }
}

/// "Left Save Enabled" / "Right Save Enabled" when a save bank is cleared (the
/// saver itself is re-armed by the world, see `saver_for_drop_bank`).
fn announce_saver(w: &mut PhysicsWorld, left_id: &str, right_id: &str, id: &str) {
    if id == left_id {
        w.show_message("Left Save Enabled", 1500);
    } else if id == right_id {
        w.show_message("Right Save Enabled", 1500);
    }
}

// ---------------------------------------------------------------------------
// Table 1: Classic (Field1Delegate)
// ---------------------------------------------------------------------------

struct Classic;

impl TableScript for Classic {
    fn handles_rollovers(&self) -> bool { true }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        // Every group completion clears the group and raises the multiplier.
        w.set_all_rollovers(id, false);
        w.increment_multiplier();
        // Shooting the ramp with all the extra-ball lights lit starts multiball.
        if id == "RampRollovers" && w.rollovers_all_lit("ExtraBallRollovers") {
            w.set_all_rollovers("ExtraBallRollovers", false);
            start_multiball(w);
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        // Any bank lights the next extra-ball light.
        if !w.rollovers_all_lit("ExtraBallRollovers") {
            w.activate_first_unlit_rollover("ExtraBallRollovers");
            if w.rollovers_all_lit("ExtraBallRollovers") {
                w.show_message("Shoot Ramp for Multiball", 1500);
            }
        }
    }

    fn scheduled(&mut self, w: &mut PhysicsWorld, token: u32) {
        if token == LAUNCH_MULTIBALL_BALL { launch_multiball_ball(w); }
    }
}

// ---------------------------------------------------------------------------
// Table 2: Round and Round (Field2Delegate; the orbiting bumpers are in movers.rs)
// ---------------------------------------------------------------------------

struct RoundAndRound;

impl TableScript for RoundAndRound {
    fn handles_rollovers(&self) -> bool { true }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        w.set_all_rollovers(id, false);
        w.increment_multiplier();
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeft", "DropTargetRight", id);
        // Each bank feeds one column of the extra-ball lights (left 0, top left
        // 1, right 2); a column's next light is three along once one is lit.
        let mut idx = match id {
            "DropTargetLeft" => 0,
            "DropTargetTopLeft" => 1,
            "DropTargetRight" => 2,
            _ => return,
        };
        let n = w.rollover_count("ExtraBallRollovers");
        while idx < n {
            if !w.rollover_lit("ExtraBallRollovers", idx) {
                w.set_rollover("ExtraBallRollovers", idx, true);
                if w.rollovers_all_lit("ExtraBallRollovers") {
                    w.show_message("Shoot Red Bumper", 1500);
                }
                break;
            }
            idx += 3;
        }
    }

    fn bumper_hit(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        // Hitting the centre red bumper with every extra-ball light lit starts multiball.
        if id == "CenterBumper1" && w.rollovers_all_lit("ExtraBallRollovers") {
            start_multiball(w);
            w.set_all_rollovers("ExtraBallRollovers", false);
        }
    }

    fn scheduled(&mut self, w: &mut PhysicsWorld, token: u32) {
        if token == LAUNCH_MULTIBALL_BALL { launch_multiball_ball(w); }
    }
}

// ---------------------------------------------------------------------------
// Table 6: Orbiters (Field6Delegate)
//
// Five planets orbit a sun; each pulls the ball. Rolling over a planet gives
// the ball that planet's colour; a ball of the right colour that then scores
// its planet's shot (a ramp, its bank or its rollovers) switches the planet
// on. Reaching all five starts multiball in the original; that is round 2, so
// here the planets simply stay lit.
// ---------------------------------------------------------------------------

fn clock_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1)
}

type Rgb = (u8, u8, u8);

const PLANET_COLORS: [Rgb; 5] = [(0xFF, 0x99, 0x00), (0x00, 0x99, 0xFF), (0xAA, 0x00, 0x00), (0x00, 0xAA, 0x66), (0xAA, 0x22, 0xCC)];
/// The colour a ball takes from planet `i` (brighter than the planet).
pub(super) const PLANET_BALL_COLORS: [Rgb; 5] = [(0xFF, 0xBB, 0x44), (0x66, 0xBB, 0xFF), (0xDD, 0x44, 0x44), (0x77, 0xDD, 0xAA), (0xCC, 0x88, 0xEE)];

const SUN_GRAVITY_FORCE: f32 = 8.0;
/// Multiplied by radius cubed for each planet.
const PLANET_GRAVITY_FORCE: f32 = 15.0;
/// The sun and planets are this far "below" the ball, which softens the pull up close.
const GRAVITY_DEPTH_SQUARED: f32 = 4.0;
/// Beyond this distance from the sun nothing pulls the ball.
const GRAVITY_RANGE_SQUARED: f32 = 64.0;

const RAMP_BONUS_SECONDS: f32 = 12.0;
const RAMP_SCORE: u64 = 2500;
const PLANET_TARGETS_SCORE: u64 = 5000;
const PLANET_ROLLOVERS_SCORE: u64 = 5000;
const PLANET_ACTIVATED_SCORE: u64 = 5000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlanetStatus { Off, InProgress, On }

struct Planet {
    angle: f32,
    /// Radians per sim second; planets nearer the sun are faster.
    angular_velocity: f32,
    status: PlanetStatus,
}

pub(super) struct Orbiters {
    planets: Vec<Planet>,
    seed: u32,
    initialized: bool,
    /// Phase of the pulse shown by planets that have a matching ball (period 4 s).
    pulse: f32,
    ramp_multiplier: u32,
    ramp_seconds_left: f32,
}

impl Orbiters {
    pub(super) fn new(seed: u32) -> Self {
        Orbiters { planets: Vec::new(), seed, initialized: false, pulse: 0.0, ramp_multiplier: 1, ramp_seconds_left: 0.0 }
    }

    fn random(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn init(&mut self, w: &mut PhysicsWorld) {
        self.initialized = true;
        // The sun is always lit.
        w.set_all_rollovers("Sun", true);
        for i in 0..w.rollover_count("Orbits") {
            let angle = self.random() * TAU;
            let jitter = self.random();
            self.planets.push(Planet {
                angle,
                angular_velocity: (0.9 + 0.2 * jitter) / (i + 1) as f32,
                status: PlanetStatus::Off,
            });
        }
    }

    fn planet_group(i: usize) -> String { format!("Planet{}", i + 1) }

    fn all_planets_on(&self) -> bool {
        !self.planets.is_empty() && self.planets.iter().all(|p| p.status == PlanetStatus::On)
    }

    /// Switches planet `i` on if the ball has that planet's colour.
    fn activate_planet_if_match(&mut self, w: &mut PhysicsWorld, ball: usize, i: usize) {
        let Some(planet) = self.planets.get(i) else { return };
        if planet.status == PlanetStatus::On { return; }
        if w.ball_color(ball) != Some(PLANET_BALL_COLORS[i]) { return; }
        self.planets[i].status = PlanetStatus::On;
        w.add_score(PLANET_ACTIVATED_SCORE);
        // With all five on the original starts multiball (round 2); until then the
        // last planet gets the same message as the others.
        let _ = self.all_planets_on();
        w.show_message(&format!("Planet {} Activated!", i + 1), 1500);
    }

    /// A ramp counts when the ball came through its `required` sensor first.
    fn check_ramp(&mut self, w: &mut PhysicsWorld, ball: usize, previous: Option<&str>, required: &str, planet: usize) {
        if previous != Some(required) { return; }
        if self.ramp_multiplier > 1 {
            w.show_message(&format!("{}x Ramp", self.ramp_multiplier), 1000);
        }
        self.ramp_seconds_left = RAMP_BONUS_SECONDS;
        w.add_score(RAMP_SCORE * self.ramp_multiplier as u64);
        self.ramp_multiplier += 1;
        self.activate_planet_if_match(w, ball, planet);
    }
}

fn blend_to_black(c: Rgb, fraction: f32) -> Rgb {
    let f = fraction.clamp(0.0, 1.0);
    let k = |v: u8| (v as f32 * (1.0 - f)) as u8;
    (k(c.0), k(c.1), k(c.2))
}

impl TableScript for Orbiters {
    fn handles_rollovers(&self) -> bool { true }

    fn tick(&mut self, w: &mut PhysicsWorld, dt: f32) {
        if !self.initialized { self.init(w); }

        // Planet status follows the balls: a planet that is not on is "in progress"
        // while any ball has its colour, and its rollover pulses between 30% and 100%.
        self.pulse = (self.pulse + TAU * dt / 4.0) % TAU;
        for i in 0..self.planets.len() {
            if self.planets[i].status != PlanetStatus::On {
                self.planets[i].status = if w.any_ball_has_color(PLANET_BALL_COLORS[i]) {
                    PlanetStatus::InProgress
                } else {
                    PlanetStatus::Off
                };
            }
            let status = self.planets[i].status;
            let id = Self::planet_group(i);
            w.set_all_rollovers(&id, status != PlanetStatus::Off);
            let dim = if status == PlanetStatus::InProgress { (1.0 + self.pulse.sin()) * 0.35 } else { 0.0 };
            w.set_rollover_color(&id, 0, Some(blend_to_black(PLANET_COLORS[i], dim)));
        }

        // Ramp bonus window.
        if self.ramp_seconds_left > 0.0 { self.ramp_seconds_left -= dt; }
        if self.ramp_seconds_left <= 0.0 { self.ramp_multiplier = 1; }

        // Planets orbit the sun (also with no ball in play).
        for i in 0..self.planets.len() {
            let p = &mut self.planets[i];
            p.angle = (p.angle + dt * p.angular_velocity).rem_euclid(TAU);
            let angle = p.angle;
            if let (Some((cx, cy)), Some(radius)) = (w.rollover_center("Orbits", i), w.rollover_radius("Orbits", i)) {
                w.set_rollover_center(&Self::planet_group(i), 0, cx + radius * angle.cos(), cy + radius * angle.sin());
            }
        }

        // The sun and the planets pull on every ball within range of the sun.
        let Some((sx, sy)) = w.rollover_center("Sun", 0) else { return };
        for ball in 0..w.ball_count() {
            let Some(pos) = w.ball_position_of(ball) else { continue };
            let (dx, dy) = (sx - pos.x, sy - pos.y);
            let sun_dist_sq = dx * dx + dy * dy;
            if sun_dist_sq > GRAVITY_RANGE_SQUARED { continue; }
            let sun_force = SUN_GRAVITY_FORCE / (GRAVITY_DEPTH_SQUARED + sun_dist_sq);
            let sun_angle = dy.atan2(dx);
            let (mut ix, mut iy) = (dt * sun_force * sun_angle.cos(), dt * sun_force * sun_angle.sin());
            for i in 0..self.planets.len() {
                let id = Self::planet_group(i);
                let (Some((px, py)), Some(radius)) = (w.rollover_center(&id, 0), w.rollover_radius(&id, 0)) else { continue };
                let (pdx, pdy) = (px - pos.x, py - pos.y);
                let force = PLANET_GRAVITY_FORCE * radius.powi(3) / (GRAVITY_DEPTH_SQUARED + pdx * pdx + pdy * pdy);
                let angle = pdy.atan2(pdx);
                ix += dt * force * angle.cos();
                iy += dt * force * angle.sin();
            }
            w.apply_impulse_to_ball(ball, Vector::new(ix, iy));
        }
    }

    fn sensor(&mut self, w: &mut PhysicsWorld, ball: usize, id: &str, previous: Option<&str>) {
        // Each ramp scores when the ball comes through its two detectors in order,
        // and counts toward one planet.
        match id {
            "LeftLoopDetector_Trigger" => self.check_ramp(w, ball, previous, "LeftLoopDetector_Enter", 3),
            "RightLoopDetector_Trigger" => self.check_ramp(w, ball, previous, "RightLoopDetector_Enter", 2),
            "OrbitDetector_Left" => self.check_ramp(w, ball, previous, "OrbitDetector_Right", 4),
            "OrbitDetector_Right" => self.check_ramp(w, ball, previous, "OrbitDetector_Left", 4),
            _ => {}
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        if id == "Planet1Targets" {
            w.add_score(PLANET_TARGETS_SCORE);
            self.activate_planet_if_match(w, ball, 0);
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, ball: usize, id: &str) {
        match id {
            "FlipperRollovers" => {
                w.set_all_rollovers(id, false);
                w.increment_multiplier();
            }
            "Planet2Rollovers" => {
                w.set_all_rollovers(id, false);
                w.add_score(PLANET_ROLLOVERS_SCORE);
                self.activate_planet_if_match(w, ball, 1);
            }
            _ => {
                // Rolling over a planet gives the ball that planet's colour; the
                // planet's own status follows in the next tick.
                if let Some(i) = (0..PLANET_BALL_COLORS.len()).find(|&i| id == Self::planet_group(i)) {
                    w.set_ball_color(ball, Some(PLANET_BALL_COLORS[i]));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Table 4: Compound Interest (Field4Delegate)
//
// Every bumper hit adds a percentage to the score multiplier and lights the
// next light in its colour's column of the 6x3 status grid. The top drop bank
// and the flipper lanes raise how much each hit adds; losing the ball resets
// it. The ramp sensors raise and lower the ramp's blocker and kicker. The ball
// locks, multiball and jackpot the grid leads to are round 2.
// ---------------------------------------------------------------------------

struct CompoundInterest {
    /// Percent each bumper hit adds to the multiplier (starts at 1).
    bumper_multiplier_increase: u32,
}

impl CompoundInterest {
    fn raise_increase(&mut self, w: &mut PhysicsWorld, by: u32) {
        self.bumper_multiplier_increase += by;
        w.show_message(&format!("Bumper Multiplier {}%", self.bumper_multiplier_increase), 1500);
    }
}

impl TableScript for CompoundInterest {
    fn handles_rollovers(&self) -> bool { true }

    fn scripted_walls(&self) -> &'static [&'static str] { &["TopRampBlocker"] }

    fn game_started(&mut self, w: &mut PhysicsWorld) {
        // The ramp's kicker starts out of the way; the launch barrier starts open.
        w.set_wall_retracted("RightRampKicker", true);
        w.set_wall_retracted("LaunchBarrier", true);
    }

    fn ball_lost(&mut self, _w: &mut PhysicsWorld) {
        self.bumper_multiplier_increase = 1;
    }

    fn bumper_hit(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        let Some(color) = id.strip_prefix("Bumper.") else { return };
        w.add_multiplier_percent(self.bumper_multiplier_increase as i32);
        // The status grid row is the number of balls locked, which is 0 until
        // the locks exist (round 2).
        w.set_rollover(&format!("Rollovers.{color}"), 0, true);
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        if id == "DropTargetTop" {
            self.raise_increase(w, 1);
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        if id == "FlipperRollovers" {
            self.raise_increase(w, 2);
            w.set_all_rollovers(id, false);
        }
    }

    fn sensor(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str, _previous: Option<&str>) {
        match id {
            "LaunchBarrierRetract" => w.set_wall_retracted("LaunchBarrier", true),
            // Going up the ramp: close the lane behind the ball, open the blocker, extend the kicker.
            "TopRampSensor" => {
                w.set_wall_retracted("LaunchBarrier", false);
                w.set_wall_retracted("TopRampBlocker", true);
                w.set_wall_retracted("RightRampKicker", false);
            }
            "RightRampSensor" => {
                w.set_wall_retracted("TopRampBlocker", false);
                w.set_wall_retracted("RightRampKicker", true);
            }
            "AfterRampKickerSensor" => {
                w.set_wall_retracted("TopRampBlocker", false);
                w.set_wall_retracted("RightRampKicker", false);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Table 5: Eye of Horus (Field5Delegate; the rotating triangle is in movers.rs)
//
// Four ramps each give the ball a colour. The centre diagram has three lights
// per colour that only a ball of that colour can light (they appear while such
// a ball is on the field), and a line joins two neighbouring lit lights. Each
// ramp's drop bank raises that colour's ramp bonus. Lighting every line makes
// multiball ready, and the extra ball, the pyramid and the multiball itself are
// round 2; the extra-ball area is here.
// ---------------------------------------------------------------------------

const COLOR_NAMES: [&str; 4] = ["Blue", "Red", "Yellow", "Green"];
const BALL_COLORS_5: [Rgb; 4] = [(0x66, 0x88, 0xEE), (0xEE, 0x88, 0x88), (0xCC, 0xCC, 0x77), (0x77, 0xDD, 0x77)];
const WALL_COLORS_5: [Rgb; 4] = [(0x00, 0x66, 0xFF), (0xCC, 0x00, 0x00), (0xCC, 0xCC, 0x00), (0x00, 0xCC, 0x00)];
const DISABLED_COLORS_5: [Rgb; 4] = [(0x00, 0x33, 0x66), (0x66, 0x00, 0x00), (0x66, 0x66, 0x00), (0x00, 0x66, 0x00)];
const RAMP_BASE_SCORE_5: u64 = 5000;

struct EyeOfHorus {
    /// Percent added to each colour's ramp score (10 per cleared ramp bank).
    ramp_bonus: [u32; 4],
}

impl EyeOfHorus {
    fn center_rollover(color: usize, n: usize) -> String { format!("CenterRollover_{}_{}", COLOR_NAMES[color], n + 1) }

    fn center_line(color: usize, n: usize) -> String {
        format!("CenterLine_{}_{}_{}", COLOR_NAMES[color], n + 1, (n + 1) % 3 + 1)
    }

    fn all_lit_for_color(w: &PhysicsWorld, color: usize) -> bool {
        (0..3).all(|n| w.rollovers_all_lit(&Self::center_rollover(color, n)))
    }

    /// A ramp counts when the ball came through its other detector first. It
    /// gives the ball the ramp's colour unless a ball already has it or the
    /// colour is complete, and scores 5000 plus the colour's bonus (doubled with
    /// more than one ball in play).
    fn check_ramp(&self, w: &mut PhysicsWorld, ball: usize, previous: Option<&str>, required: &str, color: usize) {
        if previous != Some(required) { return; }
        if !w.any_ball_has_color(BALL_COLORS_5[color]) && !Self::all_lit_for_color(w, color) {
            w.set_ball_color(ball, Some(BALL_COLORS_5[color]));
        }
        let mut percent = 100 + self.ramp_bonus[color] as u64;
        if w.ball_count() > 1 { percent *= 2; }
        w.add_score(RAMP_BASE_SCORE_5 / 100 * percent);
    }

    /// Back at the extra-ball exit: if the extra ball was won, light it out and
    /// put the four coloured barriers back.
    fn reset_extra_ball(&self, w: &mut PhysicsWorld) {
        if w.rollovers_all_lit("ExtraBallRollover") {
            w.set_all_rollovers("ExtraBallRollover", false);
            for name in COLOR_NAMES {
                w.set_wall_retracted(&format!("ExtraBallBarrier_{name}"), false);
            }
        }
    }
}

impl TableScript for EyeOfHorus {
    fn handles_rollovers(&self) -> bool { true }

    fn tick(&mut self, w: &mut PhysicsWorld, _dt: f32) {
        // The centre lights answer only to a ball of their colour, and show only
        // while such a ball is on the field (dim if already lit).
        for color in 0..4 {
            let has = w.any_ball_has_color(BALL_COLORS_5[color]);
            for n in 0..3 {
                let id = Self::center_rollover(color, n);
                w.set_group_ignore_ball(&id, !has);
                if has {
                    w.set_rollovers_visible(&id, true);
                    w.set_rollover_color(&id, 0, Some(WALL_COLORS_5[color]));
                } else {
                    let lit = w.rollovers_all_lit(&id);
                    w.set_rollovers_visible(&id, lit);
                    if lit { w.set_rollover_color(&id, 0, Some(DISABLED_COLORS_5[color])); }
                }
            }
            // A line joins each pair of neighbouring lit lights.
            for n in 0..3 {
                let both = w.rollovers_all_lit(&Self::center_rollover(color, n))
                    && w.rollovers_all_lit(&Self::center_rollover(color, (n + 1) % 3));
                w.set_decor_visible(&Self::center_line(color, n), both);
            }
        }
    }

    fn game_started(&mut self, w: &mut PhysicsWorld) {
        // The diagram starts hidden; the barriers start in place.
        for color in 0..4 {
            for n in 0..3 {
                w.set_rollovers_visible(&Self::center_rollover(color, n), false);
                w.set_decor_visible(&Self::center_line(color, n), false);
            }
        }
        for name in COLOR_NAMES {
            w.set_wall_retracted(&format!("ExtraBallBarrier_{name}"), false);
        }
    }

    fn sensor(&mut self, w: &mut PhysicsWorld, ball: usize, id: &str, previous: Option<&str>) {
        match id {
            "RampSensor_OuterLeftTop" => self.check_ramp(w, ball, previous, "RampSensor_OuterRightTop", 3),
            "RampSensor_OuterRightTop" => self.check_ramp(w, ball, previous, "RampSensor_OuterLeftTop", 0),
            "RampSensor_LeftTop" => self.check_ramp(w, ball, previous, "RampSensor_LeftMiddle", 1),
            "RampSensor_RightTop" => self.check_ramp(w, ball, previous, "RampSensor_RightMiddle", 2),
            "Sensor_ExtraBallExit" => self.reset_extra_ball(w),
            _ => {}
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        for (color, name) in COLOR_NAMES.iter().enumerate() {
            if id == format!("DropTargets_{name}Ramp") {
                self.ramp_bonus[color] += 10;
                w.show_message(&format!("{name} ramp +{}%", self.ramp_bonus[color]), 1500);
            }
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        match id {
            "FlipperRollovers" => {
                w.set_all_rollovers(id, false);
                w.increment_multiplier();
            }
            "ExtraBallRollover" => {
                w.add_extra_ball();
                w.show_message("Extra Ball!", 3000);
            }
            _ => {}
        }
    }
}
