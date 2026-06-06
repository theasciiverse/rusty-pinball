use std::cell::RefCell;
use rapier2d::prelude::*;
use crate::input::InputState;
use crate::table::{
    TableDef, ElementDef, WallDef, WallArcDef, WallPathDef,
    BumperDef, FlipperDef, DropTargetGroupDef,
};

// ---------------------------------------------------------------------------
// Public query types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BallSnapshot {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

#[derive(Debug, Clone)]
pub struct WallSnapshot {
    pub x1: f32, pub y1: f32,
    pub x2: f32, pub y2: f32,
}

#[derive(Debug, Clone)]
pub struct BumperSnapshot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub outer_radius: Option<f32>,
    pub lit: bool,          // true for one tick after a hit
}

#[derive(Debug, Clone)]
pub struct FlipperSnapshot {
    pub x1: f32, pub y1: f32,  // pivot end
    pub x2: f32, pub y2: f32,  // tip end
}

pub struct FieldSnapshot {
    pub ball: Option<BallSnapshot>,
    pub walls: Vec<WallSnapshot>,
    pub kick_walls: Vec<WallSnapshot>,
    pub bumpers: Vec<BumperSnapshot>,
    pub flippers: Vec<FlipperSnapshot>,
    pub field_width: f32,
    pub field_height: f32,
}

// ---------------------------------------------------------------------------
// Internal bookkeeping
// ---------------------------------------------------------------------------

struct FlipperInfo {
    body_handle: RigidBodyHandle,
    pivot:       [f32; 2],
    half_len:    f32,
    angle:       f32,      // logical angle: 0 = horizontal, + = up, − = down
    min_angle:   f32,      // lower limit (radians)
    max_angle:   f32,      // upper limit (radians)
    up_speed:    f32,      // rad/s toward max_angle
    down_speed:  f32,      // rad/s toward min_angle
    is_right:    bool,
}

struct BumperInfo {
    position: [f32; 2],
    radius: f32,
    outer_radius: Option<f32>,
    score: u64,
    kick_force: f32,
    lit_ticks: u8,
}

// Slingshot / kicker wall: a WallElement with a non-zero kick value.
struct KickWallInfo {
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    kick_force: f32,
    score: u64,
    debounce: u8,
}

const BUMPER_LIT_TICKS:   u8 = 6;
const KICK_WALL_DEBOUNCE: u8 = 6;

#[derive(Clone)]
pub enum PhysicsSound {
    BumperHit,
}

// ---------------------------------------------------------------------------
// PhysicsWorld
// ---------------------------------------------------------------------------

pub struct PhysicsWorld {
    // Rapier sets
    bodies:        RigidBodySet,
    colliders:     ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,

    // Rapier pipeline objects
    pipeline:      PhysicsPipeline,
    islands:       IslandManager,
    broad_phase:   DefaultBroadPhase,
    narrow_phase:  NarrowPhase,
    ccd_solver:    CCDSolver,
    query_pipeline: QueryPipeline,

    gravity:       Vector<f32>,

    // Game objects
    ball_handle:   Option<RigidBodyHandle>,
    ball_radius:   f32,
    flippers:      Vec<FlipperInfo>,
    bumpers:       Vec<BumperInfo>,
    kick_walls:    Vec<KickWallInfo>,

    // Pre-computed wall segments for rendering (no physics body needed to query)
    wall_segments: Vec<WallSnapshot>,

    // Events accumulated during step; drained by game / audio layers
    score_events:  RefCell<Vec<u64>>,
    sound_events:  RefCell<Vec<PhysicsSound>>,

    drain_y:       f32,         // y < this → ball drained
    field_width:   f32,
    field_height:  f32,
    launch_pos:    [f32; 2],
    launch_vel:    [f32; 2],
}

impl PhysicsWorld {
    pub fn from_table(table: &TableDef) -> Self {
        let mut bodies        = RigidBodySet::new();
        let mut colliders     = ColliderSet::new();
        let mut impulse_joints = ImpulseJointSet::new();
        let multibody_joints  = MultibodyJointSet::new();

        let mut flippers     = Vec::new();
        let mut bumpers_info = Vec::new();
        let mut kick_walls   = Vec::new();
        let mut wall_segs    = Vec::new();

        // Gravity points downward in rapier (negative y)
        let g = table.gravity_magnitude();
        let gravity = vector![0.0_f32, -g];

        for elem in &table.elements {
            match elem {
                ElementDef::WallElement(w) => {
                    Self::add_wall(w, &mut bodies, &mut colliders, &mut wall_segs);
                    // Register slingshot / kicker if this wall has a kick value.
                    if w.disabled != Some(true) && w.kill != Some(true) {
                        if let (Some(kick), Some([x1, y1, x2, y2])) = (w.kick, w.position) {
                            kick_walls.push(KickWallInfo {
                                x1, y1, x2, y2,
                                kick_force: kick,
                                score: w.score.unwrap_or(0),
                                debounce: 0,
                            });
                        }
                    }
                }
                ElementDef::WallArcElement(a)   => Self::add_arc(a, &mut bodies, &mut colliders, &mut wall_segs),
                ElementDef::WallPathElement(p)  => Self::add_path(p, &mut bodies, &mut colliders, &mut wall_segs),
                ElementDef::BumperElement(b)    => Self::add_bumper(b, &mut bodies, &mut colliders, &mut bumpers_info),
                ElementDef::FlipperElement(f)   => Self::add_flipper(f, &mut bodies, &mut colliders, &mut flippers),
                ElementDef::DropTargetGroupElement(d) => Self::add_drop_targets(d, &mut bodies, &mut colliders, &mut wall_segs),
                _ => {}
            }
        }

        let ball_radius = table.ball_radius();
        let launch_pos  = table.launch_position.unwrap_or([table.width * 0.95, ball_radius + 0.5]);
        let launch_vel  = table.launch_velocity.unwrap_or([0.0, g * 3.0]);

        // No physical floor — the kill wall at y≈0 is detected via drain_y.

        // Left and right outer boundary walls
        for (x, w) in [(0.0_f32, table.width), (table.width, table.width)] {
            let rb = RigidBodyBuilder::fixed().translation(vector![x, table.height / 2.0]).build();
            let h = bodies.insert(rb);
            let c = ColliderBuilder::cuboid(0.05, table.height / 2.0).build();
            colliders.insert_with_parent(c, h, &mut bodies);
            wall_segs.push(WallSnapshot { x1: x, y1: 0.0, x2: x, y2: table.height });
            let _ = w; // suppress warning
        }

        PhysicsWorld {
            bodies,
            colliders,
            impulse_joints,
            multibody_joints,
            pipeline:  PhysicsPipeline::new(),
            islands:   IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            gravity,
            ball_handle: None,
            ball_radius,
            flippers,
            bumpers: bumpers_info,
            kick_walls,
            wall_segments: wall_segs,
            score_events: RefCell::new(Vec::new()),
            sound_events: RefCell::new(Vec::new()),
            drain_y: 1.0,   // ball center < 1.0 → fell past the flipper tips
            field_width:  table.width,
            field_height: table.height,
            launch_pos,
            launch_vel,
        }
    }

    // -----------------------------------------------------------------------
    // Element builders
    // -----------------------------------------------------------------------

    fn add_wall(w: &WallDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>) {
        let pos = match &w.position { Some(p) => *p, None => return };
        if w.disabled == Some(true) || w.kill == Some(true) { return; }

        let [x1, y1, x2, y2] = pos;
        let cx = (x1 + x2) / 2.0;
        let cy = (y1 + y2) / 2.0;
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();
        let angle = dy.atan2(dx);
        let half_w = len / 2.0;
        let half_h = 0.05_f32;
        let restitution = w.restitution.unwrap_or(0.5);

        // Rotation goes on the rigid body; collider stays axis-aligned relative to it
        let rb = RigidBodyBuilder::fixed()
            .translation(vector![cx, cy])
            .rotation(angle)
            .build();
        let h = bodies.insert(rb);
        let col = ColliderBuilder::cuboid(half_w, half_h)
            .restitution(restitution)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(0.0)
            .build();
        colliders.insert_with_parent(col, h, bodies);
        segs.push(WallSnapshot { x1, y1, x2, y2 });
    }

    fn add_arc(a: &WallArcDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>) {
        for (p0, p1) in a.to_segments() {
            let dummy_def = WallDef {
                position: Some([p0[0], p0[1], p1[0], p1[1]]),
                id: None, color: None, kick: None, restitution: Some(0.8),
                kill: None, disabled: None, retract_when_hit: None, score: None,
            };
            Self::add_wall(&dummy_def, bodies, colliders, segs);
        }
    }

    fn add_path(p: &WallPathDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>) {
        for pair in p.positions.windows(2) {
            let dummy = WallDef {
                position: Some([pair[0][0], pair[0][1], pair[1][0], pair[1][1]]),
                id: None, color: None, kick: None, restitution: None,
                kill: None, disabled: None, retract_when_hit: None, score: None,
            };
            Self::add_wall(&dummy, bodies, colliders, segs);
        }
    }

    fn add_bumper(b: &BumperDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, bumpers: &mut Vec<BumperInfo>) {
        let [x, y] = b.position;
        let rb = RigidBodyBuilder::fixed().translation(vector![x, y]).build();
        let h = bodies.insert(rb);
        let col = ColliderBuilder::ball(b.radius)
            .restitution(0.8)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        colliders.insert_with_parent(col, h, bodies);

        bumpers.push(BumperInfo {
            position: b.position,
            radius: b.radius,
            outer_radius: b.outer_radius,
            score: b.score.unwrap_or(100),
            kick_force: b.kick.unwrap_or(8.0),
            lit_ticks: 0,
        });
    }

    fn add_flipper(
        f: &FlipperDef,
        bodies: &mut RigidBodySet,
        colliders: &mut ColliderSet,
        flippers: &mut Vec<FlipperInfo>,
    ) {
        let half_len   = f.abs_length() / 2.0;
        let is_right   = f.is_right_flipper();
        let min_angle  = f.minangle.to_radians();
        let max_angle  = f.maxangle.to_radians();
        let init_angle = min_angle; // start fully down

        let (cx, cy, rot) = Self::flipper_pose(f.position, half_len, init_angle, is_right);

        // Kinematic body: we move it ourselves every tick.
        let flipper_body = RigidBodyBuilder::kinematic_position_based()
            .translation(vector![cx, cy])
            .rotation(rot)
            .build();
        let h = bodies.insert(flipper_body);
        let col = ColliderBuilder::cuboid(half_len, 0.06)
            .restitution(0.3)
            .friction(0.5)
            .build();
        colliders.insert_with_parent(col, h, bodies);

        flippers.push(FlipperInfo {
            body_handle: h,
            pivot:       f.position,
            half_len,
            angle:       init_angle,
            min_angle,
            max_angle,
            up_speed:    f.upspeed,
            down_speed:  f.downspeed,
            is_right,
        });
    }

    /// Returns (center_x, center_y, body_rotation) for a flipper at the given logical angle.
    ///
    /// Angle convention (both left and right):
    ///   0   = horizontal
    ///   +θ  = tip raised  (max_angle)
    ///   −θ  = tip lowered (min_angle)
    ///
    /// Left flipper:  pivot at left end, tip to the right.
    ///   tip  = [pivot_x + len·cos(angle), pivot_y + len·sin(angle)]
    ///   body rotation = +angle
    ///
    /// Right flipper: pivot at right end, tip to the left.
    ///   tip  = [pivot_x − len·cos(angle), pivot_y + len·sin(angle)]
    ///   body rotation = −angle  (mirrors the cuboid back toward the pivot)
    fn flipper_pose(pivot: [f32; 2], half_len: f32, angle: f32, is_right: bool) -> (f32, f32, f32) {
        if is_right {
            (
                pivot[0] - half_len * angle.cos(),
                pivot[1] + half_len * angle.sin(),
                -angle,
            )
        } else {
            (
                pivot[0] + half_len * angle.cos(),
                pivot[1] + half_len * angle.sin(),
                angle,
            )
        }
    }

    fn add_drop_targets(d: &DropTargetGroupDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>) {
        for pos in &d.positions {
            let dummy = WallDef {
                position: Some(*pos),
                id: None, color: None, kick: None, restitution: Some(0.1),
                kill: None, disabled: None, retract_when_hit: None, score: None,
            };
            Self::add_wall(&dummy, bodies, colliders, segs);
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    pub fn launch_ball(&mut self) {
        // Remove any previous ball body before launching a new one
        if let Some(old) = self.ball_handle.take() {
            self.bodies.remove(old, &mut self.islands, &mut self.colliders, &mut self.impulse_joints, &mut self.multibody_joints, false);
        }

        let [lx, ly] = self.launch_pos;
        let [vx, vy] = self.launch_vel;
        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![lx, ly])
            .linvel(vector![vx, vy])
            .build();
        let h = self.bodies.insert(rb);
        let col = ColliderBuilder::ball(self.ball_radius)
            .restitution(0.8)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(0.0)
            .density(1.0)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        self.colliders.insert_with_parent(col, h, &mut self.bodies);
        self.ball_handle = Some(h);
    }

    pub fn step(&mut self, dt: f32, input: &InputState) {
        // Drive flippers kinematically
        for fl in &mut self.flippers {
            let angvel = if (fl.is_right && input.right_flipper)
                           || (!fl.is_right && input.left_flipper)
            {
                fl.up_speed
            } else {
                -fl.down_speed
            };

            fl.angle = (fl.angle + angvel * dt).clamp(fl.min_angle, fl.max_angle);

            let (cx, cy, rot) = Self::flipper_pose(fl.pivot, fl.half_len, fl.angle, fl.is_right);
            if let Some(body) = self.bodies.get_mut(fl.body_handle) {
                body.set_next_kinematic_translation(vector![cx, cy]);
                body.set_next_kinematic_rotation(Rotation::new(rot));
            }
        }

        let mut params = IntegrationParameters::default();
        params.dt = dt;

        self.pipeline.step(
            &self.gravity,
            &params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &(),
        );

        // Bumper proximity detection: collect hits, then apply kicks + score
        {
            let mut hits: Vec<(usize, f32, f32)> = Vec::new(); // (index, nx, ny)
            if let Some(ball_h) = self.ball_handle {
                if let Some(ball_body) = self.bodies.get(ball_h) {
                    let bp = *ball_body.translation();
                    for (i, bump) in self.bumpers.iter().enumerate() {
                        if bump.lit_ticks > 0 { continue; }   // debounce while lit
                        let dx = bp.x - bump.position[0];
                        let dy = bp.y - bump.position[1];
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < bump.radius + self.ball_radius + 0.05 {
                            let (nx, ny) = if dist > 1e-4 { (dx / dist, dy / dist) } else { (0.0, 1.0) };
                            hits.push((i, nx, ny));
                        }
                    }
                }
            }
            for (i, nx, ny) in hits {
                self.bumpers[i].lit_ticks = BUMPER_LIT_TICKS;
                self.score_events.borrow_mut().push(self.bumpers[i].score);
                self.sound_events.borrow_mut().push(PhysicsSound::BumperHit);
                let kick = self.bumpers[i].kick_force;
                if let Some(ball_h) = self.ball_handle {
                    if let Some(body) = self.bodies.get_mut(ball_h) {
                        body.apply_impulse(vector![nx * kick, ny * kick], true);
                    }
                }
            }
        }

        // Tick bumper flash timers
        for b in &mut self.bumpers {
            if b.lit_ticks > 0 { b.lit_ticks -= 1; }
        }

        // Kick-wall (slingshot) proximity detection
        {
            let mut hits: Vec<(usize, f32, f32)> = Vec::new();
            if let Some(ball_h) = self.ball_handle {
                if let Some(ball_body) = self.bodies.get(ball_h) {
                    let bp = *ball_body.translation();
                    for (i, kw) in self.kick_walls.iter().enumerate() {
                        if kw.debounce > 0 { continue; }
                        let (dist, nx, ny) = Self::point_to_segment_dist(bp.x, bp.y, kw.x1, kw.y1, kw.x2, kw.y2);
                        if dist < self.ball_radius + 0.08 {
                            hits.push((i, nx, ny));
                        }
                    }
                }
            }
            for (i, nx, ny) in hits {
                self.kick_walls[i].debounce = KICK_WALL_DEBOUNCE;
                if self.kick_walls[i].score > 0 {
                    self.score_events.borrow_mut().push(self.kick_walls[i].score);
                }
                let kick = self.kick_walls[i].kick_force;
                if let Some(ball_h) = self.ball_handle {
                    if let Some(body) = self.bodies.get_mut(ball_h) {
                        body.apply_impulse(vector![nx * kick, ny * kick], true);
                    }
                }
            }
        }

        // Tick kick-wall debounce timers
        for kw in &mut self.kick_walls {
            if kw.debounce > 0 { kw.debounce -= 1; }
        }
    }

    /// Distance from point (px,py) to segment (x1,y1)-(x2,y2), plus the unit
    /// normal pointing from the closest point on the segment toward (px,py).
    fn point_to_segment_dist(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32, f32) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len_sq = dx * dx + dy * dy;
        let (cx, cy) = if len_sq < 1e-10 {
            (x1, y1)
        } else {
            let t = ((px - x1) * dx + (py - y1) * dy) / len_sq;
            let t = t.clamp(0.0, 1.0);
            (x1 + t * dx, y1 + t * dy)
        };
        let ex = px - cx;
        let ey = py - cy;
        let dist = (ex * ex + ey * ey).sqrt();
        let (nx, ny) = if dist > 1e-6 {
            (ex / dist, ey / dist)
        } else {
            // Ball centre exactly on segment; use the segment's left-hand normal.
            let l = len_sq.sqrt().max(1e-6);
            (-dy / l, dx / l)
        };
        (dist, nx, ny)
    }

    pub fn ball_position(&self) -> Option<nalgebra::Vector2<f32>> {
        let h = self.ball_handle?;
        let body = self.bodies.get(h)?;
        Some(*body.translation())
    }

    pub fn is_ball_drained(&self) -> bool {
        self.ball_position()
            .map(|p| p.y < self.drain_y)
            .unwrap_or(false)
    }

    pub fn drain_score_events(&self) -> Vec<u64> {
        self.score_events.borrow_mut().drain(..).collect()
    }

    pub fn drain_sound_events(&self) -> Vec<PhysicsSound> {
        self.sound_events.borrow_mut().drain(..).collect()
    }

    pub fn remove_ball(&mut self) {
        if let Some(old) = self.ball_handle.take() {
            self.bodies.remove(
                old,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                false,
            );
        }
    }

    pub fn field_height(&self) -> f32 { self.field_height }
    pub fn field_width(&self)  -> f32 { self.field_width }

    pub fn snapshot(&self) -> FieldSnapshot {
        let ball = self.ball_handle.and_then(|h| self.bodies.get(h)).map(|b| {
            let t = b.translation();
            let v = b.linvel();
            BallSnapshot { x: t.x, y: t.y, vx: v.x, vy: v.y }
        });

        let flippers: Vec<FlipperSnapshot> = self.flippers.iter().map(|fl| {
            let len = fl.half_len * 2.0;
            let (x2, y2) = if fl.is_right {
                (fl.pivot[0] - len * fl.angle.cos(),
                 fl.pivot[1] + len * fl.angle.sin())
            } else {
                (fl.pivot[0] + len * fl.angle.cos(),
                 fl.pivot[1] + len * fl.angle.sin())
            };
            FlipperSnapshot { x1: fl.pivot[0], y1: fl.pivot[1], x2, y2 }
        }).collect();

        let bumpers = self.bumpers.iter().map(|b| BumperSnapshot {
            x: b.position[0], y: b.position[1],
            radius: b.radius,
            outer_radius: b.outer_radius,
            lit: b.lit_ticks > 0,
        }).collect();

        let kick_walls = self.kick_walls.iter()
            .map(|kw| WallSnapshot { x1: kw.x1, y1: kw.y1, x2: kw.x2, y2: kw.y2 })
            .collect();

        FieldSnapshot {
            ball,
            walls: self.wall_segments.clone(),
            kick_walls,
            bumpers,
            flippers,
            field_width:  self.field_width,
            field_height: self.field_height,
        }
    }
}
