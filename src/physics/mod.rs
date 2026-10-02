use std::cell::RefCell;
mod movers;
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

/// Debug-overlay label for an element: its index into the table JSON's
/// `elements` array, plus its `id` when the table names it — e.g. "20" for
/// an anonymous wall, "20:LeftFlipper" for a named one. The index is always
/// included so it still cross-references the raw JSON even when named.
fn label_for(elem_idx: usize, id: &Option<String>) -> String {
    match id {
        Some(id) => format!("{elem_idx}:{id}"),
        None => elem_idx.to_string(),
    }
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
    /// Some(value) when this bumper has an award ladder — the points the
    /// *next* hit will award (the currently-armed rung).
    pub next_award: Option<u64>,
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
    pub letter_groups: Vec<LetterGroupSnapshot>,
    /// None = table has no jackpot; Some(lit) once one's configured.
    pub jackpot_lit: Option<bool>,
    pub field_width: f32,
    pub field_height: f32,
    pub ball_layer: u8,
    pub ball_color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct LetterGroupSnapshot {
    pub word: String,
    /// One entry per letter of `word`, in order.
    pub lit: Vec<bool>,
    /// True while the "word complete" celebration flash is running.
    pub flashing: bool,
}

// ---------------------------------------------------------------------------
// Internal bookkeeping
// ---------------------------------------------------------------------------

struct FlipperInfo {
    body_handle:  RigidBodyHandle,
    joint_handle: ImpulseJointHandle,
    pivot:        [f32; 2],
    half_len:     f32,
    min_angle:    f32,      // lower limit (radians, logical: 0 = horizontal, + = up, − = down)
    max_angle:    f32,      // upper limit (radians)
    up_speed:     f32,      // rad/s toward max_angle
    down_speed:   f32,      // rad/s toward min_angle
    is_right:     bool,
    label:        String,
}

struct BumperInfo {
    body: RigidBodyHandle,
    position: [f32; 2],
    radius: f32,
    outer_radius: Option<f32>,
    score: u64,
    kick_force: f32,
    lit_ticks: u8,
    label: String,
    color: Option<(u8, u8, u8)>,
    outer_color: Option<(u8, u8, u8)>,
    shot_id: Option<String>,
    // Empty = no award ladder; use the flat `score` field above unchanged.
    award_ladder: Vec<u64>,
    ladder_rung: usize,
    triggers_super_mode: bool,
    // True once this bumper has fired its super-mode trigger this ball —
    // reaching the top rung again (it holds there) won't re-fire it.
    super_mode_triggered: bool,
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
    shot_id: Option<String>,
}

/// The wall that closes the shooter lane's exit behind the ball so it can't
/// roll back in (original: element id "LaunchBarrier", closed by the
/// "LaunchBarrierSensor" rect and opened by "LaunchBarrierRetract").
struct LaunchBarrierInfo {
    x1: f32, y1: f32, x2: f32, y2: f32,
    color: Option<(u8, u8, u8)>,
    collider: ColliderHandle,
    active: bool,
    close_rect: Option<[f32; 4]>,
    open_rect: Option<[f32; 4]>,
}

const LAUNCH_BARRIER_ID: &str = "LaunchBarrier";

/// A wall a table script moves or retracts while playing (table 5's rotating
/// triangle). Drawn from its live position; not part of the static wall list.
struct MovingWallInfo {
    id: String,
    body: RigidBodyHandle,
    collider: ColliderHandle,
    x1: f32, y1: f32, x2: f32, y2: f32,
    color: Option<(u8, u8, u8)>,
    active: bool,
}

struct DropTargetInfo {
    x1: f32, y1: f32, x2: f32, y2: f32,
    collider: ColliderHandle,
    active: bool,
}

struct DropTargetGroupInfo {
    targets: Vec<DropTargetInfo>,
    // Sim seconds after the last target falls until all come back. Like the
    // original, a group with no reset (or 0) stays down once cleared.
    reset_delay: Option<f32>,
    reset_timer: Option<f32>,
    score:   u64,
    label:   String,
    color:   Option<(u8, u8, u8)>,
    shot_id: Option<String>,
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
    shot_id:          Option<String>,
    // 0 = completing this group doesn't touch the multiplier.
    multiplier_step:  u32,
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
    shot_id: Option<String>,
}

// A word (e.g. "MAD") spelled by hitting one named shot per letter, in any
// order. Not part of Vector-Pinball's format — see `LetterAwardDef`.
struct LetterGroupInfo {
    word: String,
    shot_ids: Vec<String>,
    lit: Vec<bool>,
    score: u64,
    // >0 while the "word complete" flash animation is running; 0 = idle.
    flash_ticks: u8,
    // True once this word has been completed at least once since the ball
    // started — independent of `lit`, which resets after every completion
    // so the word can be spelled again. Feeds the jackpot's light condition.
    completed_since_reset: bool,
}

// A shot that lights once every letter-award word has been completed at
// least once this ball; the next hit on it then collects `score` and
// un-lights. Not part of Vector-Pinball's format — see `JackpotDef`.
struct JackpotInfo {
    shot_id: String,
    score: u64,
    lit: bool,
}

// Tick-counted timers below were tuned for 2 physics substeps per frame;
// STEP_SCALE keeps their real-time length when PHYSICS_SUBSTEPS is higher.
const PHYSICS_SUBSTEPS: u32 = 8;
const STEP_SCALE:         u8  = (PHYSICS_SUBSTEPS / 2) as u8;
// Solver iterations: Box2D (the original's engine) used 10 velocity iterations.
const SOLVER_ITERATIONS:  usize = 10;
const BUMPER_LIT_TICKS:   u8  = 6 * STEP_SCALE;
// Kick/debounce is gameplay, so it follows sim time (substeps are ~9.5 ms of
// sim time now vs ~16.7 ms before): 12 substeps is ~110 ms, as 6 used to be.
const BUMPER_DEBOUNCE_TICKS: u8 = 12;
const KICK_WALL_DEBOUNCE: u8  = 12;
// Contact model, matching the original (Box2D) Vector Pinball where it can.
// Restitution matches exactly: ball and bumpers 0.6, everything else 0, and all
// colliders combine as max like Box2D. Friction does not match for the guides:
// the original's ball/wall friction is sqrt(0.3 * 0.2) = 0.245, but Rapier
// loses much more speed to it on the curved shooter lane (measured: only about
// 5 in 12 launches reached the top with it, vs 11 or 12 in 12 with almost none),
// so lane, wall, arc, path and drop-target friction is kept low. Flippers and
// bumpers keep the original's ball-contact friction (0.245 and 0.3), because
// that is where grip and cradling matter. Rapier averages the two surfaces'
// coefficients, so each is chosen to give that pair value with the ball's.
const BALL_RESTITUTION:   f32 = 0.6;
const BALL_FRICTION:      f32 = 0.05;
const WALL_FRICTION:      f32 = 0.05;
const FLIPPER_FRICTION:   f32 = 0.44;   // (0.05 + 0.44) / 2 = 0.245
const BUMPER_FRICTION:    f32 = 0.55;   // (0.05 + 0.55) / 2 = 0.3
const WALL_RESTITUTION:   f32 = 0.0;
// Restitution of the ball against a kick wall, applied by hand before the kick.
const KICK_WALL_BOUNCE:   f32 = BALL_RESTITUTION;
const DROP_CLEAR_BONUS:   u64 = 2_000;
const LETTER_FLASH_TICKS: u8  = 32 * STEP_SCALE;
// point_to_segment_dist measures to the wall's centerline, not its physical
// surface — the wall itself has a half-thickness of 0.05 (see add_wall), so
// `ball_radius + 0.05` alone is the exact geometric touch point with zero
// real slack, not a margin on top of it. On a shallow/glancing approach the
// ball can settle a hair short of that (deflected by neighboring geometry
// before fully closing the gap) and never register. This adds real slack:
// wall half-thickness (0.05) + an actual buffer (0.05).
const KICK_WALL_TOUCH_MARGIN: f32 = 0.10;

// Flipper motor, matching Vector-Pinball's Box2D revolute-joint setup
// (density 5.0, maxMotorTorque 1000). The joint's ForceBased motor model
// solves force = damping * (vel - target_vel), clamped to ±max_force — a
// high damping value makes the motor converge to its target velocity
// almost immediately whenever there's a nonzero error, so max_force (the
// torque cap) is what actually governs acceleration, same as Box2D.
const FLIPPER_DENSITY:       f32 = 5.0;
const FLIPPER_MAX_TORQUE:    f32 = 1000.0;
const FLIPPER_MOTOR_DAMPING: f32 = 1.0e4;
// Once the flipper is within this many radians of its "up" limit while
// still moving up faster than this speed, the motor target is clamped so
// a ball resting at the top doesn't get launched by the final slam.
const FLIPPER_TOP_SLOWDOWN_ANGLE: f32 = 0.05;
const FLIPPER_TOP_SLOWDOWN_SPEED: f32 = 0.5;
// Rollover group completion flash: 4 phases × 8 sub-steps each.
// At 2 sub-steps per frame (30 fps) each phase ≈ 133 ms; total ≈ 533 ms.
// Phase 0 = unlit so the dots go dark immediately on completion (obvious feedback).
const ROLLOVER_FLASH_TICKS:  u8 = 32 * STEP_SCALE;
const ROLLOVER_FLASH_PHASES: u8 = 4;

/// Which ball saver a cleared drop-target bank re-arms, by element ids (the
/// original's delegates: Field1/3/4/5/6/7 use the "...Save" ids, table 2 uses
/// plain Left/Right).
fn saver_for_drop_bank(group_id: &str) -> Option<&'static str> {
    match group_id {
        "DropTargetLeftSave" | "DropTargetLeft" => Some("BallSaver-left"),
        "DropTargetRightSave" | "DropTargetRight" => Some("BallSaver-right"),
        _ => None,
    }
}

// ── Layer collision groups ─────────────────────────────────────────────────────
// Bits 0-3 correspond to layers 0-3.  Universal walls use ALL bits (like
// InteractionGroups::all()).  A ball on layer N has memberships=bit(N),
// filter=ALL, so it passes through walls of other layers unimpeded.

fn layer_group_for(layer: u8) -> InteractionGroups {
    let bit = Group::from_bits_truncate(1u32 << layer as u32);
    InteractionGroups::new(bit, bit, InteractionTestMode::And)
}

fn ball_groups_for(layer: u8) -> InteractionGroups {
    let bit = Group::from_bits_truncate(1u32 << layer as u32);
    InteractionGroups::new(bit, Group::ALL, InteractionTestMode::And)
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
    broad_phase:   BroadPhaseBvh,
    narrow_phase:  NarrowPhase,
    ccd_solver:    CCDSolver,
    soft_bodies:   SoftBodySet,

    gravity:       Vector,

    // Game objects
    ball_handle:        Option<RigidBodyHandle>,
    ball_collider:  Option<ColliderHandle>,   // collider for the ball body
    ball_radius:        f32,
    time_ratio:         f32,
    ball_color:         Option<(u8, u8, u8)>,
    flippers:           Vec<FlipperInfo>,
    bumpers:            Vec<BumperInfo>,
    rotating_groups:    Vec<movers::RotatingGroup>,
    launch_barrier:     Option<LaunchBarrierInfo>,
    moving_walls:       Vec<MovingWallInfo>,
    rotating_triangles: Vec<movers::RotatingTriangle>,
    kick_walls:         Vec<KickWallInfo>,
    drop_target_groups: Vec<DropTargetGroupInfo>,
    rollover_groups:    Vec<RolloverGroupInfo>,
    spinners:           Vec<SpinnerInfo>,
    letter_groups:      Vec<LetterGroupInfo>,
    jackpot:            Option<JackpotInfo>,

    // Pre-computed wall segments for rendering (no physics body needed to query)
    wall_segments: Vec<WallSnapshot>,

    // Layer system: sensors that transition the ball between layers.
    layer_sensors: Vec<LayerSensorInfo>,
    ball_layer:    u8,

    // Events accumulated during step; drained by game / audio layers
    score_events:  RefCell<Vec<u64>>,
    sound_events:  RefCell<Vec<PhysicsSound>>,
    // Signed multiplier deltas (currently only ever positive) from
    // multiplier-step rollover groups completing.
    multiplier_events: RefCell<Vec<i32>>,
    // Count of "start (or restart) the timed super mode" triggers this step.
    super_mode_events: RefCell<u32>,

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
        let mut launch_barrier: Option<LaunchBarrierInfo> = None;
        let mut moving_walls: Vec<MovingWallInfo> = Vec::new();
        let (mut barrier_close_rect, mut barrier_open_rect) = (None, None);

        // Gravity points downward in rapier (negative y)
        let g = table.gravity_magnitude();
        let gravity = Vector::new(0.0_f32, -g);

        for (elem_idx, elem) in table.elements.iter().enumerate() {
            match elem {
                ElementDef::WallElement(w) => {
                    let label = label_for(elem_idx, &w.id);
                    let wall_col = Self::add_wall(w, &mut bodies, &mut colliders, &mut wall_segs, w.layer, Some(label.clone()), parse_color(&w.color));
                    if let (Some(id), Some(col), Some([x1, y1, x2, y2])) = (w.id.as_deref(), wall_col, w.position) {
                        if id.starts_with("TriangleWall") {
                            if let Some(body) = colliders.get(col).and_then(|c| c.parent()) {
                                wall_segs.pop();
                                moving_walls.push(MovingWallInfo {
                                    id: id.to_string(), body, collider: col,
                                    x1, y1, x2, y2, color: parse_color(&w.color), active: true,
                                });
                            }
                        }
                    }
                    if w.id.as_deref() == Some(LAUNCH_BARRIER_ID) {
                        if let (Some(col), Some([x1, y1, x2, y2])) = (wall_col, w.position) {
                            // Starts open; drawn and solid only while closed.
                            colliders.get_mut(col).unwrap().set_collision_groups(InteractionGroups::none());
                            wall_segs.pop();
                            launch_barrier = Some(LaunchBarrierInfo {
                                x1, y1, x2, y2,
                                color: parse_color(&w.color),
                                collider: col,
                                active: false,
                                close_rect: None,
                                open_rect: None,
                            });
                        }
                    }
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
                                shot_id: w.id.clone(),
                            });
                            // Kick walls are rendered via the kick_walls path only;
                            // remove the segment add_wall just pushed to wall_segs.
                            wall_segs.pop();
                        }
                    }
                }
                ElementDef::WallArcElement(a)   => Self::add_arc(a, &mut bodies, &mut colliders, &mut wall_segs, label_for(elem_idx, &a.id)),
                ElementDef::WallPathElement(p)  => Self::add_path(p, &mut bodies, &mut colliders, &mut wall_segs, label_for(elem_idx, &p.id)),
                ElementDef::BumperElement(b)    => Self::add_bumper(b, &mut bodies, &mut colliders, &mut bumpers_info, label_for(elem_idx, &b.id)),
                ElementDef::FlipperElement(f)   => Self::add_flipper(f, &mut bodies, &mut colliders, &mut impulse_joints, &mut flippers, label_for(elem_idx, &f.id)),
                ElementDef::DropTargetGroupElement(d) => {
                    drop_target_groups.push(Self::build_drop_target_group(d, &mut bodies, &mut colliders, label_for(elem_idx, &d.id)));
                }
                ElementDef::RolloverGroupElement(r) => {
                    rollover_groups.push(Self::build_rollover_group(r, label_for(elem_idx, &r.id)));
                }
                ElementDef::SpinnerElement(s) => {
                    spinners.push(Self::build_spinner(s, label_for(elem_idx, &s.id)));
                }
                ElementDef::SensorElement(s) => {
                    match s.id.as_deref() {
                        Some("LaunchBarrierSensor")  => barrier_close_rect = s.rect,
                        Some("LaunchBarrierRetract") => barrier_open_rect  = s.rect,
                        _ => {}
                    }
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

        if let Some(b) = &mut launch_barrier {
            b.close_rect = barrier_close_rect;
            b.open_rect = barrier_open_rect;
        }

        let ball_radius = table.ball_radius();
        let ball_color  = table.ball_color.map(|[r, g, b]| (r, g, b));
        let launch_pos        = table.launch_position.unwrap_or([table.width * 0.95, ball_radius + 0.5]);
        let launch_vel        = table.launch_velocity.unwrap_or([0.0, g * 3.0]);
        let launch_vel_delta  = table.launch_velocity_random_delta.unwrap_or([0.0, 0.0]);

        // No physical floor — the kill wall at y≈0 is detected via drain_y.

        // Left and right outer boundary walls
        for (x, w) in [(0.0_f32, table.width), (table.width, table.width)] {
            let rb = RigidBodyBuilder::fixed().translation(Vector::new(x, table.height / 2.0)).build();
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
                RigidBodyBuilder::fixed().translation(Vector::new(cx, gate_y)).build()
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
            .translation(Vector::new(lx, ly))
            .gravity_scale(0.0)
            .build();
        let idle_h = bodies.insert(idle_rb);
        let idle_col = ColliderBuilder::ball(ball_radius)
            .restitution(BALL_RESTITUTION)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(BALL_FRICTION)
            .density(1.0)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .collision_groups(ball_groups_for(0))
            .build();
        let ball_col_h = colliders.insert_with_parent(idle_col, idle_h, &mut bodies);
        let ball_handle   = Some(idle_h);
        let ball_collider = Some(ball_col_h);

        let letter_groups = table.letter_awards.iter().map(|la| LetterGroupInfo {
            word:     la.word.clone(),
            lit:      vec![false; la.shots.len()],
            shot_ids: la.shots.clone(),
            score:    la.score.unwrap_or(0),
            flash_ticks: 0,
            completed_since_reset: false,
        }).collect();

        let jackpot = table.jackpot.as_ref().map(|j| JackpotInfo {
            shot_id: j.shot.clone(),
            score:   j.score,
            lit:     false,
        });

        let mut world = PhysicsWorld {
            bodies,
            colliders,
            impulse_joints,
            multibody_joints,
            pipeline:  PhysicsPipeline::new(),
            islands:   IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            ccd_solver: CCDSolver::new(),
            soft_bodies: SoftBodySet::new(),
            gravity,
            time_ratio: table.time_ratio(),
            ball_handle,
            ball_collider,
            ball_radius,
            ball_color,
            flippers,
            rotating_groups: movers::build_rotating_groups(table, &bumpers_info),
            rotating_triangles: movers::build_rotating_triangles(table, &moving_walls),
            moving_walls,
            launch_barrier,
            bumpers: bumpers_info,
            kick_walls,
            drop_target_groups,
            wall_segments: wall_segs,
            rollover_groups,
            spinners,
            letter_groups,
            jackpot,
            layer_sensors,
            ball_layer: 0,
            score_events: RefCell::new(Vec::new()),
            sound_events: RefCell::new(Vec::new()),
            multiplier_events: RefCell::new(Vec::new()),
            super_mode_events: RefCell::new(0),
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
        };
        // Put moving elements at their first-frame positions straight away,
        // as the original does on its first tick, so nothing jumps on frame one.
        movers::advance_rotating_groups(&mut world.rotating_groups, 0.0, &mut world.bumpers, &mut world.bodies);
        movers::advance_rotating_triangles(&mut world.rotating_triangles, 0.0, &mut world.moving_walls, &mut world.bodies);
        world
    }

    // -----------------------------------------------------------------------
    // Element builders
    // -----------------------------------------------------------------------

    fn add_wall(w: &WallDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>, layer: Option<u8>, label: Option<String>, color: Option<(u8, u8, u8)>) -> Option<ColliderHandle> {
        let pos = match &w.position { Some(p) => *p, None => return None };
        // Disabled walls are skipped, except the launch barrier, which starts
        // open and is closed later by its sensor.
        let is_barrier = w.id.as_deref() == Some(LAUNCH_BARRIER_ID);
        if (w.disabled == Some(true) && !is_barrier) || w.kill == Some(true) { return None; }

        let [x1, y1, x2, y2] = pos;
        let cx = (x1 + x2) / 2.0;
        let cy = (y1 + y2) / 2.0;
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();
        let angle = dy.atan2(dx);
        let half_w = len / 2.0;
        let half_h = 0.05_f32;
        let restitution = w.restitution.unwrap_or(WALL_RESTITUTION);

        let groups = match layer {
            Some(n) => layer_group_for(n),
            None    => InteractionGroups::all(),
        };

        let rb = RigidBodyBuilder::fixed()
            .translation(Vector::new(cx, cy))
            .rotation(angle)
            .build();
        let h = bodies.insert(rb);
        let col = ColliderBuilder::cuboid(half_w, half_h)
            .restitution(restitution)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(WALL_FRICTION)
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
                id: None, color: None, kick: None, restitution: None,
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
        let rb = RigidBodyBuilder::fixed().translation(Vector::new(x, y)).build();
        let h = bodies.insert(rb);
        let col = ColliderBuilder::ball(b.radius)
            .restitution(BALL_RESTITUTION)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(BUMPER_FRICTION)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();
        colliders.insert_with_parent(col, h, bodies);

        bumpers.push(BumperInfo {
            body: h,
            position: b.position,
            radius: b.radius,
            outer_radius: b.outer_radius,
            score: b.score.unwrap_or(100),
            kick_force: b.kick.unwrap_or(8.0),
            lit_ticks: 0,
            award_ladder: b.award_ladder.clone().unwrap_or_default(),
            ladder_rung: 0,
            triggers_super_mode: b.triggers_super_mode,
            super_mode_triggered: false,
            label,
            color: parse_color(&b.color),
            outer_color: parse_color(&b.outer_color),
            shot_id: b.id.clone(),
        });
    }

    fn add_flipper(
        f: &FlipperDef,
        bodies: &mut RigidBodySet,
        colliders: &mut ColliderSet,
        impulse_joints: &mut ImpulseJointSet,
        flippers: &mut Vec<FlipperInfo>,
        label: String,
    ) {
        let half_len   = f.abs_length() / 2.0;
        let is_right   = f.is_right_flipper();
        let min_angle  = f.minangle.to_radians();
        let max_angle  = f.maxangle.to_radians();
        let init_angle = min_angle; // start fully down

        let (cx, cy, rot) = Self::flipper_pose(f.position, half_len, init_angle, is_right);

        // Matches Vector-Pinball's Box2D setup: a dynamic body on a motorized
        // revolute joint (density 5.0, CCD/"bullet" enabled), not a kinematic
        // body — so the ball's own momentum genuinely affects the flipper,
        // instead of the flipper being an unstoppable, forceless puppet.
        //
        // can_sleep(false): a flipper resting at its limit for a few seconds
        // (no ball nearby) is exactly the low-velocity state Rapier's sleep
        // system is designed to catch — and unlike the old kinematic body,
        // this one actually can fall asleep. Once asleep, changing the
        // joint's motor target does *not* wake it back up on its own, so the
        // flipper would go permanently unresponsive the first time a player
        // left it alone for a while. Flippers are few and need to always be
        // live, so the cost of never letting them sleep is negligible.
        let flipper_body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(cx, cy))
            .rotation(rot)
            .ccd_enabled(true)
            .can_sleep(false)
            .build();
        let h = bodies.insert(flipper_body);
        let col = ColliderBuilder::cuboid(half_len, 0.12)
            .density(FLIPPER_DENSITY)
            .restitution(WALL_RESTITUTION)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(FLIPPER_FRICTION)
            .build();
        colliders.insert_with_parent(col, h, bodies);

        let anchor_body = RigidBodyBuilder::fixed()
            .translation(Vector::new(f.position[0], f.position[1]))
            .build();
        let anchor_h = bodies.insert(anchor_body);

        // In the flipper's local (unrotated) frame the pivot sits at the
        // -half_len end for a left flipper and the +half_len end for a
        // right one — see flipper_pose's doc comment for the derivation.
        let local_pivot_x = if is_right { half_len } else { -half_len };
        // The anchor body never rotates, so the joint's raw angle equals the
        // flipper body's absolute rotation directly — which is +logical_angle
        // for a left flipper and -logical_angle for a right one (again per
        // flipper_pose), so the limits and motor target need the same flip.
        let raw_limits = if is_right { [-max_angle, -min_angle] } else { [min_angle, max_angle] };
        let joint = RevoluteJointBuilder::new()
            .local_anchor1(Vector::new(0.0, 0.0))
            .local_anchor2(Vector::new(local_pivot_x, 0.0))
            .limits(raw_limits)
            .motor_model(MotorModel::ForceBased)
            .motor_max_force(FLIPPER_MAX_TORQUE)
            // Force flipper to bottom when the field is first created, same as
            // Vector-Pinball's FlipperElement.createBodies().
            .motor_velocity(Self::signed_motor_target(-f.downspeed, is_right), FLIPPER_MOTOR_DAMPING)
            .build();
        let joint_h = impulse_joints.insert(anchor_h, h, joint, true);

        flippers.push(FlipperInfo {
            body_handle: h,
            joint_handle: joint_h,
            pivot:       f.position,
            half_len,
            min_angle,
            max_angle,
            up_speed:    f.upspeed,
            down_speed:  f.downspeed,
            is_right,
            label,
        });
    }

    /// Converts a logical motor target (+ toward max_angle, − toward
    /// min_angle) into the raw joint-motor target, flipping sign for right
    /// flippers per flipper_pose's angle convention.
    fn signed_motor_target(logical_target: f32, is_right: bool) -> f32 {
        if is_right { -logical_target } else { logical_target }
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
                    .translation(Vector::new(cx, cy))
                    .rotation(angle)
                    .build()
            );
            let col = ColliderBuilder::cuboid(len / 2.0, 0.05)
                .restitution(WALL_RESTITUTION)
                .restitution_combine_rule(CoefficientCombineRule::Max)
                .friction(WALL_FRICTION)
                .build();
            let ch = colliders.insert_with_parent(col, bh, bodies);
            DropTargetInfo { x1, y1, x2, y2, collider: ch, active: true }
        }).collect();
        DropTargetGroupInfo {
            targets,
            reset_delay: d.reset.filter(|&r| r > 0.0),
            reset_timer: None,
            score: d.score.unwrap_or(0),
            label,
            color: parse_color(&d.color),
            shot_id: d.id.clone(),
        }
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
            shot_id: r.id.clone(),
            multiplier_step: r.multiplier_step.unwrap_or(0),
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
            shot_id: s.id.clone(),
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
                body.set_translation(Vector::new(lx, ly), true);
                body.set_linvel(Vector::new(actual_vx, actual_vy), true);
                body.set_angvel(0.0, true);
                body.set_gravity_scale(1.0, true);
                body.enable_ccd(true);
            }
        } else {
            let rb = RigidBodyBuilder::dynamic()
                .translation(Vector::new(lx, ly))
                .linvel(Vector::new(actual_vx, actual_vy))
                .ccd_enabled(true)
                .build();
            let h = self.bodies.insert(rb);
            let col = ColliderBuilder::ball(self.ball_radius)
                .restitution(BALL_RESTITUTION)
                .restitution_combine_rule(CoefficientCombineRule::Max)
                .friction(BALL_FRICTION)
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

        // Drive flippers via their motorized joint (see add_flipper). Unlike
        // Vector-Pinball's clean Android touch events, our InputState.left/
        // right_flipper is a decaying counter that terminal key-repeat events
        // keep refreshing — it can flicker false for a tick between repeats
        // even while the key is physically held. So the motor target is set
        // unconditionally every tick from the current state (like the old
        // kinematic code did), not gated on a state *change*: gating on
        // change made every flicker look like a real press/release, and the
        // motor kept getting told to reverse direction faster than it could
        // actually move. The top-of-swing slowdown is applied right after,
        // every tick, so it still overrides the full-speed target near the
        // limit without needing that gating either.
        for fl in &self.flippers {
            let should_engage = (fl.is_right && input.right_flipper)
                              || (!fl.is_right && input.left_flipper);
            let logical_target = if should_engage { fl.up_speed } else { -fl.down_speed };
            if let Some(joint) = self.impulse_joints.get_mut(fl.joint_handle, true) {
                joint.data.set_motor_velocity(
                    JointAxis::AngX,
                    Self::signed_motor_target(logical_target, fl.is_right),
                    FLIPPER_MOTOR_DAMPING,
                );
            }

            if should_engage {
                if let Some(body) = self.bodies.get(fl.body_handle) {
                    let raw_angle = body.rotation().angle();
                    let logical_angle = if fl.is_right { -raw_angle } else { raw_angle };
                    if (fl.max_angle - logical_angle).abs() < FLIPPER_TOP_SLOWDOWN_ANGLE {
                        if let Some(joint) = self.impulse_joints.get_mut(fl.joint_handle, true) {
                            joint.data.set_motor_velocity(
                                JointAxis::AngX,
                                Self::signed_motor_target(FLIPPER_TOP_SLOWDOWN_SPEED, fl.is_right),
                                FLIPPER_MOTOR_DAMPING,
                            );
                        }
                    }
                }
            }
        }

        let mut params = IntegrationParameters::default();
        params.dt = dt;
        params.num_solver_iterations = SOLVER_ITERATIONS;

        self.pipeline.step(
            self.gravity,
            &params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd_solver,
            &(),
            &(),
        );

        // Moving table elements (orbiting bumpers etc.) advance in sim time,
        // after the physics step like the original's per-frame element ticks.
        movers::advance_rotating_groups(&mut self.rotating_groups, dt, &mut self.bumpers, &mut self.bodies);
        movers::advance_rotating_triangles(&mut self.rotating_triangles, dt, &mut self.moving_walls, &mut self.bodies);

        // Launch barrier: closes once the ball passes its sensor, opens again
        // at the retract sensor (checked second, so it wins if they overlap).
        if let (Some(barrier), Some(ball_h)) = (&mut self.launch_barrier, self.ball_handle) {
            if let Some(body) = self.bodies.get(ball_h) {
                let p = body.translation();
                let inside = |r: Option<[f32; 4]>| r.map_or(false, |[x0, y0, x1, y1]| p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1);
                let was = barrier.active;
                if inside(barrier.close_rect) { barrier.active = true; }
                if inside(barrier.open_rect) { barrier.active = false; }
                if barrier.active != was {
                    let groups = if barrier.active { InteractionGroups::all() } else { InteractionGroups::none() };
                    if let Some(col) = self.colliders.get_mut(barrier.collider) {
                        col.set_collision_groups(groups);
                    }
                }
            }
        }

        // Bumper proximity detection: collect hits, then apply kicks + score
        {
            let mut hits: Vec<(usize, f32, f32)> = Vec::new(); // (index, nx, ny)
            if let Some(ball_h) = self.ball_handle {
                if let Some(ball_body) = self.bodies.get(ball_h) {
                    let bp = ball_body.translation();
                    for (i, bump) in self.bumpers.iter().enumerate() {
                        if bump.lit_ticks + BUMPER_DEBOUNCE_TICKS > BUMPER_LIT_TICKS { continue; }   // debounce right after a hit
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
                let bump = &mut self.bumpers[i];
                if bump.award_ladder.is_empty() {
                    self.score_events.borrow_mut().push(bump.score);
                } else {
                    let last_rung = bump.award_ladder.len() - 1;
                    self.score_events.borrow_mut().push(bump.award_ladder[bump.ladder_rung]);
                    bump.ladder_rung = (bump.ladder_rung + 1).min(last_rung);
                    if bump.triggers_super_mode && bump.ladder_rung == last_rung && !bump.super_mode_triggered {
                        bump.super_mode_triggered = true;
                        *self.super_mode_events.borrow_mut() += 1;
                    }
                }
                self.sound_events.borrow_mut().push(PhysicsSound::BumperHit);
                if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &self.bumpers[i].shot_id) {
                    self.score_events.borrow_mut().push(pts);
                }
                let kick = self.bumpers[i].kick_force;
                if let Some(ball_h) = self.ball_handle {
                    if let Some(body) = self.bodies.get_mut(ball_h) {
                        body.apply_impulse(Vector::new(nx * kick, ny * kick), true);
                    }
                }
            }
        }

        // Tick bumper flash timers
        for b in &mut self.bumpers {
            if b.lit_ticks > 0 { b.lit_ticks -= 1; }
        }

        // Kick-wall (slingshot / ball-saver) proximity detection.
        //
        // Originally detected via Rapier's CollisionEvent::Started, matching
        // Vector-Pinball's Box2D handleCollision approach — but that only
        // reliably fires on a square, fairly fast hit. A slingshot is almost
        // always brushed at a shallow/glancing angle as the ball rolls past
        // (not hit head-on), and at that angle CollisionEvent::Started often
        // never arrives at all against these thin wall colliders, so the
        // ball would silently bounce off with no kick most of the time.
        // Proximity detection (same pattern as bumpers/drop-targets below)
        // can't miss a shallow graze the way a discrete collision event can:
        // it just checks distance-to-segment every step, independent of
        // approach angle or closing speed. Kick direction is always the
        // precomputed interior_normal so the ball is pushed away from the
        // pocket and into the field.
        {
            let mut hits: Vec<(usize, f32, f32)> = Vec::new();
            if let Some(ball_h) = self.ball_handle {
                if let Some(ball_body) = self.bodies.get(ball_h) {
                    let bp = ball_body.translation();
                    for (i, kw) in self.kick_walls.iter().enumerate() {
                        if !kw.active || kw.debounce > 0 { continue; }
                        let (dist, _, _) = Self::point_to_segment_dist(bp.x, bp.y, kw.x1, kw.y1, kw.x2, kw.y2);
                        if dist < self.ball_radius + KICK_WALL_TOUCH_MARGIN {
                            hits.push((i, kw.interior_normal[0], kw.interior_normal[1]));
                        }
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
                if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &self.kick_walls[i].shot_id) {
                    self.score_events.borrow_mut().push(pts);
                }
                let kick = self.kick_walls[i].kick_force;
                if let Some(ball_h) = self.ball_handle {
                    if let Some(body) = self.bodies.get_mut(ball_h) {
                        // The kick fires on proximity, just before contact. The
                        // original applies it after the step has already bounced
                        // the ball off the wall, so do that bounce here first.
                        // Otherwise the kick only cancels the ball's approach
                        // speed, and a ball saver (which retracts right away)
                        // never actually throws the ball back into play.
                        let v = body.linvel();
                        let vn = v.x * nx + v.y * ny;
                        if vn < 0.0 {
                            let bounced = v - Vector::new(nx, ny) * ((1.0 + KICK_WALL_BOUNCE) * vn);
                            body.set_linvel(bounced, true);
                        }
                        body.apply_impulse(Vector::new(nx * kick, ny * kick), true);
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
                    let bp = ball_body.translation();
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
            let mut savers_to_restore: Vec<&'static str> = Vec::new();
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
                    if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &self.drop_target_groups[gi].shot_id) {
                        self.score_events.borrow_mut().push(pts);
                    }
                    // The targets come back after the group's reset delay (in
                    // sim time), or never if it has none, as in the original.
                    self.drop_target_groups[gi].reset_timer = self.drop_target_groups[gi].reset_delay;
                    if let Some(saver) = self.drop_target_groups[gi].shot_id.as_deref().and_then(saver_for_drop_bank) {
                        savers_to_restore.push(saver);
                    }
                }
            }
            for saver in savers_to_restore {
                self.restore_ball_saver(saver);
            }
        }

        // Cleared drop-target groups restore once their reset delay has run.
        for gi in 0..self.drop_target_groups.len() {
            let Some(timer) = self.drop_target_groups[gi].reset_timer else { continue };
            let timer = timer - dt;
            if timer > 0.0 {
                self.drop_target_groups[gi].reset_timer = Some(timer);
                continue;
            }
            self.drop_target_groups[gi].reset_timer = None;
            for target in &mut self.drop_target_groups[gi].targets {
                target.active = true;
                if let Some(col) = self.colliders.get_mut(target.collider) {
                    col.set_collision_groups(InteractionGroups::all());
                }
            }
        }

        // Rollover detection, flipper cycling, and group-completion flash.
        {
            let ball_pos = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| b.translation());
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
                                if let Some(jp) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &group.shot_id) {
                                    self.score_events.borrow_mut().push(jp);
                                }
                            }
                        }
                    }
                }

                // ── Group completion: flash, then bonus + reset ──
                let all_lit = !group.rollovers.is_empty() && group.rollovers.iter().all(|rv| rv.lit);
                if Self::tick_completion_flash(&mut group.flash_ticks, ROLLOVER_FLASH_TICKS, all_lit) {
                    if group.score > 0 {
                        self.score_events.borrow_mut().push(group.score);
                    }
                    if group.multiplier_step > 0 {
                        self.multiplier_events.borrow_mut().push(group.multiplier_step as i32);
                    }
                    for rv in &mut group.rollovers {
                        rv.lit = false;
                    }
                }

                // ── Flipper cycling — suspend while flashing ──
                // Matches the original: a right press shifts the lit dots toward
                // higher indices (rightward), a left press toward lower indices,
                // and the right flipper wins if both fire on the same frame.
                if group.cycle_on_flipper && group.flash_ticks == 0 {
                    let n = group.rollovers.len();
                    if n > 1 {
                        if input.right_just_pressed {
                            let last = group.rollovers[n - 1].lit;
                            for i in (1..n).rev() {
                                group.rollovers[i].lit = group.rollovers[i - 1].lit;
                            }
                            group.rollovers[0].lit = last;
                        } else if input.left_just_pressed {
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
                .map(|b| (b.translation(), b.linvel()));
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
                                if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &sp.shot_id) {
                                    self.score_events.borrow_mut().push(pts);
                                }
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

        // Letter-award completion flash: shots were recorded into `lit` at
        // their respective detection sites above via record_shot_opt().
        for group in &mut self.letter_groups {
            let all_lit = !group.lit.is_empty() && group.lit.iter().all(|&l| l);
            if Self::tick_completion_flash(&mut group.flash_ticks, LETTER_FLASH_TICKS, all_lit) {
                if group.score > 0 {
                    self.score_events.borrow_mut().push(group.score);
                }
                group.completed_since_reset = true;
                for l in &mut group.lit { *l = false; }
            }
        }

        // Jackpot lights once every letter-award word has been completed at
        // least once this ball (requires at least one word to exist).
        if let Some(jp) = &mut self.jackpot {
            if !jp.lit
                && !self.letter_groups.is_empty()
                && self.letter_groups.iter().all(|g| g.completed_since_reset)
            {
                jp.lit = true;
            }
        }

        // Layer-sensor detection: transition ball to a new physics layer when
        // it enters a sensor rect (and the optional from-layer filter matches).
        {
            let ball_pos = self.ball_handle
                .and_then(|h| self.bodies.get(h))
                .map(|b| b.translation());
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

    /// Re-arms the ball saver with this element id: visible again, and its
    /// collider back in the physics world.
    fn restore_ball_saver(&mut self, id: &str) {
        for kw in &mut self.kick_walls {
            if kw.retract_when_hit && kw.shot_id.as_deref() == Some(id) {
                kw.active = true;
                kw.retract_pending = false;
                if let Some(col) = self.colliders.get_mut(kw.collider_handle) {
                    col.set_collision_groups(InteractionGroups::all());
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

    pub fn ball_position(&self) -> Option<Vector> {
        let h = self.ball_handle?;
        let body = self.bodies.get(h)?;
        Some(body.translation())
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

    pub fn drain_multiplier_events(&self) -> Vec<i32> {
        self.multiplier_events.borrow_mut().drain(..).collect()
    }

    pub fn drain_super_mode_events(&self) -> u32 {
        std::mem::replace(&mut *self.super_mode_events.borrow_mut(), 0)
    }

    /// Lights the letter (in any letter-award word) whose shot id matches,
    /// and collects the jackpot if it's lit and this shot is the one that
    /// collects it — returning its score for the caller to apply. A word
    /// mid-completion-flash is left alone until it resets. Takes the
    /// letter-groups slice and jackpot directly rather than `&mut self` so
    /// it can be called from inside a loop that already holds a disjoint
    /// borrow of another `self` field (e.g. `for group in &mut self.rollover_groups`).
    fn record_shot(letter_groups: &mut [LetterGroupInfo], jackpot: &mut Option<JackpotInfo>, id: &str) -> Option<u64> {
        for group in letter_groups {
            if group.flash_ticks > 0 { continue; }
            if let Some(pos) = group.shot_ids.iter().position(|s| s == id) {
                group.lit[pos] = true;
            }
        }
        let jp = jackpot.as_mut()?;
        if jp.lit && jp.shot_id == id {
            jp.lit = false;
            return Some(jp.score);
        }
        None
    }

    /// Convenience wrapper for the common "shot id, if the element is named" case.
    fn record_shot_opt(letter_groups: &mut [LetterGroupInfo], jackpot: &mut Option<JackpotInfo>, id: &Option<String>) -> Option<u64> {
        id.as_deref().and_then(|id| Self::record_shot(letter_groups, jackpot, id))
    }

    /// Advances a group's completion-flash timer: starts it (for
    /// `flash_duration` ticks) the moment `all_lit` goes true while idle,
    /// then counts down. Returns true on exactly the tick it reaches zero —
    /// the caller's cue to award its bonus and reset. Shared by
    /// `RolloverGroupInfo` and `LetterGroupInfo`, which both use this
    /// flash-then-reset pattern for group completion.
    fn tick_completion_flash(flash_ticks: &mut u8, flash_duration: u8, all_lit: bool) -> bool {
        if *flash_ticks == 0 && all_lit {
            *flash_ticks = flash_duration;
        }
        if *flash_ticks > 0 {
            *flash_ticks -= 1;
            return *flash_ticks == 0;
        }
        false
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
                &mut self.soft_bodies,
                false,
            );
        }
    }

    /// Freeze the ball at the launch position with gravity off so it appears
    /// waiting in the shooter lane between balls.  Also resets any bumper award
    /// ladders (and their super-mode trigger) to fire again, and un-lights
    /// the jackpot (and clears letter-award completion progress toward it)
    /// for the new ball.
    pub fn park_ball(&mut self) {
        let [lx, ly] = self.launch_pos;
        if let Some(h) = self.ball_handle {
            if let Some(body) = self.bodies.get_mut(h) {
                body.set_translation(Vector::new(lx, ly), true);
                body.set_linvel(Vector::new(0.0, 0.0), true);
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
        // New ball: the launch barrier opens again (original ballLost/gameStarted).
        if let Some(barrier) = &mut self.launch_barrier {
            barrier.active = false;
            if let Some(col) = self.colliders.get_mut(barrier.collider) {
                col.set_collision_groups(InteractionGroups::none());
            }
        }
        // Ball savers are NOT re-armed here: in the original a used saver stays
        // gone until the matching drop-target bank is cleared (see
        // `saver_for_drop_bank`), not on every new ball.
        // Reset award-ladder bumpers to their first rung for the new ball,
        // and let their super-mode trigger fire again.
        for bump in &mut self.bumpers {
            bump.ladder_rung = 0;
            bump.super_mode_triggered = false;
        }
        // The jackpot must be earned fresh each ball: un-light it and clear
        // every word's "completed this ball" flag.
        if let Some(jp) = &mut self.jackpot {
            jp.lit = false;
        }
        for group in &mut self.letter_groups {
            group.completed_since_reset = false;
        }
    }

    pub fn field_height(&self) -> f32 { self.field_height }

    /// Current ball speed in sim units per sim second, if a ball exists.
    #[cfg(test)]
    fn ball_speed(&self) -> Option<f32> {
        let body = self.bodies.get(self.ball_handle?)?;
        Some(body.linvel().length())
    }

    /// Advances one rendered frame of `frame_secs` real seconds. Like the
    /// original Vector Pinball, sim time runs faster than real time (the
    /// table's `targetTimeRatio`), split into small substeps for accuracy.
    pub fn step_frame(&mut self, frame_secs: f32, input: &InputState) {
        let dt = frame_secs * self.time_ratio / PHYSICS_SUBSTEPS as f32;
        // `*_just_pressed` is a one-frame edge (e.g. it rotates the lit
        // rollovers once per flipper press), so only the first substep may
        // see it; otherwise one press would act once per substep.
        let mut later = input.clone();
        later.left_just_pressed = false;
        later.right_just_pressed = false;
        self.step(dt, input);
        for _ in 1..PHYSICS_SUBSTEPS {
            self.step(dt, &later);
        }
    }
    /// Sim seconds advanced per real second (the table's `targetTimeRatio`).
    pub fn time_ratio(&self) -> f32 { self.time_ratio }
    pub fn field_width(&self)  -> f32 { self.field_width }

    pub fn snapshot(&self) -> FieldSnapshot {
        let ball = self.ball_handle.and_then(|h| self.bodies.get(h)).map(|b| {
            let t = b.translation();
            let v = b.linvel();
            BallSnapshot { x: t.x, y: t.y, vx: v.x, vy: v.y }
        });

        // Read the tip position straight from the physics body's actual
        // transform (translation + rotation) rather than a separately
        // tracked angle, now that the flipper is a real dynamic body.
        let flippers: Vec<FlipperSnapshot> = self.flippers.iter().filter_map(|fl| {
            let body = self.bodies.get(fl.body_handle)?;
            let local_tip_x = if fl.is_right { -fl.half_len } else { fl.half_len };
            let tip = body.position() * Vector::new(local_tip_x, 0.0);
            Some(FlipperSnapshot { x1: fl.pivot[0], y1: fl.pivot[1], x2: tip.x, y2: tip.y, label: fl.label.clone() })
        }).collect();

        let bumpers = self.bumpers.iter().map(|b| BumperSnapshot {
            x: b.position[0], y: b.position[1],
            radius: b.radius,
            outer_radius: b.outer_radius,
            lit: b.lit_ticks > 0,
            label: b.label.clone(),
            color: b.color,
            outer_color: b.outer_color,
            next_award: (!b.award_ladder.is_empty()).then(|| b.award_ladder[b.ladder_rung]),
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

        let letter_groups = self.letter_groups.iter().map(|g| LetterGroupSnapshot {
            word: g.word.clone(),
            lit: g.lit.clone(),
            flashing: g.flash_ticks > 0,
        }).collect();

        FieldSnapshot {
            ball,
            walls: self.wall_segments.iter().cloned().chain(self.moving_walls.iter().filter(|w| w.active).map(|w| WallSnapshot { x1: w.x1, y1: w.y1, x2: w.x2, y2: w.y2, layer: None, label: Some(w.id.clone()), color: w.color }))
                .chain(self.launch_barrier.iter().filter(|b| b.active).map(|b| WallSnapshot { x1: b.x1, y1: b.y1, x2: b.x2, y2: b.y2, layer: None, label: None, color: b.color })).collect(),
            kick_walls,
            bumpers,
            flippers,
            drop_targets,
            rollover_groups,
            spinners,
            letter_groups,
            jackpot_lit: self.jackpot.as_ref().map(|j| j.lit),
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
        body.set_translation(Vector::new(sx, sy), true);
        body.set_linvel(Vector::new(0.0, 12.0), true); // well above default minActivationSpeed=4
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
        body.set_translation(Vector::new(sx, sy), true);
        body.set_linvel(Vector::new(0.0, 1.0), true); // below minActivationSpeed=4
        body.set_gravity_scale(0.0, true);

        let input = InputState::default();
        for _ in 0..30 {
            world.step(1.0 / 60.0, &input);
        }
        assert_eq!(world.spinners[0].status, SpinnerStatus::Inactive, "slow ball should not activate the spinner");
        assert!(world.drain_score_events().is_empty());
    }
}

#[cfg(test)]
mod all_tables_smoke {
    use super::*;
    use crate::table::load_table;
    use crate::input::InputState;

    #[test]
    fn every_table_loads_and_steps_with_flippers_toggling() {
        for n in 1..=9u8 {
            let table = load_table(n).unwrap_or_else(|e| panic!("table{n} failed to load: {e}"));
            let mut world = PhysicsWorld::from_table(&table);
            world.launch_ball();
            let mut input = InputState::default();
            for i in 0..600 {
                input.left_flipper = (i / 15) % 2 == 0;
                input.right_flipper = (i / 15) % 2 == 1;
                world.step(1.0 / 60.0, &input);
                if world.is_ball_drained() {
                    world.park_ball();
                    world.launch_ball();
                }
                if world.is_ball_in_dead_zone() {
                    world.park_ball();
                    world.launch_ball();
                }
            }
            for fl in &world.flippers {
                let body = world.bodies.get(fl.body_handle).unwrap_or_else(|| panic!("table{n}: flipper body missing"));
                let raw = body.rotation().angle();
                assert!(raw.is_finite(), "table{n}: flipper angle is not finite: {raw}");
            }
        }
    }
}

#[cfg(test)]
mod game_speed_tests {
    use super::*;
    use crate::table::load_table;

    /// At the original's ~2.3x sim speed the ball must neither tunnel out of
    /// the field nor fail to leave the shooter lane on a first launch.
    #[test]
    fn every_table_ball_stays_in_bounds_and_clears_the_launch_lane() {
        for n in 1..=9u8 {
            let table = load_table(n).unwrap_or_else(|e| panic!("table{n} failed to load: {e}"));
            let mut world = PhysicsWorld::from_table(&table);
            world.launch_ball();
            let mut input = InputState::default();
            let (mut max_y, mut max_speed) = (0.0_f32, 0.0_f32);
            for i in 0..900 {
                input.left_flipper = (i / 20) % 2 == 0;
                input.right_flipper = (i / 20) % 2 == 1;
                world.step_frame(0.033, &input);
                if let Some(p) = world.ball_position() {
                    assert!(
                        p.x > -1.0 && p.x < table.width + 1.0 && p.y > -3.0 && p.y < table.height + 1.0,
                        "table{n}: ball left the field at frame {i}: ({:.2}, {:.2})", p.x, p.y
                    );
                    max_y = max_y.max(p.y);
                }
                if let Some(v) = world.ball_speed() { max_speed = max_speed.max(v); }
                if world.is_ball_drained() || world.is_ball_in_dead_zone() {
                    // First-launch check only; once it drains, restart.
                    world.park_ball();
                    world.launch_ball();
                }
            }
            eprintln!("table{n}: max y {max_y:.1} of {:.1}, max speed {max_speed:.1}", table.height);
            assert!(max_y > table.height * 0.5, "table{n}: ball never got above y={max_y:.1} (field height {})", table.height);
        }
    }
}

#[cfg(test)]
mod jackpot_tests {
    use super::*;
    use crate::table::load_table;

    #[test]
    fn jackpot_lights_after_all_words_completed_then_collects_and_resets() {
        let table = load_table(9).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        {
            let jp = world.jackpot.as_ref().expect("Party Land should configure a jackpot");
            assert_eq!(jp.shot_id, "GhoulSkull");
            assert_eq!(jp.score, 100_000);
            assert!(!jp.lit);
        }

        // Hitting the jackpot shot before it's lit shouldn't pay it out.
        let pts = PhysicsWorld::record_shot(&mut world.letter_groups, &mut world.jackpot, "GhoulSkull");
        assert_eq!(pts, None, "jackpot shouldn't pay out before it's lit");

        // Mark every word completed (bypassing the full spell-it-out flow,
        // already covered by letter_award_tests) and step once so the
        // jackpot-lighting check re-evaluates.
        for group in &mut world.letter_groups {
            group.completed_since_reset = true;
        }
        let input = InputState::default();
        world.step(1.0 / 60.0, &input);
        assert!(world.jackpot.as_ref().unwrap().lit, "jackpot should light once every word has been completed");

        // The same shot now collects it.
        let pts = PhysicsWorld::record_shot(&mut world.letter_groups, &mut world.jackpot, "GhoulSkull");
        assert_eq!(pts, Some(100_000));
        assert!(!world.jackpot.as_ref().unwrap().lit, "collecting un-lights it");

        // A new ball clears completion progress and (re-)un-lights the jackpot.
        world.jackpot.as_mut().unwrap().lit = true; // force-relight to prove park_ball resets it too
        world.park_ball();
        assert!(!world.jackpot.as_ref().unwrap().lit, "a new ball must be earned again");
        assert!(world.letter_groups.iter().all(|g| !g.completed_since_reset));
    }
}

#[cfg(test)]
mod multiplier_tests {
    use super::*;
    use crate::table::load_table;

    #[test]
    fn puke_rollovers_completion_raises_multiplier_then_resets_for_next_completion() {
        let table = load_table(9).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let gi = world.rollover_groups.iter().position(|g| g.multiplier_step > 0)
            .expect("Party Land should have exactly one multiplier-step rollover group (Puke Rollovers)");
        assert_eq!(world.rollover_groups[gi].shot_id.as_deref(), Some("PukeRollovers"));
        assert_eq!(world.rollover_groups[gi].multiplier_step, 1);
        assert_eq!(world.rollover_groups[gi].rollovers.len(), 4);

        // Light all four rollovers directly (bypassing ball-proximity
        // detection, which is already covered by other tests) to drive
        // the group straight to completion.
        for rv in &mut world.rollover_groups[gi].rollovers {
            rv.lit = true;
        }

        let input = InputState::default();
        let mut score_events = Vec::new();
        let mut multiplier_events = Vec::new();
        for _ in 0..=ROLLOVER_FLASH_TICKS as usize {
            world.step(1.0 / 60.0, &input);
            score_events.extend(world.drain_score_events());
            multiplier_events.extend(world.drain_multiplier_events());
        }

        assert!(score_events.contains(&2_000), "completion should still award its normal group bonus, got {score_events:?}");
        assert_eq!(multiplier_events, vec![1], "completion should raise the multiplier by its configured step");
        assert!(world.rollover_groups[gi].rollovers.iter().all(|rv| !rv.lit), "rollovers reset after completion, ready to build again");
    }
}

#[cfg(test)]
mod award_ladder_tests {
    use super::*;
    use crate::table::load_table;

    #[test]
    fn ghoul_skull_climbs_the_ladder_then_holds_and_resets_on_new_ball() {
        let table = load_table(9).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let gi = world.bumpers.iter().position(|b| b.shot_id.as_deref() == Some("GhoulSkull"))
            .expect("Party Land should have a GhoulSkull bumper");
        assert_eq!(world.bumpers[gi].award_ladder, vec![5_000, 10_000, 20_000, 35_000, 50_000]);

        // Park the ball directly on the bumper, motionless, so it re-triggers
        // every time the hit debounce (BUMPER_LIT_TICKS) expires. Re-pinned
        // every round below since each hit's kick impulse shoves it away.
        let [bx, by] = world.bumpers[gi].position;
        let ball_h = world.ball_handle.unwrap();

        let input = InputState::default();
        let mut hits = Vec::new();
        let mut super_mode_events_per_hit = Vec::new();
        for _ in 0..6 {
            let body = world.bodies.get_mut(ball_h).unwrap();
            body.set_translation(Vector::new(bx, by), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_gravity_scale(0.0, true);

            // Step until exactly one hit registers, rather than a fixed tick
            // count — the debounce (BUMPER_LIT_TICKS) can otherwise let a
            // second hit sneak in on the same round.
            let mut events_this_round = 0;
            loop {
                world.step(1.0 / 60.0, &input);
                events_this_round += world.drain_super_mode_events();
                let evs = world.drain_score_events();
                if let Some(&pts) = evs.first() {
                    hits.push(pts);
                    break;
                }
            }
            super_mode_events_per_hit.push(events_this_round);
        }

        assert_eq!(hits, vec![5_000, 10_000, 20_000, 35_000, 50_000, 50_000],
            "ladder should climb rung by rung then hold at the top, got {hits:?}");
        assert_eq!(world.bumpers[gi].ladder_rung, 4, "rung stays pinned at the last index");
        assert_eq!(super_mode_events_per_hit, vec![0, 0, 0, 1, 0, 0],
            "super mode should trigger exactly once — the hit that first reaches the top rung");

        world.park_ball();
        assert_eq!(world.bumpers[gi].ladder_rung, 0, "a new ball resets the ladder to its first rung");
        assert!(!world.bumpers[gi].super_mode_triggered, "a new ball lets the trigger fire again");
    }
}

#[cfg(test)]
mod kick_wall_tests {
    use super::*;
    use crate::table::load_table;

    // Regression coverage for a real bug: kick-walls used to detect hits via
    // Rapier's CollisionEvent::Started, which only reliably fires on a
    // square, fairly fast hit. A slingshot is almost always brushed at a
    // shallow/glancing angle as the ball rolls past (not hit head-on), and
    // at that angle the event frequently never arrived at all against these
    // thin wall colliders — confirmed empirically: 0/8 glancing trials at
    // low-to-moderate speed fired before the fix (event-based), vs the
    // proximity-based detection below, which fires on every trial.
    #[test]
    fn slingshot_fires_on_a_glancing_hit_not_just_a_square_one() {
        let table = load_table(1).unwrap();

        // Element 31 on table 1: the left slingshot kick-wall.
        let (x1, y1, x2, y2) = (3.2_f32, 8.77_f32, 5.7_f32, 4.43_f32);
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();
        let (tx, ty) = (dx / len, dy / len); // along the wall's length

        // Read the engine's own interior_normal (guaranteed to point away
        // from the pocket, into the field — the same direction the kick
        // impulse uses) rather than re-deriving it and risking a sign error.
        let probe = PhysicsWorld::from_table(&table);
        let ki = probe.kick_walls.iter()
            .position(|kw| (kw.x1 - x1).abs() < 0.01 && (kw.y1 - y1).abs() < 0.01)
            .expect("table 1 element 31 should be a kick-wall");
        let [inx, iny] = probe.kick_walls[ki].interior_normal;
        let input = InputState::default();

        let fire = |vx: f32, vy: f32, start: [f32; 2]| -> bool {
            let mut world = PhysicsWorld::from_table(&table);
            let ball_h = world.ball_handle.unwrap();
            {
                let body = world.bodies.get_mut(ball_h).unwrap();
                body.set_translation(Vector::new(start[0], start[1]), true);
                body.set_linvel(Vector::new(vx, vy), true);
                body.set_gravity_scale(0.0, true);
            }
            let mut fired = false;
            for _ in 0..90 {
                world.step(1.0 / 60.0, &input);
                if !world.drain_score_events().is_empty() { fired = true; }
            }
            fired
        };

        // Square hit, approaching head-on from the field side — was already reliable.
        let mid = [(x1 + x2) / 2.0 + inx * 1.5, (y1 + y2) / 2.0 + iny * 1.5];
        assert!(fire(-inx * 6.0, -iny * 6.0, mid), "a direct perpendicular hit should always kick");

        // Glancing hit at realistic rolling speed: mostly sliding along the
        // wall's length (like a ball rolling down the guide rail into the
        // pocket), with only a small component closing the remaining gap.
        // Starts mid-segment (35% along), safely clear of the adjacent
        // guide walls that form the rest of the pocket near each end. This
        // is the shape of a normal slingshot hit, and is what the old
        // event-based detection missed entirely (0/8 fired below the fix).
        let seg_t = 0.35;
        let (segx, segy) = (x1 + seg_t * dx, y1 + seg_t * dy);
        let graze_start = [segx + inx * 0.6, segy + iny * 0.6];
        for speed in [4.0_f32, 6.0, 8.0] {
            for graze in [0.05_f32, 0.1, 0.2, 0.3] {
                let vx = tx * speed - inx * speed * graze;
                let vy = ty * speed - iny * speed * graze;
                assert!(fire(vx, vy, graze_start),
                    "glancing hit at speed={speed} graze={graze} should still kick");
            }
        }
    }

    #[test]
    fn ball_saver_still_retracts_on_contact_with_proximity_detection() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let ki = world.kick_walls.iter().position(|kw| kw.retract_when_hit)
            .expect("table 1 should have at least one retract-when-hit ball-saver");
        assert!(world.kick_walls[ki].active, "starts active");

        let (mx, my) = ((world.kick_walls[ki].x1 + world.kick_walls[ki].x2) / 2.0,
                         (world.kick_walls[ki].y1 + world.kick_walls[ki].y2) / 2.0);
        let [inx, iny] = world.kick_walls[ki].interior_normal;
        let ball_h = world.ball_handle.unwrap();
        {
            let body = world.bodies.get_mut(ball_h).unwrap();
            body.set_translation(Vector::new(mx + inx * 1.0, my + iny * 1.0), true);
            body.set_linvel(Vector::new(-inx * 5.0, -iny * 5.0), true);
            body.set_gravity_scale(0.0, true);
        }
        let input = InputState::default();
        for _ in 0..60 {
            world.step(1.0 / 60.0, &input);
        }
        assert!(!world.kick_walls[ki].active, "ball-saver retracts (deactivates) once hit");

        world.park_ball();
        assert!(!world.kick_walls[ki].active, "a used ball saver stays gone for the next ball, as in the original");
    }
}

#[cfg(test)]
mod letter_award_tests {
    use super::*;
    use crate::table::load_table;

    #[test]
    fn party_land_letter_award_lights_completes_and_resets() {
        let table = load_table(9).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        assert_eq!(world.letter_groups.len(), 3, "Party Land should define MAD/CRAZY/PARTY");

        let mad = world.letter_groups.iter().position(|g| g.word == "MAD").unwrap();
        for id in ["GhoulSkull", "CycloneRamp", "SkyrideRamp"] {
            PhysicsWorld::record_shot(&mut world.letter_groups, &mut world.jackpot, id);
        }
        assert!(world.letter_groups[mad].lit.iter().all(|&l| l), "all three MAD shots were hit");

        let input = InputState::default();
        let mut events = Vec::new();
        for _ in 0..=LETTER_FLASH_TICKS as usize {
            world.step(1.0 / 60.0, &input);
            events.extend(world.drain_score_events());
        }
        assert!(events.contains(&10_000), "MAD completion should award its configured 10000 pts, got {events:?}");
        assert!(world.letter_groups[mad].lit.iter().all(|&l| !l), "letters reset after the word completes, ready to spell again");
    }

    #[test]
    fn unrecognized_shot_id_lights_nothing() {
        let table = load_table(9).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        PhysicsWorld::record_shot(&mut world.letter_groups, &mut world.jackpot, "NotARealShot");
        assert!(world.letter_groups.iter().all(|g| g.lit.iter().all(|&l| !l)));
    }
}

#[cfg(test)]
mod flipper_input_flicker {
    use super::*;
    use crate::table::load_table;
    use crate::input::InputState;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(code: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE)
    }

    /// Regression test for a real bug: InputState.left_flipper/right_flipper
    /// aren't a stable "held" boolean — they're a decaying counter that
    /// terminal key-repeat events keep refreshing (see input.rs), so the
    /// flag can go false for a tick between repeats even while the key is
    /// physically held. Gating the motor-target update on a state *change*
    /// (rather than setting it unconditionally every tick) made each flicker
    /// look like a real press/release, so the motor kept getting told to
    /// reverse direction and the flipper barely moved. This reproduces the
    /// flicker via the actual InputState/handle_key path, at a slower-than-
    /// FLIPPER_HOLD_TICKS repeat rate, and checks the flipper still reaches
    /// the top of its swing.
    #[test]
    fn left_flipper_reaches_max_angle_despite_flickering_input() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let left_idx = world.flippers.iter().position(|f| !f.is_right).unwrap();

        let mut input = InputState::default();
        // One frame = input.tick() once, then PHYSICS_STEPS (2) physics
        // substeps, matching app.rs's tick(). Re-send the key-repeat event
        // every 4 frames — slower than FLIPPER_HOLD_TICKS (3), so
        // left_flipper genuinely drops to false for a frame between repeats.
        for frame in 0..90 {
            if frame % 4 == 0 {
                input.handle_key(press('z'));
            }
            input.tick();
            for _ in 0..2 {
                world.step(1.0 / 60.0, &input);
            }
        }

        let body = world.bodies.get(world.flippers[left_idx].body_handle).unwrap();
        let angle = body.rotation().angle();
        assert!(
            (angle - world.flippers[left_idx].max_angle).abs() < 0.1,
            "left flipper should still reach ~max_angle despite flickering input, got {angle}"
        );
    }

    #[test]
    fn right_flipper_reaches_max_angle_despite_flickering_input() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let right_idx = world.flippers.iter().position(|f| f.is_right).unwrap();

        let mut input = InputState::default();
        for frame in 0..90 {
            if frame % 4 == 0 {
                input.handle_key(press('x'));
            }
            input.tick();
            for _ in 0..2 {
                world.step(1.0 / 60.0, &input);
            }
        }

        let body = world.bodies.get(world.flippers[right_idx].body_handle).unwrap();
        let logical_angle = -body.rotation().angle();
        assert!(
            (logical_angle - world.flippers[right_idx].max_angle).abs() < 0.1,
            "right flipper should still reach ~max_angle despite flickering input, got {logical_angle}"
        );
    }

    // Also confirm a KeyEventKind::Release event (Kitty protocol terminals)
    // drops the flipper immediately rather than waiting out the hold ticks.
    #[test]
    fn flipper_releases_immediately_on_release_event() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let left_idx = world.flippers.iter().position(|f| !f.is_right).unwrap();

        let mut input = InputState::default();
        input.handle_key(press('z'));
        for _ in 0..30 {
            input.tick();
            for _ in 0..2 { world.step(1.0 / 60.0, &input); }
        }
        let mut release = press('z');
        release.kind = KeyEventKind::Release;
        input.handle_key(release);
        for _ in 0..30 {
            input.tick();
            for _ in 0..2 { world.step(1.0 / 60.0, &input); }
        }

        let body = world.bodies.get(world.flippers[left_idx].body_handle).unwrap();
        let angle = body.rotation().angle();
        assert!(
            (angle - world.flippers[left_idx].min_angle).abs() < 0.1,
            "left flipper should fall back to ~min_angle after a Release event, got {angle}"
        );
    }
}

#[cfg(test)]
mod flipper_sleep {
    use super::*;
    use crate::table::load_table;
    use crate::input::InputState;

    /// Regression test for a real bug: a dynamic flipper resting at its
    /// limit with no ball nearby is exactly the low-velocity state Rapier's
    /// sleep system is designed to catch, and once asleep, changing the
    /// joint's motor target does *not* wake the body back up on its own —
    /// so the flipper went permanently unresponsive the first time a player
    /// left it alone for a while (add_flipper now sets can_sleep(false)).
    #[test]
    fn flipper_responds_after_sitting_idle_for_15_seconds() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let left_idx = world.flippers.iter().position(|f| !f.is_right).unwrap();

        let idle_input = InputState::default();
        for _ in 0..(15 * 60) {
            world.step(1.0 / 60.0, &idle_input);
        }
        assert!(
            !world.bodies.get(world.flippers[left_idx].body_handle).unwrap().is_sleeping(),
            "flipper body should never be allowed to sleep"
        );

        let mut engaged_input = InputState::default();
        engaged_input.left_flipper = true;
        for _ in 0..30 {
            world.step(1.0 / 60.0, &engaged_input);
        }
        let angle = world.bodies.get(world.flippers[left_idx].body_handle).unwrap().rotation().angle();
        assert!(
            (angle - world.flippers[left_idx].max_angle).abs() < 0.1,
            "left flipper should still respond after sitting idle, got {angle}"
        );
    }
}

#[cfg(test)]
mod time_ratio_tests {
    use super::*;
    use crate::table::load_table;

    /// The original runs sim time at `targetTimeRatio` x real time, so a ball
    /// in free fall drops 0.5 * g * (ratio * t)^2 after `t` real seconds. This
    /// pins the "zippy, not underwater" feel against the original's numbers.
    #[test]
    fn free_fall_matches_original_time_scaling() {
        let table = load_table(1).expect("table 1 should load");
        let mut world = PhysicsWorld::from_table(&table);
        let ball_h = world.ball_handle.expect("ball should exist");
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(10.0, 25.0), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(1.0, true);

        let input = InputState::default();
        let frame = 0.033;
        let frames = 9; // ~0.3 s of real time
        for _ in 0..frames {
            world.step_frame(frame, &input);
        }
        let t = frame * frames as f32;
        let expected = 0.5 * table.gravity_magnitude() * (table.time_ratio() * t).powi(2);
        let dropped = 25.0 - world.ball_position().expect("ball").y;
        assert!(
            (dropped - expected).abs() < expected * 0.06,
            "dropped {dropped:.3} units in {t:.3}s real, expected ~{expected:.3}"
        );
    }
}

#[cfg(test)]
mod flipper_cycle_tests {
    use super::*;
    use crate::table::load_table;

    /// On Classic, pressing a flipper must shift the lit rollover dot by exactly
    /// one position per press (right flipper = rightward, as in the original),
    /// however many physics substeps a frame has, in both flipper-cycled groups.
    #[test]
    fn one_flipper_press_rotates_lit_rollovers_by_exactly_one() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let groups: Vec<usize> = world.rollover_groups.iter().enumerate()
            .filter(|(_, g)| g.cycle_on_flipper).map(|(i, _)| i).collect();
        assert_eq!(groups.len(), 2, "Classic has two flipper-cycled groups (bottom lanes and top lanes)");
        let lit = |w: &PhysicsWorld, gi: usize| -> Vec<bool> { w.rollover_groups[gi].rollovers.iter().map(|r| r.lit).collect() };
        for &gi in &groups {
            for (i, r) in world.rollover_groups[gi].rollovers.iter_mut().enumerate() { r.lit = i == 1; }
        }

        let mut input = InputState::default();
        input.right_flipper = true;
        input.right_just_pressed = true;
        world.step_frame(0.033, &input);
        for &gi in &groups {
            assert_eq!(lit(&world, gi), vec![false, false, true, false], "right press should move the lit dot right one");
        }

        input.right_just_pressed = false;
        input.right_flipper = false;
        input.left_flipper = true;
        input.left_just_pressed = true;
        world.step_frame(0.033, &input);
        input.left_just_pressed = false;
        world.step_frame(0.033, &input); // holding must not keep cycling
        for &gi in &groups {
            assert_eq!(lit(&world, gi), vec![false, true, false, false], "left press should move it back left one");
        }
    }
}

#[cfg(test)]
mod ball_saver_tests {
    use super::*;
    use crate::table::load_table;

    /// A ball dropped onto a ball saver must be thrown back up into play (the
    /// original bounces it off the wall, then adds the saver's kick), not just
    /// have its fall cancelled and drop through when the saver retracts.
    #[test]
    fn ball_saver_throws_a_falling_ball_back_up_into_play() {
        let table = load_table(1).unwrap();
        for (label, x) in [("left", 0.78_f32), ("right", 18.05_f32)] {
            let mut world = PhysicsWorld::from_table(&table);
            let ball_h = world.ball_handle.expect("ball");
            let body = world.bodies.get_mut(ball_h).unwrap();
            body.set_translation(Vector::new(x, 8.0), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_gravity_scale(1.0, true);

            let input = InputState::default();
            let (mut min_y, mut max_y_after) = (f32::MAX, 0.0_f32);
            let mut hit = false;
            for _ in 0..60 {
                world.step_frame(0.033, &input);
                let y = world.ball_position().expect("ball").y;
                min_y = min_y.min(y);
                if min_y < 6.5 { hit = true; } // on the way down to the saver
                if hit { max_y_after = max_y_after.max(y); }
            }
            assert!(min_y > 4.0, "{label} saver: ball fell through to y={min_y:.2}");
            assert!(max_y_after > 8.5, "{label} saver: ball only rebounded to y={max_y_after:.2}, expected a strong throw back up");
        }
    }
}

#[cfg(test)]
mod slingshot_tests {
    use super::*;
    use crate::table::load_table;

    /// A ball driven into a slingshot must come off it clearly faster than it
    /// arrived (bounce plus the wall's kick), not have its approach merely
    /// cancelled. Original: out = 0.6 * in + kick / ball_mass (kick 3.5, mass ~0.785).
    #[test]
    fn slingshot_throws_an_approaching_ball_away_hard() {
        let table = load_table(1).unwrap();
        for incoming in [3.0_f32, 6.0, 10.0] {
            let mut world = PhysicsWorld::from_table(&table);
            let ki = world.kick_walls.iter().position(|k| !k.retract_when_hit && k.kick_force > 3.0 && k.x1 < 8.0)
                .expect("Classic has a left slingshot");
            let kw = &world.kick_walls[ki];
            let (mx, my, n) = ((kw.x1 + kw.x2) / 2.0, (kw.y1 + kw.y2) / 2.0, kw.interior_normal);
            let ball_h = world.ball_handle.unwrap();
            let body = world.bodies.get_mut(ball_h).unwrap();
            body.set_translation(Vector::new(mx + n[0] * 2.5, my + n[1] * 2.5), true);
            body.set_linvel(Vector::new(-n[0] * incoming, -n[1] * incoming), true);
            body.set_gravity_scale(0.0, true);

            let input = InputState::default();
            let mut best_out = f32::MIN;
            for _ in 0..40 {
                world.step_frame(0.033, &input);
                let v = world.bodies.get(ball_h).unwrap().linvel();
                best_out = best_out.max(v.x * n[0] + v.y * n[1]);
            }
            let original_like = 0.6 * incoming + 3.5 / 0.785;
            assert!(
                best_out > original_like * 0.97,
                "approach {incoming}: left the slingshot at {best_out:.2}, expected about {original_like:.2} or more"
            );
        }
    }
}

#[cfg(test)]
mod rotating_bumper_tests {
    use super::*;
    use crate::table::load_table;

    fn pos(w: &PhysicsWorld, id: &str) -> [f32; 2] {
        w.bumpers.iter().find(|b| b.shot_id.as_deref() == Some(id)).expect(id).position
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 { (a[0] - b[0]).hypot(a[1] - b[1]) }

    /// Table 2: four bumpers orbit CenterBumper1 at 0.75 rad/s and two orbit
    /// (9.35, 13) at -0.5 rad/s, in sim time, at constant radius, with the
    /// physics bodies following the drawn positions.
    #[test]
    fn table_2_bumpers_orbit_at_the_original_speeds() {
        let table = load_table(2).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        assert_eq!(world.rotating_groups.len(), 2, "table 2 has two rotating groups");

        let center1 = pos(&world, "CenterBumper1");
        let center2 = [9.35_f32, 13.0];
        let ids1 = ["RotatingBumper1A", "RotatingBumper1B", "RotatingBumper1C", "RotatingBumper1D"];
        let ids2 = ["RotatingBumper2A", "RotatingBumper2B"];
        let r1 = dist(pos(&world, ids1[0]), center1);
        let r2 = dist(pos(&world, ids2[0]), center2);
        let angle = |p: [f32; 2], c: [f32; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
        let (a1, a2) = (angle(pos(&world, ids1[0]), center1), angle(pos(&world, ids2[0]), center2));

        // Two real seconds is about 4.6 sim seconds at the table's time ratio.
        let input = InputState::default();
        let frames = 60;
        for _ in 0..frames { world.step_frame(0.033, &input); }
        let sim_seconds = 0.033 * frames as f32 * table.time_ratio();

        for id in ids1 { assert!((dist(pos(&world, id), center1) - r1).abs() < 1e-3, "{id} radius drifted"); }
        for id in ids2 { assert!((dist(pos(&world, id), center2) - r2).abs() < 1e-3, "{id} radius drifted"); }

        let turned = |now: f32, was: f32| (now - was + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let d1 = turned(angle(pos(&world, ids1[0]), center1), a1);
        let d2 = turned(angle(pos(&world, ids2[0]), center2), a2);
        assert!((d1 - 0.75 * sim_seconds).abs() < 0.02 || (d1 - (0.75 * sim_seconds - std::f32::consts::TAU)).abs() < 0.02, "group 1 turned {d1}, expected {}", 0.75 * sim_seconds);
        assert!((d2 + 0.5 * sim_seconds).abs() < 0.02 || (d2 - (-0.5 * sim_seconds + std::f32::consts::TAU)).abs() < 0.02, "group 2 turned {d2}, expected {}", -0.5 * sim_seconds);

        // The collider moves with the bumper, and the opposite bumper stays opposite.
        for id in ids1.iter().chain(ids2.iter()) {
            let b = world.bumpers.iter().find(|b| b.shot_id.as_deref() == Some(id)).unwrap();
            let t = world.bodies.get(b.body).unwrap().translation();
            assert!(dist([t.x, t.y], b.position) < 1e-4, "{id}: body not at drawn position");
        }
        let (pa, pc) = (pos(&world, ids1[0]), pos(&world, ids1[2]));
        assert!(dist([(pa[0] + pc[0]) / 2.0, (pa[1] + pc[1]) / 2.0], center1) < 1e-3, "A and C should stay opposite");
    }

    #[test]
    fn tables_without_movers_have_no_rotating_groups() {
        for n in [1u8, 3, 4, 8, 9] {
            let world = PhysicsWorld::from_table(&load_table(n).unwrap());
            assert!(world.rotating_groups.is_empty(), "table {n} should not rotate anything");
        }
    }

    /// The ball must hit a rotating bumper where it is *now*, not where the
    /// table file placed it.
    #[test]
    fn ball_collides_with_a_rotating_bumper_at_its_current_position() {
        let table = load_table(2).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let input = InputState::default();
        for _ in 0..40 { world.step_frame(0.033, &input); } // let it move well away from the file's position

        let moved = pos(&world, "RotatingBumper1A");
        let file_pos = [14.0_f32, 19.6];
        assert!(dist(moved, file_pos) > 1.5, "bumper should have moved away from its JSON position, at {moved:?}");

        // Fire the ball radially inward at the bumper from the free side, outside the ring.
        let center = pos(&world, "CenterBumper1");
        let (dx, dy) = (moved[0] - center[0], moved[1] - center[1]);
        let len = dx.hypot(dy);
        let out = [dx / len, dy / len];
        let ball_h = world.ball_handle.unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(moved[0] + out[0] * 3.0, moved[1] + out[1] * 3.0), true);
        body.set_linvel(Vector::new(-out[0] * 5.0, -out[1] * 5.0), true);
        body.set_gravity_scale(0.0, true);

        let mut bounced = false;
        for _ in 0..30 {
            world.step_frame(0.033, &input);
            let v = world.bodies.get(ball_h).unwrap().linvel();
            if v.x * out[0] + v.y * out[1] > 1.0 { bounced = true; break; }
        }
        assert!(bounced, "ball passed through the rotating bumper's current position");
    }
}

#[cfg(test)]
mod drop_bank_tests {
    use super::*;
    use crate::table::load_table;

    /// Knocks down all but the last target of the bank with this id, then puts
    /// the ball on the last one and steps a frame so it falls. Returns the
    /// group's index.
    fn clear_bank(world: &mut PhysicsWorld, id: &str) -> usize {
        let gi = world.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no drop bank {id}"));
        let last = world.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last {
            world.drop_target_groups[gi].targets[ti].active = false;
            let ch = world.drop_target_groups[gi].targets[ti].collider;
            world.colliders.get_mut(ch).unwrap().set_collision_groups(InteractionGroups::none());
        }
        let t = &world.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        let ball_h = world.ball_handle.unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(mx, my), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
        world.step_frame(0.033, &InputState::default());
        gi
    }

    fn all_up(world: &PhysicsWorld, gi: usize) -> bool { world.drop_target_groups[gi].targets.iter().all(|t| t.active) }
    fn all_down(world: &PhysicsWorld, gi: usize) -> bool { world.drop_target_groups[gi].targets.iter().all(|t| !t.active) }

    /// Table 2's banks reset 2.0 sim seconds after the last target falls (about
    /// 0.87 s of real time at 2.3x), not instantly.
    #[test]
    fn a_cleared_bank_comes_back_after_its_reset_delay() {
        let table = load_table(2).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let gi = clear_bank(&mut world, "DropTargetLeft");
        assert!(all_down(&world, gi), "the bank should be down right after the last target falls");

        // Park the ball somewhere harmless, then let time pass: 1.6 sim seconds is still before 2.0.
        let ball_h = world.ball_handle.unwrap();
        world.bodies.get_mut(ball_h).unwrap().set_translation(Vector::new(9.0, 3.0), true);
        let input = InputState::default();
        let frame_sim = 0.033 * table.time_ratio();
        for _ in 0..(1.6 / frame_sim) as usize { world.step_frame(0.033, &input); }
        assert!(all_down(&world, gi), "still down before the 2 second delay");
        for _ in 0..(0.9 / frame_sim) as usize + 1 { world.step_frame(0.033, &input); }
        assert!(all_up(&world, gi), "back up after the delay");
    }

    /// Original: a bank with no reset value stays down once cleared (table 8's inline targets).
    #[test]
    fn a_bank_without_a_reset_value_stays_down() {
        let table = load_table(8).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let gi = clear_bank(&mut world, "InlineTargets");
        let ball_h = world.ball_handle.unwrap();
        world.bodies.get_mut(ball_h).unwrap().set_translation(Vector::new(9.0, 3.0), true);
        let input = InputState::default();
        for _ in 0..300 { world.step_frame(0.033, &input); }
        assert!(all_down(&world, gi), "a bank with no reset must not come back");
    }

    /// Clearing the left bank re-arms the left ball saver only; savers are not
    /// re-armed by a new ball.
    #[test]
    fn clearing_the_matching_bank_restores_that_ball_saver_only() {
        for (n, left_bank, right_bank) in [(1u8, "DropTargetLeftSave", "DropTargetRightSave"), (2, "DropTargetLeft", "DropTargetRight")] {
            let table = load_table(n).unwrap();
            let mut world = PhysicsWorld::from_table(&table);
            let used_up = |w: &mut PhysicsWorld| for kw in &mut w.kick_walls {
                if kw.retract_when_hit {
                    kw.active = false;
                    w.colliders.get_mut(kw.collider_handle).unwrap().set_collision_groups(InteractionGroups::none());
                }
            };
            let saver = |w: &PhysicsWorld, id: &str| w.kick_walls.iter().find(|k| k.shot_id.as_deref() == Some(id)).unwrap().active;

            used_up(&mut world);
            world.park_ball();
            assert!(!saver(&world, "BallSaver-left") && !saver(&world, "BallSaver-right"), "table {n}: a new ball does not re-arm savers");

            clear_bank(&mut world, left_bank);
            assert!(saver(&world, "BallSaver-left"), "table {n}: left bank restores the left saver");
            assert!(!saver(&world, "BallSaver-right"), "table {n}: and not the right one");

            clear_bank(&mut world, right_bank);
            assert!(saver(&world, "BallSaver-right"), "table {n}: right bank restores the right saver");
        }
    }
}

#[cfg(test)]
mod launch_barrier_tests {
    use super::*;
    use crate::table::load_table;

    fn place_ball(world: &mut PhysicsWorld, x: f32, y: f32, vx: f32, vy: f32) {
        let h = world.ball_handle.unwrap();
        let body = world.bodies.get_mut(h).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(vx, vy), true);
        body.set_gravity_scale(0.0, true);
    }

    fn centre(r: [f32; 4]) -> (f32, f32) { ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0) }

    #[test]
    fn the_barrier_closes_at_its_sensor_opens_at_the_retract_sensor_and_on_a_new_ball() {
        for n in [1u8, 2, 3, 5, 6, 7] {
            let table = load_table(n).unwrap();
            let mut world = PhysicsWorld::from_table(&table);
            let (close, open) = {
                let b = world.launch_barrier.as_ref().unwrap_or_else(|| panic!("table {n} has a launch barrier"));
                (b.close_rect.expect("close sensor"), b.open_rect.expect("retract sensor"))
            };
            let input = InputState::default();
            assert!(!world.launch_barrier.as_ref().unwrap().active, "table {n}: starts open");

            let (cx, cy) = centre(close);
            place_ball(&mut world, cx, cy, 0.0, 0.0);
            world.step_frame(0.033, &input);
            assert!(world.launch_barrier.as_ref().unwrap().active, "table {n}: closes at its sensor");
            let col = world.launch_barrier.as_ref().unwrap().collider;
            assert_eq!(world.colliders.get(col).unwrap().collision_groups(), InteractionGroups::all(), "table {n}: solid while closed");
            assert_eq!(world.snapshot().walls.len(), world.wall_segments.len() + 1, "table {n}: drawn while closed");

            let (ox, oy) = centre(open);
            place_ball(&mut world, ox, oy, 0.0, 0.0);
            world.step_frame(0.033, &input);
            assert!(!world.launch_barrier.as_ref().unwrap().active, "table {n}: opens at the retract sensor");

            let (cx, cy) = centre(close);
            place_ball(&mut world, cx, cy, 0.0, 0.0);
            world.step_frame(0.033, &input);
            assert!(world.launch_barrier.as_ref().unwrap().active);
            world.park_ball();
            assert!(!world.launch_barrier.as_ref().unwrap().active, "table {n}: a new ball opens it again");
            assert_eq!(world.snapshot().walls.len(), world.wall_segments.len(), "table {n}: not drawn when open");
        }
    }

    /// Table 2: the barrier is a vertical wall at x=10 at the top of the lane's
    /// exit. Closed, a ball coming from the left cannot get back in.
    #[test]
    fn a_closed_barrier_stops_the_ball_re_entering_the_lane() {
        let table = load_table(2).unwrap();
        let input = InputState::default();
        for (closed, expect_blocked) in [(true, true), (false, false)] {
            let mut world = PhysicsWorld::from_table(&table);
            if closed {
                let close = world.launch_barrier.as_ref().unwrap().close_rect.unwrap();
                let (cx, cy) = centre(close);
                place_ball(&mut world, cx, cy, 0.0, 0.0);
                world.step_frame(0.033, &input);
                assert!(world.launch_barrier.as_ref().unwrap().active);
            }
            // y=29.3 is the middle of the 1.2 gap between the lane wall end (28.7) and the top.
            // Closed case: start left of the closing sensor (x 9.0 to 9.5). Open case: start
            // just right of it, since crossing that sensor closes the barrier by design.
            let start_x = if closed { 8.4 } else { 9.6 };
            place_ball(&mut world, start_x, 29.3, 6.0, 0.0);
            let mut max_x = 0.0_f32;
            for _ in 0..30 {
                world.step_frame(0.033, &input);
                max_x = max_x.max(world.ball_position().unwrap().x);
            }
            if expect_blocked { assert!(max_x < 10.0, "closed barrier let the ball through to x={max_x:.2}"); }
            else { assert!(max_x > 10.5, "open barrier should let the ball pass, got x={max_x:.2}"); }
        }
    }

    /// The plunger has to clear the lane reliably (the launch jitter allows an
    /// occasional short shot). Guards the friction settings: with the
    /// original's wall friction under Rapier only about 40% got over the top.
    #[test]
    fn a_launch_normally_gets_over_the_top_of_the_lane() {
        for n in [1u8, 2, 3] {
            let table = load_table(n).unwrap();
            let input = InputState::default();
            let mut over = 0;
            for _ in 0..24 {
                let mut world = PhysicsWorld::from_table(&table);
                world.launch_ball();
                let mut max_y = 0.0_f32;
                for _ in 0..200 {
                    world.step_frame(0.033, &input);
                    max_y = max_y.max(world.ball_position().unwrap().y);
                }
                if max_y > table.height - 1.2 { over += 1; }
            }
            assert!(over >= 16, "table {n}: only {over}/24 launches got over the top of the lane");
        }
    }
}

#[cfg(test)]
mod rotating_triangle_tests {
    use super::*;
    use crate::table::load_table;

    fn wall<'a>(w: &'a PhysicsWorld, id: &str) -> &'a MovingWallInfo {
        w.moving_walls.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("no moving wall {id}"))
    }

    /// Table 5: an equilateral triangle (side 2) spins about (10, 18.86) at
    /// 1 rad/s of sim time; its walls keep their length, their bodies sit at
    /// the drawn midpoints, and the vertices stay on the circle.
    #[test]
    fn table_5_triangle_spins_as_a_rigid_equilateral_triangle() {
        let table = load_table(5).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        assert_eq!(world.rotating_triangles.len(), 1);
        assert_eq!(world.moving_walls.len(), 3);

        let first_vertex = |w: &PhysicsWorld| { let m = wall(w, "TriangleWall1"); [m.x1, m.y1] };
        let angle_of = |p: [f32; 2]| (p[1] - 18.86).atan2(p[0] - 10.0);
        let start = angle_of(first_vertex(&world));
        let input = InputState::default();
        let frames = 40;
        for _ in 0..frames { world.step_frame(0.033, &input); }
        let sim_seconds = 0.033 * frames as f32 * table.time_ratio();

        for id in ["TriangleWall1", "TriangleWall2", "TriangleWall3"] {
            let m = wall(&world, id);
            let len = (m.x2 - m.x1).hypot(m.y2 - m.y1);
            assert!((len - 2.0).abs() < 1e-3, "{id} length {len}");
            for (x, y) in [(m.x1, m.y1), (m.x2, m.y2)] {
                let r = (x - 10.0).hypot(y - 18.86);
                assert!((r - 3.0_f32.sqrt() * 2.0 / 3.0).abs() < 1e-3, "{id} vertex off the circle: r={r}");
            }
            let t = world.bodies.get(m.body).unwrap().translation();
            assert!((t.x - (m.x1 + m.x2) / 2.0).abs() < 1e-3 && (t.y - (m.y1 + m.y2) / 2.0).abs() < 1e-3, "{id}: body not at drawn midpoint");
        }
        let turned = (angle_of(first_vertex(&world)) - start + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let expected = (sim_seconds + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        assert!((turned - expected).abs() < 0.02, "turned {turned}, expected {expected} for {sim_seconds}s");
    }

    /// The ball hits a triangle wall where it is now, and the walls appear in the snapshot.
    #[test]
    fn the_ball_bounces_off_a_rotated_triangle_wall() {
        let table = load_table(5).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let input = InputState::default();
        for _ in 0..25 { world.step_frame(0.033, &input); }
        let snap_walls = world.snapshot().walls.iter().filter(|w| w.label.as_deref().map_or(false, |l| l.starts_with("TriangleWall"))).count();
        assert_eq!(snap_walls, 3, "triangle walls are drawn");

        let m = wall(&world, "TriangleWall2");
        let (mx, my) = ((m.x1 + m.x2) / 2.0, (m.y1 + m.y2) / 2.0);
        let (ox, oy) = (mx - 10.0, my - 18.86);
        let l = ox.hypot(oy);
        let out = [ox / l, oy / l];
        let ball_h = world.ball_handle.unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(mx + out[0] * 2.5, my + out[1] * 2.5), true);
        body.set_linvel(Vector::new(-out[0] * 5.0, -out[1] * 5.0), true);
        body.set_gravity_scale(0.0, true);
        let mut bounced = false;
        for _ in 0..20 {
            world.step_frame(0.033, &input);
            let v = world.bodies.get(ball_h).unwrap().linvel();
            if v.x * out[0] + v.y * out[1] > 1.0 { bounced = true; break; }
        }
        assert!(bounced, "ball passed through the triangle's current position");
    }

    #[test]
    fn other_tables_have_no_moving_walls() {
        for n in [1u8, 2, 3, 4, 6, 7, 8, 9] {
            let w = PhysicsWorld::from_table(&load_table(n).unwrap());
            assert!(w.rotating_triangles.is_empty() && w.moving_walls.is_empty(), "table {n}");
        }
    }
}
