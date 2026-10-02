// Table elements that move on their own, ported from the original Vector
// Pinball's per-table delegate scripts (their `tick` methods). Positions are
// driven in sim time, so they scale with the table's targetTimeRatio like
// everything else.

use rapier2d::prelude::*;
use std::f32::consts::TAU;

use super::BumperInfo;
use crate::table::TableDef;

/// Bumpers spread evenly around a circle that rotates at a constant speed
/// (original: `RotatingGroup` in Field2Delegate).
#[derive(Clone)]
pub(super) struct RotatingGroup {
    /// Indices into the world's bumpers, in the order the original lists them.
    bumpers: Vec<usize>,
    center: [f32; 2],
    radius: f32,
    angle: f32,
    /// Radians per sim second; negative turns clockwise.
    speed: f32,
}

impl RotatingGroup {
    /// Like the original: the radius and starting angle come from where the
    /// first bumper sits relative to the center.
    fn new(bumpers: Vec<usize>, center: [f32; 2], speed: f32, all: &[BumperInfo]) -> Self {
        let first = all[bumpers[0]].position;
        let (dx, dy) = (first[0] - center[0], first[1] - center[1]);
        RotatingGroup {
            bumpers,
            center,
            radius: dx.hypot(dy),
            angle: dy.atan2(dx),
            speed,
        }
    }

    /// Bumper `i` sits at `angle + i * (TAU / n)`.
    fn position_of(&self, i: usize) -> [f32; 2] {
        let a = self.angle + (TAU / self.bumpers.len() as f32) * i as f32;
        [self.center[0] + self.radius * a.cos(), self.center[1] + self.radius * a.sin()]
    }
}

fn find_bumper(bumpers: &[BumperInfo], id: &str) -> Option<usize> {
    bumpers.iter().position(|b| b.shot_id.as_deref() == Some(id))
}

fn find_all(bumpers: &[BumperInfo], ids: &[&str]) -> Option<Vec<usize>> {
    ids.iter().map(|id| find_bumper(bumpers, id)).collect()
}

/// Builds the rotating groups a table's delegate defines. Missing ids or
/// variables just mean no motion, so a table edited in the editor never panics.
pub(super) fn build_rotating_groups(table: &TableDef, bumpers: &[BumperInfo]) -> Vec<RotatingGroup> {
    let mut groups = Vec::new();
    if table.delegate.as_deref() == Some("Field2Delegate") {
        // Round and Round: four bumpers circle the center bumper, two more
        // circle a point given by the table's variables.
        if let (Some(ids), Some(center), Some(speed)) = (
            find_all(bumpers, &["RotatingBumper1A", "RotatingBumper1B", "RotatingBumper1C", "RotatingBumper1D"]),
            find_bumper(bumpers, "CenterBumper1"),
            table.variable("RotatingBumper1Speed"),
        ) {
            groups.push(RotatingGroup::new(ids, bumpers[center].position, speed, bumpers));
        }
        if let (Some(ids), Some(cx), Some(cy), Some(speed)) = (
            find_all(bumpers, &["RotatingBumper2A", "RotatingBumper2B"]),
            table.variable("RotatingBumper2CenterX"),
            table.variable("RotatingBumper2CenterY"),
            table.variable("RotatingBumper2Speed"),
        ) {
            groups.push(RotatingGroup::new(ids, [cx, cy], speed, bumpers));
        }
    }
    groups
}

/// Advances every group by `dt` sim seconds and moves its bumpers (their
/// bodies, so the ball collides with them where they are drawn).
pub(super) fn advance_rotating_groups(
    groups: &mut [RotatingGroup],
    dt: f32,
    bumpers: &mut [BumperInfo],
    bodies: &mut RigidBodySet,
) {
    for g in groups {
        g.angle = (g.angle + dt * g.speed).rem_euclid(TAU);
        for (i, &bi) in g.bumpers.iter().enumerate() {
            let pos = g.position_of(i);
            bumpers[bi].position = pos;
            if let Some(body) = bodies.get_mut(bumpers[bi].body) {
                body.set_translation(Vector::new(pos[0], pos[1]), false);
            }
        }
    }
}
