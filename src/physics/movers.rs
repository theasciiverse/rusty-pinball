// Table elements that move on their own, ported from the original Vector
// Pinball's per-table delegate scripts (their `tick` methods). Positions are
// driven in sim time, so they scale with the table's targetTimeRatio like
// everything else.

use rapier2d::prelude::*;
use std::f32::consts::TAU;

use super::{BumperInfo, MovingWallInfo};
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

/// Three walls forming an equilateral triangle that spins about a point
/// (original: the rotating triangle in Field5Delegate, table 5). Each wall
/// keeps its length; every frame its ends move to the next vertices.
#[derive(Clone)]
pub(super) struct RotatingTriangle {
    /// Indices into the world's moving walls: wall `i` runs from vertex `i` to vertex `i + 1`.
    walls: [usize; 3],
    center: [f32; 2],
    radius: f32,
    angle: f32,
    /// Radians per sim second at multiplier 1.
    base_speed: f32,
    /// 1 normally; the original sets -2 during multiball (backward, twice as fast).
    pub(super) speed_multiplier: f32,
}

impl RotatingTriangle {
    fn vertex(&self, k: usize) -> [f32; 2] {
        // Vertex 0 at `angle`, vertex 1 a third of a turn ahead, vertex 2 a third behind.
        let a = self.angle + match k % 3 { 0 => 0.0, 1 => TAU / 3.0, _ => -TAU / 3.0 };
        [self.center[0] + self.radius * a.cos(), self.center[1] + self.radius * a.sin()]
    }
}

pub(super) fn build_rotating_triangles(table: &TableDef, walls: &[MovingWallInfo]) -> Vec<RotatingTriangle> {
    if table.delegate.as_deref() != Some("Field5Delegate") { return Vec::new(); }
    let find = |id: &str| walls.iter().position(|w| w.id == id);
    let (Some(a), Some(b), Some(c)) = (find("TriangleWall1"), find("TriangleWall2"), find("TriangleWall3")) else {
        return Vec::new();
    };
    vec![RotatingTriangle {
        walls: [a, b, c],
        center: [10.0, 18.86],
        // Equilateral triangle with sides of length 2.
        radius: 3.0_f32.sqrt() * 2.0 / 3.0,
        angle: TAU / 4.0,
        base_speed: 1.0,
        speed_multiplier: 1.0,
    }]
}

/// Advances every triangle by `dt` sim seconds and moves its walls (bodies and
/// the segments the renderer draws).
pub(super) fn advance_rotating_triangles(
    triangles: &mut [RotatingTriangle],
    dt: f32,
    walls: &mut [MovingWallInfo],
    bodies: &mut RigidBodySet,
) {
    for t in triangles {
        t.angle = (t.angle + t.speed_multiplier * t.base_speed * dt).rem_euclid(TAU);
        for i in 0..3 {
            let (p, q) = (t.vertex(i), t.vertex(i + 1));
            let w = &mut walls[t.walls[i]];
            w.x1 = p[0]; w.y1 = p[1]; w.x2 = q[0]; w.y2 = q[1];
            if let Some(body) = bodies.get_mut(w.body) {
                let mid = Vector::new((p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0);
                body.set_position(Pose::new(mid, (q[1] - p[1]).atan2(q[0] - p[0])), false);
            }
        }
    }
}
