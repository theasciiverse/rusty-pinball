use std::cell::RefCell;
use rapier2d::prelude::*;
use crate::input::InputState;
use crate::table::{
    TableDef, ElementDef, WallDef, WallArcDef, WallPathDef,
    BumperDef, FlipperDef, DropTargetGroupDef, RolloverGroupDef, SpinnerDef,
};

// ---------------------------------------------------------------------------
// Public query types
// ---------------------------------------------------------------------------

/// Reads a table JSON `color`/`outerColor` value (`[r,g,b]` or `[r,g,b,a]`,
/// alpha ignored) into an RGB triple for the renderer. `None` means "no
/// color was specified" — callers fall back to their element-type default.
fn parse_color(v: &Option<serde_json::Value>) -> Option<(u8, u8, u8)> {
    let arr = v.as_ref()?.as_array()?;
    let at = |i: usize| -> Option<u8> { arr.get(i)?.as_u64().map(|n| n as u8) };
    Some((at(0)?, at(1)?, at(2)?))
}

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
    /// None = universal (always visible). Some(n) = only active on layer n.
    pub layer: Option<u8>,
    /// Debug label: index (and sub-index) into the table JSON's `elements` array.
    pub label: Option<String>,
    /// Explicit color from the table JSON, if any. `None` = fall back to the
    /// renderer's layer-based default.
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct BumperSnapshot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub outer_radius: Option<f32>,
    pub lit: bool,          // true for one tick after a hit
    pub label: String,
    pub color: Option<(u8, u8, u8)>,
    pub outer_color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct FlipperSnapshot {
    pub x1: f32, pub y1: f32,  // pivot end
    pub x2: f32, pub y2: f32,  // tip end
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct RolloverSnapshot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub lit: bool,
    pub label: String,
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct RolloverGroupSnapshot {
    pub rollovers: Vec<RolloverSnapshot>,
}

#[derive(Debug, Clone)]
pub struct SpinnerSnapshot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    /// True while the spinner is mid-spin (ball passed over it fast enough).
    pub spinning: bool,
    /// True on the "on" half of the current flash cycle.
    pub flash_on: bool,
    pub label: String,
}

pub struct FieldSnapshot {
    pub ball: Option<BallSnapshot>,
    pub walls: Vec<WallSnapshot>,
    pub kick_walls: Vec<WallSnapshot>,
    pub bumpers: Vec<BumperSnapshot>,
    pub flippers: Vec<FlipperSnapshot>,
    pub drop_targets: Vec<WallSnapshot>,
    pub rollover_groups: Vec<RolloverGroupSnapshot>,
    pub spinners: Vec<SpinnerSnapshot>,
    pub field_width: f32,
    pub field_height: f32,
    pub ball_layer: u8,
    pub ball_color: Option<(u8, u8, u8)>,
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
    label:       String,
}

struct BumperInfo {
    position: [f32; 2],
    radius: f32,
    outer_radius: Option<f32>,
    score: u64,
    kick_force: f32,
    lit_ticks: u8,
    label: String,
    color: Option<(u8, u8, u8)>,
    outer_color: Option<(u8, u8, u8)>,
}

// Slingshot / kicker wall: a WallElement with a non-zero kick value.
struct KickWallInfo {
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    kick_force: f32,
    score: u64,
    debounce: u8,
    label: String,
    retract_when_hit: bool,
    active: bool,
    // When true the rapier collider will be disabled at the START of the next
    // step() call — one physics sub-step after the kick impulse was applied.
    // This gives rapier one full integration step to move the ball away before
    // the collider disappears.
    retract_pending: bool,
    // Perpendicular direction pointing toward the field interior (precomputed).
    interior_normal: [f32; 2],
    // Rapier collider handle — used for collision-event-based kick detection.
    collider_handle: ColliderHandle,
}

struct DropTargetInfo {
    x1: f32, y1: f32, x2: f32, y2: f32,
    collider: ColliderHandle,
    active: bool,
}

struct DropTargetGroupInfo {
    targets: Vec<DropTargetInfo>,
    score:   u64,
    label:   String,
    color:   Option<(u8, u8, u8)>,
}

struct RolloverInfo {
    x: f32, y: f32,
    radius: f32,
    score: u64,
    lit: bool,
    color: Option<(u8, u8, u8)>,
}

struct RolloverGroupInfo {
    rollovers:        Vec<RolloverInfo>,
    score:            u64,   // group-level score: per-rollover fallback AND completion bonus
    cycle_on_flipper: bool,
    ignore_ball:      bool,
    // >0 while the "all lit" flash animation is running; 0 = idle.
    flash_ticks:      u8,
    label:            String,
}

struct LayerSensorInfo {
    rect:         [f32; 4],   // [xmin, ymin, xmax, ymax]
    target_layer: u8,
    from_layer:   Option<u8>, // None = trigger from any layer
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SpinnerStatus { Inactive, ActiveVisible, ActiveInvisible }

// No physical body: a spinner is a virtual zone the ball passes through.
// Passing over it at/above `min_activation_speed` starts it spinning; it
// flashes on/off at a decaying frequency, scoring once per cycle, until it
// drops below `min_cycles_per_second` and goes idle again.
struct SpinnerInfo {
    position: [f32; 2],
    radius: f32,
    min_activation_speed: f32,
    base_cycles_per_second: f32,
    min_cycles_per_second: f32,
    speed_decay_rate: f32,
    score: u64,
    status: SpinnerStatus,
    current_cycles_per_second: f32,
    seconds_to_next_cycle: f32,
    label: String,
}

const BUMPER_LIT_TICKS:   u8  = 6;
const KICK_WALL_DEBOUNCE: u8  = 6;
const DROP_CLEAR_BONUS:   u64 = 2_000;
// Rollover group completion flash: 4 phases × 8 sub-steps each.
// At 2 sub-steps per frame (30 fps) each phase ≈ 133 ms; total ≈ 533 ms.
// Phase 0 = unlit so the dots go dark immediately on completion (obvious feedback).
const ROLLOVER_FLASH_TICKS:  u8 = 32;
const ROLLOVER_FLASH_PHASES: u8 = 4;

// ── Layer collision groups ─────────────────────────────────────────────────────
// Bits 0-3 correspond to layers 0-3.  Universal walls use ALL bits (like
// InteractionGroups::all()).  A ball on layer N has memberships=bit(N),
// filter=ALL, so it passes through walls of other layers unimpeded.

fn layer_group_for(layer: u8) -> InteractionGroups {
    let bit = Group::from_bits_truncate(1u32 << layer as u32);
    InteractionGroups::new(bit, bit)
}

fn ball_groups_for(layer: u8) -> InteractionGroups {
    let bit = Group::from_bits_truncate(1u32 << layer as u32);
    InteractionGroups::new(bit, Group::ALL)
}

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
    ball_handle:        Option<RigidBodyHandle>,
    ball_collider:  Option<ColliderHandle>,   // collider for the ball body
    ball_radius:        f32,
    ball_color:         Option<(u8, u8, u8)>,
    flippers:           Vec<FlipperInfo>,
    bumpers:            Vec<BumperInfo>,
    kick_walls:         Vec<KickWallInfo>,
    drop_target_groups: Vec<DropTargetGroupInfo>,
    rollover_groups:    Vec<RolloverGroupInfo>,
    spinners:           Vec<SpinnerInfo>,

    // Collision-event channel: pipeline writes Started/Stopped events here.
    kick_evt_send: rapier2d::crossbeam::channel::Sender<CollisionEvent>,
    kick_evt_recv: rapier2d::crossbeam::channel::Receiver<CollisionEvent>,
    // Force-event channel: we don't use force events but must keep the sender
    // alive so the channel doesn't disconnect and cause silent errors.
    _force_send:   rapier2d::crossbeam::channel::Sender<rapier2d::geometry::ContactForceEvent>,
    _force_recv:   rapier2d::crossbeam::channel::Receiver<rapier2d::geometry::ContactForceEvent>,

    // Pre-computed wall segments for rendering (no physics body needed to query)
    wall_segments: Vec<WallSnapshot>,

    // Layer system: sensors that transition the ball between layers.
    layer_sensors: Vec<LayerSensorInfo>,
    ball_layer:    u8,

    // Events accumulated during step; drained by game / audio layers
    score_events:  RefCell<Vec<u64>>,
    sound_events:  RefCell<Vec<PhysicsSound>>,

    drain_y:       f32,
    field_width:   f32,
    field_height:  f32,
    launch_pos:       [f32; 2],
    launch_vel:       [f32; 2],
    launch_vel_delta: [f32; 2],

    // Shooter-lane gate: closes once ball escapes, preventing it falling back.
    dead_zone:     Option<[f32; 4]>,   // [xmin, ymin, xmax, ymax]
    gate_y:        f32,                // threshold ball must cross to clear lane
    gate_collider: Option<ColliderHandle>,
    gate_cleared:  bool,
}

impl PhysicsWorld {
    pub fn from_table(table: &TableDef) -> Self {
        let mut bodies        = RigidBodySet::new();
        let mut colliders     = ColliderSet::new();
        let mut impulse_joints = ImpulseJointSet::new();
        let multibody_joints  = MultibodyJointSet::new();

        let mut flippers           = Vec::new();
        let mut bumpers_info       = Vec::new();
        let mut kick_walls         = Vec::new();
        let mut drop_target_groups = Vec::new();
        let mut rollover_groups    = Vec::new();
        let mut spinners           = Vec::new();
        let mut wall_segs          = Vec::new();
        let mut layer_sensors      = Vec::new();

        // Gravity points downward in rapier (negative y)
        let g = table.gravity_magnitude();
        let gravity = vector![0.0_f32, -g];

        for (elem_idx, elem) in table.elements.iter().enumerate() {
            let label = elem_idx.to_string();
            match elem {
                ElementDef::WallElement(w) => {
                    let wall_col = Self::add_wall(w, &mut bodies, &mut colliders, &mut wall_segs, w.layer, Some(label.clone()), parse_color(&w.color));
                    // Register slingshot / kicker if this wall has a kick value.
                    if w.disabled != Some(true) && w.kill != Some(true) {
                        if let (Some(kick), Some([x1, y1, x2, y2]), Some(col)) =
                            (w.kick, w.position, wall_col)
                        {
                            let dx = x2 - x1;
                            let dy = y2 - y1;
                            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                            let nx = dy / len;
                            let ny = -dx / len;
                            let midx = (x1 + x2) / 2.0;
                            let midy = (y1 + y2) / 2.0;
                            let fcx  = table.width  / 2.0;
                            let fcy  = table.height / 2.0;
                            let interior_normal = if (fcx - midx) * nx + (fcy - midy) * ny >= 0.0 {
                                [nx, ny]
                            } else {
                                [-nx, -ny]
                            };
                            kick_walls.push(KickWallInfo {
                                x1, y1, x2, y2,
                                kick_force: kick,
                                score: w.score.unwrap_or(0),
                                debounce: 0,
                                label: label.clone(),
                                retract_when_hit: w.retract_when_hit == Some(true),
                                active: true,
                                retract_pending: false,
                                interior_normal,
                                collider_handle: col,
                            });
                            // Kick walls are rendered via the kick_walls path only;
                            // remove the segment add_wall just pushed to wall_segs.
                            wall_segs.pop();
                        }
                    }
                }
                ElementDef::WallArcElement(a)   => Self::add_arc(a, &mut bodies, &mut colliders, &mut wall_segs, label),
                ElementDef::WallPathElement(p)  => Self::add_path(p, &mut bodies, &mut colliders, &mut wall_segs, label),
                ElementDef::BumperElement(b)    => Self::add_bumper(b, &mut bodies, &mut colliders, &mut bumpers_info, label),
                ElementDef::FlipperElement(f)   => Self::add_flipper(f, &mut bodies, &mut colliders, &mut flippers, label),
                ElementDef::DropTargetGroupElement(d) => {
                    drop_target_groups.push(Self::build_drop_target_group(d, &mut bodies, &mut colliders, label));
                }
                ElementDef::RolloverGroupElement(r) => {
                    rollover_groups.push(Self::build_rollover_group(r, label));
                }
                ElementDef::SpinnerElement(s) => {
                    spinners.push(Self::build_spinner(s, label));
                }
                ElementDef::SensorElement(s) => {
                    if let (Some(rect), Some(target)) = (s.rect, s.ball_layer) {
                        layer_sensors.push(LayerSensorInfo {
                            rect,
                            target_layer: target,
                            from_layer:   s.ball_layer_from,
                        });
                    }
                }
                _ => {}
            }
        }

        let ball_radius = table.ball_radius();
        let ball_color  = table.ball_color.map(|[r, g, b]| (r, g, b));
        let launch_pos        = table.launch_position.unwrap_or([table.width * 0.95, ball_radius + 0.5]);
        let launch_vel        = table.launch_velocity.unwrap_or([0.0, g * 3.0]);
        let launch_vel_delta  = table.launch_velocity_random_delta.unwrap_or([0.0, 0.0]);

        // No physical floor — the kill wall at y≈0 is detected via drain_y.

        // Left and right outer boundary walls
        for (x, w) in [(0.0_f32, table.width), (table.width, table.width)] {
            let rb = RigidBodyBuilder::fixed().translation(vector![x, table.height / 2.0]).build();
            let h = bodies.insert(rb);
            let c = ColliderBuilder::cuboid(0.05, table.height / 2.0).build();
            colliders.insert_with_parent(c, h, &mut bodies);
            wall_segs.push(WallSnapshot { x1: x, y1: 0.0, x2: x, y2: table.height, layer: None, label: None, color: None });
            let _ = w; // suppress warning
        }

        // Shooter-lane gate: a horizontal wall across the lane that starts
        // disabled and is enabled once the ball escapes upward.
        let dead_zone  = table.launch_dead_zone;
        let gate_y     = launch_pos[1] + 1.0;   // just above the launch point
        let gate_collider = dead_zone.map(|[dz_xmin, _, dz_xmax, _]| {
            let cx     = (dz_xmin + dz_xmax) / 2.0;
            let half_w = (dz_xmax - dz_xmin) / 2.0;
            let bh = bodies.insert(
                RigidBodyBuilder::fixed().translation(vector![cx, gate_y]).build()
            );
            let col = ColliderBuilder::cuboid(half_w, 0.05)
                .collision_groups(InteractionGroups::none())  // disabled until ball clears
                .build();
            colliders.insert_with_parent(col, bh, &mut bodies)
        });

        // Place the ball at the launch position with gravity disabled so it
        // sits visibly in the shooter lane before the player fires it.
        let [lx, ly] = launch_pos;
        let idle_rb = RigidBodyBuilder::dynamic()
            .translation(vector![lx, ly])
            .gravity_scale(0.0)
            .build();
        let idle_h = bodies.insert(idle_rb);
        let idle_col = ColliderBuilder::ball(ball_radius)
            .restitution(0.8)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(0.0)
            .density(1.0)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .collision_groups(ball_groups_for(0))
            .build();
        let ball_col_h = colliders.insert_with_parent(idle_col, idle_h, &mut bodies);
        let ball_handle   = Some(idle_h);
        let ball_collider = Some(ball_col_h);

        let (kick_evt_send, kick_evt_recv) =
            rapier2d::crossbeam::channel::unbounded::<CollisionEvent>();
        let (_force_send, _force_recv) =
            rapier2d::crossbeam::channel::unbounded::<rapier2d::geometry::ContactForceEvent>();

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
            ball_handle,
            ball_collider,
            ball_radius,
            ball_color,
            flippers,
            bumpers: bumpers_info,
            kick_walls,
            drop_target_groups,
            wall_segments: wall_segs,
            rollover_groups,
            spinners,
            layer_sensors,
            ball_layer: 0,
            score_events: RefCell::new(Vec::new()),
            sound_events: RefCell::new(Vec::new()),
            drain_y: 1.0,
            field_width:  table.width,
            field_height: table.height,
            launch_pos,
            launch_vel,
            launch_vel_delta,
            dead_zone,
            gate_y,
            gate_collider,
            gate_cleared: false,
            kick_evt_send,
            kick_evt_recv,
            _force_send,
            _force_recv,
        }
    }

    // -----------------------------------------------------------------------
    // Element builders
    // -----------------------------------------------------------------------

    fn add_wall(w: &WallDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>, layer: Option<u8>, label: Option<String>, color: Option<(u8, u8, u8)>) -> Option<ColliderHandle> {
        let pos = match &w.position { Some(p) => *p, None => return None };
        if w.disabled == Some(true) || w.kill == Some(true) { return None; }

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

        let groups = match layer {
            Some(n) => layer_group_for(n),
            None    => InteractionGroups::all(),
        };

        let rb = RigidBodyBuilder::fixed()
            .translation(vector![cx, cy])
            .rotation(angle)
            .build();
        let h = bodies.insert(rb);
        let col = ColliderBuilder::cuboid(half_w, half_h)
            .restitution(restitution)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(0.0)
            .collision_groups(groups)
            .build();
        let col_h = colliders.insert_with_parent(col, h, bodies);
        segs.push(WallSnapshot { x1, y1, x2, y2, layer, label, color });
        Some(col_h)
    }

    fn add_arc(a: &WallArcDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>, label: String) {
        let color = parse_color(&a.color);
        for (i, (p0, p1)) in a.to_segments().into_iter().enumerate() {
            let dummy_def = WallDef {
                position: Some([p0[0], p0[1], p1[0], p1[1]]),
                id: None, color: None, kick: None, restitution: Some(0.8),
                kill: None, disabled: None, retract_when_hit: None, score: None,
                layer: None,
            };
            let seg_label = if i == 0 { Some(label.clone()) } else { None };
            let _ = Self::add_wall(&dummy_def, bodies, colliders, segs, a.layer, seg_label, color);
        }
    }

    fn add_path(p: &WallPathDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>, label: String) {
        let color = parse_color(&p.color);
        for (i, pair) in p.positions.windows(2).enumerate() {
            let dummy = WallDef {
                position: Some([pair[0][0], pair[0][1], pair[1][0], pair[1][1]]),
                id: None, color: None, kick: None, restitution: None,
                kill: None, disabled: None, retract_when_hit: None, score: None,
                layer: None,
            };
            let seg_label = if i == 0 { Some(label.clone()) } else { None };
            let _ = Self::add_wall(&dummy, bodies, colliders, segs, p.layer, seg_label, color);
        }
    }

    fn add_bumper(b: &BumperDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, bumpers: &mut Vec<BumperInfo>, label: String) {
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
            label,
            color: parse_color(&b.color),
            outer_color: parse_color(&b.outer_color),
        });
    }

    fn add_flipper(
        f: &FlipperDef,
        bodies: &mut RigidBodySet,
        colliders: &mut ColliderSet,
        flippers: &mut Vec<FlipperInfo>,
        label: String,
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
            label,
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

    fn build_drop_target_group(d: &DropTargetGroupDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, label: String) -> DropTargetGroupInfo {
        let targets = d.positions.iter().map(|&[x1, y1, x2, y2]| {
            let cx    = (x1 + x2) / 2.0;
            let cy    = (y1 + y2) / 2.0;
            let dx    = x2 - x1;
            let dy    = y2 - y1;
            let len   = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let bh = bodies.insert(
                RigidBodyBuilder::fixed()
                    .translation(vector![cx, cy])
                    .rotation(angle)
                    .build()
            );
            let col = ColliderBuilder::cuboid(len / 2.0, 0.05)
                .restitution(0.5)
                .restitution_combine_rule(CoefficientCombineRule::Max)
                .friction(0.0)
                .build();
            let ch = colliders.insert_with_parent(col, bh, bodies);
            DropTargetInfo { x1, y1, x2, y2, collider: ch, active: true }
        }).collect();
        DropTargetGroupInfo { targets, score: d.score.unwrap_or(0), label, color: parse_color(&d.color) }
    }

    fn build_rollover_group(r: &RolloverGroupDef, label: String) -> RolloverGroupInfo {
        let group_radius = r.radius.unwrap_or(0.4);
        let group_score  = r.score.unwrap_or(0);
        let group_color  = parse_color(&r.color);
        let rollovers = r.rollovers.iter().map(|rv| RolloverInfo {
            x:      rv.position[0],
            y:      rv.position[1],
            radius: rv.radius.unwrap_or(group_radius),
            score:  rv.score.unwrap_or(group_score),
            lit:    false,
            color:  parse_color(&rv.color).or(group_color),
        }).collect();
        RolloverGroupInfo {
            rollovers,
            score:            group_score,
            cycle_on_flipper: r.cycle_on_flipper.unwrap_or(false),
            ignore_ball:      r.ignore_ball.unwrap_or(false),
            flash_ticks:      0,
            label,
        }
    }

    fn build_spinner(s: &SpinnerDef, label: String) -> SpinnerInfo {
        SpinnerInfo {
            position: s.position,
            radius: s.radius.unwrap_or(0.5),
            min_activation_speed: s.min_activation_speed.unwrap_or(4.0),
            base_cycles_per_second: s.base_cycles_per_second.unwrap_or(8.0),
            min_cycles_per_second: s.min_cycles_per_second.unwrap_or(1.5),
            speed_decay_rate: s.speed_decay_rate.unwrap_or(0.9),
            score: s.score.unwrap_or(0),
            status: SpinnerStatus::Inactive,
            current_cycles_per_second: 0.0,
            seconds_to_next_cycle: 0.0,
            label,
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    pub fn launch_ball(&mut self) {
        let [lx, ly] = self.launch_pos;
        let [vx, vy] = self.launch_vel;
        let [dx, dy] = self.launch_vel_delta;

        // Random top-up in [0, delta]: base is always enough to clear the lane;
        // the delta is the table's launchVelocityRandomDelta (additive only).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let lcg  = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let frac = (lcg >> 8) as f32 / (1u32 << 24) as f32; // 0..1
        let actual_vy = vy + frac * dy;
        let actual_vx = vx + (frac - 0.5) * dx.max(0.1);    // tiny centred lateral nudge

        if let Some(h) = self.ball_handle {
            if let Some(body) = self.bodies.get_mut(h) {
                body.set_translation(vector![lx, ly], true);
                body.set_linvel(vector![actual_vx, actual_vy], true);
                body.set_angvel(0.0, true);
                body.set_gravity_scale(1.0, true);
                body.enable_ccd(true);
            }
        } else {
            let rb = RigidBodyBuilder::dynamic()
                .translation(vector![lx, ly])
                .linvel(vector![actual_vx, actual_vy])
                .ccd_enabled(true)
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
    }

    pub fn step(&mut self, dt: f32, input: &InputState) {
        // Deferred collider disable for retractWhenHit walls.
        // We set retract_pending in the previous sub-step so that the kick
        // impulse has one full integration step to move the ball away before
        // the wall disappears from the physics world.
        for kw in &mut self.kick_walls {
            if kw.retract_pending {
                kw.retract_pending = false;
                if let Some(col) = self.colliders.get_mut(kw.collider_handle) {
                    col.set_collision_groups(InteractionGroups::none());
                }
            }
        }

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

        let evt_handler = ChannelEventCollector::new(
            self.kick_evt_send.clone(),
            self._force_send.clone(),
        );
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
            &evt_handler,
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

        // Kick-wall (slingshot / ball-saver) collision-event detection.
        //
        // Rapier emits CollisionEvent::Started when the ball first contacts a
        // kick-wall collider.  This matches the original Vector-Pinball approach
        // (Box2D handleCollision) and fires reliably regardless of ball speed.
        // Kick direction is always the precomputed interior_normal so the ball
        // is pushed away from the pocket and into the field.
        {
            let ball_col = self.ball_collider;
            // Drain all collision events queued during the step above.
            let events: Vec<CollisionEvent> = {
                let mut v = Vec::new();
                while let Ok(e) = self.kick_evt_recv.try_recv() { v.push(e); }
                v
            };
            let mut hits: Vec<(usize, f32, f32)> = Vec::new();
            for event in events {
                if let CollisionEvent::Started(h1, h2, _) = event {
                    let other = match ball_col {
                        Some(bc) if bc == h1 => h2,
                        Some(bc) if bc == h2 => h1,
                        _ => continue,
                    };
                    if let Some(ki) = self.kick_walls.iter().position(|kw| {
                        kw.active && kw.debounce == 0 && kw.collider_handle == other
                    }) {
                        let kw = &self.kick_walls[ki];
                        hits.push((ki, kw.interior_normal[0], kw.interior_normal[1]));
                    }
                }
            }
            for (i, nx, ny) in hits {
                if self.kick_walls[i].retract_when_hit {
                    // Visual disappearance is immediate; the rapier collider is
                    // disabled one sub-step later (via retract_pending) so the
                    // kick impulse has a full integration step to move the ball
                    // clear before the wall vanishes from the physics world.
                    self.kick_walls[i].active = false;
                    self.kick_walls[i].retract_pending = true;
                } else {
                    self.kick_walls[i].debounce = KICK_WALL_DEBOUNCE;
                }
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

        // Drop-target hit detection
        {
            let mut hits: Vec<(usize, usize)> = Vec::new();
            if let Some(ball_h) = self.ball_handle {
                if let Some(ball_body) = self.bodies.get(ball_h) {
                    let bp = *ball_body.translation();
                    for (gi, group) in self.drop_target_groups.iter().enumerate() {
                        for (ti, target) in group.targets.iter().enumerate() {
                            if !target.active { continue; }
                            let (dist, _, _) = Self::point_to_segment_dist(
                                bp.x, bp.y,
                                target.x1, target.y1, target.x2, target.y2,
                            );
                            if dist < self.ball_radius + 0.05 {
                                hits.push((gi, ti));
                            }
                        }
                    }
                }
            }
            for (gi, ti) in hits {
                let score = self.drop_target_groups[gi].score;
                if score > 0 { self.score_events.borrow_mut().push(score); }

                self.drop_target_groups[gi].targets[ti].active = false;
                let ch = self.drop_target_groups[gi].targets[ti].collider;
                if let Some(col) = self.colliders.get_mut(ch) {
                    col.set_collision_groups(InteractionGroups::none());
                }

                let all_clear = self.drop_target_groups[gi].targets.iter().all(|t| !t.active);
                if all_clear {
                    self.score_events.borrow_mut().push(DROP_CLEAR_BONUS);
                    for target in &mut self.drop_target_groups[gi].targets {
                        target.active = true;
                        if let Some(col) = self.colliders.get_mut(target.collider) {
                            col.set_collision_groups(InteractionGroups::all());
                        }
                    }
                }
            }
        }

        // Rollover detection, flipper cycling, and group-completion flash.
        {
            let ball_pos = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| *b.translation());
            for group in &mut self.rollover_groups {
                // ── Ball activation (skip while flashing — group is mid-reset) ──
                if !group.ignore_ball && group.flash_ticks == 0 {
                    if let Some(bp) = ball_pos {
                        for rv in &mut group.rollovers {
                            if rv.lit { continue; }
                            let dx = bp.x - rv.x;
                            let dy = bp.y - rv.y;
                            if dx * dx + dy * dy <= (rv.radius + self.ball_radius).powi(2) {
                                rv.lit = true;
                                let pts = if rv.score > 0 { rv.score } else { group.score };
                                if pts > 0 {
                                    self.score_events.borrow_mut().push(pts);
                                }
                            }
                        }
                    }
                }

                // ── Group completion: start flash when all rollovers just became lit ──
                if group.flash_ticks == 0
                    && !group.rollovers.is_empty()
                    && group.rollovers.iter().all(|rv| rv.lit)
                {
                    group.flash_ticks = ROLLOVER_FLASH_TICKS;
                }

                // ── Flash countdown → bonus + reset on completion ──
                if group.flash_ticks > 0 {
                    group.flash_ticks -= 1;
                    if group.flash_ticks == 0 {
                        if group.score > 0 {
                            self.score_events.borrow_mut().push(group.score);
                        }
                        for rv in &mut group.rollovers {
                            rv.lit = false;
                        }
                    }
                }

                // ── Flipper cycling — suspend while flashing ──
                // Left press  → rotate_right(1): lit dot shifts toward higher index.
                // Right press → rotate_left(1):  lit dot shifts toward lower index.
                if group.cycle_on_flipper && group.flash_ticks == 0 {
                    let n = group.rollovers.len();
                    if n > 1 {
                        if input.left_just_pressed {
                            let last = group.rollovers[n - 1].lit;
                            for i in (1..n).rev() {
                                group.rollovers[i].lit = group.rollovers[i - 1].lit;
                            }
                            group.rollovers[0].lit = last;
                        } else if input.right_just_pressed {
                            let first = group.rollovers[0].lit;
                            for i in 0..n - 1 {
                                group.rollovers[i].lit = group.rollovers[i + 1].lit;
                            }
                            group.rollovers[n - 1].lit = first;
                        }
                    }
                }
            }
        }

        // Spinner tick: virtual (non-colliding) zones. A ball crossing one at
        // or above min_activation_speed starts it spinning; it then flashes
        // on/off at a decaying frequency, scoring once per cycle, until the
        // frequency decays below min_cycles_per_second and it goes idle.
        {
            let ball = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| (*b.translation(), *b.linvel()));
            for sp in &mut self.spinners {
                if sp.status == SpinnerStatus::Inactive {
                    if let Some((pos, vel)) = ball {
                        let dx = pos.x - sp.position[0];
                        let dy = pos.y - sp.position[1];
                        if dx * dx + dy * dy <= sp.radius * sp.radius {
                            let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
                            if speed >= sp.min_activation_speed {
                                sp.current_cycles_per_second =
                                    (speed / sp.min_activation_speed) * sp.base_cycles_per_second;
                                sp.seconds_to_next_cycle = 1.0 / sp.current_cycles_per_second;
                                sp.status = SpinnerStatus::ActiveInvisible;
                            }
                        }
                    }
                } else {
                    sp.seconds_to_next_cycle -= dt;
                    if sp.seconds_to_next_cycle <= 0.0 {
                        sp.status = if sp.status == SpinnerStatus::ActiveVisible {
                            SpinnerStatus::ActiveInvisible
                        } else {
                            SpinnerStatus::ActiveVisible
                        };
                        if sp.score > 0 {
                            self.score_events.borrow_mut().push(sp.score);
                        }
                        sp.current_cycles_per_second *= sp.speed_decay_rate;
                        sp.seconds_to_next_cycle = 1.0 / sp.current_cycles_per_second;
                        if sp.status == SpinnerStatus::ActiveInvisible
                            && sp.current_cycles_per_second < sp.min_cycles_per_second
                        {
                            sp.status = SpinnerStatus::Inactive;
                        }
                    }
                }
            }
        }

        // Layer-sensor detection: transition ball to a new physics layer when
        // it enters a sensor rect (and the optional from-layer filter matches).
        {
            let ball_pos = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| *b.translation());
            if let Some(bp) = ball_pos {
                let mut new_layer = self.ball_layer;
                for sensor in &self.layer_sensors {
                    let [xmin, ymin, xmax, ymax] = sensor.rect;
                    if bp.x >= xmin && bp.x <= xmax && bp.y >= ymin && bp.y <= ymax {
                        if sensor.from_layer.map_or(true, |fl| fl == self.ball_layer) {
                            new_layer = sensor.target_layer;
                            break;
                        }
                    }
                }
                if new_layer != self.ball_layer {
                    self.ball_layer = new_layer;
                    if let Some(bc) = self.ball_collider {
                        if let Some(col) = self.colliders.get_mut(bc) {
                            col.set_collision_groups(ball_groups_for(new_layer));
                        }
                    }
                }
            }
        }

        // Shooter-lane gate: enable once the ball clears gate_y going upward.
        if !self.gate_cleared {
            if let (Some(ball_h), Some(gate_col_h)) = (self.ball_handle, self.gate_collider) {
                let in_lane = self.dead_zone.map(|[xmin, _, xmax, _]| {
                    self.bodies.get(ball_h)
                        .map(|b| { let x = b.translation().x; x >= xmin - 0.2 && x <= xmax + 0.2 })
                        .unwrap_or(false)
                }).unwrap_or(false);
                if in_lane {
                    let ball_y = self.bodies.get(ball_h).map(|b| b.translation().y).unwrap_or(0.0);
                    if ball_y > self.gate_y {
                        self.gate_cleared = true;
                        if let Some(col) = self.colliders.get_mut(gate_col_h) {
                            col.set_collision_groups(InteractionGroups::all());
                        }
                    }
                }
            }
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
        let pos = match self.ball_position() { Some(p) => p, None => return false };
        if pos.y >= self.drain_y { return false; }
        // Ball sitting in the shooter-lane dead zone is not a drain.
        if let Some([xmin, _, xmax, _]) = self.dead_zone {
            if pos.x >= xmin - 0.2 && pos.x <= xmax + 0.2 { return false; }
        }
        true
    }

    /// True when the ball is in the shooter lane and should be re-parked:
    /// - before gate clears: ball is inside the dead-zone y-range
    /// - after gate clears: ball returned down the lane and is sitting at or
    ///   below gate level (trapped on the gate or falling back toward it)
    pub fn is_ball_in_dead_zone(&self) -> bool {
        let dz = match self.dead_zone { Some(d) => d, None => return false };
        let [xmin, ymin, xmax, ymax] = dz;
        let pos = match self.ball_position() { Some(p) => p, None => return false };
        if !(pos.x >= xmin - 0.2 && pos.x <= xmax + 0.2) { return false; }
        if self.gate_cleared {
            // Ball returned down the lane: must be descending (vy ≤ 0) so a
            // freshly launched ball — still rising through the gate region —
            // is never mistaken for a returning one.
            let vy = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| b.linvel().y)
                .unwrap_or(0.0);
            vy <= 0.0 && pos.y <= self.gate_y + self.ball_radius + 0.2
        } else {
            pos.y >= ymin && pos.y <= ymax
        }
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

    /// Freeze the ball at the launch position with gravity off so it appears
    /// waiting in the shooter lane between balls.  Also reactivates any
    /// retract-when-hit walls (ball savers) for the new ball.
    pub fn park_ball(&mut self) {
        let [lx, ly] = self.launch_pos;
        if let Some(h) = self.ball_handle {
            if let Some(body) = self.bodies.get_mut(h) {
                body.set_translation(vector![lx, ly], true);
                body.set_linvel(vector![0.0, 0.0], true);
                body.set_angvel(0.0, true);
                body.set_gravity_scale(0.0, true);
            }
        }
        // Return ball to main-field layer.
        self.ball_layer = 0;
        if let Some(bc) = self.ball_collider {
            if let Some(col) = self.colliders.get_mut(bc) {
                col.set_collision_groups(ball_groups_for(0));
            }
        }
        // Re-open the shooter-lane gate for the next launch attempt.
        self.gate_cleared = false;
        if let Some(gate_col_h) = self.gate_collider {
            if let Some(col) = self.colliders.get_mut(gate_col_h) {
                col.set_collision_groups(InteractionGroups::none());
            }
        }
        // Reactivate ball-saver walls and re-enable their colliders.
        for kw in &mut self.kick_walls {
            if kw.retract_when_hit {
                kw.active = true;
                kw.retract_pending = false;
                if let Some(col) = self.colliders.get_mut(kw.collider_handle) {
                    col.set_collision_groups(InteractionGroups::all());
                }
            }
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
            FlipperSnapshot { x1: fl.pivot[0], y1: fl.pivot[1], x2, y2, label: fl.label.clone() }
        }).collect();

        let bumpers = self.bumpers.iter().map(|b| BumperSnapshot {
            x: b.position[0], y: b.position[1],
            radius: b.radius,
            outer_radius: b.outer_radius,
            lit: b.lit_ticks > 0,
            label: b.label.clone(),
            color: b.color,
            outer_color: b.outer_color,
        }).collect();

        let kick_walls = self.kick_walls.iter()
            .filter(|kw| kw.active)
            .map(|kw| WallSnapshot { x1: kw.x1, y1: kw.y1, x2: kw.x2, y2: kw.y2, layer: None, label: Some(kw.label.clone()), color: None })
            .collect();

        let drop_targets: Vec<WallSnapshot> = self.drop_target_groups.iter()
            .flat_map(|g| g.targets.iter().enumerate().filter(|(_, t)| t.active)
                .map(move |(i, t)| (g.label.clone(), g.color, i, t)))
            .map(|(glabel, color, i, t)| WallSnapshot {
                x1: t.x1, y1: t.y1, x2: t.x2, y2: t.y2, layer: None,
                label: Some(format!("{glabel}.{i}")),
                color,
            })
            .collect();

        let rollover_groups = self.rollover_groups.iter().map(|g| {
            // During the completion flash the renderer overrides all rollovers
            // to blink together.  4 phases: lit → unlit → lit → unlit → reset.
            let flash_override = if g.flash_ticks > 0 {
                let phase = (ROLLOVER_FLASH_TICKS - g.flash_ticks)
                    / (ROLLOVER_FLASH_TICKS / ROLLOVER_FLASH_PHASES);
                Some(phase % 2 == 1) // phase 0 → unlit immediately; phases 1,3 → lit
            } else {
                None
            };
            RolloverGroupSnapshot {
                rollovers: g.rollovers.iter().enumerate().map(|(i, rv)| RolloverSnapshot {
                    x: rv.x, y: rv.y, radius: rv.radius,
                    lit: flash_override.unwrap_or(rv.lit),
                    label: format!("{}.{}", g.label, i),
                    color: rv.color,
                }).collect(),
            }
        }).collect();

        let spinners = self.spinners.iter().map(|sp| SpinnerSnapshot {
            x: sp.position[0], y: sp.position[1],
            radius: sp.radius,
            spinning: sp.status != SpinnerStatus::Inactive,
            flash_on: sp.status == SpinnerStatus::ActiveVisible,
            label: sp.label.clone(),
        }).collect();

        FieldSnapshot {
            ball,
            walls: self.wall_segments.clone(),
            kick_walls,
            bumpers,
            flippers,
            drop_targets,
            rollover_groups,
            spinners,
            field_width:  self.field_width,
            field_height: self.field_height,
            ball_layer: self.ball_layer,
            ball_color: self.ball_color,
        }
    }
}

#[cfg(test)]
mod spinner_tests {
    use super::*;
    use crate::table::load_table;

    #[test]
    fn spinner_activates_flashes_and_scores_then_decays_to_idle() {
        let table = load_table(8).expect("table8.json should load (regression check for the -1 layer bug)");
        let mut world = PhysicsWorld::from_table(&table);
        assert_eq!(world.spinners.len(), 1, "table 8 should have exactly one spinner (LeftOrbitSpinner)");

        // Drive the ball straight through the spinner's zone at a speed well
        // above its activation threshold.
        let sp = &world.spinners[0];
        let [sx, sy] = sp.position;
        assert!((sx - 1.26).abs() < 0.01 && (sy - 19.19).abs() < 0.01);
        let ball_h = world.ball_handle.expect("ball should exist");
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(vector![sx, sy], true);
        body.set_linvel(vector![0.0, 12.0], true); // well above default minActivationSpeed=4
        body.set_gravity_scale(0.0, true); // isolate spinner behavior from gravity/drain

        let input = InputState::default();
        world.step(1.0 / 60.0, &input);
        assert_ne!(world.spinners[0].status, SpinnerStatus::Inactive, "spinner should activate once the fast ball crosses its zone");

        // Run enough steps for several flash cycles; collect score events.
        // (Other proximity-based mechanics near the spinner's coordinates,
        // e.g. bumpers, may also fire — so check that 500-pt spinner scores
        // occurred at all rather than asserting on the event stream's total.)
        let mut events = Vec::new();
        let mut saw_visible = false;
        for _ in 0..600 {
            world.step(1.0 / 60.0, &input);
            if world.spinners[0].status == SpinnerStatus::ActiveVisible {
                saw_visible = true;
            }
            events.extend(world.drain_score_events());
        }

        assert!(saw_visible, "spinner should flash to its visible phase at least once");
        let spinner_cycles = events.iter().filter(|&&pts| pts == 500).count();
        assert!(spinner_cycles >= 2, "spinner should score its configured 500 pts on multiple cycles, got {spinner_cycles} in {events:?}");
        assert_eq!(world.spinners[0].status, SpinnerStatus::Inactive, "spinner should decay back to idle (minCyclesPerSecond=1.5, decay=0.9) within 600 steps");
    }

    #[test]
    fn spinner_stays_idle_below_activation_speed() {
        let table = load_table(8).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let [sx, sy] = world.spinners[0].position;
        let ball_h = world.ball_handle.unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(vector![sx, sy], true);
        body.set_linvel(vector![0.0, 1.0], true); // below minActivationSpeed=4
        body.set_gravity_scale(0.0, true);

        let input = InputState::default();
        for _ in 0..30 {
            world.step(1.0 / 60.0, &input);
        }
        assert_eq!(world.spinners[0].status, SpinnerStatus::Inactive, "slow ball should not activate the spinner");
        assert!(world.drain_score_events().is_empty());
    }
}
