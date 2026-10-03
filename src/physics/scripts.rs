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
