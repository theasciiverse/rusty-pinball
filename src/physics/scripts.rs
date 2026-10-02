// Per-table rules, ported from the original Vector Pinball's delegate classes
// (fields/Field1Delegate.java and so on). A script reacts to events the physics
// world reports (a bumper was hit, a drop bank cleared, a rollover group was
// completed by a ball, a scheduled timer fired) and acts on the world through
// its helper methods: show a message, light rollovers, launch another ball,
// re-arm a ball saver, raise the multiplier.
//
// Scripts are plain Rust, selected by the table's `delegate` name. A table
// with no script plays with the generic element behavior only.

use super::PhysicsWorld;
use crate::table::TableDef;

pub(super) trait TableScript {
    /// When true, rollover groups completed by a ball are the script's business:
    /// the world does not flash, pay a completion bonus or reset them on its own
    /// (the original leaves all of that to the delegate).
    fn handles_rollovers(&self) -> bool { false }

    fn bumper_hit(&mut self, _w: &mut PhysicsWorld, _id: &str) {}
    fn drop_bank_cleared(&mut self, _w: &mut PhysicsWorld, _id: &str) {}
    /// A ball lit the last rollover of a group (not called for script-lit groups).
    fn rollover_group_completed(&mut self, _w: &mut PhysicsWorld, _id: &str) {}
    /// A timer from `PhysicsWorld::schedule` fired.
    fn scheduled(&mut self, _w: &mut PhysicsWorld, _token: u32) {}
}

pub(super) fn script_for(table: &TableDef) -> Option<Box<dyn TableScript>> {
    match table.delegate.as_deref() {
        Some("Field1Delegate") => Some(Box::new(Classic)),
        Some("Field2Delegate") => Some(Box::new(RoundAndRound)),
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

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, id: &str) {
        // Every group completion clears the group and raises the multiplier.
        w.set_all_rollovers(id, false);
        w.increment_multiplier();
        // Shooting the ramp with all the extra-ball lights lit starts multiball.
        if id == "RampRollovers" && w.rollovers_all_lit("ExtraBallRollovers") {
            w.set_all_rollovers("ExtraBallRollovers", false);
            start_multiball(w);
        }
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, id: &str) {
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

    fn rollover_group_completed(&mut self, w: &mut PhysicsWorld, id: &str) {
        w.set_all_rollovers(id, false);
        w.increment_multiplier();
    }

    fn drop_bank_cleared(&mut self, w: &mut PhysicsWorld, id: &str) {
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

    fn bumper_hit(&mut self, w: &mut PhysicsWorld, id: &str) {
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
