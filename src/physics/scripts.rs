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

use super::stars::{self, Mode, StarState};
use super::{PhysicsWorld, Shape};
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
    /// One target of a bank was hit (the original's `processCollision` for drop targets).
    fn drop_target_hit(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str) {}
    fn spinner_activated(&mut self, _w: &mut PhysicsWorld, _ball: usize, _id: &str) {}
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
        Some("Field3Delegate") => Some(Box::new(Ringmaster::new())),
        Some("Field4Delegate") => Some(Box::new(CompoundInterest { bumper_multiplier_increase: 1 })),
        Some("Field5Delegate") => Some(Box::new(EyeOfHorus { ramp_bonus: [0; 4] })),
        Some("Field6Delegate") => Some(Box::new(Orbiters::new(clock_seed()))),
        Some("Field7Delegate") => Some(Box::new(AdAstra::new(clock_seed()))),
        Some("Field8Delegate") => Some(Box::new(PoolParty::new())),
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

// ---------------------------------------------------------------------------
// Table 3: Ringmaster (Field3Delegate)
//
// Hitting a bumper scores 500 and charges a heat meter shown by the centre
// ring. Two target banks upgrade the bumper bonus (a stronger multiplier, a
// longer duration) and light their own rings. A ramp rollover starts the
// bonus: for a while each bumper hit scores extra, and the bumpers glow from
// red back down to blue as it runs out. A full meter and both banks done light
// the multiball-ready ring; the multiball itself is round 2.
// ---------------------------------------------------------------------------

const BASE_BUMPER_SCORE: u64 = 500;
const BASE_BONUS_SECONDS: f32 = 15.0;
const BASE_BONUS_MULTIPLIER: f32 = 5.0;
const MAX_BUMPER_ENERGY: f32 = 200.0;
const MAX_UPPER_GROUPS: u32 = 2;
const MAX_LOWER_GROUPS: u32 = 5;

/// The 53-entry heat palette: blue, cyan, green, yellow, red.
fn temperature_palette() -> Vec<Rgb> {
    let mut p = Vec::with_capacity(53);
    for i in 0..16u8 { p.push((0, 16 * i, 255)); }          // blue to cyan
    for i in 0..8u8 { p.push((0, 255, 240 - 32 * i)); }     // cyan to green
    for i in 0..8u8 { p.push((16 + 32 * i, 255, 0)); }      // green to yellow
    for i in 0..21u8 { p.push((255, 240 - 12 * i, 0)); }    // yellow to red
    p
}

struct Ringmaster {
    palette: Vec<Rgb>,
    bonus_active: bool,
    bonus_elapsed: f32,
    bonus_duration: f32,
    bonus_multiplier: f32,
    upper_groups: u32,
    lower_groups: u32,
    energy: f32,
}

impl Ringmaster {
    fn new() -> Self {
        Ringmaster {
            palette: temperature_palette(),
            bonus_active: false,
            bonus_elapsed: 0.0,
            bonus_duration: BASE_BONUS_SECONDS,
            bonus_multiplier: BASE_BONUS_MULTIPLIER,
            upper_groups: 0,
            lower_groups: 0,
            energy: 0.0,
        }
    }

    /// Rounded like the original's `Math.round((len - 1) * ratio)`.
    fn color_for_ratio(&self, ratio: f32) -> Rgb {
        let last = self.palette.len() - 1;
        if ratio <= 0.0 { return self.palette[0]; }
        if ratio >= 1.0 { return self.palette[last]; }
        self.palette[((last as f32) * ratio + 0.5).floor() as usize]
    }

    fn reset_bonuses(&mut self, w: &mut PhysicsWorld) {
        self.bonus_duration = BASE_BONUS_SECONDS;
        self.bonus_multiplier = BASE_BONUS_MULTIPLIER;
        self.energy = 0.0;
        self.upper_groups = 0;
        self.lower_groups = 0;
        for ring in ["UpperTargetIndicator", "LowerTargetIndicator", "BumperIndicator"] {
            w.set_rollover_color(ring, 0, None);
        }
        w.set_all_rollovers("BumperIndicator", false);
    }

    fn end_bonus(&mut self, w: &mut PhysicsWorld) {
        self.bonus_active = false;
        w.set_bumper_colors(None);
    }

    /// Meter full and both banks upgraded all the way: the ready ring lights.
    fn check_multiball_ready(&self, w: &mut PhysicsWorld) {
        if self.energy >= MAX_BUMPER_ENERGY && self.upper_groups >= MAX_UPPER_GROUPS && self.lower_groups >= MAX_LOWER_GROUPS {
            w.set_all_rollovers("BumperIndicator", true);
        }
    }
}

impl TableScript for Ringmaster {
    fn handles_rollovers(&self) -> bool { true }

    fn game_started(&mut self, w: &mut PhysicsWorld) {
        self.reset_bonuses(w);
    }

    fn ball_lost(&mut self, w: &mut PhysicsWorld) {
        self.end_bonus(w);
    }

    fn tick(&mut self, w: &mut PhysicsWorld, dt: f32) {
        if !self.bonus_active { return; }
        self.bonus_elapsed += dt;
        if self.bonus_elapsed >= self.bonus_duration {
            self.end_bonus(w);
        } else {
            let remaining = 1.0 - self.bonus_elapsed / self.bonus_duration;
            w.set_bumper_colors(Some(self.color_for_ratio(remaining)));
        }
    }

    fn bumper_hit(&mut self, w: &mut PhysicsWorld, _ball: usize, _id: &str) {
        // The bumpers' JSON score is 0; each hit scores 500, plus a bonus that
        // fades over the bonus's duration, rounded to the nearest 10.
        let mut extra_energy = 0.0;
        let mut score = BASE_BUMPER_SCORE;
        if self.bonus_active {
            let remaining = 1.0 - self.bonus_elapsed / self.bonus_duration;
            extra_energy = remaining * self.bonus_multiplier;
            score += ((BASE_BUMPER_SCORE as f32 * extra_energy / 10.0 + 0.5).floor() as u64) * 10;
        }
        w.add_score(score);
        self.energy = (self.energy + 1.0 + extra_energy).min(MAX_BUMPER_ENERGY);
        w.set_rollover_color("BumperIndicator", 0, Some(self.color_for_ratio(self.energy / MAX_BUMPER_ENERGY)));
        self.check_multiball_ready(w);
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        match id {
            // The lower bank lengthens the bonus by a second, up to five times.
            "LowerMultiballTargets" if self.lower_groups < MAX_LOWER_GROUPS => {
                self.bonus_duration += 1.0;
                self.lower_groups += 1;
                let ratio = self.lower_groups as f32 / MAX_LOWER_GROUPS as f32;
                w.set_rollover_color("LowerTargetIndicator", 0, Some(self.color_for_ratio(ratio)));
                self.check_multiball_ready(w);
            }
            // The upper bank raises the bonus multiplier, up to twice.
            "UpperMultiballTargets" if self.upper_groups < MAX_UPPER_GROUPS => {
                self.bonus_multiplier += 1.0;
                self.upper_groups += 1;
                let ratio = self.upper_groups as f32 / MAX_UPPER_GROUPS as f32;
                w.set_rollover_color("UpperTargetIndicator", 0, Some(self.color_for_ratio(ratio)));
                self.check_multiball_ready(w);
            }
            _ => {}
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        if id == "LeftRampRollover" || id == "RightRampRollover" {
            // A ramp starts (or restarts) the bumper bonus. (Multiball: round 2.)
            self.bonus_active = true;
            self.bonus_elapsed = 0.0;
        } else {
            w.set_all_rollovers(id, false);
            w.increment_multiplier();
        }
    }
}

#[cfg(test)]
pub(super) fn temperature_palette_for_test() -> Vec<(u8, u8, u8)> { temperature_palette() }

// ---------------------------------------------------------------------------
// Table 8: Pool Party (Field8Delegate)
//
// Sink the pool balls (the right-hand bank) and then the 8-ball. Each sunk
// ball lights a bonus insert worth 7000 at the end of the ball; a full rack is
// worth 56000 more every ball after. The inline targets raise the end-of-ball
// bonus multiplier, the lane rollovers raise the spinner and the 8-ball value,
// and holding the left flipper while launching gives one shot at the 8-ball
// (the "breakshot"). Three gates open and close on timers. No multiball here.
// ---------------------------------------------------------------------------

const TOKEN_LAUNCH_GATE_CLOSE: u32 = 10;
const TOKEN_LEFT_GATE_CLOSE: u32 = 11;
const TOKEN_RIGHT_GATE_CLOSE: u32 = 12;
const TOKEN_RIGHT_BANK_UP: u32 = 13;
/// 20 + i lights insert i of the right bank during the light show.
const TOKEN_LIGHT_SHOW: u32 = 20;

const BALL_BONUS: u64 = 7_000;
const RACK_BONUS: u64 = 56_000;
const BASE_EIGHT_BALL_VALUE: u64 = 20_000;

/// 20000 as "20,000".
pub(super) fn with_commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out
}

struct PoolParty {
    racks_completed: u64,
    bonus_multiplier: u64,
    eight_ball_value: u64,
    right_bank_completed: bool,
    skill_shot_active: bool,
}

impl PoolParty {
    fn new() -> Self {
        PoolParty { racks_completed: 0, bonus_multiplier: 1, eight_ball_value: BASE_EIGHT_BALL_VALUE, right_bank_completed: false, skill_shot_active: false }
    }

    fn reset_variables(&mut self, first_ball: bool) {
        if first_ball { self.racks_completed = 0; }
        self.bonus_multiplier = 1;
        self.eight_ball_value = BASE_EIGHT_BALL_VALUE;
        self.right_bank_completed = false;
        self.skill_shot_active = false;
    }

    /// The right bank's inserts light one by one and the bank comes back up.
    fn right_bank_light_show(w: &mut PhysicsWorld) {
        for i in 0..7 {
            w.schedule(500 + 100 * i as u32, TOKEN_LIGHT_SHOW + i as u32);
        }
        w.schedule(1200, TOKEN_RIGHT_BANK_UP);
    }

    fn reset_elements(&mut self, w: &mut PhysicsWorld, first_ball: bool) {
        // The sunk balls carry over from ball to ball unless the bank was completed.
        if self.right_bank_completed || first_ball {
            w.set_all_rollovers("BonusInserts", false);
            self.right_bank_completed = false;
            Self::right_bank_light_show(w);
        }
        w.set_all_rollovers("LaneRollovers", false);
        w.set_all_rollovers("BonusCollectSaucer", false);
        w.set_rollovers_visible("BonusCollectSaucer", false);
        w.set_rollovers_visible("LeftOutlane", false);
        w.set_rollovers_visible("RightOutlane", false);

        w.set_all_rollovers("EightBallInsert", false);
        w.set_all_rollovers("BonusMultiplierInserts", false);
        w.set_all_rollovers("LeftOrbitValueInserts", false);
        w.activate_first_unlit_rollover("LeftOrbitValueInserts");
        w.set_all_rollovers("EightBallValueInserts", false);
        w.activate_first_unlit_rollover("EightBallValueInserts");

        w.make_targets_up("EightBallTarget");
        w.make_targets_up("InlineTargets");
        w.set_spinner_score("LeftOrbitSpinner", 500);
    }

    /// The end-of-ball bonus: 7000 per sunk ball, plus 56000 per rack, times the multiplier.
    fn bonus_total(&self, w: &PhysicsWorld) -> u64 {
        (self.racks_completed * RACK_BONUS + w.lit_count("BonusInserts") as u64 * BALL_BONUS) * self.bonus_multiplier
    }

    /// Sinks the whole rack: the eight-ball shot at the breakshot or with the bank done.
    fn collect_rack(&mut self, w: &mut PhysicsWorld) {
        w.set_all_rollovers("BonusInserts", true);
        w.set_all_rollovers("RightBankInserts", false);
        let mut knocked_down = 0u64;
        for i in 0..7 {
            if !w.drop_target_down("RightBankTargets", i) {
                knocked_down += 1;
                w.set_drop_target_down("RightBankTargets", i, true);
            }
        }
        w.add_score(3000 * knocked_down);
        w.add_score(self.eight_ball_value);
        self.skill_shot_active = false;
        self.right_bank_completed = true;
        w.set_rollovers_visible("BonusCollectSaucer", true);
        self.racks_completed += 1;
    }
}

impl TableScript for PoolParty {
    fn handles_rollovers(&self) -> bool { true }

    fn scripted_walls(&self) -> &'static [&'static str] { &["LaunchGate"] }

    fn game_started(&mut self, w: &mut PhysicsWorld) {
        // The launch gate starts retracted (the table marks it disabled).
        w.set_wall_retracted("LaunchGate", true);
        self.reset_elements(w, true);
        self.reset_variables(true);
    }

    fn ball_lost(&mut self, w: &mut PhysicsWorld) {
        let bonus = self.bonus_total(w);
        w.add_score(bonus);
        w.show_message(&format!("Bonus: {}", with_commas(bonus)), 3000);
        self.reset_elements(w, false);
        self.reset_variables(false);
    }

    fn drop_target_hit(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        match id {
            // Each sunk ball moves its light from the bank's row to the bonus row (mirrored).
            "RightBankTargets" => {
                for i in 0..7 {
                    if w.drop_target_down("RightBankTargets", i) {
                        w.set_rollover("RightBankInserts", 6 - i, false);
                        w.set_rollover("BonusInserts", 6 - i, true);
                    }
                }
            }
            // The inline targets raise the end-of-ball bonus multiplier, up to 5.
            "InlineTargets" => {
                if self.bonus_multiplier != 5 { self.bonus_multiplier += 1; }
                w.activate_first_unlit_rollover("BonusMultiplierInserts");
            }
            _ => {}
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        match id {
            "EightBallTarget" => {
                if self.right_bank_completed {
                    self.collect_rack(w);
                    w.show_message("Bonus Collect Lit", 3000);
                } else if self.skill_shot_active {
                    self.collect_rack(w);
                    w.show_message("Breakshot!", 3000);
                } else {
                    // A plain corner pocket: the target comes straight back up.
                    w.make_targets_up("EightBallTarget");
                    w.add_score(self.eight_ball_value);
                    w.show_message(&format!("Corner Pocket: {}", with_commas(self.eight_ball_value)), 3000);
                }
            }
            "RightBankTargets" if !self.right_bank_completed => {
                self.right_bank_completed = true;
                w.set_rollover("EightBallInsert", 0, true);
                w.show_message("Shoot the Eight Ball!", 3000);
            }
            _ => {}
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        match id {
            "LaneRollovers" => {
                w.show_message("Lanes Completed", 3000);
                w.set_all_rollovers("LaneRollovers", false);
                // Each completion raises the spinner and the eight-ball value, up to 1500 a spin.
                let spin = w.spinner_score("LeftOrbitSpinner");
                if spin < 1500 {
                    w.set_spinner_score("LeftOrbitSpinner", spin + 500);
                    w.activate_first_unlit_rollover("LeftOrbitValueInserts");
                    self.eight_ball_value += 20_000;
                    w.activate_first_unlit_rollover("EightBallValueInserts");
                }
            }
            "BonusCollectSaucer" => {
                let bonus = self.bonus_total(w);
                w.add_score(bonus);
                w.show_message(&format!("Bonus Collected: {}", with_commas(bonus)), 3000);
            }
            "LeftOutlane" | "RightOutlane" => w.show_message("Scratch!", 3000),
            _ => {}
        }
    }

    fn spinner_activated(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        if id == "LeftOrbitSpinner" {
            // The left orbit's gate opens for two seconds.
            w.set_wall_retracted("LeftGate", true);
            w.schedule(2000, TOKEN_LEFT_GATE_CLOSE);
        }
    }

    fn sensor(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str, _previous: Option<&str>) {
        match id {
            "LaunchGateRetract" => {
                w.set_wall_retracted("LaunchGate", true);
                w.schedule(1000, TOKEN_LAUNCH_GATE_CLOSE);
                // Holding the left flipper as the ball is launched arms the breakshot.
                if w.left_flipper_held() {
                    self.skill_shot_active = true;
                    w.set_wall_retracted("LeftGate", true);
                    w.schedule(2000, TOKEN_LEFT_GATE_CLOSE);
                }
            }
            "RightGateRetract" => {
                w.set_wall_retracted("RightGate", true);
                w.schedule(1000, TOKEN_RIGHT_GATE_CLOSE);
            }
            "EndSkillShot" => self.skill_shot_active = false,
            _ => {}
        }
    }

    fn scheduled(&mut self, w: &mut PhysicsWorld, token: u32) {
        match token {
            TOKEN_LAUNCH_GATE_CLOSE => w.set_wall_retracted("LaunchGate", false),
            TOKEN_LEFT_GATE_CLOSE => w.set_wall_retracted("LeftGate", false),
            TOKEN_RIGHT_GATE_CLOSE => w.set_wall_retracted("RightGate", false),
            TOKEN_RIGHT_BANK_UP => w.make_targets_up("RightBankTargets"),
            t if (TOKEN_LIGHT_SHOW..TOKEN_LIGHT_SHOW + 7).contains(&t) => {
                w.set_rollover("RightBankInserts", (t - TOKEN_LIGHT_SHOW) as usize, true);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Table 7: Ad Astra (Field7Delegate)
//
// The circle in the middle of the table is a map of the real sky. It drifts
// slowly across the stars until a ball shoots one of the loops, which flies the
// view to a random constellation. Rolling the ball over that constellation's
// stars lights them (500 each, drawing the lines between lit neighbours) and
// finishing it makes a ball lock ready. The locks, the multiball, the jackpot
// and the layered mini-table are round 2.
// ---------------------------------------------------------------------------

const STAR_SCORE: u64 = 500;
const BASE_RAMP_SCORE: u64 = 5000;
const RAMP_SCORE_INCREMENT: u64 = 1000;
/// Seconds for one pulse of the guide arrows.
const GUIDE_PERIOD: f64 = 5.0;
const LOOP_GUIDE_COLOR: Rgb = (0x00, 0xAA, 0x66);
const LOCK_GUIDE_COLOR: Rgb = (0xCC, 0xCC, 0x00);

const ACTIVE_STAR_ACTIVE_CONSTELLATION: Rgb = (240, 240, 0);
const INACTIVE_STAR_ACTIVE_CONSTELLATION: Rgb = (0, 240, 0);
const ACTIVE_STAR_INACTIVE_CONSTELLATION: Rgb = (192, 192, 0);
const INACTIVE_STAR_INACTIVE_CONSTELLATION: Rgb = (240, 240, 240);
/// The original's line colour is white at alpha 192.
const CONSTELLATION_LINE: Rgb = (180, 180, 180);

pub(super) struct AdAstra {
    stars: StarState,
    seed: u32,
    ramp_score: u64,
    guide_seconds: f64,
    /// Centre and radius of the star circle (the `StarViewBoundary` rollover), read on the first tick.
    view: Option<(f32, f32, f32)>,
}

impl AdAstra {
    pub(super) fn new(seed: u32) -> Self {
        AdAstra { stars: StarState::new(), seed, ramp_score: BASE_RAMP_SCORE, guide_seconds: 0.0, view: None }
    }

    fn star_color(&self, star: usize) -> Rgb {
        let active = self.stars.activated.contains(&star);
        let in_current = self.stars.current_constellation().map_or(false, |c| c.star_indices.contains(&star));
        match (in_current, active) {
            (true, true) => ACTIVE_STAR_ACTIVE_CONSTELLATION,
            (true, false) => INACTIVE_STAR_ACTIVE_CONSTELLATION,
            (false, true) => ACTIVE_STAR_INACTIVE_CONSTELLATION,
            (false, false) => INACTIVE_STAR_INACTIVE_CONSTELLATION,
        }
    }

    /// The dots and lines of the current view: brighter stars are bigger and
    /// more opaque, and drawn last so they sit on top.
    fn shapes(&self) -> Vec<Shape> {
        let Some((cx, cy, view_radius)) = self.view else { return Vec::new() };
        let target_radius = self.stars.target.angular_radius as f32;
        if target_radius <= 0.0 { return Vec::new(); }
        let scale = view_radius / target_radius;
        let base_radius = view_radius * 0.015;
        let mut shapes = Vec::with_capacity(self.stars.projection.len() + 16);
        for p in &self.stars.projection {
            let alpha = if p.magnitude <= 0.0 { 255 } else { ((255.0 - 30.0 * p.magnitude) as i32).max(0) };
            let (r, g, b) = self.star_color(p.index);
            let dim = |v: u8| (v as i32 * alpha / 255) as u8;
            let rmul = if p.magnitude <= 0.0 { 1.5 } else if p.magnitude >= 4.0 { 0.75 } else { 1.0 };
            shapes.push(Shape::Circle {
                x: cx + p.x as f32 * scale,
                y: cy + p.y as f32 * scale,
                radius: rmul * base_radius,
                color: (dim(r), dim(g), dim(b)),
            });
        }
        shapes.reverse(); // brightest (lowest magnitude, first in the catalog) last
        for c in stars::constellations() {
            for &star in &c.star_indices {
                if !self.stars.activated.contains(&star) { continue; }
                let Some(ends) = c.segments_by_index.get(&star) else { continue };
                for &end in ends {
                    if !self.stars.activated.contains(&end) { continue; }
                    let (Some(a), Some(b)) = (self.stars.projected(star), self.stars.projected(end)) else { continue };
                    shapes.push(Shape::Line {
                        x1: cx + a.x as f32 * scale, y1: cy + a.y as f32 * scale,
                        x2: cx + b.x as f32 * scale, y2: cy + b.y as f32 * scale,
                        color: CONSTELLATION_LINE,
                    });
                }
            }
        }
        shapes
    }

    /// Lights stars of the current constellation near any ball on the main layer.
    fn update_activated_stars(&mut self, w: &mut PhysicsWorld) {
        let Some((cx, cy, view_radius)) = self.view else { return };
        let mut new_stars = 0;
        for ball in 0..w.ball_count() {
            if w.ball_layer_of(ball) != 0 { continue; }
            let Some(pos) = w.ball_position_of(ball) else { continue };
            let (bx, by) = (((pos.x - cx) / view_radius) as f64, ((pos.y - cy) / view_radius) as f64);
            new_stars += self.stars.activate_near(bx, by);
        }
        if new_stars > 0 {
            w.add_score(STAR_SCORE * new_stars as u64);
            if self.stars.all_stars_in_current_active() {
                // A ball lock is ready (the locks themselves are round 2).
                w.show_message("Ball Lock Ready", 2000);
            }
        }
    }

    /// The loop and lock guide arrows pulse while they are useful.
    fn update_guides(&mut self, w: &mut PhysicsWorld, dt: f32) {
        self.guide_seconds = (self.guide_seconds + dt as f64) % GUIDE_PERIOD;
        let pulse = (1.0 + (std::f64::consts::TAU * self.guide_seconds / GUIDE_PERIOD).sin()) / 2.0;
        let alpha = (75.0 + 180.0 * pulse) as u32;
        let (mut loop_alpha, mut lock_alpha) = (0, 0);
        if self.stars.all_stars_in_current_active() {
            lock_alpha = alpha; // lock ready
        } else if self.stars.mode == Mode::Wandering {
            loop_alpha = alpha; // shoot a loop to pick a constellation
        }
        let shade = |c: Rgb, a: u32| ((c.0 as u32 * a / 255) as u8, (c.1 as u32 * a / 255) as u8, (c.2 as u32 * a / 255) as u8);
        for id in ["LeftLoopGuide", "RightLoopGuide"] {
            w.set_decor_visible(id, loop_alpha > 0);
            w.set_decor_color(id, shade(LOOP_GUIDE_COLOR, loop_alpha));
        }
        w.set_decor_visible("LockAndJackpotGuide", lock_alpha > 0);
        w.set_decor_color("LockAndJackpotGuide", shade(LOCK_GUIDE_COLOR, lock_alpha));
    }

    fn handle_loop(&mut self, w: &mut PhysicsWorld) {
        w.add_score(self.ramp_score);
        if !self.stars.all_stars_in_current_active() {
            let mut seed = self.seed;
            let picked = {
                let mut rng = || {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 8) as f64 / (1u32 << 24) as f64
                };
                self.stars.switch_to_random_unlocked(&mut rng)
            };
            self.seed = seed;
            if picked {
                if let Some(c) = self.stars.current_constellation() {
                    w.show_message(c.name, 3000);
                }
            }
        }
    }
}

impl TableScript for AdAstra {
    fn handles_rollovers(&self) -> bool { true }

    fn tick(&mut self, w: &mut PhysicsWorld, dt: f32) {
        if self.view.is_none() {
            if let (Some((x, y)), Some(r)) = (w.rollover_center("StarViewBoundary", 0), w.rollover_radius("StarViewBoundary", 0)) {
                self.view = Some((x, y, r));
            }
        }
        self.stars.tick(dt as f64);
        self.update_activated_stars(w);
        self.update_guides(w, dt);
        w.set_shapes(self.shapes());
    }

    fn sensor(&mut self, w: &mut PhysicsWorld, ball: usize, id: &str, previous: Option<&str>) {
        match id {
            // Each loop counts once per pass, not once per step the ball is inside.
            "LeftLoopDetector" | "RightLoopDetector" if previous != Some(id) => self.handle_loop(w),
            // The ball dropping into an inlane is stopped dead at the top of it.
            "LeftFlipperDropSensor" | "RightFlipperDropSensor" => w.set_ball_velocity(ball, 0.0, 0.0),
            // Round the centre bumpers, either way.
            "InnerOrbitLeftTrigger" if previous == Some("InnerOrbitRightTrigger") => w.add_score(self.ramp_score),
            "InnerOrbitRightTrigger" if previous == Some("InnerOrbitLeftTrigger") => w.add_score(self.ramp_score),
            _ => {}
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        announce_saver(w, "DropTargetLeftSave", "DropTargetRightSave", id);
        if id == "MiniFieldTopTargets" || id == "MiniFieldLeftTargets" {
            self.ramp_score += RAMP_SCORE_INCREMENT;
            w.show_message("Ramp bonus increased", 1500);
        }
    }

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, _ball: usize, id: &str) {
        if id == "FlipperRollovers" || id == "TopRollovers" {
            w.set_all_rollovers(id, false);
            w.increment_multiplier();
        }
    }
}
