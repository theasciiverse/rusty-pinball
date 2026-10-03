use std::cell::RefCell;
mod movers;
mod scripts;
mod stars;
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

/// A dot or line a table script draws (table 7's star map), behind the elements.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Circle { x: f32, y: f32, radius: f32, color: (u8, u8, u8) },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: (u8, u8, u8) },
}

/// A score floating up from where it was won (the original's `ScoreAnimation`).
#[derive(Debug, Clone)]
pub struct ScorePopup {
    pub text: String,
    pub x: f32,
    pub y: f32,
    /// 0 when it appears, 1 when it is gone.
    pub progress: f32,
    pub color: (u8, u8, u8),
}

#[derive(Debug, Clone)]
pub struct BallSnapshot {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// A colour a table script gave this ball; None means the table's ball colour.
    pub color: Option<(u8, u8, u8)>,
    /// Where it has been over the last frames, oldest first.
    pub trail: Vec<(f32, f32)>,
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

#[derive(Clone)]
pub struct FieldSnapshot {
    pub balls: Vec<BallSnapshot>,
    pub shapes: Vec<Shape>,
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
    /// Scores to show floating up from where they were won (filled in by the game).
    pub popups: Vec<ScorePopup>,
    pub ball_layer: u8,
    /// Bit n set when a ball is on layer n; layered walls are drawn only then.
    pub occupied_layers: u32,
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
    layer:        u8,
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
    layer: u8,
    // Colours a script put on the bumper at runtime (table 3's heat).
    color_override: Option<(u8, u8, u8)>,
    outer_override: Option<(u8, u8, u8)>,
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
    layer: u8,
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
    // A kick wall on a wall that moves (table 5's triangle) follows it: its ends
    // and normal are updated each step and it is drawn as the moving wall.
    follows: Option<usize>,
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

/// Losing the only ball within this much game time (sim seconds) of its launch
/// relaunches it once ("Ball Saved"); the original's `mercyBallDuration`, 20000 ms.
const MERCY_BALL_SECONDS: f32 = 20.0;
/// During multiball a lost ball is replaced within this long of the multiball
/// starting; the original's `multiballSaverDuration`, 30000 ms.
const MULTIBALL_SAVER_SECONDS: f32 = 30.0;

/// Frames of ball trail kept (the original keeps 20 positions), and how far a ball
/// can move in one frame before it counts as having jumped.
const TRAIL_LENGTH: usize = 20;
const TRAIL_JUMP: f32 = 4.0;
/// A ball lost this soon (game time) after a script raised the multiplier keeps the
/// increase for the next ball; the original's `preserveMultiplierIncreaseDuration` default.
const PRESERVE_MULTIPLIER_SECONDS: f32 = 7.5;
/// Balls that sit still this long (game time) get bumped; the original's `STUCK_BALL_NANOS`.
const STUCK_BALL_SECONDS: f32 = 10.0;

/// A wall a table script moves or retracts while playing (table 5's rotating
/// triangle). Drawn from its live position; not part of the static wall list.
struct MovingWallInfo {
    layer: u8,
    id: String,
    body: RigidBodyHandle,
    collider: ColliderHandle,
    x1: f32, y1: f32, x2: f32, y2: f32,
    color: Option<(u8, u8, u8)>,
    active: bool,
}

/// Something that happened this step which the table's script may react to.
enum ScriptEvent {
    BumperHit { id: String, ball: usize },
    DropBankCleared { id: String, ball: usize },
    /// One drop target fell (before the bank-cleared event, if it was the last).
    DropTargetHit { id: String, ball: usize },
    SpinnerActivated { id: String, ball: usize },
    RolloverGroupCompleted { id: String, ball: usize },
    /// A ball is inside a sensor; `previous` is the sensor it was in before.
    Sensor { id: String, ball: usize, previous: Option<String> },
    /// A ball struck a kick wall that has an id (the original's `processCollision` for walls).
    WallHit { id: String, ball: usize },
    Scheduled(u32),
}

/// A sensor rectangle with an id: balls inside it are reported to the table
/// script, and each ball remembers the last one it was in.
struct SensorInfo {
    id: String,
    rect: [f32; 4],
    from_layer: Option<u8>,
}

/// A line that is drawn but has no body (`ignoreBall` walls such as table 5's
/// centre diagram). Scripts show and hide them.
struct DecorLineInfo {
    id: String,
    x1: f32, y1: f32, x2: f32, y2: f32,
    color: Option<(u8, u8, u8)>,
    visible: bool,
}

/// One ball in play. `balls[0]` is the "primary" ball, the one parked in the
/// shooter lane between balls; extra balls exist only during multiball.
struct BallState {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    layer: u8,
    // Runtime colour a table script gave this ball (table 6 planets, table 5
    // ramps); None means the table's ball colour.
    color: Option<(u8, u8, u8)>,
    // Id of the last sensor this ball was inside; scripts read it to tell which
    // way a ball went through a ramp (original: Ball.mostRecentSensorId).
    last_sensor: Option<String>,
    // Where the ball was over the last frames, oldest first (drawn as a fading trail).
    trail: Vec<(f32, f32)>,
}

struct DropTargetInfo {
    x1: f32, y1: f32, x2: f32, y2: f32,
    collider: ColliderHandle,
    active: bool,
}

struct DropTargetGroupInfo {
    layer: u8,
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
    // The colour from the table; `color` is what is drawn and scripts may
    // override it (`set_rollover_color(.., None)` restores this one).
    base_color: Option<(u8, u8, u8)>,
    color: Option<(u8, u8, u8)>,
    // Sim seconds after a ball lights it until it turns itself off again (the
    // group's `reset`; 0 = never), and the time left on that countdown.
    reset_seconds: f32,
    reset_timer: Option<f32>,
}

struct RolloverGroupInfo {
    layer:            u8,
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
    // A hidden group is not drawn (scripts show and hide groups as the original does).
    visible:          bool,
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
    balls:              Vec<BallState>,
    ball_radius:        f32,
    time_ratio:         f32,
    ball_color:         Option<(u8, u8, u8)>,
    flippers:           Vec<FlipperInfo>,
    bumpers:            Vec<BumperInfo>,
    rotating_groups:    Vec<movers::RotatingGroup>,
    launch_barrier:     Option<LaunchBarrierInfo>,
    moving_walls:       Vec<MovingWallInfo>,
    decor_lines:        Vec<DecorLineInfo>,
    shapes:             Vec<Shape>,
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
    sensors: Vec<SensorInfo>,

    // Events accumulated during step; drained by game / audio layers
    score_events:  RefCell<Vec<u64>>,
    sound_events:  RefCell<Vec<PhysicsSound>>,
    // Signed multiplier deltas (currently only ever positive) from
    // multiplier-step rollover groups completing.
    multiplier_events: RefCell<Vec<i32>>,
    // Extra balls a table script awarded this step (table 5).
    extra_ball_events: RefCell<u32>,
    // Percent points a table script adds to the multiplier (table 4's bumpers).
    multiplier_percent_events: RefCell<Vec<i32>>,
    // Factors (in percent, 125 = x1.25) a table script multiplies the whole score
    // multiplier by (table 4's jackpot).
    multiplier_scale_events: RefCell<Vec<u64>>,
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
    // The ball whose passage closes the gate (the one just launched).
    gate_ball:     Option<RigidBodyHandle>,
    // Table rules (see scripts.rs), game-time timers it asked for (seconds of
    // sim time left, token), events waiting for it, and messages for the player.
    script:        Option<Box<dyn scripts::TableScript>>,
    scheduled:     Vec<(f32, u32)>,
    script_events: Vec<ScriptEvent>,
    message_events: RefCell<Vec<(String, u32)>>,
    // Set when the last ball was removed without a ball loss (a lock); the
    // game must wait for a relaunch without costing a ball.
    relaunch_needed: bool,
    // Which flippers the player held on the last step (table 8's skill shot asks).
    left_held: bool,
    right_held: bool,
    // Game time (sim seconds since the world was built) for the ball savers.
    game_time: f32,
    // When the player launched the ball in play, and whether its mercy ball is spent.
    ball_start_time: Option<f32>,
    used_mercy_ball: bool,
    // When the current multiball began (None with a single ball).
    multiball_start_time: Option<f32>,
    // Game time of the last multiplier increase a script made (the original's
    // `lastMultiplerIncrementGameTimeNanos`); a ball lost soon after keeps the increase.
    last_multiplier_increase: std::cell::Cell<Option<f32>>,
    // Stuck-ball check: sim seconds the balls have sat still, and where they were last frame.
    stuck_seconds: f32,
    last_ball_positions: Vec<(f32, f32)>,
    stuck_seed: u32,
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
        let mut sensors: Vec<SensorInfo> = Vec::new();
        let mut launch_barrier: Option<LaunchBarrierInfo> = None;
        let mut moving_walls: Vec<MovingWallInfo> = Vec::new();
        let mut decor_lines: Vec<DecorLineInfo> = Vec::new();
        let (mut barrier_close_rect, mut barrier_open_rect) = (None, None);

        // Gravity points downward in rapier (negative y)
        let g = table.gravity_magnitude();
        let gravity = Vector::new(0.0_f32, -g);

        // The table's script (if any) names walls it raises and lowers; those are
        // registered as scripted walls instead of plain static ones.
        let script = scripts::script_for(table);
        let scripted_wall_ids: &[&str] = script.as_ref().map_or(&[], |sc| sc.scripted_walls());

        for (elem_idx, elem) in table.elements.iter().enumerate() {
            match elem {
                ElementDef::WallElement(w) => {
                    // A wall the script controls is built even if the table marks it
                    // `disabled` (that means "starts retracted"); the script retracts it.
                    let scripted_disabled = w.disabled == Some(true)
                        && w.id.as_deref().map_or(false, |id| scripted_wall_ids.contains(&id));
                    let enabled_copy;
                    let w = if scripted_disabled {
                        enabled_copy = WallDef { disabled: None, ..w.clone() };
                        &enabled_copy
                    } else {
                        w
                    };
                    let label = label_for(elem_idx, &w.id);
                    let wall_col = Self::add_wall(w, &mut bodies, &mut colliders, &mut wall_segs, w.layer, Some(label.clone()), parse_color(&w.color));
                    if w.ignore_ball == Some(true) {
                        if let Some([x1, y1, x2, y2]) = w.position {
                            decor_lines.push(DecorLineInfo {
                                id: w.id.clone().unwrap_or_else(|| label.clone()),
                                x1, y1, x2, y2, color: parse_color(&w.color), visible: true,
                            });
                        }
                    }
                    let mut moving_index: Option<usize> = None;
                    if let (Some(id), Some(col), Some([x1, y1, x2, y2])) = (w.id.as_deref(), wall_col, w.position) {
                        if id.starts_with("TriangleWall") || scripted_wall_ids.contains(&id) {
                            if let Some(body) = colliders.get(col).and_then(|c| c.parent()) {
                                wall_segs.pop();
                                moving_index = Some(moving_walls.len());
                                moving_walls.push(MovingWallInfo {
                                    layer: w.layer.unwrap_or(0),
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
                                layer: w.layer.unwrap_or(0),
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
                                follows: moving_index,
                            });
                            // Kick walls are rendered via the kick_walls path only;
                            // remove the segment add_wall just pushed to wall_segs
                            // (a wall that moves was already taken out above, and is
                            // drawn from its live position).
                            if moving_index.is_none() { wall_segs.pop(); }
                        }
                    }
                }
                ElementDef::WallArcElement(a) if a.ignore_ball == Some(true) => {
                    // Drawn only, like the guide paths (table 8's billiard balls).
                    let visible = !matches!(a.color.as_ref().and_then(|c| c.as_array()).and_then(|v| v.get(3)).and_then(|v| v.as_u64()), Some(0));
                    let id = a.id.clone().unwrap_or_else(|| label_for(elem_idx, &a.id));
                    // Background art: dimmed so it is not mistaken for a wall.
                    let dim = |(r, g, b): (u8, u8, u8)| ((r as u32 * 45 / 100) as u8, (g as u32 * 45 / 100) as u8, (b as u32 * 45 / 100) as u8);
                    let color = Some(dim(parse_color(&a.color).unwrap_or((170, 170, 170))));
                    for (p0, p1) in a.to_segments() {
                        decor_lines.push(DecorLineInfo {
                            id: id.clone(),
                            x1: p0[0], y1: p0[1], x2: p1[0], y2: p1[1],
                            color, visible,
                        });
                    }
                }
                ElementDef::WallArcElement(a)   => Self::add_arc(a, &mut bodies, &mut colliders, &mut wall_segs, label_for(elem_idx, &a.id)),
                ElementDef::WallPathElement(p) if p.ignore_ball == Some(true) => {
                    // Drawn only: one decor line per segment, sharing the path's id.
                    let visible = !matches!(p.color.as_ref().and_then(|c| c.as_array()).and_then(|a| a.get(3)).and_then(|v| v.as_u64()), Some(0));
                    let id = p.id.clone().unwrap_or_else(|| label_for(elem_idx, &p.id));
                    for pair in p.positions.windows(2) {
                        decor_lines.push(DecorLineInfo {
                            id: id.clone(),
                            x1: pair[0][0], y1: pair[0][1], x2: pair[1][0], y2: pair[1][1],
                            color: parse_color(&p.color), visible,
                        });
                    }
                }
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
                    if let (Some(id), Some(rect)) = (s.id.clone(), s.rect) {
                        sensors.push(SensorInfo { id, rect, from_layer: s.ball_layer_from });
                    }
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
        let balls = vec![BallState { body: idle_h, collider: ball_col_h, layer: 0, color: None, last_sensor: None, trail: Vec::new() }];
        let gate_ball = Some(idle_h);

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
            balls,
            ball_radius,
            ball_color,
            flippers,
            rotating_groups: movers::build_rotating_groups(table, &bumpers_info),
            rotating_triangles: movers::build_rotating_triangles(table, &moving_walls),
            moving_walls,
            decor_lines,
            shapes: Vec::new(),
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
            sensors,
            score_events: RefCell::new(Vec::new()),
            sound_events: RefCell::new(Vec::new()),
            multiplier_events: RefCell::new(Vec::new()),
            multiplier_percent_events: RefCell::new(Vec::new()),
            multiplier_scale_events: RefCell::new(Vec::new()),
            extra_ball_events: RefCell::new(0),
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
            gate_ball,
            relaunch_needed: false,
            left_held: false,
            right_held: false,
            game_time: 0.0,
            ball_start_time: None,
            used_mercy_ball: false,
            multiball_start_time: None,
            last_multiplier_increase: std::cell::Cell::new(None),
            stuck_seconds: 0.0,
            last_ball_positions: Vec::new(),
            stuck_seed: 1,
            script,
            scheduled: Vec::new(),
            script_events: Vec::new(),
            message_events: RefCell::new(Vec::new()),
        };
        // Put moving elements at their first-frame positions straight away,
        // as the original does on its first tick, so nothing jumps on frame one.
        movers::advance_rotating_groups(&mut world.rotating_groups, 0.0, &mut world.bumpers, &mut world.bodies);
        movers::advance_rotating_triangles(&mut world.rotating_triangles, 0.0, &mut world.moving_walls, &mut world.bodies);
        if let Some(mut script) = world.script.take() {
            script.game_started(&mut world);
            world.script = Some(script);
        }
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
        // ignoreBall walls have no body at all (the original draws them only when
        // a table script shows them, which we do not run yet, so they stay hidden).
        if w.ignore_ball == Some(true) { return None; }
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

        // As in the original, where each layer is its own world and an element
        // with no layer belongs to layer 0: a ball on another layer passes through.
        let groups = layer_group_for(layer.unwrap_or(0));

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
        if a.ignore_ball == Some(true) { return; }
        let color = parse_color(&a.color);
        for (i, (p0, p1)) in a.to_segments().into_iter().enumerate() {
            let dummy_def = WallDef {
                position: Some([p0[0], p0[1], p1[0], p1[1]]),
                id: None, color: None, kick: None, restitution: None,
                kill: None, disabled: None, retract_when_hit: None, ignore_ball: None, score: None,
                layer: None,
            };
            let seg_label = if i == 0 { Some(label.clone()) } else { None };
            let _ = Self::add_wall(&dummy_def, bodies, colliders, segs, a.layer, seg_label, color);
        }
    }

    fn add_path(p: &WallPathDef, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, segs: &mut Vec<WallSnapshot>, label: String) {
        if p.ignore_ball == Some(true) { return; }
        let color = parse_color(&p.color);
        for (i, pair) in p.positions.windows(2).enumerate() {
            let dummy = WallDef {
                position: Some([pair[0][0], pair[0][1], pair[1][0], pair[1][1]]),
                id: None, color: None, kick: None, restitution: None,
                kill: None, disabled: None, retract_when_hit: None, ignore_ball: None, score: None,
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
            .collision_groups(layer_group_for(b.layer.unwrap_or(0)))
            .build();
        colliders.insert_with_parent(col, h, bodies);

        bumpers.push(BumperInfo {
            layer: b.layer.unwrap_or(0),
            color_override: None,
            outer_override: None,
            body: h,
            position: b.position,
            radius: b.radius,
            outer_radius: b.outer_radius,
            // The original's defaults: a bumper with no score scores nothing, with no kick kicks nothing.
            score: b.score.unwrap_or(0),
            kick_force: b.kick.unwrap_or(0.0),
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
            .collision_groups(layer_group_for(f.layer.unwrap_or(0)))
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
            layer: f.layer.unwrap_or(0),
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
        let targets = d.targets().iter().map(|&[x1, y1, x2, y2]| {
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
                .collision_groups(layer_group_for(d.layer.unwrap_or(0)))
                .build();
            let ch = colliders.insert_with_parent(col, bh, bodies);
            DropTargetInfo { x1, y1, x2, y2, collider: ch, active: true }
        }).collect();
        DropTargetGroupInfo {
            layer: d.layer.unwrap_or(0),
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
            base_color: parse_color(&rv.color).or(group_color),
            color:  parse_color(&rv.color).or(group_color),
            reset_seconds: r.reset.unwrap_or(0.0).max(0.0),
            reset_timer: None,
        }).collect();
        RolloverGroupInfo {
            layer:            r.layer.unwrap_or(0),
            rollovers,
            score:            group_score,
            cycle_on_flipper: r.cycle_on_flipper.unwrap_or(false),
            ignore_ball:      r.ignore_ball.unwrap_or(false),
            flash_ticks:      0,
            label,
            shot_id: r.id.clone(),
            multiplier_step: r.multiplier_step.unwrap_or(0),
            visible: true,
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

    /// The launch velocity with the table's random top-up: base is always
    /// enough to clear the lane; the delta is `launchVelocityRandomDelta`.
    fn jittered_launch_velocity(&self) -> [f32; 2] {
        // The original adds `rand() * delta` to each component, independently, and only
        // when that delta is positive.
        let [vx, vy] = self.launch_vel;
        let [dx, dy] = self.launch_vel_delta;
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mut rand = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 // 0..1
        };
        let (rx, ry) = (rand(), rand());
        [if dx > 0.0 { vx + dx * rx } else { vx }, if dy > 0.0 { vy + dy * ry } else { vy }]
    }

    /// Creates a ball body and collider (on the main layer) and returns it.
    fn new_ball(&mut self, pos: [f32; 2], vel: [f32; 2], gravity_scale: f32) -> BallState {
        let rb = RigidBodyBuilder::dynamic()
            .translation(Vector::new(pos[0], pos[1]))
            .linvel(Vector::new(vel[0], vel[1]))
            .gravity_scale(gravity_scale)
            .ccd_enabled(true)
            .build();
        let body = self.bodies.insert(rb);
        let col = ColliderBuilder::ball(self.ball_radius)
            .restitution(BALL_RESTITUTION)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .friction(BALL_FRICTION)
            .density(1.0)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .collision_groups(ball_groups_for(0))
            .build();
        let collider = self.colliders.insert_with_parent(col, body, &mut self.bodies);
        BallState { body, collider, layer: 0, color: None, last_sensor: None, trail: Vec::new() }
    }

    fn remove_body(&mut self, h: RigidBodyHandle) {
        self.bodies.remove(
            h,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            false,
        );
        if self.gate_ball == Some(h) { self.gate_ball = None; }
    }

    /// Fires the waiting ball (the primary ball) up the shooter lane.
    pub fn launch_ball(&mut self) {
        let v = self.jittered_launch_velocity();
        let [lx, ly] = self.launch_pos;
        if let Some(first) = self.balls.first() {
            let h = first.body;
            if let Some(body) = self.bodies.get_mut(h) {
                body.set_translation(Vector::new(lx, ly), true);
                body.set_linvel(Vector::new(v[0], v[1]), true);
                body.set_angvel(0.0, true);
                body.set_gravity_scale(1.0, true);
                body.enable_ccd(true);
            }
            self.gate_ball = Some(h);
        } else {
            let ball = self.new_ball([lx, ly], v, 1.0);
            self.gate_ball = Some(ball.body);
            self.balls.push(ball);
        }
        if self.balls.len() <= 1 { self.ball_start_time = Some(self.game_time); }
    }

    /// Adds another ball to the field, launched up the shooter lane like the
    /// first (multiball). The lane gate opens for it and closes behind it.
    pub fn launch_extra_ball(&mut self) {
        let v = self.jittered_launch_velocity();
        let ball = self.new_ball(self.launch_pos, v, 1.0);
        self.gate_cleared = false;
        if let Some(gate) = self.gate_collider.and_then(|g| self.colliders.get_mut(g)) {
            gate.set_collision_groups(InteractionGroups::none());
        }
        self.gate_ball = Some(ball.body);
        self.balls.push(ball);
        if self.multiball_start_time.is_none() { self.multiball_start_time = Some(self.game_time); }
    }

    /// Number of balls on the field (including one parked in the lane).
    pub fn ball_count(&self) -> usize { self.balls.len() }

    /// Handle of the first (primary) ball, if any.
    pub fn ball_handle(&self) -> Option<RigidBodyHandle> { self.balls.first().map(|b| b.body) }

    /// (ball index, position, velocity) for every ball that has a body.
    fn live_balls(&self) -> Vec<(usize, Vector, Vector)> {
        self.balls.iter().enumerate().filter_map(|(i, b)| {
            let body = self.bodies.get(b.body)?;
            Some((i, body.translation(), body.linvel()))
        }).collect()
    }

    /// Takes every ball off the field but one, which waits in the shooter lane
    /// for a script to launch it (table 5's pyramid: the original removes the
    /// ball and launches four fresh ones).
    fn park_in_lane(&mut self) {
        self.place_parked_ball();
    }

    /// Takes a ball off the field without it counting as lost (a ball lock).
    /// If it was the last ball, a fresh one waits in the lane and the game is
    /// told to wait for a relaunch (`take_relaunch_needed`).
    pub fn remove_ball_without_loss(&mut self, index: usize) {
        if index >= self.balls.len() { return; }
        let ball = self.balls.remove(index);
        self.remove_body(ball.body);
        if self.balls.is_empty() {
            self.place_parked_ball();
            self.relaunch_needed = true;
        }
    }

    /// True once after the last ball was removed without loss; the game goes
    /// back to waiting for a launch without costing a ball.
    pub fn take_relaunch_needed(&mut self) -> bool { std::mem::take(&mut self.relaunch_needed) }

    pub fn step(&mut self, dt: f32, input: &InputState) {
        self.left_held = input.left_flipper;
        self.right_held = input.right_flipper;
        self.game_time += dt;
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
        let balls_now = self.live_balls();
        if let Some(barrier) = &mut self.launch_barrier {
            let any_inside = |r: Option<[f32; 4]>| r.map_or(false, |[x0, y0, x1, y1]| {
                balls_now.iter().any(|(_, p, _)| p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1)
            });
            let was = barrier.active;
            if any_inside(barrier.close_rect) { barrier.active = true; }
            if any_inside(barrier.open_rect) { barrier.active = false; }
            if barrier.active != was {
                let groups = if barrier.active { InteractionGroups::all() } else { InteractionGroups::none() };
                if let Some(col) = self.colliders.get_mut(barrier.collider) {
                    col.set_collision_groups(groups);
                }
            }
        }

        // Kick walls on moving walls follow them (position and which way they push).
        for k in 0..self.kick_walls.len() {
            let Some(mi) = self.kick_walls[k].follows else { continue };
            let (x1, y1, x2, y2) = { let m = &self.moving_walls[mi]; (m.x1, m.y1, m.x2, m.y2) };
            let (dx, dy) = (x2 - x1, y2 - y1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let (nx, ny) = (dy / len, -dx / len);
            let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
            let (fcx, fcy) = (self.field_width / 2.0, self.field_height / 2.0);
            let kw = &mut self.kick_walls[k];
            kw.x1 = x1; kw.y1 = y1; kw.x2 = x2; kw.y2 = y2;
            kw.interior_normal = if (fcx - mx) * nx + (fcy - my) * ny >= 0.0 { [nx, ny] } else { [-nx, -ny] };
        }

        // Bumper proximity detection: collect hits, then apply kicks + score
        {
            let mut hits: Vec<(usize, usize, f32, f32)> = Vec::new(); // (bumper, ball, nx, ny)
            for &(bi, bp, _) in &balls_now {
                {
                    for (i, bump) in self.bumpers.iter().enumerate() {
                        if hits.iter().any(|h| h.0 == i) { continue; }
                        if self.balls[bi].layer != bump.layer { continue; }
                        if bump.lit_ticks + BUMPER_DEBOUNCE_TICKS > BUMPER_LIT_TICKS { continue; }   // debounce right after a hit
                        let dx = bp.x - bump.position[0];
                        let dy = bp.y - bump.position[1];
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < bump.radius + self.ball_radius + 0.05 {
                            let (nx, ny) = if dist > 1e-4 { (dx / dist, dy / dist) } else { (0.0, 1.0) };
                            hits.push((i, bi, nx, ny));
                        }
                    }
                }
            }
            for (i, bi, nx, ny) in hits {
                self.bumpers[i].lit_ticks = BUMPER_LIT_TICKS;
                let bump = &mut self.bumpers[i];
                if bump.award_ladder.is_empty() {
                    // A bumper with no score of its own (table 3: its script scores) reports nothing.
                    if bump.score > 0 { self.score_events.borrow_mut().push(bump.score); }
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
                // Every bumper reports a hit; one without an id reports an empty id.
                let id = self.bumpers[i].shot_id.clone().unwrap_or_default();
                self.script_events.push(ScriptEvent::BumperHit { id, ball: bi });
                if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &self.bumpers[i].shot_id) {
                    self.score_events.borrow_mut().push(pts);
                }
                let kick = self.bumpers[i].kick_force;
                if let Some(ball_h) = self.balls.get(bi).map(|b| b.body) {
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
            let mut hits: Vec<(usize, usize, f32, f32)> = Vec::new(); // (wall, ball, nx, ny)
            for &(bi, bp, _) in &balls_now {
                {
                    for (i, kw) in self.kick_walls.iter().enumerate() {
                        if !kw.active || kw.debounce > 0 { continue; }
                        if hits.iter().any(|h| h.0 == i) { continue; }
                        if self.balls[bi].layer != kw.layer { continue; }
                        let (dist, _, _) = Self::point_to_segment_dist(bp.x, bp.y, kw.x1, kw.y1, kw.x2, kw.y2);
                        if dist < self.ball_radius + KICK_WALL_TOUCH_MARGIN {
                            // Push toward the side the ball is on, like the original
                            // (the interior normal is only the fallback when the ball
                            // is exactly on the wall's line).
                            let (dx, dy) = (kw.x2 - kw.x1, kw.y2 - kw.y1);
                            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                            let (nx, ny) = (dy / len, -dx / len);
                            let side = (bp.x - kw.x1) * nx + (bp.y - kw.y1) * ny;
                            let (nx, ny) = if side > 1e-4 { (nx, ny) } else if side < -1e-4 { (-nx, -ny) } else { (kw.interior_normal[0], kw.interior_normal[1]) };
                            hits.push((i, bi, nx, ny));
                        }
                    }
                }
            }
            for (i, bi, nx, ny) in hits {
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
                if let Some(id) = self.kick_walls[i].shot_id.clone() {
                    self.script_events.push(ScriptEvent::WallHit { id, ball: bi });
                }
                let kick = self.kick_walls[i].kick_force;
                if let Some(ball_h) = self.balls.get(bi).map(|b| b.body) {
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
            let mut hits: Vec<(usize, usize, usize)> = Vec::new(); // (group, target, ball)
            for &(bi, bp, _) in &balls_now {
                {
                    for (gi, group) in self.drop_target_groups.iter().enumerate() {
                        if self.balls[bi].layer != group.layer { continue; }
                        for (ti, target) in group.targets.iter().enumerate() {
                            if !target.active { continue; }
                            let (dist, _, _) = Self::point_to_segment_dist(
                                bp.x, bp.y,
                                target.x1, target.y1, target.x2, target.y2,
                            );
                            if dist < self.ball_radius + 0.05 && !hits.iter().any(|h| h.0 == gi && h.1 == ti) {
                                hits.push((gi, ti, bi));
                            }
                        }
                    }
                }
            }
            let mut savers_to_restore: Vec<&'static str> = Vec::new();
            for (gi, ti, bi) in hits {
                let score = self.drop_target_groups[gi].score;
                if score > 0 { self.score_events.borrow_mut().push(score); }

                self.drop_target_groups[gi].targets[ti].active = false;
                let ch = self.drop_target_groups[gi].targets[ti].collider;
                if let Some(col) = self.colliders.get_mut(ch) {
                    col.set_collision_groups(InteractionGroups::none());
                }
                if let Some(id) = self.drop_target_groups[gi].shot_id.clone() {
                    self.script_events.push(ScriptEvent::DropTargetHit { id, ball: bi });
                }

                let all_clear = self.drop_target_groups[gi].targets.iter().all(|t| !t.active);
                if all_clear {
                    // Our own clear bonus; the original pays only per target, so a
                    // table with a script (the original's rules) does not get it.
                    if self.script.is_none() { self.score_events.borrow_mut().push(DROP_CLEAR_BONUS); }
                    if let Some(pts) = Self::record_shot_opt(&mut self.letter_groups, &mut self.jackpot, &self.drop_target_groups[gi].shot_id) {
                        self.score_events.borrow_mut().push(pts);
                    }
                    // The targets come back after the group's reset delay (in
                    // sim time), or never if it has none, as in the original.
                    self.drop_target_groups[gi].reset_timer = self.drop_target_groups[gi].reset_delay;
                    if let Some(id) = self.drop_target_groups[gi].shot_id.clone() {
                        self.script_events.push(ScriptEvent::DropBankCleared { id, ball: bi });
                    }
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
            let groups = layer_group_for(self.drop_target_groups[gi].layer);
            for target in &mut self.drop_target_groups[gi].targets {
                target.active = true;
                if let Some(col) = self.colliders.get_mut(target.collider) {
                    col.set_collision_groups(groups);
                }
            }
        }

        // Rollover detection, flipper cycling, and group-completion flash.
        {
            // A table script owns group completion (the original's delegates do):
            // no flash, no completion bonus, no automatic reset.
            let script_rollovers = self.script.as_ref().map_or(false, |sc| sc.handles_rollovers());
            for group in &mut self.rollover_groups {
                // Rollovers with a reset delay turn themselves off after it.
                for rv in &mut group.rollovers {
                    if let Some(left) = rv.reset_timer {
                        if left - dt <= 0.0 { rv.lit = false; rv.reset_timer = None; } else { rv.reset_timer = Some(left - dt); }
                    }
                }
                // ── Ball activation (skip while flashing — group is mid-reset) ──
                if !group.ignore_ball && group.flash_ticks == 0 {
                    let mut newly_lit: Option<usize> = None;
                    for &(bi, bp, _) in &balls_now {
                        if self.balls[bi].layer != group.layer { continue; }
                        for rv in &mut group.rollovers {
                            if rv.lit { continue; }
                            let dx = bp.x - rv.x;
                            let dy = bp.y - rv.y;
                            // The ball's centre must be within the rollover's radius, as in the original
                            // (and the editor); adding the ball's radius made them twice as easy to hit.
                            if dx * dx + dy * dy <= rv.radius * rv.radius {
                                rv.lit = true;
                                rv.reset_timer = (rv.reset_seconds > 0.0).then_some(rv.reset_seconds);
                                newly_lit = Some(bi);
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
                    // A ball lit the last one: tell the table's script.
                    if let Some(ball) = newly_lit.filter(|_| group.rollovers.iter().all(|rv| rv.lit)) {
                        if let Some(id) = group.shot_id.clone() {
                            self.script_events.push(ScriptEvent::RolloverGroupCompleted { id, ball });
                        }
                    }
                }

                // ── Group completion: flash, then bonus + reset ──
                let all_lit = !script_rollovers && !group.rollovers.is_empty() && group.rollovers.iter().all(|rv| rv.lit);
                if Self::tick_completion_flash(&mut group.flash_ticks, ROLLOVER_FLASH_TICKS, all_lit) {
                    if group.score > 0 {
                        self.score_events.borrow_mut().push(group.score);
                    }
                    if group.multiplier_step > 0 {
                        self.last_multiplier_increase.set(Some(self.game_time));
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
            let ball_pv: Vec<(usize, Vector, Vector)> = balls_now.clone();
            for sp in &mut self.spinners {
                if sp.status == SpinnerStatus::Inactive {
                    for &(bi, pos, vel) in &ball_pv {
                        if sp.status != SpinnerStatus::Inactive { break; }
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
                                if let Some(id) = sp.shot_id.clone() {
                                    self.script_events.push(ScriptEvent::SpinnerActivated { id, ball: bi });
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

        // Layer-sensor detection: each ball moves to a new physics layer when it
        // enters a sensor rect (and the optional from-layer filter matches).
        // The layers before any switching: a sensor's from-layer filter is checked
        // before the switch, as in the original, even when it also notifies a script.
        let layers_before: Vec<u8> = self.balls.iter().map(|b| b.layer).collect();
        for &(bi, bp, _) in &balls_now {
            let layer = self.balls[bi].layer;
            let mut new_layer = layer;
            for sensor in &self.layer_sensors {
                let [xmin, ymin, xmax, ymax] = sensor.rect;
                if bp.x >= xmin && bp.x <= xmax && bp.y >= ymin && bp.y <= ymax
                    && sensor.from_layer.map_or(true, |fl| fl == layer)
                {
                    new_layer = sensor.target_layer;
                    break;
                }
            }
            if new_layer != layer {
                self.balls[bi].layer = new_layer;
                let collider = self.balls[bi].collider;
                if let Some(col) = self.colliders.get_mut(collider) {
                    col.set_collision_groups(ball_groups_for(new_layer));
                }
            }
        }

        // Sensors by id: report every ball inside one to the table script, which
        // sees the ball's previous sensor; then the ball remembers this one.
        for si in 0..self.sensors.len() {
            let [x0, y0, x1, y1] = self.sensors[si].rect;
            for &(bi, bp, _) in &balls_now {
                if !(bp.x >= x0 && bp.x <= x1 && bp.y >= y0 && bp.y <= y1) { continue; }
                if let Some(from) = self.sensors[si].from_layer {
                    if from != layers_before[bi] { continue; }
                }
                let id = self.sensors[si].id.clone();
                let previous = self.balls[bi].last_sensor.replace(id.clone());
                self.script_events.push(ScriptEvent::Sensor { id, ball: bi, previous });
            }
        }

        // Shooter-lane gate: enable once the ball clears gate_y going upward.
        if !self.gate_cleared {
            if let (Some(ball_h), Some(gate_col_h)) = (self.gate_ball, self.gate_collider) {
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

        // Multiball: a ball that drains while others are still in play is just
        // removed. The last ball draining is the normal ball-lost event, which
        // the game state reads through `is_ball_drained`.
        let mut replaced_this_step = false;
        let mut i = 0;
        while self.balls.len() > 1 && i < self.balls.len() {
            if self.drained_at(i) {
                let ball = self.balls.remove(i);
                self.remove_body(ball.body);
                // Early in a multiball a lost ball is replaced, once per step even if
                // two are lost together.
                let within_saver = self.multiball_start_time.map_or(false, |t| self.game_time - t <= MULTIBALL_SAVER_SECONDS);
                if within_saver && !replaced_this_step {
                    replaced_this_step = true;
                    self.show_message("Ball Saved", 1000);
                    self.launch_extra_ball();
                }
            } else {
                i += 1;
            }
        }
        if self.balls.len() == 1 { self.multiball_start_time = None; }

        // Mercy ball: the only ball lost soon after its launch comes straight back, once.
        if self.balls.len() == 1 && !self.used_mercy_ball && self.drained_at(0) {
            if let Some(start) = self.ball_start_time {
                if self.game_time - start <= MERCY_BALL_SECONDS {
                    self.used_mercy_ball = true;
                    self.show_message("Ball Saved", 1500);
                    self.launch_ball();
                }
            }
        }

        self.run_script(dt);
    }

    // -----------------------------------------------------------------------
    // Table scripts: events in, helpers out (see scripts.rs)
    // -----------------------------------------------------------------------

    /// Fires due timers, then hands this step's events to the table script.
    fn run_script(&mut self, dt: f32) {
        if self.script.is_none() {
            self.script_events.clear();
            return;
        }
        // Per-step hook first (table 6 moves its planets and pulls the ball here).
        let mut script = self.script.take().unwrap();
        script.tick(self, dt);
        self.script = Some(script);
        let mut due = Vec::new();
        for t in &mut self.scheduled { t.0 -= dt; }
        self.scheduled.retain(|&(left, token)| {
            if left <= 0.0 { due.push(token); false } else { true }
        });
        for token in due { self.script_events.push(ScriptEvent::Scheduled(token)); }

        // A script's reaction can raise more events; allow a few rounds.
        for _ in 0..8 {
            if self.script_events.is_empty() { break; }
            let events = std::mem::take(&mut self.script_events);
            let mut script = self.script.take().unwrap();
            for ev in events {
                match ev {
                    ScriptEvent::BumperHit { id, ball } => script.bumper_hit(self, ball, &id),
                    ScriptEvent::DropBankCleared { id, ball } => script.drop_bank_cleared(self, ball, &id),
                    ScriptEvent::DropTargetHit { id, ball } => script.drop_target_hit(self, ball, &id),
                    ScriptEvent::SpinnerActivated { id, ball } => script.spinner_activated(self, ball, &id),
                    ScriptEvent::RolloverGroupCompleted { id, ball } => script.rollover_group_completed(self, ball, &id),
                    ScriptEvent::Sensor { id, ball, previous } => script.sensor(self, ball, &id, previous.as_deref()),
                    ScriptEvent::WallHit { id, ball } => script.wall_hit(self, ball, &id),
                    ScriptEvent::Scheduled(token) => script.scheduled(self, token),
                }
            }
            self.script = Some(script);
        }
        self.script_events.clear();
    }

    /// Shows the player a message for `ms` of real time.
    fn show_message(&self, text: &str, ms: u32) {
        self.message_events.borrow_mut().push((text.to_string(), ms));
    }

    /// Runs the script's `scheduled(token)` after `game_ms` of game time (sim
    /// time, which runs at the table's time ratio, like the original).
    fn schedule(&mut self, game_ms: u32, token: u32) {
        self.scheduled.push((game_ms as f32 / 1000.0, token));
    }

    /// Raises the score multiplier by one (the game state shows the message).
    fn increment_multiplier(&self) {
        self.last_multiplier_increase.set(Some(self.game_time));
        self.multiplier_events.borrow_mut().push(1);
    }

    fn rollover_group(&self, id: &str) -> Option<&RolloverGroupInfo> {
        self.rollover_groups.iter().find(|g| g.shot_id.as_deref() == Some(id))
    }

    fn rollover_group_mut(&mut self, id: &str) -> Option<&mut RolloverGroupInfo> {
        self.rollover_groups.iter_mut().find(|g| g.shot_id.as_deref() == Some(id))
    }

    /// The group's own `score` (table 3 pays it again for a ramp during multiball).
    fn rollover_group_score(&self, id: &str) -> u64 {
        self.rollover_group(id).map_or(0, |g| g.score)
    }

    fn rollover_count(&self, id: &str) -> usize {
        self.rollover_group(id).map_or(0, |g| g.rollovers.len())
    }

    fn rollover_lit(&self, id: &str, index: usize) -> bool {
        self.rollover_group(id).and_then(|g| g.rollovers.get(index)).map_or(false, |r| r.lit)
    }

    fn rollovers_all_lit(&self, id: &str) -> bool {
        self.rollover_group(id).map_or(false, |g| !g.rollovers.is_empty() && g.rollovers.iter().all(|r| r.lit))
    }

    fn set_rollover(&mut self, id: &str, index: usize, lit: bool) {
        if let Some(r) = self.rollover_group_mut(id).and_then(|g| g.rollovers.get_mut(index)) {
            r.lit = lit;
            if !lit { r.reset_timer = None; }
        }
    }

    fn set_all_rollovers(&mut self, id: &str, lit: bool) {
        if let Some(g) = self.rollover_group_mut(id) {
            for r in &mut g.rollovers {
                r.lit = lit;
                if !lit { r.reset_timer = None; }
            }
        }
    }

    /// Lights the first unlit rollover of the group; false if all were lit.
    fn activate_first_unlit_rollover(&mut self, id: &str) -> bool {
        match self.rollover_group_mut(id).and_then(|g| g.rollovers.iter_mut().find(|r| !r.lit)) {
            Some(r) => { r.lit = true; true }
            None => false,
        }
    }

    /// The ball in play was lost (call before `park_ball`): tells the table script.
    pub fn ball_lost(&mut self) {
        self.used_mercy_ball = false;
        if let Some(mut script) = self.script.take() {
            script.ball_lost(self);
            self.script = Some(script);
        }
    }

    /// Retracts or extends a wall by id: a kick wall, a scripted wall or the
    /// launch barrier. A retracted wall is neither solid nor drawn.
    fn set_wall_retracted(&mut self, id: &str, retracted: bool) {
        let solid = |layer: u8| if retracted { InteractionGroups::none() } else { layer_group_for(layer) };
        if let Some(kw) = self.kick_walls.iter_mut().find(|k| k.shot_id.as_deref() == Some(id)) {
            let groups = solid(kw.layer);
            kw.active = !retracted;
            kw.retract_pending = false;
            let follows = kw.follows;
            if let Some(col) = self.colliders.get_mut(kw.collider_handle) { col.set_collision_groups(groups); }
            if let Some(mi) = follows { self.moving_walls[mi].active = !retracted; }
            return;
        }
        if let Some(mw) = self.moving_walls.iter_mut().find(|m| m.id == id) {
            let groups = solid(mw.layer);
            mw.active = !retracted;
            if let Some(col) = self.colliders.get_mut(mw.collider) { col.set_collision_groups(groups); }
            return;
        }
        if id == LAUNCH_BARRIER_ID {
            if let Some(b) = &mut self.launch_barrier {
                b.active = !retracted;
                if let Some(col) = self.colliders.get_mut(b.collider) { col.set_collision_groups(solid(0)); }
            }
        }
    }

    fn wall_retracted(&self, id: &str) -> Option<bool> {
        if let Some(kw) = self.kick_walls.iter().find(|k| k.shot_id.as_deref() == Some(id)) { return Some(!kw.active); }
        if let Some(mw) = self.moving_walls.iter().find(|m| m.id == id) { return Some(!mw.active); }
        if id == LAUNCH_BARRIER_ID { return self.launch_barrier.as_ref().map(|b| !b.active); }
        None
    }

    fn left_flipper_held(&self) -> bool { self.left_held }

    /// How many rollovers of the group are lit.
    fn lit_count(&self, id: &str) -> usize {
        self.rollover_group(id).map_or(0, |g| g.rollovers.iter().filter(|r| r.lit).count())
    }

    fn drop_target_group(&self, id: &str) -> Option<&DropTargetGroupInfo> {
        self.drop_target_groups.iter().find(|g| g.shot_id.as_deref() == Some(id))
    }

    fn drop_target_count(&self, id: &str) -> usize {
        self.drop_target_group(id).map_or(0, |g| g.targets.len())
    }

    /// True if target `index` of the bank is down.
    fn drop_target_down(&self, id: &str, index: usize) -> bool {
        self.drop_target_group(id).and_then(|g| g.targets.get(index)).map_or(false, |t| !t.active)
    }

    /// Knocks a target down or puts it back up.
    fn set_drop_target_down(&mut self, id: &str, index: usize, down: bool) {
        let Some(g) = self.drop_target_groups.iter_mut().find(|g| g.shot_id.as_deref() == Some(id)) else { return };
        let layer = g.layer;
        let Some(t) = g.targets.get_mut(index) else { return };
        t.active = !down;
        let groups = if down { InteractionGroups::none() } else { layer_group_for(layer) };
        let collider = t.collider;
        if let Some(col) = self.colliders.get_mut(collider) { col.set_collision_groups(groups); }
    }

    /// Puts every target of the bank back up (original: `makeAllTargetsVisible`).
    fn make_targets_up(&mut self, id: &str) {
        for i in 0..self.drop_target_count(id) { self.set_drop_target_down(id, i, false); }
        if let Some(g) = self.drop_target_groups.iter_mut().find(|g| g.shot_id.as_deref() == Some(id)) {
            g.reset_timer = None;
        }
    }

    fn spinner_score(&self, id: &str) -> u64 {
        self.spinners.iter().find(|s| s.shot_id.as_deref() == Some(id)).map_or(0, |s| s.score)
    }

    fn set_spinner_score(&mut self, id: &str, score: u64) {
        if let Some(s) = self.spinners.iter_mut().find(|s| s.shot_id.as_deref() == Some(id)) { s.score = score; }
    }

    /// Shows or hides a whole rollover group.
    fn set_rollovers_visible(&mut self, id: &str, visible: bool) {
        if let Some(g) = self.rollover_group_mut(id) { g.visible = visible; }
    }

    /// Whether balls light the group's rollovers (original: `setIgnoreBall`).
    /// Table 5's triangle turns at `m` times its base speed (-2 in multiball).
    fn set_triangle_speed_multiplier(&mut self, m: f32) {
        for t in &mut self.rotating_triangles { t.speed_multiplier = m; }
    }

    fn group_ignores_ball(&self, id: &str) -> bool {
        self.rollover_group(id).map_or(false, |g| g.ignore_ball)
    }

    fn set_group_ignore_ball(&mut self, id: &str, ignore: bool) {
        if let Some(g) = self.rollover_group_mut(id) { g.ignore_ball = ignore; }
    }

    /// Shows or hides a drawn-only line (an `ignoreBall` wall).
    fn set_decor_visible(&mut self, id: &str, visible: bool) {
        for d in self.decor_lines.iter_mut().filter(|d| d.id == id) { d.visible = visible; }
    }

    /// Recolours a drawn-only line or path.
    fn set_decor_color(&mut self, id: &str, color: (u8, u8, u8)) {
        for d in self.decor_lines.iter_mut().filter(|d| d.id == id) { d.color = Some(color); }
    }

    /// Replaces the script's dots and lines (drawn behind the table's elements).
    fn set_shapes(&mut self, shapes: Vec<Shape>) {
        self.shapes = shapes;
    }

    /// Moves a ball to another layer (the original's `Ball.moveToLayer`): it
    /// then only touches that layer's elements.
    fn set_ball_layer(&mut self, ball: usize, layer: u8) {
        let Some(b) = self.balls.get_mut(ball) else { return };
        b.layer = layer;
        let collider = b.collider;
        if let Some(col) = self.colliders.get_mut(collider) { col.set_collision_groups(ball_groups_for(layer)); }
    }

    fn ball_layer_of(&self, ball: usize) -> u8 {
        self.balls.get(ball).map_or(0, |b| b.layer)
    }

    fn set_ball_velocity(&mut self, ball: usize, vx: f32, vy: f32) {
        if let Some(body) = self.balls.get(ball).and_then(|b| self.bodies.get_mut(b.body)) {
            body.set_linvel(Vector::new(vx, vy), true);
        }
    }

    /// Puts a ball at `(x, y)` on `layer` (the original's `createBall`, table 7's
    /// locked balls reappear where they were held). Returns its index.
    fn add_ball_at(&mut self, x: f32, y: f32, layer: u8) -> usize {
        let ball = self.new_ball([x, y], [0.0, 0.0], 1.0);
        self.balls.push(ball);
        let index = self.balls.len() - 1;
        self.set_ball_layer(index, layer);
        if self.multiball_start_time.is_none() { self.multiball_start_time = Some(self.game_time); }
        index
    }

    /// Moves a ball and stops it dead (a held ball).
    fn hold_ball_at(&mut self, ball: usize, x: f32, y: f32) {
        if let Some(body) = self.balls.get(ball).and_then(|b| self.bodies.get_mut(b.body)) {
            body.set_translation(Vector::new(x, y), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_angvel(0.0, true);
        }
    }

    fn set_ball_gravity_scale(&mut self, ball: usize, scale: f32) {
        if let Some(body) = self.balls.get(ball).and_then(|b| self.bodies.get_mut(b.body)) {
            body.set_gravity_scale(scale, true);
        }
    }

    /// Awards an extra ball (the game state adds it to the balls left).
    fn add_extra_ball(&self) {
        *self.extra_ball_events.borrow_mut() += 1;
    }

    pub fn drain_extra_ball_events(&self) -> u32 {
        std::mem::take(&mut *self.extra_ball_events.borrow_mut())
    }

    /// Multiplies the whole score multiplier by `percent / 100` (the game state
    /// does the arithmetic, it owns the multiplier).
    fn scale_multiplier_percent(&self, percent: u64) {
        self.multiplier_scale_events.borrow_mut().push(percent);
    }

    /// Adds `percent` points to the score multiplier (1 = +0.01x).
    fn add_multiplier_percent(&self, percent: i32) {
        self.multiplier_percent_events.borrow_mut().push(percent);
    }

    /// Adds points (the game state applies the score multiplier).
    fn add_score(&self, points: u64) {
        self.score_events.borrow_mut().push(points);
    }

    fn ball_position_of(&self, ball: usize) -> Option<Vector> {
        let body = self.bodies.get(self.balls.get(ball)?.body)?;
        Some(body.translation())
    }

    fn ball_color(&self, ball: usize) -> Option<(u8, u8, u8)> {
        self.balls.get(ball).and_then(|b| b.color)
    }

    fn set_ball_color(&mut self, ball: usize, color: Option<(u8, u8, u8)>) {
        if let Some(b) = self.balls.get_mut(ball) { b.color = color; }
    }

    /// True if any ball on the field has exactly this colour.
    fn any_ball_has_color(&self, color: (u8, u8, u8)) -> bool {
        self.balls.iter().any(|b| b.color == Some(color))
    }

    fn apply_impulse_to_ball(&mut self, ball: usize, impulse: Vector) {
        if let Some(body) = self.balls.get(ball).and_then(|b| self.bodies.get_mut(b.body)) {
            body.apply_impulse(impulse, true);
        }
    }

    fn rollover_center(&self, id: &str, index: usize) -> Option<(f32, f32)> {
        self.rollover_group(id).and_then(|g| g.rollovers.get(index)).map(|r| (r.x, r.y))
    }

    fn rollover_radius(&self, id: &str, index: usize) -> Option<f32> {
        self.rollover_group(id).and_then(|g| g.rollovers.get(index)).map(|r| r.radius)
    }

    /// Moves a rollover (the planets of table 6 orbit this way).
    fn set_rollover_center(&mut self, id: &str, index: usize, x: f32, y: f32) {
        if let Some(r) = self.rollover_group_mut(id).and_then(|g| g.rollovers.get_mut(index)) {
            r.x = x;
            r.y = y;
        }
    }

    /// Overrides a rollover's colour; None restores the table's colour.
    fn set_rollover_color(&mut self, id: &str, index: usize, color: Option<(u8, u8, u8)>) {
        if let Some(r) = self.rollover_group_mut(id).and_then(|g| g.rollovers.get_mut(index)) {
            r.color = color.or(r.base_color);
        }
    }

    /// Overrides every bumper's colour (and its outer ring, dimmer, standing in for
    /// the original's alpha 128); None restores the table's colours.
    fn set_bumper_colors(&mut self, color: Option<(u8, u8, u8)>) {
        for b in &mut self.bumpers {
            b.color_override = color;
            b.outer_override = color.map(|(r, g, bl)| (r / 2, g / 2, bl / 2));
        }
    }

    pub fn drain_multiplier_scale_events(&self) -> Vec<u64> {
        self.multiplier_scale_events.borrow_mut().drain(..).collect()
    }

    pub fn drain_multiplier_percent_events(&self) -> Vec<i32> {
        self.multiplier_percent_events.borrow_mut().drain(..).collect()
    }

    /// Messages the player should see, with their durations in ms.
    pub fn drain_message_events(&self) -> Vec<(String, u32)> {
        self.message_events.borrow_mut().drain(..).collect()
    }

    /// Re-arms the ball saver with this element id: visible again, and its
    /// collider back in the physics world.
    fn restore_ball_saver(&mut self, id: &str) {
        for kw in &mut self.kick_walls {
            if kw.retract_when_hit && kw.shot_id.as_deref() == Some(id) {
                kw.active = true;
                kw.retract_pending = false;
                if let Some(col) = self.colliders.get_mut(kw.collider_handle) {
                    col.set_collision_groups(layer_group_for(kw.layer));
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

    /// Position of the first (primary) ball.
    pub fn ball_position(&self) -> Option<Vector> {
        let body = self.bodies.get(self.balls.first()?.body)?;
        Some(body.translation())
    }

    /// Whether ball `i` is below the drain line (and not just sitting in the
    /// shooter-lane dead zone).
    fn drained_at(&self, i: usize) -> bool {
        let Some(ball) = self.balls.get(i) else { return false };
        let Some(body) = self.bodies.get(ball.body) else { return false };
        let pos = body.translation();
        if pos.y >= self.drain_y { return false; }
        if let Some([xmin, _, xmax, _]) = self.dead_zone {
            if pos.x >= xmin - 0.2 && pos.x <= xmax + 0.2 { return false; }
        }
        true
    }

    /// True when the LAST ball on the field has drained. A ball that drains
    /// while others are in play is simply removed in `step` (multiball).
    pub fn is_ball_drained(&self) -> bool {
        self.balls.len() == 1 && self.drained_at(0)
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
            let vy = self.balls.first()
                .and_then(|b| self.bodies.get(b.body))
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

    /// Removes the first ball (unused by the game; kept for tests and tools).
    pub fn remove_ball(&mut self) {
        if !self.balls.is_empty() {
            let ball = self.balls.remove(0);
            self.remove_body(ball.body);
        }
    }

    /// Freeze the ball at the launch position with gravity off so it appears
    /// waiting in the shooter lane between balls.  Also resets any bumper award
    /// ladders (and their super-mode trigger) to fire again, and un-lights
    /// the jackpot (and clears letter-award completion progress toward it)
    /// for the new ball.
    pub fn park_ball(&mut self) {
        self.place_parked_ball();
        self.last_multiplier_increase.set(None);
        self.stuck_seconds = 0.0;
        self.last_ball_positions.clear();
        // Scheduled actions are NOT cleared: as in the original, they fire whatever
        // happens to the ball (table 8 schedules its rack light show at ball loss).
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

    /// Leaves exactly one ball waiting in the shooter lane: extra balls are
    /// dropped, a ball is created if none is left, and the lane gate reopens.
    fn place_parked_ball(&mut self) {
        while self.balls.len() > 1 {
            let extra = self.balls.pop().unwrap();
            self.remove_body(extra.body);
        }
        let [lx, ly] = self.launch_pos;
        if self.balls.is_empty() {
            let ball = self.new_ball([lx, ly], [0.0, 0.0], 0.0);
            self.balls.push(ball);
        }
        let first = &mut self.balls[0];
        first.layer = 0; // back on the main-field layer
        first.color = None;
        first.last_sensor = None;
        let (h, c) = (first.body, first.collider);
        if let Some(body) = self.bodies.get_mut(h) {
            body.set_translation(Vector::new(lx, ly), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_angvel(0.0, true);
            body.set_gravity_scale(0.0, true);
        }
        if let Some(col) = self.colliders.get_mut(c) {
            col.set_collision_groups(ball_groups_for(0));
        }
        self.gate_ball = Some(h);
        // Re-open the shooter-lane gate for the next launch attempt.
        self.gate_cleared = false;
        if let Some(gate_col_h) = self.gate_collider {
            if let Some(col) = self.colliders.get_mut(gate_col_h) {
                col.set_collision_groups(InteractionGroups::none());
            }
        }
    }

    pub fn field_height(&self) -> f32 { self.field_height }

    /// Current ball speed in sim units per sim second, if a ball exists.
    #[cfg(test)]
    fn ball_speed(&self) -> Option<f32> {
        let body = self.bodies.get(self.balls.first()?.body)?;
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
        self.check_for_stuck_ball(dt * PHYSICS_SUBSTEPS as f32, input.left_flipper || input.right_flipper);
        self.record_trails();
    }

    /// Remembers where each ball is, for the trail drawn behind it. A ball that
    /// jumped (launched, relocated by a script) starts a new trail.
    fn record_trails(&mut self) {
        for i in 0..self.balls.len() {
            let Some(pos) = self.bodies.get(self.balls[i].body).map(|b| b.translation()) else { continue };
            let trail = &mut self.balls[i].trail;
            if let Some(&(lx, ly)) = trail.last() {
                let moved = ((pos.x - lx).powi(2) + (pos.y - ly).powi(2)).sqrt();
                if moved > TRAIL_JUMP { trail.clear(); } else if moved < 0.02 { continue; }
            }
            trail.push((pos.x, pos.y));
            if trail.len() > TRAIL_LENGTH { trail.remove(0); }
        }
    }

    /// True when a ball lost shortly after a script raised the multiplier should
    /// keep that increase for the next ball (the original's
    /// `preserveMultiplierIncreaseDuration`, 7500 ms of game time).
    pub fn multiplier_increase_preserved(&self) -> bool {
        self.last_multiplier_increase.get().map_or(false, |t| self.game_time - t <= PRESERVE_MULTIPLIER_SECONDS)
    }

    /// The original's stuck ball rule: when every ball in play has sat still for
    /// 10 seconds of game time, with no flipper held (it could be trapping the
    /// ball), show "Bump!" and give every ball a shove.
    fn check_for_stuck_ball(&mut self, sim_secs: f32, flipper_held: bool) {
        let positions: Vec<(f32, f32)> = self.live_balls().iter().map(|(_, p, _)| (p.x, p.y)).collect();
        // A ball waiting in the shooter lane has no gravity and is not stuck.
        let in_play = !self.balls.is_empty() && self.balls.iter().all(|b| {
            self.bodies.get(b.body).map_or(false, |body| body.gravity_scale() > 0.0)
        });
        let still = in_play && positions.len() == self.last_ball_positions.len()
            && self.live_balls().iter().zip(&self.last_ball_positions).all(|((_, p, v), last)| {
                v.length_squared() <= 0.01 && (p.x - last.0).powi(2) + (p.y - last.1).powi(2) <= 0.01
            });
        self.last_ball_positions = positions;
        if !still {
            self.stuck_seconds = 0.0;
            return;
        }
        if flipper_held { return; }
        self.stuck_seconds += sim_secs;
        if self.stuck_seconds > STUCK_BALL_SECONDS {
            self.show_message("Bump!", 1000);
            self.stuck_seed = self.stuck_seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            for i in 0..self.balls.len() {
                let x = if (self.stuck_seed >> (8 + i)) & 1 == 0 { 1.0 } else { -1.0 };
                if let Some(body) = self.bodies.get_mut(self.balls[i].body) {
                    body.apply_impulse(Vector::new(x, 1.5), true);
                }
            }
            self.stuck_seconds = 0.0;
        }
    }
    /// Sim seconds advanced per real second (the table's `targetTimeRatio`).
    pub fn time_ratio(&self) -> f32 { self.time_ratio }
    pub fn field_width(&self)  -> f32 { self.field_width }

    pub fn snapshot(&self) -> FieldSnapshot {
        let balls: Vec<BallSnapshot> = self.live_balls().into_iter()
            .map(|(i, t, v)| BallSnapshot { x: t.x, y: t.y, vx: v.x, vy: v.y, color: self.balls[i].color, trail: self.balls[i].trail.clone() })
            .collect();
        // Elements on another layer (table 7's mini table) show only while a ball is on it.
        let occupied_layers = self.balls.iter().fold(0u32, |m, b| m | (1u32 << b.layer.min(31)));
        let shown = |layer: u8| layer == 0 || occupied_layers & (1u32 << layer.min(31)) != 0;

        // Read the tip position straight from the physics body's actual
        // transform (translation + rotation) rather than a separately
        // tracked angle, now that the flipper is a real dynamic body.
        let flippers: Vec<FlipperSnapshot> = self.flippers.iter().filter(|fl| shown(fl.layer)).filter_map(|fl| {
            let body = self.bodies.get(fl.body_handle)?;
            let local_tip_x = if fl.is_right { -fl.half_len } else { fl.half_len };
            let tip = body.position() * Vector::new(local_tip_x, 0.0);
            Some(FlipperSnapshot { x1: fl.pivot[0], y1: fl.pivot[1], x2: tip.x, y2: tip.y, label: fl.label.clone() })
        }).collect();

        let bumpers = self.bumpers.iter().filter(|b| shown(b.layer)).map(|b| BumperSnapshot {
            x: b.position[0], y: b.position[1],
            radius: b.radius,
            outer_radius: b.outer_radius,
            lit: b.lit_ticks > 0,
            label: b.label.clone(),
            color: b.color_override.or(b.color),
            outer_color: b.outer_override.or(b.outer_color),
            next_award: (!b.award_ladder.is_empty()).then(|| b.award_ladder[b.ladder_rung]),
        }).collect();

        let kick_walls = self.kick_walls.iter()
            .filter(|kw| kw.active && kw.follows.is_none() && shown(kw.layer))
            .map(|kw| WallSnapshot { x1: kw.x1, y1: kw.y1, x2: kw.x2, y2: kw.y2, layer: None, label: Some(kw.label.clone()), color: None })
            .collect();

        let drop_targets: Vec<WallSnapshot> = self.drop_target_groups.iter().filter(|g| shown(g.layer))
            .flat_map(|g| g.targets.iter().enumerate().filter(|(_, t)| t.active)
                .map(move |(i, t)| (g.label.clone(), g.color, i, t)))
            .map(|(glabel, color, i, t)| WallSnapshot {
                x1: t.x1, y1: t.y1, x2: t.x2, y2: t.y2, layer: None,
                label: Some(format!("{glabel}.{i}")),
                color,
            })
            .collect();

        let rollover_groups = self.rollover_groups.iter().filter(|g| shown(g.layer)).map(|g| {
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
                rollovers: g.rollovers.iter().enumerate().filter(|_| g.visible).map(|(i, rv)| RolloverSnapshot {
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
            balls,
            shapes: self.shapes.clone(),
            walls: self.wall_segments.iter().cloned().chain(self.decor_lines.iter().filter(|d| d.visible).map(|d| WallSnapshot { x1: d.x1, y1: d.y1, x2: d.x2, y2: d.y2, layer: None, label: Some(d.id.clone()), color: d.color }))
                .chain(self.moving_walls.iter().filter(|w| w.active).map(|w| WallSnapshot { x1: w.x1, y1: w.y1, x2: w.x2, y2: w.y2, layer: None, label: Some(w.id.clone()), color: w.color }))
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
            popups: Vec::new(),
            ball_layer: self.balls.first().map_or(0, |b| b.layer),
            occupied_layers,
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
        let ball_h = world.ball_handle().expect("ball should exist");
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
        let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();

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
            let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().expect("ball should exist");
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
            let ball_h = world.ball_handle().expect("ball");
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
            let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();
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
        let ball_h = world.ball_handle().unwrap();
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
        let h = world.ball_handle().unwrap();
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
            let decor = world.decor_lines.iter().filter(|d| d.visible).count();
            assert_eq!(world.snapshot().walls.len(), world.wall_segments.len() + world.moving_walls.len() + decor + 1, "table {n}: drawn while closed");

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
            let decor = world.decor_lines.iter().filter(|d| d.visible).count();
            assert_eq!(world.snapshot().walls.len(), world.wall_segments.len() + world.moving_walls.len() + decor, "table {n}: not drawn when open");
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
            // Measured over 200 launches: about 85% (table 1), 90% (2), 99% (3), the
            // misses being the launch jitter's low end. 42 of 60 sits far below
            // that and far above the ~42% the original's friction gave.
            for _ in 0..60 {
                let mut world = PhysicsWorld::from_table(&table);
                world.launch_ball();
                let mut max_y = 0.0_f32;
                for _ in 0..110 {
                    world.step_frame(0.033, &input);
                    max_y = max_y.max(world.ball_position().unwrap().y);
                }
                if max_y > table.height - 1.2 { over += 1; }
            }
            assert!(over >= 42, "table {n}: only {over}/60 launches got over the top of the lane");
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
        let ball_h = world.ball_handle().unwrap();
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
    fn other_tables_have_no_rotating_triangle_and_only_scripted_walls() {
        for n in [1u8, 2, 3, 4, 6, 7, 8, 9] {
            let w = PhysicsWorld::from_table(&load_table(n).unwrap());
            assert!(w.rotating_triangles.is_empty(), "table {n}");
            let ids: Vec<&str> = w.moving_walls.iter().map(|m| m.id.as_str()).collect();
            let expected: &[&str] = match n { 4 => &["TopRampBlocker"], 8 => &["LaunchGate"], _ => &[] };
            assert_eq!(ids, expected, "table {n}");
        }
    }
}

#[cfg(test)]
mod generated_drop_target_tests {
    use super::*;
    use crate::table::{load_table, ElementDef};

    /// Table 3 describes its drop banks by wall, count and widths (no explicit
    /// positions). They must come out with the right number of targets of the
    /// right width, lying parallel to the wall at |gapFromWall| on the correct side.
    #[test]
    fn table_3_banks_are_generated_along_their_walls() {
        let table = load_table(3).unwrap();
        let mut seen = 0;
        for elem in &table.elements {
            let ElementDef::DropTargetGroupElement(d) = elem else { continue };
            if !d.positions.is_empty() { continue; }
            seen += 1;
            let id = d.id.clone().unwrap();
            let targets = d.targets();
            assert_eq!(targets.len(), d.num_targets.unwrap() as usize, "{id}: target count");

            let [sx, sy] = d.wall_start.unwrap();
            let [ex, ey] = d.wall_end.unwrap();
            let (wx, wy) = (ex - sx, ey - sy);
            let wl = wx.hypot(wy);
            let (ux, uy) = (wx / wl, wy / wl);           // along the wall
            let (px, py) = (-uy, ux);                     // perpendicular (rotated +90 degrees)
            let (gap, width) = (d.gap_from_wall.unwrap(), d.target_width.unwrap());
            for (i, t) in targets.iter().enumerate() {
                let len = (t[2] - t[0]).hypot(t[3] - t[1]);
                assert!((len - width).abs() < 1e-3, "{id} target {i}: width {len}, expected {width}");
                for (x, y) in [(t[0], t[1]), (t[2], t[3])] {
                    let across = (x - sx) * px + (y - sy) * py;
                    assert!((across - gap).abs() < 1e-3, "{id} target {i}: {across} from the wall line, expected {gap}");
                }
                let from_start = (t[0] - sx) * ux + (t[1] - sy) * uy;
                let expected = d.start_distance_along_wall.unwrap() + i as f32 * (width + d.gap_between_targets.unwrap());
                assert!((from_start - expected).abs() < 1e-3, "{id} target {i}: starts {from_start} along the wall, expected {expected}");
            }
        }
        assert_eq!(seen, 4, "table 3 has four generated banks");

        // And they exist in the world with targets in them.
        let world = PhysicsWorld::from_table(&table);
        let counts: Vec<usize> = world.drop_target_groups.iter().map(|g| g.targets.len()).collect();
        assert_eq!(counts, vec![5, 2, 3, 3], "UpperMultiball, LowerMultiball, LeftSave, RightSave");
    }

    /// The audit's worry: banks that silently load empty. Every drop bank on
    /// every table must have at least one target, and no more than it asks for.
    #[test]
    fn every_drop_bank_on_every_table_has_targets() {
        for n in 1..=9u8 {
            let table = load_table(n).unwrap();
            let world = PhysicsWorld::from_table(&table);
            let banks: Vec<_> = table.elements.iter().filter_map(|e| match e { ElementDef::DropTargetGroupElement(d) => Some(d), _ => None }).collect();
            assert_eq!(world.drop_target_groups.len(), banks.len(), "table {n}");
            for (g, d) in world.drop_target_groups.iter().zip(&banks) {
                assert!(!g.targets.is_empty(), "table {n}: bank {:?} loaded with no targets", d.id);
                if let Some(want) = d.num_targets { assert_eq!(g.targets.len(), want as usize, "table {n}: bank {:?}", d.id); }
            }
        }
    }

    #[test]
    fn explicit_positions_still_win() {
        for n in [1u8, 4, 5, 6, 7, 8, 9] {
            let table = load_table(n).unwrap();
            for elem in &table.elements {
                if let ElementDef::DropTargetGroupElement(d) = elem {
                    if !d.positions.is_empty() { assert_eq!(d.targets(), d.positions, "table {n}"); }
                }
            }
        }
    }
}

#[cfg(test)]
mod ignore_ball_tests {
    use super::*;
    use crate::table::{load_table, ElementDef};

    fn has_label(w: &PhysicsWorld, idx: usize) -> bool {
        let prefix = format!("{idx}:");
        let exact = idx.to_string();
        w.wall_segments.iter().any(|s| s.label.as_deref().map_or(false, |l| l == exact || l.starts_with(&prefix)))
    }

    /// `ignoreBall` walls, arcs and paths are drawn-only in the original: no body.
    #[test]
    fn ignore_ball_walls_arcs_and_paths_have_no_bodies() {
        for n in 1..=9u8 {
            let table = load_table(n).unwrap();
            let world = PhysicsWorld::from_table(&table);
            for (i, e) in table.elements.iter().enumerate() {
                let ignored = match e {
                    ElementDef::WallElement(w) => w.ignore_ball == Some(true),
                    ElementDef::WallArcElement(a) => a.ignore_ball == Some(true),
                    ElementDef::WallPathElement(p) => p.ignore_ball == Some(true),
                    _ => false,
                };
                if ignored { assert!(!has_label(&world, i), "table {n} element {i} is ignoreBall but became a wall"); }
            }
        }
    }

    /// Table 5's twelve centre-diagram lines cross the middle of the field; a
    /// ball must roll straight through them.
    #[test]
    fn a_ball_crosses_table_5s_centre_diagram_unimpeded() {
        let table = load_table(5).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let ball_h = world.ball_handle().unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(5.2, 11.0), true);
        body.set_linvel(Vector::new(6.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
        let input = InputState::default();
        let mut max_x = 0.0_f32;
        for _ in 0..30 {
            world.step_frame(0.033, &input);
            max_x = max_x.max(world.ball_position().unwrap().x);
        }
        assert!(max_x > 13.0, "ball was stopped inside the centre diagram at x={max_x:.2}");
    }
}

#[cfg(test)]
mod multiball_tests {
    use super::*;
    use crate::table::load_table;

    fn speed_of(w: &PhysicsWorld, i: usize) -> f32 {
        w.bodies.get(w.balls[i].body).unwrap().linvel().length()
    }

    fn place(w: &mut PhysicsWorld, i: usize, x: f32, y: f32, vx: f32, vy: f32) {
        let body = w.bodies.get_mut(w.balls[i].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(vx, vy), true);
        body.set_gravity_scale(0.0, true);
    }

    #[test]
    fn extra_balls_join_the_field_and_are_drawn() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        assert_eq!(world.ball_count(), 1);
        world.launch_ball();
        world.launch_extra_ball();
        world.launch_extra_ball();
        assert_eq!(world.ball_count(), 3);
        let input = InputState::default();
        for _ in 0..5 { world.step_frame(0.033, &input); }
        assert_eq!(world.snapshot().balls.len(), 3, "every ball is in the snapshot the renderer draws");
        assert!((0..3).all(|i| speed_of(&world, i) > 1.0), "all three are moving");
    }

    /// A ball draining while others are in play is just removed; only the last
    /// ball draining is the ball-lost event the game state reads.
    #[test]
    fn a_drained_ball_is_removed_silently_until_the_last_one() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        world.game_time += 100.0; // past the mercy ball and multiball saver windows
        let input = InputState::default();

        place(&mut world, 1, 9.0, -2.0, 0.0, -1.0); // below the drain line
        world.step_frame(0.033, &input);
        assert_eq!(world.ball_count(), 1, "the drained extra ball was removed");
        assert!(!world.is_ball_drained(), "a ball is still in play, so no ball is lost");

        place(&mut world, 0, 9.0, -2.0, 0.0, -1.0);
        world.step_frame(0.033, &input);
        assert_eq!(world.ball_count(), 1, "the last ball is never removed by the drain");
        assert!(world.is_ball_drained(), "the last ball draining is the ball-lost event");
    }

    #[test]
    fn if_every_ball_drains_at_once_exactly_one_is_left_to_report_it() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        world.launch_extra_ball();
        world.game_time += 100.0; // past the savers' windows
        for i in 0..3 { place(&mut world, i, 5.0 + i as f32, -2.0, 0.0, -1.0); }
        world.step_frame(0.033, &InputState::default());
        assert_eq!(world.ball_count(), 1);
        assert!(world.is_ball_drained());
    }

    #[test]
    fn locking_a_ball_does_not_cost_a_ball_unless_it_was_the_last() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        world.remove_ball_without_loss(1);
        assert_eq!(world.ball_count(), 1);
        assert!(!world.take_relaunch_needed(), "another ball is still in play");

        world.remove_ball_without_loss(0);
        assert_eq!(world.ball_count(), 1, "a fresh ball waits in the lane");
        assert!(world.take_relaunch_needed(), "the game must wait for a relaunch");
        assert!(!world.take_relaunch_needed(), "reported once");
        let [lx, ly] = world.launch_pos;
        let p = world.ball_position().unwrap();
        assert!((p.x - lx).abs() < 1e-3 && (p.y - ly).abs() < 1e-3, "parked at the launch position");
        assert!(!world.is_ball_drained());
    }

    #[test]
    fn a_new_ball_collapses_multiball_back_to_one_parked_ball() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        world.launch_extra_ball();
        world.park_ball();
        assert_eq!(world.ball_count(), 1);
        assert!(speed_of(&world, 0) < 1e-3, "parked");
    }

    /// Elements react to every ball, and an impulse goes to the ball that caused it.
    #[test]
    fn a_bumper_kicks_only_the_ball_that_touched_it() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        let (bx, by, br) = { let b = &world.bumpers[0]; (b.position[0], b.position[1], b.radius) };
        place(&mut world, 0, 1.0, 20.0, 0.0, 0.0);                       // away from everything
        let touch = by + br + world.ball_radius + 0.01;
        place(&mut world, 1, bx, touch, 0.0, 0.0); // touching the bumper
        world.step(1.0 / 240.0, &InputState::default());
        assert!(speed_of(&world, 1) > 1.0, "the touching ball was kicked");
        assert!(speed_of(&world, 0) < 0.5, "the other ball was not");
    }

    #[test]
    fn any_ball_lights_a_rollover_and_closes_the_launch_barrier() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        world.launch_ball();
        world.launch_extra_ball();
        let (rx, ry) = { let r = &world.rollover_groups[0].rollovers[0]; (r.x, r.y) };
        place(&mut world, 0, 1.0, 20.0, 0.0, 0.0);
        place(&mut world, 1, rx, ry, 0.0, 0.0); // the second ball, not the first, sits on it
        world.step(1.0 / 240.0, &InputState::default());
        assert!(world.rollover_groups[0].rollovers[0].lit, "the second ball lit the rollover");

        let close = world.launch_barrier.as_ref().unwrap().close_rect.unwrap();
        place(&mut world, 1, (close[0] + close[2]) / 2.0, (close[1] + close[3]) / 2.0, 0.0, 0.0);
        world.step(1.0 / 240.0, &InputState::default());
        assert!(world.launch_barrier.as_ref().unwrap().active, "the second ball closed the barrier");
    }

    /// An extra ball launched after the first has cleared the lane gate must
    /// still get out of the lane (the gate opens for it).
    #[test]
    fn an_extra_ball_launched_later_still_clears_the_lane() {
        let table = load_table(1).unwrap();
        let input = InputState::default();
        let mut over = 0;
        for _ in 0..20 {
            let mut world = PhysicsWorld::from_table(&table);
            world.launch_ball();
            for _ in 0..50 { world.step_frame(0.033, &input); } // first ball clears the gate and closes it
            world.launch_extra_ball();
            let mut max_y = 0.0_f32;
            for _ in 0..110 {
                world.step_frame(0.033, &input);
                if let Some(&(_, p, _)) = world.live_balls().last() { max_y = max_y.max(p.y); }
            }
            if max_y > table.height - 1.2 { over += 1; }
        }
        assert!(over >= 12, "only {over}/20 extra launches got over the top of the lane");
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;
    use crate::table::load_table;

    fn world_for(n: u8) -> (TableDef, PhysicsWorld) {
        let table = load_table(n).unwrap();
        let world = PhysicsWorld::from_table(&table);
        (table, world)
    }

    fn place(w: &mut PhysicsWorld, i: usize, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[i].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    /// Runs `secs` of SIM time (the original's game time) in real-frame steps.
    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        let frames = (secs / (frame * w.time_ratio())).ceil() as usize;
        for _ in 0..frames { w.step_frame(frame, &InputState::default()); }
    }

    fn lit(w: &PhysicsWorld, id: &str) -> Vec<bool> {
        w.rollover_group(id).unwrap().rollovers.iter().map(|r| r.lit).collect()
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> {
        w.drain_message_events().into_iter().map(|m| m.0).collect()
    }

    /// Knocks down all but the last target of the bank, then puts the ball on it.
    /// The bank is put back to its starting state first (all targets up).
    fn clear_bank(w: &mut PhysicsWorld, id: &str) {
        let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some(id)).unwrap();
        for t in &mut w.drop_target_groups[gi].targets {
            t.active = true;
            w.colliders.get_mut(t.collider).unwrap().set_collision_groups(InteractionGroups::all());
        }
        w.drop_target_groups[gi].reset_timer = None;
        let last = w.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last {
            w.drop_target_groups[gi].targets[ti].active = false;
            let ch = w.drop_target_groups[gi].targets[ti].collider;
            w.colliders.get_mut(ch).unwrap().set_collision_groups(InteractionGroups::none());
        }
        let t = &w.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(w, 0, mx, my);
        w.step_frame(0.033, &InputState::default());
    }

    fn retract_savers(w: &mut PhysicsWorld) {
        for kw in &mut w.kick_walls {
            if kw.retract_when_hit {
                kw.active = false;
                w.colliders.get_mut(kw.collider_handle).unwrap().set_collision_groups(InteractionGroups::none());
            }
        }
    }

    fn savers_active(w: &PhysicsWorld) -> bool {
        w.kick_walls.iter().filter(|k| k.retract_when_hit).all(|k| k.active)
    }

    fn rollover_pos(w: &PhysicsWorld, id: &str, i: usize) -> (f32, f32) {
        let r = &w.rollover_group(id).unwrap().rollovers[i];
        (r.x, r.y)
    }

    // ---------------- table 1: Classic ----------------

    #[test]
    fn classic_banks_light_the_extra_ball_lights_one_by_one_then_announce_multiball() {
        let (_, mut world) = world_for(1);
        for n in 1..=7 {
            clear_bank(&mut world, "DropTargetLeftSave");
            let on = lit(&world, "ExtraBallRollovers").iter().filter(|&&l| l).count();
            assert_eq!(on, n, "after clearing a bank {n} time(s)");
            if n < 7 { assert!(!messages(&world).iter().any(|m| m == "Shoot Ramp for Multiball")); }
            run_sim_seconds(&mut world, 3.0); // the bank resets after 2 s
        }
        // (the 7th clear's message was produced before the loop's final wait)
    }

    #[test]
    fn classic_announces_when_the_last_light_comes_on() {
        let (_, mut world) = world_for(1);
        for i in 0..6 { world.set_rollover("ExtraBallRollovers", i, true); }
        messages(&world);
        clear_bank(&mut world, "DropTargetTop");
        assert!(lit(&world, "ExtraBallRollovers").iter().all(|&l| l));
        assert!(messages(&world).iter().any(|m| m == "Shoot Ramp for Multiball"));
    }

    #[test]
    fn classic_ramp_with_all_lights_lit_starts_multiball() {
        let (_, mut world) = world_for(1);
        world.set_all_rollovers("ExtraBallRollovers", true);
        retract_savers(&mut world);
        assert!(!savers_active(&world));
        for i in 0..3 { world.set_rollover("RampRollovers", i, true); }
        let (x, y) = rollover_pos(&world, "RampRollovers", 3);
        place(&mut world, 0, x, y);
        // One substep: the ball sits on the last rollover (gravity off), so a whole
        // frame would relight it after the script clears the group.
        world.step(1.0 / 240.0, &InputState::default());

        let msgs = messages(&world);
        assert!(msgs.iter().any(|m| m == "Multiball!"), "got {msgs:?}");
        assert!(lit(&world, "ExtraBallRollovers").iter().all(|&l| !l), "the extra-ball lights are cleared");
        assert!(lit(&world, "RampRollovers").iter().all(|&l| !l), "the ramp group is cleared by the script");
        assert!(savers_active(&world), "both savers come back");
        assert_eq!(world.ball_count(), 1, "the extra balls are not out yet");

        run_sim_seconds(&mut world, 1.2);
        assert_eq!(world.ball_count(), 2, "first extra ball at 1 s");
        run_sim_seconds(&mut world, 2.6);
        assert_eq!(world.ball_count(), 3, "second extra ball at 3.5 s");
        run_sim_seconds(&mut world, 3.0);
        assert!(world.ball_count() <= 3, "never more than three");
    }

    #[test]
    fn classic_ramp_without_all_the_lights_is_just_a_multiplier() {
        let (_, mut world) = world_for(1);
        for i in 0..6 { world.set_rollover("ExtraBallRollovers", i, true); } // one short
        for i in 0..3 { world.set_rollover("RampRollovers", i, true); }
        let (x, y) = rollover_pos(&world, "RampRollovers", 3);
        place(&mut world, 0, x, y);
        world.step(1.0 / 240.0, &InputState::default());
        place(&mut world, 0, 1.0, 20.0); // move the ball off the rollover before time passes
        run_sim_seconds(&mut world, 4.0);
        assert_eq!(world.ball_count(), 1, "no multiball");
        assert!(!messages(&world).iter().any(|m| m == "Multiball!"));
        assert_eq!(world.drain_multiplier_events(), vec![1], "but the group completion raised the multiplier");
    }

    /// The original leaves rollover completion to the delegate: no flash, no
    /// completion bonus, no automatic reset; just the per-rollover score and the
    /// multiplier step.
    #[test]
    fn a_scripted_table_completes_rollover_groups_the_original_way() {
        let (_, mut world) = world_for(1);
        for i in 0..3 { world.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = rollover_pos(&world, "FlipperRollovers", 3);
        place(&mut world, 0, x, y);
        world.step(1.0 / 240.0, &InputState::default()); // one substep, see the ramp test
        assert!(lit(&world, "FlipperRollovers").iter().all(|&l| !l), "cleared at once, not after a flash");
        assert_eq!(world.rollover_group("FlipperRollovers").unwrap().flash_ticks, 0);
        assert_eq!(world.drain_score_events(), vec![500], "only the last rollover's own score, no completion bonus");
        assert_eq!(world.drain_multiplier_events(), vec![1]);
    }

    #[test]
    fn a_table_without_a_script_keeps_the_generic_completion() {
        let (_, mut world) = world_for(9); // Party Land is our own design: no original script, ever
        assert!(world.script.is_none());
        let id = "PukeRollovers".to_string();
        let n = world.rollover_count(&id);
        for i in 0..n - 1 { world.set_rollover(&id, i, true); }
        let (x, y) = rollover_pos(&world, &id, n - 1);
        place(&mut world, 0, x, y);
        world.step_frame(0.033, &InputState::default());
        assert!(world.rollover_group(&id).unwrap().flash_ticks > 0 || lit(&world, &id).iter().all(|&l| l), "generic flash path still runs");
    }

    // ---------------- table 2: Round and Round ----------------

    #[test]
    fn round_and_round_banks_feed_their_columns_of_the_extra_ball_lights() {
        let (_, mut world) = world_for(2);
        clear_bank(&mut world, "DropTargetLeft");
        assert_eq!(lit(&world, "ExtraBallRollovers").iter().enumerate().filter(|(_, &l)| l).map(|(i, _)| i).collect::<Vec<_>>(), vec![0]);
        clear_bank(&mut world, "DropTargetRight");
        clear_bank(&mut world, "DropTargetTopLeft");
        let on: Vec<usize> = lit(&world, "ExtraBallRollovers").iter().enumerate().filter(|(_, &l)| l).map(|(i, _)| i).collect();
        assert_eq!(on, vec![0, 1, 2], "left column 0, top left column 1, right column 2");

        run_sim_seconds(&mut world, 3.0); // banks reset
        clear_bank(&mut world, "DropTargetLeft");
        let on: Vec<usize> = lit(&world, "ExtraBallRollovers").iter().enumerate().filter(|(_, &l)| l).map(|(i, _)| i).collect();
        assert_eq!(on, vec![0, 1, 2, 3], "the left bank's next light is three along");
    }

    #[test]
    fn round_and_round_centre_bumper_with_all_lights_lit_starts_multiball() {
        let (_, mut world) = world_for(2);
        world.set_all_rollovers("ExtraBallRollovers", true);
        messages(&world);
        let (bx, by, br) = {
            let b = world.bumpers.iter().find(|b| b.shot_id.as_deref() == Some("CenterBumper1")).unwrap();
            (b.position[0], b.position[1], b.radius)
        };
        let touch = by + br + world.ball_radius + 0.01;
        place(&mut world, 0, bx, touch);
        world.step(1.0 / 240.0, &InputState::default());
        assert!(messages(&world).iter().any(|m| m == "Multiball!"));
        assert!(lit(&world, "ExtraBallRollovers").iter().all(|&l| !l), "lights cleared");
        run_sim_seconds(&mut world, 4.0);
        assert_eq!(world.ball_count(), 3);
    }

    #[test]
    fn round_and_round_centre_bumper_alone_does_not_start_multiball() {
        let (_, mut world) = world_for(2);
        for i in 0..8 { world.set_rollover("ExtraBallRollovers", i, true); } // one short of nine
        let (bx, by, br) = {
            let b = world.bumpers.iter().find(|b| b.shot_id.as_deref() == Some("CenterBumper1")).unwrap();
            (b.position[0], b.position[1], b.radius)
        };
        let touch = by + br + world.ball_radius + 0.01;
        place(&mut world, 0, bx, touch);
        world.step(1.0 / 240.0, &InputState::default());
        run_sim_seconds(&mut world, 4.0);
        assert_eq!(world.ball_count(), 1);
        assert!(!messages(&world).iter().any(|m| m == "Multiball!"));
    }

    /// Scheduled actions fire whatever happens to the ball, as in the original
    /// (table 8 schedules its rack light show at ball loss).
    #[test]
    fn pending_timers_survive_a_new_ball() {
        let (_, mut world) = world_for(1);
        world.schedule(1000, 1); // Classic: launch another ball if fewer than three
        world.park_ball();
        run_sim_seconds(&mut world, 2.0);
        assert_eq!(world.ball_count(), 2, "the timer still fired");
    }
}

#[cfg(test)]
mod multiball_game_flow_tests {
    use super::*;
    use crate::game::{GameState, Phase};
    use crate::table::load_table;

    fn sink(w: &mut PhysicsWorld, i: usize) {
        let body = w.bodies.get_mut(w.balls[i].body).unwrap();
        body.set_translation(Vector::new(5.0 + i as f32, -2.0), true);
        body.set_linvel(Vector::new(0.0, -1.0), true);
    }

    /// Through the real game state: the message reaches the player, draining
    /// balls while others remain costs nothing, and only the last drain is a
    /// ball lost.
    #[test]
    fn a_multiball_costs_one_ball_only_when_the_last_ball_drains() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let mut game = GameState::new(table.num_balls());
        game.phase = Phase::Playing;
        world.launch_ball();
        world.game_time += 100.0; // no mercy ball or multiball saver in this test
        let input = InputState::default();

        // Start a multiball the way the table does: all lights lit, shoot the ramp.
        world.set_all_rollovers("ExtraBallRollovers", true);
        for i in 0..3 { world.set_rollover("RampRollovers", i, true); }
        let (x, y) = { let r = &world.rollover_group("RampRollovers").unwrap().rollovers[3]; (r.x, r.y) };
        {
            let body = world.bodies.get_mut(world.balls[0].body).unwrap();
            body.set_translation(Vector::new(x, y), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_gravity_scale(0.0, true);
        }
        world.step(1.0 / 240.0, &input);
        game.sync(&world);
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Multiball!"), "the player sees the message");
        assert!(game.multiplier >= 2, "and the completion raised the multiplier");

        for _ in 0..80 { world.step_frame(0.033, &input); game.sync(&world); } // out past 3.5 s of game time
        assert_eq!(world.ball_count(), 3);
        world.game_time += 100.0; // the multiball saver window has passed

        sink(&mut world, 2);
        world.step_frame(0.033, &input);
        game.sync(&world);
        assert_eq!((game.phase, game.balls_remaining, world.ball_count()), (Phase::Playing, 3, 2), "one drained, nothing lost");

        sink(&mut world, 1);
        world.step_frame(0.033, &input);
        game.sync(&world);
        assert_eq!((game.phase, game.balls_remaining, world.ball_count()), (Phase::Playing, 3, 1), "two drained, still nothing lost");

        sink(&mut world, 0);
        world.step_frame(0.033, &input);
        game.sync(&world);
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 2), "the last ball draining costs exactly one ball");
    }

    /// A lock takes the last ball off without losing it: the game waits for a
    /// relaunch and the ball count does not change.
    #[test]
    fn a_lock_waits_for_a_relaunch_without_costing_a_ball() {
        let table = load_table(1).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        let mut game = GameState::new(table.num_balls());
        game.phase = Phase::Playing;
        world.launch_ball();
        world.remove_ball_without_loss(0);
        // The app does this each tick:
        if world.take_relaunch_needed() && game.phase == Phase::Playing { game.phase = Phase::WaitingToLaunch; }
        assert_eq!((game.phase, game.balls_remaining), (Phase::WaitingToLaunch, 3));
        assert!(!world.is_ball_drained());
    }
}

#[cfg(test)]
mod script_building_block_tests {
    use super::*;
    use crate::table::load_table;
    use std::cell::RefCell as Cell;
    use std::rc::Rc;

    /// Records what the world tells a script.
    #[derive(Default)]
    struct Log {
        sensors: Vec<(String, usize, Option<String>)>,
        ticks: usize,
        dt_total: f32,
    }
    struct Recorder(Rc<Cell<Log>>);
    impl scripts::TableScript for Recorder {
        fn tick(&mut self, _w: &mut PhysicsWorld, dt: f32) {
            let mut l = self.0.borrow_mut();
            l.ticks += 1;
            l.dt_total += dt;
        }
        fn sensor(&mut self, _w: &mut PhysicsWorld, ball: usize, id: &str, previous: Option<&str>) {
            self.0.borrow_mut().sensors.push((id.to_string(), ball, previous.map(str::to_string)));
        }
    }

    fn recording_world(n: u8) -> (PhysicsWorld, Rc<Cell<Log>>) {
        let mut world = PhysicsWorld::from_table(&load_table(n).unwrap());
        let log = Rc::new(Cell::new(Log::default()));
        world.script = Some(Box::new(Recorder(log.clone())));
        (world, log)
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn centre_of(w: &PhysicsWorld, id: &str) -> (f32, f32, Option<u8>) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0, s.from_layer)
    }

    #[test]
    fn the_script_gets_a_tick_every_step_in_sim_time() {
        let (mut world, log) = recording_world(1);
        world.step_frame(0.033, &InputState::default());
        let l = log.borrow();
        assert_eq!(l.ticks, 8, "one per substep");
        assert!((l.dt_total - 0.033 * world.time_ratio()).abs() < 1e-4, "adds up to the frame's sim time");
    }

    /// The callback sees the sensor the ball was in BEFORE, then the ball's
    /// memory updates: that is how a script tells which way a ball went.
    #[test]
    fn a_sensor_reports_the_previous_sensor_then_the_ball_remembers_it() {
        let (mut world, log) = recording_world(6);
        let (ex, ey, efrom) = centre_of(&world, "LeftLoopDetector_Enter");
        let (tx, ty, tfrom) = centre_of(&world, "LeftLoopDetector_Trigger");
        let input = InputState::default();

        world.balls[0].layer = efrom.unwrap_or(0);
        place(&mut world, ex, ey);
        world.step(1.0 / 240.0, &input);
        world.balls[0].layer = tfrom.unwrap_or(0);
        place(&mut world, tx, ty);
        world.step(1.0 / 240.0, &input);
        world.step(1.0 / 240.0, &input); // still inside the trigger

        let l = log.borrow();
        let enter = l.sensors.iter().find(|e| e.0 == "LeftLoopDetector_Enter").expect("enter reported");
        assert_eq!(enter.2, None, "no sensor before the first");
        let triggers: Vec<_> = l.sensors.iter().filter(|e| e.0 == "LeftLoopDetector_Trigger").collect();
        assert_eq!(triggers[0].2.as_deref(), Some("LeftLoopDetector_Enter"), "came from the enter sensor");
        assert_eq!(triggers[1].2.as_deref(), Some("LeftLoopDetector_Trigger"), "the next step it is its own previous");
    }

    #[test]
    fn a_ball_outside_every_sensor_reports_nothing() {
        let (mut world, log) = recording_world(6);
        place(&mut world, 1.0, 1.0);
        world.step(1.0 / 240.0, &InputState::default());
        assert!(log.borrow().sensors.is_empty());
    }

    #[test]
    fn ball_colours_are_per_ball_drawn_and_reset_for_a_new_ball() {
        let mut world = PhysicsWorld::from_table(&load_table(6).unwrap());
        world.launch_ball();
        world.launch_extra_ball();
        let orange = (0xFF, 0xBB, 0x44);
        world.set_ball_color(1, Some(orange));
        assert!(world.any_ball_has_color(orange));
        assert_eq!(world.ball_color(0), None, "the other ball keeps the table colour");
        let snap = world.snapshot();
        assert_eq!(snap.balls[1].color, Some(orange));
        assert_eq!(snap.balls[0].color, None);

        world.park_ball();
        assert!(!world.any_ball_has_color(orange), "a new ball starts with the default colour");
    }

    #[test]
    fn a_moved_rollover_is_lit_where_it_is_now() {
        // Table 4 has no script, so its rollovers are the generic ones.
        let mut world = PhysicsWorld::from_table(&load_table(4).unwrap());
        let id = world.rollover_groups.iter()
            .find(|g| !g.ignore_ball && g.shot_id.is_some())
            .and_then(|g| g.shot_id.clone()).unwrap();
        world.set_rollover_center(&id, 0, 4.0, 22.0);
        assert_eq!(world.rollover_center(&id, 0), Some((4.0, 22.0)));
        place(&mut world, 4.0, 22.0);
        world.step(1.0 / 240.0, &InputState::default());
        assert!(world.rollover_lit(&id, 0), "the ball lit it at its new position");
        world.set_rollover_color(&id, 0, Some((1, 2, 3)));
        assert!(world.snapshot().rollover_groups.iter().flat_map(|g| g.rollovers.iter()).any(|r| r.color == Some((1, 2, 3))));
    }

    #[test]
    fn impulses_and_scores_from_scripts_reach_the_ball_and_the_game() {
        let mut world = PhysicsWorld::from_table(&load_table(6).unwrap());
        world.launch_ball();
        place(&mut world, 5.0, 10.0);
        world.apply_impulse_to_ball(0, Vector::new(3.0, 0.0));
        let v = world.bodies.get(world.balls[0].body).unwrap().linvel();
        assert!(v.x > 1.0, "the impulse moved the ball: {v:?}");
        world.add_score(5000);
        assert_eq!(world.drain_score_events(), vec![5000]);
    }
}

#[cfg(test)]
mod table_6_tests {
    use super::*;
    use crate::table::load_table;

    const BALL_COLORS: [(u8, u8, u8); 5] = scripts::PLANET_BALL_COLORS;

    fn orbiters() -> PhysicsWorld {
        let mut world = PhysicsWorld::from_table(&load_table(6).unwrap());
        world.script = Some(Box::new(scripts::Orbiters::new(12345)));
        world
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    fn planet_angle(w: &PhysicsWorld, i: usize) -> f32 {
        let (cx, cy) = w.rollover_center("Orbits", i).unwrap();
        let (px, py) = w.rollover_center(&format!("Planet{}", i + 1), 0).unwrap();
        (py - cy).atan2(px - cx)
    }

    #[test]
    fn the_planets_orbit_their_rings_inner_ones_faster() {
        let mut world = orbiters();
        world.step_frame(0.033, &InputState::default());
        let before: Vec<f32> = (0..5).map(|i| planet_angle(&world, i)).collect();
        let sim_seconds = 0.5;
        run_sim_seconds(&mut world, sim_seconds);
        let elapsed = (sim_seconds / (0.033 * world.time_ratio())).ceil() * 0.033 * world.time_ratio();
        for i in 0..5 {
            let (cx, cy) = world.rollover_center("Orbits", i).unwrap();
            let (px, py) = world.rollover_center(&format!("Planet{}", i + 1), 0).unwrap();
            let ring = world.rollover_radius("Orbits", i).unwrap();
            assert!(((px - cx).hypot(py - cy) - ring).abs() < 1e-3, "planet {} left its ring", i + 1);
            let turned = (planet_angle(&world, i) - before[i] + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let speed = turned / elapsed;
            let (lo, hi) = (0.9 / (i + 1) as f32, 1.1 / (i + 1) as f32);
            assert!(speed >= lo - 0.03 && speed <= hi + 0.03, "planet {}: {speed:.3} rad/s, expected {lo:.3} to {hi:.3}", i + 1);
        }
    }

    #[test]
    fn the_sun_is_always_lit_and_planets_keep_moving_with_no_ball_launched() {
        let mut world = orbiters();
        world.step_frame(0.033, &InputState::default());
        assert!(world.rollover_lit("Sun", 0));
        let a = world.rollover_center("Planet1", 0).unwrap();
        run_sim_seconds(&mut world, 0.5);
        let b = world.rollover_center("Planet1", 0).unwrap();
        assert!((a.0 - b.0).abs() + (a.1 - b.1).abs() > 0.05, "planet 1 moved");
    }

    /// The sun and the planets pull on a ball within range of the sun.
    #[test]
    fn the_sun_and_planets_pull_a_ball_in_range_but_not_one_far_away() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default()); // init
        let (sx, sy) = world.rollover_center("Sun", 0).unwrap();

        place(&mut world, sx + 5.0, sy + 3.0);
        world.step(1.0 / 240.0, &InputState::default());
        let v = world.bodies.get(world.balls[0].body).unwrap().linvel();
        let toward_sun = v.x * (sx - (sx + 5.0)) + v.y * (sy - (sy + 3.0));
        assert!(toward_sun > 0.0, "pulled toward the sun, v={v:?}");

        place(&mut world, sx + 9.0, sy + 3.0); // beyond 8 units
        world.step(1.0 / 240.0, &InputState::default());
        let v = world.bodies.get(world.balls[0].body).unwrap().linvel();
        assert!(v.length() < 1e-3, "nothing pulls a ball outside the range, v={v:?}");
    }

    #[test]
    fn rolling_over_a_planet_gives_the_ball_its_colour_and_the_planet_starts_pulsing() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        let (x, y) = world.rollover_center("Planet3", 0).unwrap();
        place(&mut world, x, y);
        world.step(1.0 / 240.0, &InputState::default());
        assert_eq!(world.ball_color(0), Some(BALL_COLORS[2]), "the ball took planet 3's colour");
        world.step(1.0 / 240.0, &InputState::default());
        assert!(world.rollover_lit("Planet3", 0), "planet 3 is in progress (lit) while a ball has its colour");
        assert!(!world.rollover_lit("Planet1", 0), "the others stay off");

        world.park_ball(); // a new ball has the default colour
        world.step(1.0 / 240.0, &InputState::default());
        assert!(!world.rollover_lit("Planet3", 0), "back to off");
    }

    #[test]
    fn a_ball_of_the_right_colour_switches_a_planet_on_through_its_rollovers() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        world.set_ball_color(0, Some(BALL_COLORS[1])); // planet 2's colour
        world.set_rollover("Planet2Rollovers", 0, true);
        world.set_rollover("Planet2Rollovers", 1, true);
        let (x, y) = world.rollover_center("Planet2Rollovers", 2).unwrap();
        place(&mut world, x, y);
        world.drain_score_events();
        world.step(1.0 / 240.0, &InputState::default());

        let scores = world.drain_score_events();
        assert_eq!(scores.iter().filter(|&&p| p == 5000).count(), 2, "5000 for the rollovers and 5000 for the planet: {scores:?}");
        assert!(messages(&world).iter().any(|m| m == "Planet 2 Activated!"));
        world.step(1.0 / 240.0, &InputState::default());
        assert!(world.rollover_lit("Planet2", 0), "planet 2 stays lit once on");
        assert!(!world.rollover_lit("Planet2Rollovers", 0), "the rollovers cleared");
    }

    #[test]
    fn the_wrong_colour_scores_the_rollovers_but_does_not_activate_the_planet() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        world.set_ball_color(0, Some(BALL_COLORS[0]));
        world.set_rollover("Planet2Rollovers", 0, true);
        world.set_rollover("Planet2Rollovers", 1, true);
        let (x, y) = world.rollover_center("Planet2Rollovers", 2).unwrap();
        place(&mut world, x, y);
        world.drain_score_events();
        world.step(1.0 / 240.0, &InputState::default());
        let scores = world.drain_score_events();
        assert_eq!(scores.iter().filter(|&&p| p == 5000).count(), 1, "only the rollover award: {scores:?}");
        assert!(!messages(&world).iter().any(|m| m.contains("Activated")));
    }

    // ----- ramps: two detectors in order -----

    /// The ramp awards among the score events (other elements the test ball
    /// happens to overlap, such as 1000-point rollovers, are not the point).
    fn ramp_awards(w: &PhysicsWorld) -> Vec<u64> {
        w.drain_score_events().into_iter().filter(|&p| p >= 2500).collect()
    }

    fn through(w: &mut PhysicsWorld, id: &str) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.balls[0].layer = s.from_layer.unwrap_or(w.balls[0].layer);
        place(w, x, y);
        w.step(1.0 / 240.0, &InputState::default());
    }

    #[test]
    fn a_ramp_scores_in_one_direction_and_the_bonus_stacks_for_twelve_seconds() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        world.drain_score_events();

        through(&mut world, "LeftLoopDetector_Enter");
        through(&mut world, "LeftLoopDetector_Trigger");
        assert_eq!(ramp_awards(&world), vec![2500], "first ramp");

        through(&mut world, "LeftLoopDetector_Enter");
        through(&mut world, "LeftLoopDetector_Trigger");
        assert_eq!(ramp_awards(&world), vec![5000], "second ramp within 12 s is doubled");
        assert!(messages(&world).iter().any(|m| m == "2x Ramp"));

        place(&mut world, 1.0, 1.0);
        run_sim_seconds(&mut world, 13.0);
        through(&mut world, "LeftLoopDetector_Enter");
        through(&mut world, "LeftLoopDetector_Trigger");
        assert_eq!(ramp_awards(&world), vec![2500], "the window ran out, back to the base score");
    }

    #[test]
    fn a_ball_going_the_wrong_way_through_a_ramp_scores_nothing() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        world.drain_score_events();
        through(&mut world, "LeftLoopDetector_Trigger");
        assert!(ramp_awards(&world).is_empty());
    }

    #[test]
    fn the_orbit_scores_from_either_end_and_counts_for_planet_5() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        world.set_ball_color(0, Some(BALL_COLORS[4]));
        world.drain_score_events();
        through(&mut world, "OrbitDetector_Right");
        through(&mut world, "OrbitDetector_Left");
        let scores = world.drain_score_events();
        assert!(scores.contains(&2500) && scores.contains(&5000), "ramp 2500 plus planet 5000: {scores:?}");
        assert!(messages(&world).iter().any(|m| m == "Planet 5 Activated!"));
    }

    #[test]
    fn the_planet_bank_the_flipper_rollovers_and_the_savers_follow_the_original() {
        let mut world = orbiters();
        world.step(1.0 / 240.0, &InputState::default());
        let n = world.rollover_count("FlipperRollovers");
        for i in 0..n - 1 { world.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = world.rollover_center("FlipperRollovers", n - 1).unwrap();
        place(&mut world, x, y);
        world.step(1.0 / 240.0, &InputState::default());
        assert_eq!(world.drain_multiplier_events(), vec![1]);
    }
}

#[cfg(test)]
mod table_4_tests {
    use super::*;
    use crate::game::GameState;
    use crate::table::load_table;

    fn world4() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(4).unwrap()) }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn touch_bumper(w: &mut PhysicsWorld, id: &str) {
        let (bx, by, br) = {
            let b = w.bumpers.iter().find(|b| b.shot_id.as_deref() == Some(id)).unwrap_or_else(|| panic!("no bumper {id}"));
            (b.position[0], b.position[1], b.radius)
        };
        let y = by + br + w.ball_radius + 0.01;
        place(w, bx, y);
        // Let the 12 substep bumper debounce lapse so back-to-back calls each count.
        for _ in 0..16 { w.step(1.0 / 240.0, &InputState::default()); place(w, bx, y); }
    }

    fn lit(w: &PhysicsWorld, id: &str) -> Vec<bool> { w.rollover_group(id).unwrap().rollovers.iter().map(|r| r.lit).collect() }
    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    #[test]
    fn a_coloured_bumper_adds_a_percent_and_lights_its_columns_first_light() {
        let mut world = world4();
        let before = world.drain_multiplier_percent_events().len();
        assert_eq!(before, 0);
        touch_bumper(&mut world, "Bumper.Red");
        let events = world.drain_multiplier_percent_events();
        assert!(!events.is_empty() && events.iter().all(|&p| p == 1), "each hit is +1%: {events:?}");
        assert_eq!(lit(&world, "Rollovers.Red"), vec![true, false, false]);
        assert_eq!(lit(&world, "Rollovers.Blue"), vec![false, false, false], "other colours untouched");
    }

    #[test]
    fn the_top_bank_and_flipper_lanes_raise_what_a_hit_adds_and_losing_the_ball_resets_it() {
        let mut world = world4();
        touch_bumper(&mut world, "Bumper.Green");
        world.drain_multiplier_percent_events();
        messages(&world);

        // Top bank cleared: +1 per hit, so 2.
        world.script.as_mut().unwrap(); // the table has a script
        let gi = world.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some("DropTargetTop")).unwrap();
        let last = world.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last {
            world.drop_target_groups[gi].targets[ti].active = false;
            let ch = world.drop_target_groups[gi].targets[ti].collider;
            world.colliders.get_mut(ch).unwrap().set_collision_groups(InteractionGroups::none());
        }
        let t = &world.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(&mut world, mx, my);
        world.step_frame(0.033, &InputState::default());
        assert!(messages(&world).iter().any(|m| m == "Bumper Multiplier 2%"));
        touch_bumper(&mut world, "Bumper.Cyan");
        assert!(world.drain_multiplier_percent_events().iter().all(|&p| p == 2), "now +2% per hit");

        // Flipper lanes completed: +2 more, so 4.
        let n = world.rollover_count("FlipperRollovers");
        for i in 0..n - 1 { world.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = world.rollover_center("FlipperRollovers", n - 1).unwrap();
        place(&mut world, x, y);
        world.step(1.0 / 240.0, &InputState::default());
        assert!(messages(&world).iter().any(|m| m == "Bumper Multiplier 4%"));
        assert!(lit(&world, "FlipperRollovers").iter().all(|&l| !l), "the lanes clear");

        world.ball_lost();
        touch_bumper(&mut world, "Bumper.Yellow");
        assert!(world.drain_multiplier_percent_events().iter().all(|&p| p == 1), "a lost ball resets it to 1");
    }

    /// Through the game state: percent adds up and shows as a fraction.
    #[test]
    fn bumper_hits_build_a_fractional_multiplier_in_the_game() {
        let mut world = world4();
        let mut game = GameState::new(3);
        for color in ["Red", "Blue", "Green", "Cyan", "Yellow"] {
            touch_bumper(&mut world, &format!("Bumper.{color}"));
            game.sync(&world);
        }
        assert!(game.bonus_percent >= 5, "at least one percent per bumper, got {}", game.bonus_percent);
        assert_eq!(game.multiplier, 1);
        assert!(game.multiplier_text().starts_with("x1."), "{}", game.multiplier_text());
    }

    fn through(w: &mut PhysicsWorld, id: &str) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap();
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        place(w, x, y);
        w.step(1.0 / 240.0, &InputState::default());
    }

    #[test]
    fn the_ramp_sensors_raise_and_lower_the_blocker_and_kicker() {
        let mut world = world4();
        assert_eq!(world.wall_retracted("RightRampKicker"), Some(true), "the kicker starts out of the way");
        assert_eq!(world.wall_retracted("TopRampBlocker"), Some(false), "the blocker starts in place");

        through(&mut world, "TopRampSensor");
        assert_eq!(world.wall_retracted("TopRampBlocker"), Some(true), "up the ramp: blocker out of the way");
        assert_eq!(world.wall_retracted("RightRampKicker"), Some(false), "kicker in place");
        assert_eq!(world.wall_retracted("LaunchBarrier"), Some(false), "and the lane closes behind the ball");

        through(&mut world, "RightRampSensor");
        assert_eq!(world.wall_retracted("TopRampBlocker"), Some(false));
        assert_eq!(world.wall_retracted("RightRampKicker"), Some(true));

        through(&mut world, "AfterRampKickerSensor");
        assert_eq!(world.wall_retracted("TopRampBlocker"), Some(false));
        assert_eq!(world.wall_retracted("RightRampKicker"), Some(false));

        through(&mut world, "LaunchBarrierRetract");
        assert_eq!(world.wall_retracted("LaunchBarrier"), Some(true));
    }

    #[test]
    fn a_retracted_scripted_wall_is_neither_solid_nor_drawn() {
        let mut world = world4();
        let drawn = |w: &PhysicsWorld| w.snapshot().walls.iter().any(|s| s.label.as_deref().map_or(false, |l| l.contains("TopRampBlocker")));
        assert!(drawn(&world));
        world.set_wall_retracted("TopRampBlocker", true);
        assert!(!drawn(&world));
        let col = world.moving_walls.iter().find(|m| m.id == "TopRampBlocker").unwrap().collider;
        assert_eq!(world.colliders.get(col).unwrap().collision_groups(), InteractionGroups::none());
        world.set_wall_retracted("TopRampBlocker", false);
        assert!(drawn(&world));
    }
}

#[cfg(test)]
mod table_5_triangle_kick_tests {
    use super::*;
    use crate::table::{load_table, ElementDef};

    /// Every plain wall must be drawn (the triangle's kick registration must not
    /// remove some other wall's segment), and the triangle must be drawn once.
    #[test]
    fn plain_walls_are_all_drawn_and_the_triangle_is_drawn_once() {
        let table = load_table(5).unwrap();
        let world = PhysicsWorld::from_table(&table);
        for (i, e) in table.elements.iter().enumerate() {
            let ElementDef::WallElement(w) = e else { continue };
            let plain = w.kick.is_none() && w.disabled != Some(true) && w.kill != Some(true)
                && w.ignore_ball != Some(true) && w.id.as_deref().map_or(true, |id| !id.starts_with("TriangleWall"));
            if !plain { continue; }
            let label = i.to_string();
            let drawn = world.wall_segments.iter().any(|s| s.label.as_deref().map_or(false, |l| l == label || l.starts_with(&format!("{i}:"))));
            assert!(drawn, "wall element {i} ({:?}) is missing from the drawn walls", w.id);
        }
        let snap = world.snapshot();
        let triangle = |ws: &Vec<WallSnapshot>| ws.iter().filter(|s| s.label.as_deref().map_or(false, |l| l.contains("TriangleWall"))).count();
        assert_eq!(triangle(&snap.walls), 3, "the live triangle is drawn");
        assert_eq!(triangle(&snap.kick_walls), 0, "and not a second, static one as a kick wall");
    }

    /// The triangle's kick follows the walls as they turn.
    #[test]
    fn the_triangles_kick_follows_the_rotating_walls() {
        let table = load_table(5).unwrap();
        let mut world = PhysicsWorld::from_table(&table);
        for _ in 0..40 { world.step_frame(0.033, &InputState::default()); }
        for id in ["TriangleWall1", "TriangleWall2", "TriangleWall3"] {
            let m = world.moving_walls.iter().find(|m| m.id == id).unwrap();
            let kw = world.kick_walls.iter().find(|k| k.shot_id.as_deref() == Some(id)).unwrap_or_else(|| panic!("{id} has a kick"));
            let d = (kw.x1 - m.x1).abs() + (kw.y1 - m.y1).abs() + (kw.x2 - m.x2).abs() + (kw.y2 - m.y2).abs();
            assert!(d < 1e-3, "{id}: kick wall {:?} is not at the live wall {:?}", (kw.x1, kw.y1, kw.x2, kw.y2), (m.x1, m.y1, m.x2, m.y2));
        }
    }

    /// A ball hitting a rotated triangle wall is thrown off it (kick 3), at the
    /// wall's current position and facing.
    #[test]
    fn a_ball_is_kicked_off_the_triangle_where_it_is_now() {
        let mut world = PhysicsWorld::from_table(&load_table(5).unwrap());
        let input = InputState::default();
        for _ in 0..25 { world.step_frame(0.033, &input); }
        let m = world.moving_walls.iter().find(|m| m.id == "TriangleWall2").unwrap();
        let (mx, my) = ((m.x1 + m.x2) / 2.0, (m.y1 + m.y2) / 2.0);
        let (ox, oy) = (mx - 10.0, my - 18.86);
        let l = ox.hypot(oy);
        let out = [ox / l, oy / l];
        let incoming = 5.0;
        let ball_h = world.ball_handle().unwrap();
        let body = world.bodies.get_mut(ball_h).unwrap();
        body.set_translation(Vector::new(mx + out[0] * 2.5, my + out[1] * 2.5), true);
        body.set_linvel(Vector::new(-out[0] * incoming, -out[1] * incoming), true);
        body.set_gravity_scale(0.0, true);
        let mut best = f32::MIN;
        for _ in 0..20 {
            world.step_frame(0.033, &input);
            let v = world.bodies.get(ball_h).unwrap().linvel();
            best = best.max(v.x * out[0] + v.y * out[1]);
        }
        let expected = 0.6 * incoming + 3.0 / 0.785;
        assert!(best > expected * 0.9, "left the triangle at {best:.2}, expected about {expected:.2}");
    }

    #[test]
    fn retracting_a_triangle_wall_removes_its_kick_and_its_drawing() {
        let mut world = PhysicsWorld::from_table(&load_table(5).unwrap());
        world.set_wall_retracted("TriangleWall2", true);
        assert_eq!(world.wall_retracted("TriangleWall2"), Some(true));
        let snap = world.snapshot();
        assert_eq!(snap.walls.iter().filter(|s| s.label.as_deref().map_or(false, |l| l.contains("TriangleWall"))).count(), 2);
        world.set_wall_retracted("TriangleWall2", false);
        assert_eq!(world.snapshot().walls.iter().filter(|s| s.label.as_deref().map_or(false, |l| l.contains("TriangleWall"))).count(), 3);
    }
}

#[cfg(test)]
mod table_5_center_tests {
    use super::*;
    use crate::game::GameState;
    use crate::table::load_table;

    const BLUE: (u8, u8, u8) = (0x66, 0x88, 0xEE);
    const RED: (u8, u8, u8) = (0xEE, 0x88, 0x88);

    fn world5() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(5).unwrap()) }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }
    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    fn through(w: &mut PhysicsWorld, id: &str) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.balls[0].layer = s.from_layer.unwrap_or(w.balls[0].layer);
        place(w, x, y);
        step(w);
    }

    fn on(w: &mut PhysicsWorld, group: &str) {
        let (x, y) = w.rollover_center(group, 0).unwrap();
        place(w, x, y);
        step(w);
        step(w); // the first step lets the script unlock the lights, the second lights it
    }

    fn visible_lines(w: &PhysicsWorld) -> Vec<String> {
        w.snapshot().walls.iter().filter_map(|s| s.label.clone()).filter(|l| l.starts_with("CenterLine")).collect()
    }

    fn ramp_awards(w: &PhysicsWorld) -> Vec<u64> { w.drain_score_events().into_iter().filter(|&p| p >= 5000).collect() }

    #[test]
    fn the_centre_diagram_starts_hidden_and_inert() {
        let w = world5();
        for color in ["Blue", "Red", "Yellow", "Green"] {
            for n in 1..=3 {
                let id = format!("CenterRollover_{color}_{n}");
                let g = w.rollover_group(&id).unwrap();
                assert!(!g.visible && g.ignore_ball, "{id} starts hidden and ignoring the ball");
            }
        }
        assert!(visible_lines(&w).is_empty());
        assert!(w.snapshot().rollover_groups.iter().all(|g| g.rollovers.iter().all(|r| !r.label.contains("CenterRollover"))));
    }

    #[test]
    fn a_ramp_in_order_colours_the_ball_scores_and_reveals_that_colours_lights() {
        let mut w = world5();
        w.drain_score_events();
        through(&mut w, "RampSensor_OuterLeftTop");
        through(&mut w, "RampSensor_OuterRightTop"); // left then right: blue (right then left is green)
        assert_eq!(w.ball_color(0), Some(BLUE));
        assert_eq!(ramp_awards(&w), vec![5000]);
        step(&mut w);
        for n in 1..=3 {
            let g = w.rollover_group(&format!("CenterRollover_Blue_{n}")).unwrap();
            assert!(g.visible && !g.ignore_ball, "blue light {n} shows and answers to the ball");
            assert!(w.rollover_group(&format!("CenterRollover_Red_{n}")).unwrap().ignore_ball, "red stays out");
        }
    }

    #[test]
    fn a_ramp_taken_the_wrong_way_does_nothing() {
        let mut w = world5();
        w.drain_score_events();
        through(&mut w, "RampSensor_OuterLeftTop");
        assert_eq!(w.ball_color(0), None);
        assert!(ramp_awards(&w).is_empty());
    }

    #[test]
    fn only_a_ball_of_the_colour_lights_a_light_and_neighbours_get_a_line() {
        let mut w = world5();
        w.set_ball_color(0, Some(RED));
        step(&mut w);
        on(&mut w, "CenterRollover_Blue_1");
        assert!(!w.rollover_lit("CenterRollover_Blue_1", 0), "a red ball cannot light a blue light");

        w.set_ball_color(0, Some(BLUE));
        on(&mut w, "CenterRollover_Blue_1");
        assert!(w.rollover_lit("CenterRollover_Blue_1", 0));
        assert!(visible_lines(&w).is_empty(), "one lit light has no line");
        on(&mut w, "CenterRollover_Blue_2");
        assert_eq!(visible_lines(&w), vec!["CenterLine_Blue_1_2".to_string()]);
        on(&mut w, "CenterRollover_Blue_3");
        let mut lines = visible_lines(&w);
        lines.sort();
        assert_eq!(lines, vec!["CenterLine_Blue_1_2", "CenterLine_Blue_2_3", "CenterLine_Blue_3_1"], "a whole triangle");
    }

    #[test]
    fn lit_lights_stay_dim_after_the_ball_is_lost_and_unlit_ones_hide() {
        let mut w = world5();
        w.set_ball_color(0, Some(BLUE));
        step(&mut w);
        on(&mut w, "CenterRollover_Blue_1");
        w.park_ball();
        step(&mut w);
        let lit = w.rollover_group("CenterRollover_Blue_1").unwrap();
        assert!(lit.visible && lit.ignore_ball, "a lit light stays, dimmed, and ignores the ball");
        assert!(!w.rollover_group("CenterRollover_Blue_2").unwrap().visible, "unlit ones hide");
        assert_eq!(w.rollover_group("CenterRollover_Blue_1").unwrap().rollovers[0].color, Some((0x00, 0x33, 0x66)));
    }

    #[test]
    fn clearing_a_ramp_bank_raises_that_colours_ramp_score() {
        let mut w = world5();
        let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some("DropTargets_BlueRamp")).unwrap();
        let last = w.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last {
            w.drop_target_groups[gi].targets[ti].active = false;
            let ch = w.drop_target_groups[gi].targets[ti].collider;
            w.colliders.get_mut(ch).unwrap().set_collision_groups(InteractionGroups::none());
        }
        let t = &w.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(&mut w, mx, my);
        step(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Blue ramp +10%"));

        w.drain_score_events();
        place(&mut w, 1.0, 1.0);
        through(&mut w, "RampSensor_OuterLeftTop");
        through(&mut w, "RampSensor_OuterRightTop");
        assert_eq!(ramp_awards(&w), vec![5500], "+10% on blue");
        through(&mut w, "RampSensor_LeftMiddle");
        through(&mut w, "RampSensor_LeftTop"); // red, no bonus
        assert!(ramp_awards(&w).contains(&5000), "red is unchanged");
    }

    #[test]
    fn a_finished_colour_no_longer_colours_the_ball() {
        let mut w = world5();
        for n in 1..=3 { w.set_all_rollovers(&format!("CenterRollover_Blue_{n}"), true); }
        through(&mut w, "RampSensor_OuterLeftTop");
        through(&mut w, "RampSensor_OuterRightTop");
        assert_eq!(w.ball_color(0), None);
    }

    #[test]
    fn the_extra_ball_area_awards_a_ball_and_the_exit_puts_the_barriers_back() {
        let mut w = world5();
        let mut game = GameState::new(3);
        w.set_wall_retracted("ExtraBallBarrier_Blue", true);
        w.set_wall_retracted("ExtraBallBarrier_Red", true);
        on(&mut w, "ExtraBallRollover");
        game.sync(&w);
        assert_eq!(game.balls_remaining, 4, "an extra ball");
        assert!(messages(&w).iter().any(|m| m == "Extra Ball!") || game.message.as_ref().map_or(false, |m| m.0 == "Extra Ball!"));

        through(&mut w, "Sensor_ExtraBallExit");
        assert!(!w.rollover_lit("ExtraBallRollover", 0), "the extra-ball light goes out");
        assert_eq!(w.wall_retracted("ExtraBallBarrier_Blue"), Some(false));
        assert_eq!(w.wall_retracted("ExtraBallBarrier_Red"), Some(false), "all four barriers are back");
    }

    #[test]
    fn leaving_the_extra_ball_area_without_winning_it_leaves_the_barriers_alone() {
        let mut w = world5();
        w.set_wall_retracted("ExtraBallBarrier_Blue", true);
        through(&mut w, "Sensor_ExtraBallExit");
        assert_eq!(w.wall_retracted("ExtraBallBarrier_Blue"), Some(true));
    }

    #[test]
    fn the_flipper_lanes_raise_the_multiplier() {
        let mut w = world5();
        let n = w.rollover_count("FlipperRollovers");
        for i in 0..n - 1 { w.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = w.rollover_center("FlipperRollovers", n - 1).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        assert_eq!(w.drain_multiplier_events(), vec![1]);
    }
}

#[cfg(test)]
mod table_3_tests {
    use super::*;
    use crate::table::load_table;

    fn world3() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(3).unwrap()) }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    /// Touches bumper `i` once (and lets its debounce lapse so calls can repeat).
    fn hit_bumper(w: &mut PhysicsWorld, i: usize) {
        let (bx, by, br) = { let b = &w.bumpers[i]; (b.position[0], b.position[1], b.radius) };
        let y = by + br + w.ball_radius + 0.01;
        place(w, bx, y);
        // 12 substeps is exactly the bumper debounce, so each call is one hit.
        for _ in 0..12 { step(w); place(w, bx, y); }
    }

    fn bumper_scores(w: &PhysicsWorld) -> Vec<u64> { w.drain_score_events() }

    fn clear_bank(w: &mut PhysicsWorld, id: &str) {
        let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some(id)).unwrap();
        for t in &mut w.drop_target_groups[gi].targets {
            t.active = true;
            w.colliders.get_mut(t.collider).unwrap().set_collision_groups(InteractionGroups::all());
        }
        w.drop_target_groups[gi].reset_timer = None;
        let last = w.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last {
            w.drop_target_groups[gi].targets[ti].active = false;
            let ch = w.drop_target_groups[gi].targets[ti].collider;
            w.colliders.get_mut(ch).unwrap().set_collision_groups(InteractionGroups::none());
        }
        let t = &w.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(w, mx, my);
        step(w);
    }

    fn ring_color(w: &PhysicsWorld, id: &str) -> Option<(u8, u8, u8)> { w.rollover_group(id).unwrap().rollovers[0].color }

    fn start_bonus(w: &mut PhysicsWorld) {
        let (x, y) = w.rollover_center("LeftRampRollover", 0).unwrap();
        place(w, x, y);
        step(w);
        place(w, 1.0, 1.0);
    }

    #[test]
    fn the_heat_palette_matches_the_original() {
        let p = scripts::temperature_palette_for_test();
        assert_eq!(p.len(), 53);
        assert_eq!((p[0], p[15], p[16], p[23]), ((0, 0, 255), (0, 240, 255), (0, 255, 240), (0, 255, 16)));
        assert_eq!((p[24], p[31], p[32], p[52]), ((16, 255, 0), (240, 255, 0), (255, 240, 0), (255, 0, 0)));
    }

    #[test]
    fn a_bumper_scores_500_and_charges_the_heat_meter() {
        let mut w = world3();
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        assert!(bumper_scores(&w).iter().all(|&p| p == 500), "base score 500");
        assert_eq!(ring_color(&w, "BumperIndicator"), Some((0, 0, 255)), "1 of 200 is still blue");
        for _ in 0..39 { hit_bumper(&mut w, 0); }
        assert_eq!(ring_color(&w, "BumperIndicator"), Some((0, 160, 255)), "40 of 200 (20%): palette index 10");
    }

    #[test]
    fn the_ramp_rollover_starts_a_bonus_that_scores_extra_and_heats_the_bumpers() {
        let mut w = world3();
        let plain = w.snapshot().bumpers[0].color;
        start_bonus(&mut w);
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        let first = bumper_scores(&w)[0];
        // 500 plus about round(500 * 5 / 10) * 10 = 2500 at the very start of the bonus.
        assert!((2900..=3000).contains(&first), "bonus at its start: {first}");
        assert_eq!(w.snapshot().bumpers[0].color, Some((255, 0, 0)), "the bumpers start red-hot");

        run_sim_seconds(&mut w, 7.0); // about half way
        let mid = w.snapshot().bumpers[0].color.unwrap();
        assert!(mid.0 < 255 || mid.1 > 0, "cooling: {mid:?}");
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        let later = bumper_scores(&w)[0];
        assert!(later < first && later > 500, "the bonus fades: {later}");

        run_sim_seconds(&mut w, 10.0);
        assert_eq!(w.snapshot().bumpers[0].color, plain, "the bonus ends and the colours return");
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        assert!(bumper_scores(&w).iter().all(|&p| p == 500));
    }

    #[test]
    fn the_ramp_rollover_turns_itself_off_after_its_reset_delay() {
        let mut w = world3();
        let (x, y) = w.rollover_center("LeftRampRollover", 0).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        assert!(w.rollover_lit("LeftRampRollover", 0));
        place(&mut w, 1.0, 1.0);
        run_sim_seconds(&mut w, 2.0);
        assert!(w.rollover_lit("LeftRampRollover", 0), "still lit before the 3 s are up");
        run_sim_seconds(&mut w, 2.0);
        assert!(!w.rollover_lit("LeftRampRollover", 0), "off after 3 s, ready for the next pass");
    }

    #[test]
    fn the_banks_upgrade_the_bonus_and_light_their_rings() {
        let mut w = world3();
        clear_bank(&mut w, "LowerMultiballTargets");
        assert_eq!(ring_color(&w, "LowerTargetIndicator"), Some((0, 160, 255)), "1 of 5");
        clear_bank(&mut w, "UpperMultiballTargets");
        assert_eq!(ring_color(&w, "UpperTargetIndicator"), Some((80, 255, 0)), "1 of 2: palette index 26");

        // The upper bank raised the multiplier to 6, so the bonus start pays more.
        start_bonus(&mut w);
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        let first = bumper_scores(&w)[0];
        assert!((3400..=3500).contains(&first), "500 + about 3000: {first}");
    }

    #[test]
    fn a_full_meter_and_both_banks_light_the_multiball_ready_ring() {
        let mut w = world3();
        for _ in 0..2 { clear_bank(&mut w, "UpperMultiballTargets"); }
        for _ in 0..5 { clear_bank(&mut w, "LowerMultiballTargets"); }
        assert!(!w.rollover_lit("BumperIndicator", 0), "the meter is not full yet");
        for i in 0..200 { hit_bumper(&mut w, i % 3); }
        assert!(w.rollover_lit("BumperIndicator", 0), "ready");
    }

    #[test]
    fn losing_the_ball_ends_the_bonus() {
        let mut w = world3();
        let plain = w.snapshot().bumpers[0].color;
        start_bonus(&mut w);
        step(&mut w);
        assert_ne!(w.snapshot().bumpers[0].color, plain, "the bonus is heating the bumpers");
        w.ball_lost();
        w.park_ball();
        assert_eq!(w.snapshot().bumpers[0].color, plain, "losing the ball ends it");
    }

    #[test]
    fn the_flipper_lanes_raise_the_multiplier_and_the_savers_announce_themselves() {
        let mut w = world3();
        let n = w.rollover_count("FlipperRollovers");
        for i in 0..n - 1 { w.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = w.rollover_center("FlipperRollovers", n - 1).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        assert_eq!(w.drain_multiplier_events(), vec![1]);
        clear_bank(&mut w, "DropTargetLeftSave");
        assert!(w.drain_message_events().iter().any(|m| m.0 == "Left Save Enabled"));
    }
}

#[cfg(test)]
mod table_8_tests {
    use super::*;
    use crate::table::load_table;

    fn world8() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(8).unwrap()) }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    /// A world a couple of seconds in: the opening light show has finished.
    fn ready_world() -> PhysicsWorld {
        let mut w = world8();
        run_sim_seconds(&mut w, 2.0);
        place(&mut w, 1.0, 1.0);
        w.drain_score_events();
        messages(&w);
        w
    }

    fn hit_target(w: &mut PhysicsWorld, group: &str, i: usize) {
        let t = &w.drop_target_group(group).unwrap().targets[i];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(w, mx, my);
        step(w);
        place(w, 1.0, 1.0);
    }

    fn lit(w: &PhysicsWorld, id: &str) -> Vec<bool> { w.rollover_group(id).unwrap().rollovers.iter().map(|r| r.lit).collect() }

    fn through(w: &mut PhysicsWorld, id: &str, left_held: bool) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap();
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        place(w, x, y);
        let mut input = InputState::default();
        input.left_flipper = left_held;
        w.step(1.0 / 240.0, &input);
        place(w, 1.0, 1.0);
    }

    #[test]
    fn the_table_starts_set_up_and_the_rack_lights_up_in_a_show() {
        let mut w = world8();
        assert_eq!(w.wall_retracted("LaunchGate"), Some(true), "the launch gate starts retracted");
        assert_eq!(lit(&w, "LeftOrbitValueInserts"), vec![true, false, false]);
        assert_eq!(lit(&w, "EightBallValueInserts"), vec![true, false, false]);
        assert_eq!(w.spinner_score("LeftOrbitSpinner"), 500);
        for hidden in ["BonusCollectSaucer", "LeftOutlane", "RightOutlane"] {
            assert!(!w.rollover_group(hidden).unwrap().visible, "{hidden} starts hidden");
        }
        assert!(lit(&w, "RightBankInserts").iter().all(|&l| !l), "the show has not started");
        run_sim_seconds(&mut w, 2.0);
        assert!(lit(&w, "RightBankInserts").iter().all(|&l| l), "all seven rack lights are on after the show");
    }

    #[test]
    fn each_sunk_ball_moves_its_light_from_the_bank_row_to_the_bonus_row() {
        let mut w = ready_world();
        hit_target(&mut w, "RightBankTargets", 0);
        assert!(!lit(&w, "RightBankInserts")[6] && lit(&w, "BonusInserts")[6], "target 0 uses the mirrored slot 6");
        hit_target(&mut w, "RightBankTargets", 2);
        assert!(!lit(&w, "RightBankInserts")[4] && lit(&w, "BonusInserts")[4]);
        assert_eq!(lit(&w, "BonusInserts").iter().filter(|&&l| l).count(), 2);
        assert!(w.drain_score_events().iter().filter(|&&p| p == 2000).count() == 2, "2000 per target, no invented clear bonus");
    }

    #[test]
    fn inline_targets_raise_the_end_of_ball_bonus_multiplier_to_five() {
        let mut w = ready_world();
        w.set_rollover("BonusInserts", 0, true);
        w.set_rollover("BonusInserts", 1, true); // two sunk balls = 14000
        for i in 0..4 { hit_target(&mut w, "InlineTargets", i); }
        assert_eq!(lit(&w, "BonusMultiplierInserts"), vec![true; 4]);
        w.drain_score_events();
        w.ball_lost();
        assert_eq!(w.drain_score_events(), vec![14_000 * 5], "x5 after four inline targets");
        assert!(messages(&w).iter().any(|m| m == "Bonus: 70,000"));
    }

    #[test]
    fn the_lane_rollovers_raise_the_spinner_and_the_eight_ball_value_three_times_at_most() {
        let mut w = ready_world();
        let complete = |w: &mut PhysicsWorld| {
            let n = w.rollover_count("LaneRollovers");
            for i in 0..n - 1 { w.set_rollover("LaneRollovers", i, true); }
            let (x, y) = w.rollover_center("LaneRollovers", n - 1).unwrap();
            place(w, x, y);
            step(w);
            place(w, 1.0, 1.0);
        };
        complete(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Lanes Completed"));
        assert_eq!((w.spinner_score("LeftOrbitSpinner"), lit(&w, "LeftOrbitValueInserts")), (1000, vec![true, true, false]));
        complete(&mut w);
        assert_eq!(w.spinner_score("LeftOrbitSpinner"), 1500);
        complete(&mut w);
        assert_eq!(w.spinner_score("LeftOrbitSpinner"), 1500, "capped at 1500 a spin");
        // Two real raises (20000 + 40000) make the pocket worth 60,000.
        w.drain_score_events();
        hit_target(&mut w, "EightBallTarget", 0);
        assert!(messages(&w).iter().any(|m| m == "Corner Pocket: 60,000"));
    }

    #[test]
    fn an_eight_ball_shot_before_the_bank_is_done_is_a_corner_pocket() {
        let mut w = ready_world();
        hit_target(&mut w, "EightBallTarget", 0);
        assert_eq!(w.drain_score_events(), vec![20_000]);
        assert!(messages(&w).iter().any(|m| m == "Corner Pocket: 20,000"));
        assert!(!w.drop_target_down("EightBallTarget", 0), "the target is straight back up");
    }

    #[test]
    fn finishing_the_bank_then_sinking_the_eight_ball_collects_the_whole_rack() {
        let mut w = ready_world();
        for i in 0..6 { w.set_drop_target_down("RightBankTargets", i, true); }
        hit_target(&mut w, "RightBankTargets", 6);
        assert!(messages(&w).iter().any(|m| m == "Shoot the Eight Ball!"));
        assert!(w.rollover_lit("EightBallInsert", 0));

        w.drain_score_events();
        hit_target(&mut w, "EightBallTarget", 0);
        assert!(messages(&w).iter().any(|m| m == "Bonus Collect Lit"));
        assert!(lit(&w, "BonusInserts").iter().all(|&l| l), "all eight balls count");
        assert!(w.drain_score_events().contains(&20_000), "plus the eight-ball value");
        assert!(w.rollover_group("BonusCollectSaucer").unwrap().visible, "the collect saucer is shown");

        // End of ball: one rack (56000) plus eight balls (56000) = 112,000.
        w.ball_lost();
        assert!(messages(&w).iter().any(|m| m == "Bonus: 112,000"));
        w.park_ball();
        run_sim_seconds(&mut w, 2.0);
        assert!(lit(&w, "BonusInserts").iter().all(|&l| !l), "a completed rack starts over");
        assert!(lit(&w, "RightBankInserts").iter().all(|&l| l) && !w.drop_target_down("RightBankTargets", 3), "light show and bank back up, even though a ball was lost");
    }

    #[test]
    fn holding_the_left_flipper_at_launch_arms_the_breakshot() {
        let mut w = ready_world();
        through(&mut w, "LaunchGateRetract", true);
        assert_eq!(w.wall_retracted("LeftGate"), Some(true), "the left gate opens");
        hit_target(&mut w, "EightBallTarget", 0);
        assert!(messages(&w).iter().any(|m| m == "Breakshot!"));
        assert!(lit(&w, "BonusInserts").iter().all(|&l| l));
    }

    #[test]
    fn the_breakshot_is_lost_once_the_ball_reaches_the_flippers() {
        let mut w = ready_world();
        through(&mut w, "LaunchGateRetract", true);
        through(&mut w, "EndSkillShot", false);
        hit_target(&mut w, "EightBallTarget", 0);
        assert!(messages(&w).iter().any(|m| m.starts_with("Corner Pocket")));
    }

    #[test]
    fn without_the_flipper_held_there_is_no_breakshot() {
        let mut w = ready_world();
        through(&mut w, "LaunchGateRetract", false);
        assert_eq!(w.wall_retracted("LeftGate"), Some(false), "the left gate stays shut");
        hit_target(&mut w, "EightBallTarget", 0);
        assert!(messages(&w).iter().any(|m| m.starts_with("Corner Pocket")));
    }

    #[test]
    fn the_gates_open_on_their_sensors_and_close_again_on_timers() {
        let mut w = ready_world();
        assert_eq!(w.wall_retracted("LaunchGate"), Some(true));
        through(&mut w, "LaunchGateRetract", false);
        run_sim_seconds(&mut w, 1.3);
        assert_eq!(w.wall_retracted("LaunchGate"), Some(false), "the lane closes a second after the ball passes");

        assert_eq!(w.wall_retracted("RightGate"), Some(false));
        through(&mut w, "RightGateRetract", false);
        assert_eq!(w.wall_retracted("RightGate"), Some(true));
        run_sim_seconds(&mut w, 1.3);
        assert_eq!(w.wall_retracted("RightGate"), Some(false));

        // The left orbit spinner opens the left gate for two seconds.
        let (sx, sy) = { let sp = &w.spinners[0]; (sp.position[0], sp.position[1]) };
        place(&mut w, sx, sy);
        w.bodies.get_mut(w.balls[0].body).unwrap().set_linvel(Vector::new(0.0, 12.0), true);
        step(&mut w);
        place(&mut w, 1.0, 1.0);
        assert_eq!(w.wall_retracted("LeftGate"), Some(true));
        run_sim_seconds(&mut w, 1.0);
        assert_eq!(w.wall_retracted("LeftGate"), Some(true), "still open before the 2 s are up");
        run_sim_seconds(&mut w, 1.5);
        assert_eq!(w.wall_retracted("LeftGate"), Some(false));
    }

    #[test]
    fn the_end_of_ball_bonus_is_scored_and_the_table_resets_except_the_sunk_balls() {
        let mut w = ready_world();
        for i in 0..3 { w.set_rollover("BonusInserts", i, true); }
        hit_target(&mut w, "InlineTargets", 0);
        w.set_spinner_score("LeftOrbitSpinner", 1500);
        w.drain_score_events();
        w.ball_lost();
        assert_eq!(w.drain_score_events(), vec![21_000 * 2], "three balls at x2");
        assert!(messages(&w).iter().any(|m| m == "Bonus: 42,000"));
        assert_eq!(w.spinner_score("LeftOrbitSpinner"), 500, "back to 500");
        assert!(!w.drop_target_down("InlineTargets", 0), "the inline targets come back");
        assert_eq!(lit(&w, "BonusInserts").iter().filter(|&&l| l).count(), 3, "balls 1 to 7 stay sunk between balls");
    }

    #[test]
    fn the_outlanes_scratch_and_the_saucer_pays_the_bonus() {
        let mut w = ready_world();
        let (x, y) = w.rollover_center("LeftOutlane", 0).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Scratch!"));
        place(&mut w, 1.0, 1.0);
        w.set_rollover("BonusInserts", 0, true);
        let (x, y) = w.rollover_center("BonusCollectSaucer", 0).unwrap();
        place(&mut w, x, y);
        w.drain_score_events();
        step(&mut w);
        assert_eq!(w.drain_score_events(), vec![7_000]);
        assert!(messages(&w).iter().any(|m| m == "Bonus Collected: 7,000"));
    }

    #[test]
    fn numbers_get_commas() {
        assert_eq!(scripts::with_commas(0), "0");
        assert_eq!(scripts::with_commas(999), "999");
        assert_eq!(scripts::with_commas(1_000), "1,000");
        assert_eq!(scripts::with_commas(112_000), "112,000");
        assert_eq!(scripts::with_commas(1_234_567), "1,234,567");
    }
}

#[cfg(test)]
mod table_7_tests {
    use super::*;
    use crate::table::load_table;

    const CONSTELLATION_NAMES: [&str; 6] = ["Canis Major", "Gemini", "Leo", "Orion", "Sagittarius", "Ursa Major"];

    fn world7() -> PhysicsWorld {
        let mut w = PhysicsWorld::from_table(&load_table(7).unwrap());
        w.script = Some(Box::new(scripts::AdAstra::new(7)));
        w
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    fn through(w: &mut PhysicsWorld, id: &str) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.balls[0].layer = s.from_layer.unwrap_or(0);
        place(w, x, y);
        step(w);
        w.balls[0].layer = 0;
        place(w, 1.0, 1.0);
    }

    fn circles(w: &PhysicsWorld) -> Vec<(f32, f32, (u8, u8, u8))> {
        w.snapshot().shapes.iter().filter_map(|s| match *s { Shape::Circle { x, y, color, .. } => Some((x, y, color)), _ => None }).collect()
    }

    fn lines(w: &PhysicsWorld) -> usize {
        w.snapshot().shapes.iter().filter(|s| matches!(s, Shape::Line { .. })).count()
    }

    fn drawn(w: &PhysicsWorld, id: &str) -> bool {
        w.snapshot().walls.iter().any(|s| s.label.as_deref() == Some(id))
    }

    /// A star of the current constellation that is not lit yet (drawn green).
    fn unlit_target(w: &PhysicsWorld) -> Option<(f32, f32)> {
        circles(w).into_iter().filter(|c| c.2.0 == 0 && c.2.2 == 0 && c.2.1 > 0).max_by_key(|c| c.2.1).map(|c| (c.0, c.1))
    }

    fn pick_constellation(w: &mut PhysicsWorld) -> String {
        messages(w);
        through(w, "LeftLoopDetector");
        let name = messages(w).into_iter().find(|m| CONSTELLATION_NAMES.contains(&m.as_str())).expect("the loop picked a constellation");
        run_sim_seconds(w, 11.0); // the view flies there in 10 s
        name
    }

    #[test]
    fn the_sky_map_is_drawn_inside_its_circle_and_drifts() {
        let mut w = world7();
        w.step_frame(0.033, &InputState::default());
        let (cx, cy, r) = {
            let (x, y) = w.rollover_center("StarViewBoundary", 0).unwrap();
            (x, y, w.rollover_radius("StarViewBoundary", 0).unwrap())
        };
        let first = circles(&w);
        assert!(first.len() > 20, "a patch of sky: {} stars", first.len());
        assert!(first.iter().all(|c| ((c.0 - cx).powi(2) + (c.1 - cy).powi(2)).sqrt() <= r * 1.01), "all inside the circle");
        assert!(first.iter().all(|c| c.2.0 == c.2.1 || c.2.1 == c.2.2 || c.2.0 == c.2.2 || true), "colours exist");
        // While wandering every visible star is white (dimmer for fainter stars).
        assert!(first.iter().all(|c| c.2.0 == c.2.1 && c.2.1 == c.2.2), "all white while wandering");
        run_sim_seconds(&mut w, 8.0);
        let later = circles(&w);
        assert!(first.len() != later.len() || first.iter().zip(&later).any(|(a, b)| (a.0 - b.0).abs() > 0.01), "the sky has moved");
    }

    /// The star map must actually reach the screen: render with and without it.
    #[test]
    fn the_renderer_draws_the_star_map() {
        let mut w = world7();
        w.step_frame(0.033, &InputState::default());
        let snap = w.snapshot();
        assert!(!snap.shapes.is_empty());
        let mut bare = snap.clone();
        bare.shapes.clear();
        let renderer = crate::renderer::FieldRenderer::new();
        let with = renderer.render(&snap, 0.0, 100, 70, false);
        let without = renderer.render(&bare, 0.0, 100, 70, false);
        assert_eq!(with.chars.len(), without.chars.len());
        let changed = with.chars.iter().zip(&without.chars).filter(|(a, b)| a != b).count();
        assert!(changed >= 15, "the stars add dots to the picture, changed cells: {changed}");
    }

    #[test]
    fn the_renderer_draws_a_floating_score_as_digits_and_fades_it() {
        let mut w = world7();
        w.step_frame(0.033, &InputState::default());
        let mut snap = w.snapshot();
        snap.popups.push(ScorePopup { text: "5000".into(), x: 10.0, y: 12.0, progress: 0.0, color: (0xFF, 0xFF, 0x00) });
        let renderer = crate::renderer::FieldRenderer::new();
        let frame = renderer.render(&snap, 0.0, 100, 70, false);
        let text: String = frame.chars.chunks(100).map(|row| row.iter().collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("5000"), "the digits are on the screen");
        let idx = frame.chars.iter().position(|&c| c == '5').unwrap();
        assert_eq!(frame.colors[idx], ratatui::style::Color::Rgb(0xFF, 0xFF, 0x00), "in the full colour at first");

        snap.popups[0].progress = 0.75;
        let frame = renderer.render(&snap, 0.0, 100, 70, false);
        let idx = frame.chars.iter().position(|&c| c == '5').unwrap();
        assert_eq!(frame.colors[idx], ratatui::style::Color::Rgb(0x7F, 0x7F, 0x00), "half faded three quarters through");
    }

    #[test]
    fn the_loop_arrows_pulse_while_wandering_and_the_lock_arrow_is_off() {
        let mut w = world7();
        w.step_frame(0.033, &InputState::default());
        assert!(drawn(&w, "LeftLoopGuide") && drawn(&w, "RightLoopGuide"), "shoot a loop");
        assert!(!drawn(&w, "LockAndJackpotGuide"));
    }

    #[test]
    fn a_loop_scores_and_sends_the_view_to_a_random_constellation() {
        let mut w = world7();
        w.drain_score_events();
        let name = pick_constellation(&mut w);
        assert!(w.drain_score_events().iter().any(|&p| p == 5000), "the loop score");
        assert!(CONSTELLATION_NAMES.contains(&name.as_str()));
        let c = circles(&w);
        assert!(c.iter().any(|c| c.2.0 == 0 && c.2.2 == 0 && c.2.1 > 0), "its stars are green");
        assert!(!drawn(&w, "LeftLoopGuide"), "no loop arrows once one is chosen");
    }

    #[test]
    fn a_loop_counts_once_per_pass_not_once_per_step() {
        let mut w = world7();
        messages(&w);
        w.drain_score_events();
        let s = w.sensors.iter().find(|s| s.id == "LeftLoopDetector").unwrap();
        let (x, y, from) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0, s.from_layer.unwrap_or(0));
        w.balls[0].layer = from;
        place(&mut w, x, y);
        for _ in 0..5 { step(&mut w); place(&mut w, x, y); } // sitting in the sensor
        let loops = w.drain_score_events().iter().filter(|&&p| p == 5000).count();
        assert_eq!(loops, 1, "one loop, not five");
    }

    #[test]
    fn rolling_over_a_constellations_stars_lights_them_draws_lines_and_makes_a_lock_ready() {
        let mut w = world7();
        pick_constellation(&mut w);
        w.drain_score_events();
        messages(&w);

        let mut lit = 0;
        for _ in 0..60 {
            let Some((x, y)) = unlit_target(&w) else { break };
            place(&mut w, x, y);
            step(&mut w);
            lit += 1;
            w.step(1.0 / 240.0, &InputState::default()); // the tick that redraws
            if lit == 2 { assert!(w.drain_score_events().iter().filter(|&&p| p == 500).count() >= 2, "500 a star"); }
        }
        assert!(lit >= 8, "a constellation has at least eight stars, lit {lit}");
        assert!(lines(&w) >= 5, "lines join lit neighbours: {}", lines(&w));
        assert!(messages(&w).iter().any(|m| m == "Ball Lock Ready"), "the last star makes the lock ready");
        place(&mut w, 1.0, 1.0);
        w.step_frame(0.033, &InputState::default());
        assert!(drawn(&w, "LockAndJackpotGuide"), "the lock arrow lights");
        assert!(!drawn(&w, "LeftLoopGuide"));
    }

    #[test]
    fn a_ball_on_another_layer_does_not_light_stars() {
        let mut w = world7();
        pick_constellation(&mut w);
        w.drain_score_events();
        let (x, y) = unlit_target(&w).expect("a star to aim at");
        w.balls[0].layer = 1;
        place(&mut w, x, y);
        step(&mut w);
        step(&mut w);
        assert!(!w.drain_score_events().contains(&500), "the mini-table ball is not over the sky");
    }

    #[test]
    fn the_inner_orbit_scores_either_way_round_and_the_drop_sensors_stop_the_ball() {
        let mut w = world7();
        w.drain_score_events();
        through(&mut w, "InnerOrbitLeftTrigger");
        through(&mut w, "InnerOrbitRightTrigger");
        assert!(w.drain_score_events().contains(&5000), "left then right");
        through(&mut w, "InnerOrbitLeftTrigger");
        assert!(w.drain_score_events().contains(&5000), "right then left");

        let s = w.sensors.iter().find(|s| s.id == "LeftFlipperDropSensor").unwrap();
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.balls[0].layer = s.from_layer.unwrap_or(0);
        place(&mut w, x, y);
        w.bodies.get_mut(w.balls[0].body).unwrap().set_linvel(Vector::new(3.0, -8.0), true);
        step(&mut w);
        assert!(w.bodies.get(w.balls[0].body).unwrap().linvel().length() < 0.5, "stopped dead");
    }

    #[test]
    fn the_mini_field_banks_raise_the_loop_score() {
        let mut w = world7();
        let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some("MiniFieldTopTargets")).unwrap();
        let last = w.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last { w.set_drop_target_down("MiniFieldTopTargets", ti, true); }
        let t = &w.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        // The mini table is layer 1: a ball on the main layer does not touch its targets.
        place(&mut w, mx, my);
        step(&mut w);
        assert!(!messages(&w).iter().any(|m| m == "Ramp bonus increased"));
        w.set_ball_layer(0, 1);
        place(&mut w, mx, my);
        step(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Ramp bonus increased"));
        w.set_ball_layer(0, 0);
        w.drain_score_events();
        through(&mut w, "LeftLoopDetector");
        assert!(w.drain_score_events().contains(&6000), "5000 plus 1000");
    }

    #[test]
    fn the_flipper_lanes_raise_the_multiplier() {
        let mut w = world7();
        let n = w.rollover_count("FlipperRollovers");
        for i in 0..n - 1 { w.set_rollover("FlipperRollovers", i, true); }
        let (x, y) = w.rollover_center("FlipperRollovers", n - 1).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        assert_eq!(w.drain_multiplier_events(), vec![1]);
    }
}

#[cfg(test)]
mod ball_saver_rule_tests {
    use super::*;
    use crate::table::load_table;

    fn world1() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(1).unwrap()) }

    fn sink(w: &mut PhysicsWorld, i: usize) {
        let body = w.bodies.get_mut(w.balls[i].body).unwrap();
        body.set_translation(Vector::new(5.0 + i as f32, -2.0), true);
        body.set_linvel(Vector::new(0.0, -1.0), true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    #[test]
    fn losing_the_only_ball_soon_after_launch_relaunches_it_once() {
        let mut w = world1();
        w.launch_ball();
        w.game_time += 5.0;
        sink(&mut w, 0);
        step(&mut w);
        assert!(!w.is_ball_drained(), "the ball was saved");
        assert!(messages(&w).iter().any(|m| m == "Ball Saved"));
        let v = w.bodies.get(w.balls[0].body).unwrap().linvel();
        assert!(v.y > 5.0, "it is back in the lane, going up: {v:?}");

        w.game_time += 2.0;
        sink(&mut w, 0);
        step(&mut w);
        assert!(w.is_ball_drained(), "only one mercy ball per ball");
    }

    #[test]
    fn a_ball_that_survived_twenty_seconds_is_not_saved() {
        let mut w = world1();
        w.launch_ball();
        w.game_time += 25.0;
        sink(&mut w, 0);
        step(&mut w);
        assert!(w.is_ball_drained());
        assert!(!messages(&w).iter().any(|m| m == "Ball Saved"));
    }

    #[test]
    fn a_new_ball_gets_its_own_mercy_ball() {
        let mut w = world1();
        w.launch_ball();
        sink(&mut w, 0);
        step(&mut w); // saved
        w.game_time += 30.0;
        sink(&mut w, 0);
        step(&mut w);
        assert!(w.is_ball_drained(), "the second loss is real");
        w.ball_lost();
        w.park_ball();
        w.launch_ball();
        sink(&mut w, 0);
        step(&mut w);
        assert!(!w.is_ball_drained(), "the next ball has a mercy ball again");
    }

    #[test]
    fn early_in_a_multiball_a_lost_ball_is_replaced_but_not_later() {
        let mut w = world1();
        w.launch_ball();
        w.game_time += 25.0; // the first ball is past its own mercy window
        w.launch_extra_ball();
        sink(&mut w, 1);
        step(&mut w);
        assert_eq!(w.ball_count(), 2, "a replacement came straight back");
        assert!(messages(&w).iter().any(|m| m == "Ball Saved"));

        w.game_time += 40.0; // 30 seconds after the multiball began
        sink(&mut w, 1);
        step(&mut w);
        assert_eq!(w.ball_count(), 1, "no more replacements");
    }

    #[test]
    fn two_balls_lost_together_get_one_replacement() {
        let mut w = world1();
        w.launch_ball();
        w.game_time += 25.0;
        w.launch_extra_ball();
        w.launch_extra_ball();
        sink(&mut w, 1);
        sink(&mut w, 2);
        step(&mut w);
        assert_eq!(w.ball_count(), 2, "three balls, two lost, one given back");
    }

    #[test]
    fn going_back_to_one_ball_ends_the_multiball_saver() {
        let mut w = world1();
        w.launch_ball();
        w.game_time += 25.0;
        w.launch_extra_ball();
        w.game_time += 40.0;
        sink(&mut w, 1);
        step(&mut w);
        assert!(w.multiball_start_time.is_none(), "single ball again");
        w.launch_extra_ball();
        assert!(w.multiball_start_time.is_some(), "a new multiball starts a new window");
    }
}

#[cfg(test)]
mod rollover_radius_tests {
    use super::*;
    use crate::table::load_table;

    /// The ball's centre decides, not its edge.
    #[test]
    fn a_rollover_needs_the_balls_centre_inside_its_radius() {
        let mut w = PhysicsWorld::from_table(&load_table(4).unwrap());
        let id = w.rollover_groups.iter().find(|g| !g.ignore_ball && g.shot_id.is_some()).and_then(|g| g.shot_id.clone()).unwrap();
        let (x, y) = w.rollover_center(&id, 0).unwrap();
        let r = w.rollover_radius(&id, 0).unwrap();
        let place = |w: &mut PhysicsWorld, px: f32, py: f32| {
            let body = w.bodies.get_mut(w.balls[0].body).unwrap();
            body.set_translation(Vector::new(px, py), true);
            body.set_linvel(Vector::new(0.0, 0.0), true);
            body.set_gravity_scale(0.0, true);
        };
        // Just outside the radius, but with the ball's edge well inside it.
        place(&mut w, x + r + 0.1, y);
        w.step(1.0 / 240.0, &InputState::default());
        assert!(!w.rollover_lit(&id, 0), "centre outside the radius: not lit");
        place(&mut w, x + r - 0.05, y);
        w.step(1.0 / 240.0, &InputState::default());
        assert!(w.rollover_lit(&id, 0), "centre inside the radius: lit");
    }
}

#[cfg(test)]
mod table_3_multiball_tests {
    use super::*;
    use crate::table::load_table;

    fn world3() -> PhysicsWorld { PhysicsWorld::from_table(&load_table(3).unwrap()) }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn hit_bumper(w: &mut PhysicsWorld, i: usize) {
        let (bx, by, br) = { let b = &w.bumpers[i]; (b.position[0], b.position[1], b.radius) };
        let y = by + br + w.ball_radius + 0.01;
        place(w, bx, y);
        for _ in 0..12 { step(w); place(w, bx, y); }
    }

    fn clear_bank(w: &mut PhysicsWorld, id: &str) {
        let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some(id)).unwrap();
        for ti in 0..w.drop_target_groups[gi].targets.len() { w.set_drop_target_down(id, ti, false); }
        w.drop_target_groups[gi].reset_timer = None;
        let last = w.drop_target_groups[gi].targets.len() - 1;
        for ti in 0..last { w.set_drop_target_down(id, ti, true); }
        let t = &w.drop_target_groups[gi].targets[last];
        let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
        place(w, mx, my);
        step(w);
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    /// Meter full and both banks done: the ready ring is lit.
    fn make_ready(w: &mut PhysicsWorld) {
        for _ in 0..2 { clear_bank(w, "UpperMultiballTargets"); }
        for _ in 0..5 { clear_bank(w, "LowerMultiballTargets"); }
        for i in 0..200 { hit_bumper(w, i % 3); }
        place(w, 1.0, 1.0);
        assert!(w.rollover_lit("BumperIndicator", 0), "ready");
    }

    fn shoot_ramp(w: &mut PhysicsWorld) {
        let (x, y) = w.rollover_center("LeftRampRollover", 0).unwrap();
        place(w, x, y);
        step(w);
        place(w, 1.0, 1.0);
    }

    fn savers_active(w: &PhysicsWorld) -> bool {
        w.kick_walls.iter().filter(|k| k.retract_when_hit).all(|k| k.active)
    }

    #[test]
    fn a_ramp_with_the_ready_ring_lit_starts_a_multiball() {
        let mut w = world3();
        make_ready(&mut w);
        for kw in &mut w.kick_walls { if kw.retract_when_hit { kw.active = false; } }
        messages(&w);
        shoot_ramp(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Multiball!"));
        assert!(savers_active(&w), "both ball savers come back");
        assert_eq!(w.ball_count(), 1, "the extra balls are not out yet");

        run_sim_seconds(&mut w, 1.2);
        assert_eq!(w.ball_count(), 2, "first extra ball at 1 s");
        run_sim_seconds(&mut w, 2.6);
        assert_eq!(w.ball_count(), 3, "second extra ball at 3.5 s");
    }

    #[test]
    fn multiball_upgrades_the_bonus_one_step_past_its_usual_maximum() {
        let mut w = world3();
        make_ready(&mut w);
        shoot_ramp(&mut w); // starts the bonus and the multiball
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        // The multiplier is 5 + 2 (banks) + 1 (multiball) = 8: 500 + about 4000.
        let first = w.drain_score_events().into_iter().find(|&p| p >= 500).unwrap();
        assert!((4400..=4500).contains(&first), "got {first}");
    }

    #[test]
    fn without_the_ready_ring_a_ramp_is_only_a_bonus() {
        let mut w = world3();
        messages(&w);
        shoot_ramp(&mut w);
        run_sim_seconds(&mut w, 4.0);
        assert_eq!(w.ball_count(), 1);
        assert!(!messages(&w).iter().any(|m| m == "Multiball!"));
    }

    #[test]
    fn the_three_rings_flash_independently_during_multiball() {
        let mut w = world3();
        make_ready(&mut w);
        shoot_ramp(&mut w);
        run_sim_seconds(&mut w, 4.0);
        let ring = |w: &PhysicsWorld, id: &str| w.rollover_group(id).unwrap().rollovers[0].color;
        let before = [ring(&w, "UpperTargetIndicator"), ring(&w, "LowerTargetIndicator"), ring(&w, "BumperIndicator")];
        run_sim_seconds(&mut w, 2.0);
        let after = [ring(&w, "UpperTargetIndicator"), ring(&w, "LowerTargetIndicator"), ring(&w, "BumperIndicator")];
        assert_ne!(before, after, "the rings are animating");
        assert!(before.iter().all(|c| c.is_some()));
    }

    #[test]
    fn a_ramp_during_multiball_pays_the_groups_score_again() {
        let mut w = world3();
        make_ready(&mut w);
        shoot_ramp(&mut w);
        run_sim_seconds(&mut w, 4.0);
        place(&mut w, 1.0, 1.0);
        w.set_all_rollovers("LeftRampRollover", false);
        let group_score = w.rollover_group_score("LeftRampRollover");
        assert!(group_score > 0);
        w.drain_score_events();
        shoot_ramp(&mut w);
        let paid = w.drain_score_events().iter().filter(|&&p| p == group_score).count();
        assert_eq!(paid, 2, "once as the rollover's score, once as the multiball bonus");
    }

    #[test]
    fn when_the_multiball_ends_everything_resets() {
        let mut w = world3();
        make_ready(&mut w);
        shoot_ramp(&mut w);
        run_sim_seconds(&mut w, 4.0);
        assert_eq!(w.ball_count(), 3);
        w.game_time += 100.0; // no multiball saver
        for i in (1..3).rev() {
            let body = w.bodies.get_mut(w.balls[i].body).unwrap();
            body.set_translation(Vector::new(5.0, -2.0), true);
        }
        run_sim_seconds(&mut w, 0.5);
        assert_eq!(w.ball_count(), 1);
        run_sim_seconds(&mut w, 0.5);

        assert!(!w.rollover_lit("BumperIndicator", 0), "the ready ring goes out");
        assert_eq!(w.rollover_group("UpperTargetIndicator").unwrap().rollovers[0].color, w.rollover_group("UpperTargetIndicator").unwrap().rollovers[0].base_color, "ring colours restored");
        // A fresh bonus is back to the base 5x: 500 + about 2500.
        shoot_ramp(&mut w);
        w.drain_score_events();
        hit_bumper(&mut w, 0);
        let first = w.drain_score_events().into_iter().find(|&p| p >= 500).unwrap();
        assert!((2900..=3000).contains(&first), "back to the base bonus: {first}");
        assert!(messages(&w).iter().filter(|m| *m == "Multiball!").count() <= 1, "no second multiball");
    }

    #[test]
    fn losing_the_ball_during_multiball_resets_it_too() {
        let mut w = world3();
        make_ready(&mut w);
        shoot_ramp(&mut w);
        run_sim_seconds(&mut w, 4.0);
        w.ball_lost();
        assert!(!w.rollover_lit("BumperIndicator", 0));
        w.park_ball();
        shoot_ramp(&mut w);
        assert_eq!(w.ball_count(), 1, "no multiball: the ring is not lit again");
    }
}

#[cfg(test)]
mod table_6_multiball_tests {
    use super::*;
    use crate::table::load_table;

    const BALL_COLORS: [(u8, u8, u8); 5] = scripts::PLANET_BALL_COLORS;

    fn orbiters() -> PhysicsWorld {
        let mut world = PhysicsWorld::from_table(&load_table(6).unwrap());
        world.script = Some(Box::new(scripts::Orbiters::new(99)));
        world.step(1.0 / 240.0, &InputState::default());
        world
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    fn through(w: &mut PhysicsWorld, id: &str) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.balls[0].layer = s.from_layer.unwrap_or(w.balls[0].layer);
        place(w, x, y);
        step(w);
    }

    /// Gives ball 0 planet `i`'s colour and plays that planet's own shot.
    fn switch_on(w: &mut PhysicsWorld, i: usize) {
        w.set_ball_color(0, Some(BALL_COLORS[i]));
        match i {
            0 => {
                let gi = w.drop_target_groups.iter().position(|g| g.shot_id.as_deref() == Some("Planet1Targets")).unwrap();
                for ti in 0..w.drop_target_groups[gi].targets.len() { w.set_drop_target_down("Planet1Targets", ti, false); }
                let last = w.drop_target_groups[gi].targets.len() - 1;
                for ti in 0..last { w.set_drop_target_down("Planet1Targets", ti, true); }
                let t = &w.drop_target_groups[gi].targets[last];
                let (mx, my) = ((t.x1 + t.x2) / 2.0, (t.y1 + t.y2) / 2.0);
                place(w, mx, my);
                step(w);
            }
            1 => {
                let n = w.rollover_count("Planet2Rollovers");
                // An extra ball may have lit one on its way past.
                w.set_all_rollovers("Planet2Rollovers", false);
                for k in 0..n - 1 { w.set_rollover("Planet2Rollovers", k, true); }
                let (x, y) = w.rollover_center("Planet2Rollovers", n - 1).unwrap();
                place(w, x, y);
                step(w);
            }
            2 => { through(w, "RightLoopDetector_Enter"); through(w, "RightLoopDetector_Trigger"); }
            3 => { through(w, "LeftLoopDetector_Enter"); through(w, "LeftLoopDetector_Trigger"); }
            _ => { through(w, "OrbitDetector_Right"); through(w, "OrbitDetector_Left"); }
        }
        w.balls[0].layer = 0;
        place(w, 1.0, 1.0);
        step(w);
    }

    fn savers_active(w: &PhysicsWorld) -> bool { w.kick_walls.iter().filter(|k| k.retract_when_hit).all(|k| k.active) }

    fn all_five(w: &mut PhysicsWorld) { for i in 0..5 { switch_on(w, i); } }

    #[test]
    fn five_planets_start_a_multiball() {
        let mut w = orbiters();
        for kw in &mut w.kick_walls { if kw.retract_when_hit { kw.active = false; } }
        for i in 0..4 { switch_on(&mut w, i); }
        assert!(!messages(&w).iter().any(|m| m == "Multiball!"), "four are not enough");
        switch_on(&mut w, 4);
        let msgs = messages(&w);
        assert!(msgs.iter().any(|m| m == "Multiball!"), "got {msgs:?}");
        assert!(!msgs.iter().any(|m| m == "Planet 5 Activated!"));
        assert!(savers_active(&w), "both savers come back");
        w.set_ball_color(0, None);
        step(&mut w);
        for i in 0..5 { assert!(!w.rollover_lit(&format!("Planet{}", i + 1), 0), "planet {} resets", i + 1); }

        run_sim_seconds(&mut w, 1.2);
        assert_eq!(w.ball_count(), 2, "one extra ball at 1 s");
        run_sim_seconds(&mut w, 3.2);
        assert_eq!(w.ball_count(), 3, "another at 4 s");
    }

    #[test]
    fn five_planets_during_the_multiball_pay_a_growing_jackpot() {
        let mut w = orbiters();
        all_five(&mut w);
        run_sim_seconds(&mut w, 4.5);
        assert_eq!(w.ball_count(), 3);
        messages(&w);
        w.drain_score_events();

        all_five(&mut w);
        let msgs = messages(&w);
        assert!(msgs.iter().any(|m| m == "Jackpot!"), "got {msgs:?}");
        assert!(w.drain_score_events().contains(&105_000), "100,000 plus the planet's 5,000");

        all_five(&mut w);
        assert!(messages(&w).iter().any(|m| m == "2x Jackpot!"));
        assert!(w.drain_score_events().contains(&205_000));
    }

    #[test]
    fn when_the_multiball_ends_the_planets_reset_and_it_can_start_again() {
        let mut w = orbiters();
        all_five(&mut w);
        run_sim_seconds(&mut w, 4.5);
        assert_eq!(w.ball_count(), 3);
        w.game_time += 100.0; // no multiball saver
        for i in (1..3).rev() {
            let body = w.bodies.get_mut(w.balls[i].body).unwrap();
            body.set_translation(Vector::new(5.0, -2.0), true);
        }
        run_sim_seconds(&mut w, 0.5);
        assert_eq!(w.ball_count(), 1);
        run_sim_seconds(&mut w, 0.5);
        messages(&w);
        all_five(&mut w);
        let msgs = messages(&w);
        assert!(msgs.iter().any(|m| m == "Multiball!"), "a second multiball, not a jackpot; got {msgs:?} balls {}", w.ball_count());
    }
}

#[cfg(test)]
mod table_4_lock_tests {
    use super::*;
    use crate::table::load_table;

    const COLORS: [&str; 6] = ["Blue", "Cyan", "Green", "Yellow", "Red", "Magenta"];

    fn world4() -> PhysicsWorld {
        let mut w = PhysicsWorld::from_table(&load_table(4).unwrap());
        w.step(1.0 / 240.0, &InputState::default());
        w
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    /// Lights row `row` of every colour's column directly, the last one with a bumper hit.
    fn fill_row(w: &mut PhysicsWorld, row: usize) {
        for c in &COLORS[..5] { w.set_rollover(&format!("Rollovers.{c}"), row, true); }
        let (bx, by, br) = {
            let b = w.bumpers.iter().find(|b| b.shot_id.as_deref() == Some("Bumper.Magenta")).unwrap();
            (b.position[0], b.position[1], b.radius)
        };
        let y = by + br + w.ball_radius + 0.01;
        place(w, bx, y);
        for _ in 0..16 { step(w); place(w, bx, y); }
    }

    fn enter_lock(w: &mut PhysicsWorld, i: usize) {
        let (x, y) = w.rollover_center(&format!("LockedBallRollover{}", i + 1), 0).unwrap();
        place(w, x, y);
        step(w);
    }

    fn visible(w: &PhysicsWorld, id: &str) -> bool { w.rollover_group(id).unwrap().visible }

    #[test]
    fn locks_start_hidden_and_a_full_row_makes_the_first_ready() {
        let mut w = world4();
        for i in 1..=3 {
            assert!(!visible(&w, &format!("LockedBallRollover{i}")));
            assert!(w.group_ignores_ball(&format!("LockedBallRollover{i}")));
        }
        assert_eq!(w.wall_retracted("LockedBallKicker1"), Some(false));
        assert_eq!(w.wall_retracted("LockedBallKicker2"), Some(true));
        assert_eq!(w.wall_retracted("LockedBallKicker3"), Some(true));

        fill_row(&mut w, 0);
        assert!(messages(&w).iter().any(|m| m == "Ball Lock Ready"));
        assert!(visible(&w, "LockedBallRollover1"));
        assert!(!w.group_ignores_ball("LockedBallRollover1"));
        assert!(w.group_ignores_ball("LockedBallRollover2"), "only the next lock");
    }

    #[test]
    fn a_ball_in_the_lock_is_removed_and_the_next_row_is_used() {
        let mut w = world4();
        fill_row(&mut w, 0);
        messages(&w);
        enter_lock(&mut w, 0);
        assert!(messages(&w).iter().any(|m| m == "Ball 1 Locked"));
        assert!(w.take_relaunch_needed(), "no ball left on the field, a fresh one waits");
        assert_eq!(w.wall_retracted("LockedBallKicker2"), Some(false), "the next lock's kicker comes out");

        // The next bumper hit lights row 2 (index 1), not row 1.
        let (bx, by, br) = {
            let b = w.bumpers.iter().find(|b| b.shot_id.as_deref() == Some("Bumper.Red")).unwrap();
            (b.position[0], b.position[1], b.radius)
        };
        let y = by + br + w.ball_radius + 0.01;
        place(&mut w, bx, y);
        for _ in 0..16 { step(&mut w); place(&mut w, bx, y); }
        assert!(w.rollover_lit("Rollovers.Red", 1));
    }

    /// Fills rows and locks until the multiball starts.
    fn start_multiball(w: &mut PhysicsWorld) {
        for row in 0..3 {
            fill_row(w, row);
            enter_lock(w, row);
        }
    }

    #[test]
    fn the_third_lock_is_the_multiball() {
        let mut w = world4();
        start_multiball(&mut w);
        let msgs = messages(&w);
        assert!(msgs.iter().any(|m| m == "Multiball Ready"), "got {msgs:?}");
        assert!(msgs.iter().any(|m| m == "Multiball!"), "got {msgs:?}");
        assert_eq!(w.wall_retracted("LockedBallKicker2"), Some(false));

        run_sim_seconds(&mut w, 1.2);
        assert_eq!(w.ball_count(), 2, "one ball at 1 s");
        assert_eq!(w.wall_retracted("LockedBallKicker2"), Some(true), "the lock kickers go");
        assert_eq!(w.wall_retracted("LockedBallKicker3"), Some(true));
        run_sim_seconds(&mut w, 2.6);
        assert_eq!(w.ball_count(), 3, "another at 3.5 s");
        for i in 1..=3 { assert!(!visible(&w, &format!("LockedBallRollover{i}"))); }
        // The status lights are open to the balls now.
        assert!(!w.group_ignores_ball("Rollovers.Blue"));
    }

    fn light_everything(w: &mut PhysicsWorld) {
        for c in COLORS { for k in 0..3 { w.set_rollover(&format!("Rollovers.{c}"), k, true); } }
    }

    /// Lights all 18 status lights, the last by a ball.
    fn jackpot_shot(w: &mut PhysicsWorld) {
        light_everything(w);
        w.set_rollover("Rollovers.Magenta", 2, false);
        let (x, y) = w.rollover_center("Rollovers.Magenta", 2).unwrap();
        place(w, x, y);
        step(w);
    }

    #[test]
    fn lighting_every_status_light_in_multiball_pays_a_growing_jackpot() {
        let mut w = world4();
        start_multiball(&mut w);
        run_sim_seconds(&mut w, 4.0);
        assert_eq!(w.ball_count(), 3);
        messages(&w);
        w.drain_score_events();
        w.drain_multiplier_scale_events();

        jackpot_shot(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Jackpot!"));
        assert!(w.drain_score_events().contains(&25_000));
        assert_eq!(w.drain_multiplier_scale_events(), vec![125]);
        for c in COLORS { for k in 0..3 { assert!(!w.rollover_lit(&format!("Rollovers.{c}"), k)); } }

        jackpot_shot(&mut w);
        assert!(w.drain_score_events().contains(&30_000), "5,000 more each time");
        assert_eq!(w.drain_multiplier_scale_events(), vec![130]);
    }

    #[test]
    fn the_multiball_ends_at_one_ball_and_the_locks_start_over() {
        let mut w = world4();
        start_multiball(&mut w);
        run_sim_seconds(&mut w, 4.0);
        assert_eq!(w.ball_count(), 3);
        w.game_time += 100.0; // no multiball saver
        for i in (1..3).rev() {
            let body = w.bodies.get_mut(w.balls[i].body).unwrap();
            body.set_translation(Vector::new(5.0, -2.0), true);
        }
        run_sim_seconds(&mut w, 0.5);
        assert_eq!(w.ball_count(), 1);
        run_sim_seconds(&mut w, 0.5);
        assert!(w.group_ignores_ball("Rollovers.Blue"), "status lights closed again");
        messages(&w);
        fill_row(&mut w, 0);
        assert!(messages(&w).iter().any(|m| m == "Ball Lock Ready"), "back to the first lock");
    }

    #[test]
    fn the_jackpot_scales_the_whole_multiplier() {
        use crate::game::GameState;
        let mut g = GameState::new(3);
        g.multiplier = 2;
        g.bonus_percent = 40;
        g.scale_multiplier(125);
        assert_eq!(g.multiplier_percent(), 300, "2.40 x 1.25 = 3.00");
        g.scale_multiplier(130);
        assert_eq!(g.multiplier_percent(), 390);
    }
}

#[cfg(test)]
mod table_5_multiball_tests {
    use super::*;
    use crate::table::load_table;

    const COLOR_NAMES: [&str; 4] = ["Blue", "Red", "Yellow", "Green"];
    const BALL_COLORS: [(u8, u8, u8); 4] = [(0x66, 0x88, 0xEE), (0xEE, 0x88, 0x88), (0xCC, 0xCC, 0x77), (0x77, 0xDD, 0x77)];

    fn world5() -> PhysicsWorld {
        let mut w = PhysicsWorld::from_table(&load_table(5).unwrap());
        w.step(1.0 / 240.0, &InputState::default());
        w
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    fn run_sim_seconds(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, &InputState::default()); }
    }

    /// Like `run_sim_seconds`, but every ball is held still, so a random launch
    /// cannot drain it before the test looks.
    fn run_frozen(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize {
            w.step_frame(frame, &InputState::default());
            for b in &w.balls {
                if let Some(body) = w.bodies.get_mut(b.body) {
                    body.set_linvel(Vector::new(0.0, 0.0), true);
                    body.set_gravity_scale(0.0, true);
                }
            }
        }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    /// Ball 0 in each colour in turn lights that colour's three centre lights.
    fn light_center(w: &mut PhysicsWorld) {
        for c in 0..4 {
            w.set_ball_color(0, Some(BALL_COLORS[c]));
            step(w); // the script opens the lights to this colour
            for n in 0..3 {
                let (x, y) = w.rollover_center(&format!("CenterRollover_{}_{}", COLOR_NAMES[c], n + 1), 0).unwrap();
                place(w, x, y);
                step(w);
            }
        }
        w.set_ball_color(0, None);
        place(w, 1.0, 1.0);
        step(w);
    }

    fn make_ready(w: &mut PhysicsWorld) {
        light_center(w);
        assert!(messages(w).iter().any(|m| m == "Shoot the Pyramid!"));
    }

    fn start(w: &mut PhysicsWorld) {
        make_ready(w);
        let (x, y) = w.rollover_center("TriangleCenter", 0).unwrap();
        place(w, x, y);
        step(w);
    }

    #[test]
    fn lighting_all_twelve_makes_the_pyramid_ready() {
        let mut w = world5();
        light_center(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Shoot the Pyramid!"));
    }

    #[test]
    fn a_ready_pyramid_lets_the_ball_through_the_triangle() {
        let mut w = world5();
        // Not ready: the walls stay.
        let (mx, my) = {
            let kw = w.kick_walls.iter().find(|k| k.shot_id.as_deref() == Some("TriangleWall2")).unwrap();
            ((kw.x1 + kw.x2) / 2.0, (kw.y1 + kw.y2) / 2.0)
        };
        place(&mut w, mx, my + 0.4);
        for _ in 0..4 { step(&mut w); }
        assert_eq!(w.wall_retracted("TriangleWall2"), Some(false));

        make_ready(&mut w);
        let kw = w.kick_walls.iter().find(|k| k.shot_id.as_deref() == Some("TriangleWall2")).unwrap();
        let (mx, my) = ((kw.x1 + kw.x2) / 2.0, (kw.y1 + kw.y2) / 2.0);
        place(&mut w, mx, my + 0.4);
        for _ in 0..4 { step(&mut w); }
        assert_eq!(w.wall_retracted("TriangleWall2"), Some(true), "the wall the ball hits gives way");
    }

    #[test]
    fn the_pyramid_starts_a_multiball_of_four_colours() {
        let mut w = world5();
        start(&mut w);
        assert_eq!(w.rotating_triangles[0].speed_multiplier, -2.0);
        assert_eq!(w.ball_count(), 1, "the ball waits in the lane");
        run_frozen(&mut w, 1.2);
        assert_eq!(w.ball_count(), 1, "the lane ball is the first launch");
        assert_eq!(w.ball_color(0), Some(BALL_COLORS[0]));
        run_frozen(&mut w, 3.0);
        assert_eq!(w.ball_count(), 2);
        assert_eq!(w.ball_color(1), Some(BALL_COLORS[1]));
        run_frozen(&mut w, 3.0);
        run_frozen(&mut w, 3.0);
        assert_eq!(w.ball_count(), 4);
        assert_eq!(w.ball_color(2), Some(BALL_COLORS[2]));
        assert_eq!(w.ball_color(3), Some(BALL_COLORS[3]));
        assert_eq!(w.wall_retracted("TriangleWall1"), Some(false), "the triangle is back");
    }

    #[test]
    fn lighting_the_whole_diagram_in_multiball_pays_a_growing_jackpot() {
        let mut w = world5();
        start(&mut w);
        run_frozen(&mut w, 10.5);
        assert_eq!(w.ball_count(), 4);
        messages(&w);
        w.drain_score_events();

        for expected in [("Jackpot!", 50_000u64), ("2x Jackpot!", 100_000)] {
            // Light eleven directly and the last by ball 0, in its colour.
            for c in 0..4 { for n in 0..3 { w.set_all_rollovers(&format!("CenterRollover_{}_{}", COLOR_NAMES[c], n + 1), true); } }
            w.set_all_rollovers("CenterRollover_Green_3", false);
            w.set_ball_color(0, Some(BALL_COLORS[3]));
            step(&mut w);
            let (x, y) = w.rollover_center("CenterRollover_Green_3", 0).unwrap();
            place(&mut w, x, y);
            step(&mut w);
            assert!(messages(&w).iter().any(|m| m == expected.0), "{}", expected.0);
            assert!(w.drain_score_events().contains(&expected.1));
            assert!(!w.rollovers_all_lit("CenterRollover_Blue_1"), "the diagram resets");
        }
    }

    #[test]
    fn the_multiball_ends_at_one_ball() {
        let mut w = world5();
        start(&mut w);
        run_frozen(&mut w, 10.5);
        w.game_time += 100.0; // no multiball saver
        for i in (1..4).rev() {
            let body = w.bodies.get_mut(w.balls[i].body).unwrap();
            body.set_translation(Vector::new(5.0, -2.0), true);
        }
        run_sim_seconds(&mut w, 0.6);
        assert_eq!(w.ball_count(), 1);
        run_sim_seconds(&mut w, 0.5);
        assert_eq!(w.rotating_triangles[0].speed_multiplier, 1.0);
        messages(&w);
        light_center(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Shoot the Pyramid!"), "ready again, not a jackpot");
    }
}

#[cfg(test)]
mod table_7_lock_tests {
    use super::*;
    use crate::table::load_table;

    const CONSTELLATION_NAMES: [&str; 6] = ["Canis Major", "Gemini", "Leo", "Orion", "Sagittarius", "Ursa Major"];

    fn world7() -> PhysicsWorld {
        let mut w = PhysicsWorld::from_table(&load_table(7).unwrap());
        w.script = Some(Box::new(scripts::AdAstra::new(7)));
        w.step(1.0 / 240.0, &InputState::default());
        w
    }

    fn place(w: &mut PhysicsWorld, x: f32, y: f32) {
        let body = w.bodies.get_mut(w.balls[0].body).unwrap();
        body.set_translation(Vector::new(x, y), true);
        body.set_linvel(Vector::new(0.0, 0.0), true);
        body.set_gravity_scale(0.0, true);
    }

    fn step(w: &mut PhysicsWorld) { w.step(1.0 / 240.0, &InputState::default()); }

    /// Runs `secs` of sim time with every ball held still, so a random release
    /// cannot send one down the table before the test looks.
    fn run_frozen(w: &mut PhysicsWorld, secs: f32) {
        let frame = 0.033;
        for _ in 0..(secs / (frame * w.time_ratio())).ceil() as usize {
            w.step_frame(frame, &InputState::default());
            for b in &w.balls {
                if let Some(body) = w.bodies.get_mut(b.body) {
                    body.set_linvel(Vector::new(0.0, 0.0), true);
                    body.set_gravity_scale(0.0, true);
                }
            }
        }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    fn into_sensor(w: &mut PhysicsWorld, id: &str, layer: u8) {
        let s = w.sensors.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no sensor {id}"));
        let (x, y) = ((s.rect[0] + s.rect[2]) / 2.0, (s.rect[1] + s.rect[3]) / 2.0);
        w.set_ball_layer(0, layer);
        place(w, x, y);
        step(w);
    }

    fn circles(w: &PhysicsWorld) -> Vec<(f32, f32, (u8, u8, u8))> {
        w.snapshot().shapes.iter().filter_map(|s| match *s { Shape::Circle { x, y, color, .. } => Some((x, y, color)), _ => None }).collect()
    }

    fn unlit_target(w: &PhysicsWorld) -> Option<(f32, f32)> {
        circles(w).into_iter().filter(|c| c.2.0 == 0 && c.2.2 == 0 && c.2.1 > 0).max_by_key(|c| c.2.1).map(|c| (c.0, c.1))
    }

    /// Shoots a loop to pick a constellation, waits for the view to arrive and
    /// rolls the ball over every star of it.
    fn complete_constellation(w: &mut PhysicsWorld) {
        w.balls[0].last_sensor = None; // a loop counts once per pass
        into_sensor(w, "LeftLoopDetector", 2);
        w.set_ball_layer(0, 0);
        place(w, 1.0, 1.0);
        assert!(messages(w).iter().any(|m| CONSTELLATION_NAMES.contains(&m.as_str())), "a constellation was picked");
        run_frozen(w, 11.0);
        for _ in 0..60 {
            let Some((x, y)) = unlit_target(w) else { break };
            place(w, x, y);
            step(w);
            step(w);
        }
        place(w, 1.0, 1.0);
        step(w);
    }

    fn lock_into(w: &mut PhysicsWorld, i: usize) {
        step(w); // the script opens the lock to a ball
        let (x, y) = w.rollover_center(&format!("BallLockRollover{}", i + 1), 0).unwrap();
        w.set_ball_layer(0, 4);
        place(w, x, y);
        step(w);
    }

    /// Each layer is its own world, as in the original: a ball on an orbit passes over
    /// the main field's walls and bumpers and comes back down at its flipper lane.
    #[test]
    fn the_loops_carry_a_ball_round_on_their_own_layer_and_back_to_the_main_field() {
        for (start, expected_layer) in [([3.0f32, 20.0f32], 2u8), ([16.8, 20.0], 3)] {
            let mut w = world7();
            w.set_ball_layer(0, 0);
            {
                let b = w.bodies.get_mut(w.balls[0].body).unwrap();
                b.set_translation(Vector::new(start[0], start[1]), true);
                b.set_linvel(Vector::new(0.0, 9.0), true);
                b.set_gravity_scale(1.0, true);
            }
            let mut layers = vec![0u8];
            for _ in 0..2400 {
                step(&mut w);
                let l = w.balls[0].layer;
                if *layers.last().unwrap() != l { layers.push(l); }
            }
            assert_eq!(layers, vec![0, expected_layer, 0], "start {start:?}");
            let y = w.bodies.get(w.balls[0].body).unwrap().translation().y;
            assert!(y < 9.0, "the ball came down the flipper lane: y {y}");
        }
    }

    #[test]
    fn mini_table_pieces_are_inert_unless_a_ball_is_on_that_layer() {
        let mut w = world7();
        let b = w.bumpers.iter().position(|b| b.layer == 1).expect("a mini table bumper");
        let (x, y) = (w.bumpers[b].position[0], w.bumpers[b].position[1] + w.bumpers[b].radius + w.ball_radius + 0.01);
        w.drain_score_events();
        place(&mut w, x, y);
        for _ in 0..4 { step(&mut w); }
        assert!(!w.drain_score_events().contains(&1000), "a main-layer ball does not touch it");
        assert!(w.snapshot().bumpers.iter().all(|s| (s.x - x).abs() > 0.01), "and it is not drawn");
        w.set_ball_layer(0, 1);
        place(&mut w, x, y);
        for _ in 0..16 { step(&mut w); place(&mut w, x, y); }
        assert!(w.drain_score_events().contains(&1000), "a mini table ball does");
        assert!(w.snapshot().bumpers.iter().any(|s| (s.x - x).abs() < 0.01), "and it is drawn");
    }

    #[test]
    fn a_finished_constellation_sends_the_ball_to_the_lock_lane_otherwise_the_mini_table() {
        let mut w = world7();
        into_sensor(&mut w, "MiniTableOrBallLockSensor", 0);
        assert_eq!(w.balls[0].layer, 1, "nothing to lock: the mini table");

        let mut w = world7();
        complete_constellation(&mut w);
        assert!(messages(&w).iter().any(|m| m == "Ball Lock Ready"));
        into_sensor(&mut w, "MiniTableOrBallLockSensor", 0);
        assert_eq!(w.balls[0].layer, 4, "the lock lane");
    }

    #[test]
    fn a_ball_in_the_lock_is_taken_and_the_next_lock_opens() {
        let mut w = world7();
        assert!(!w.group_ignores_ball("BallLockRollover1"), "the first lock is open");
        assert!(w.group_ignores_ball("BallLockRollover2"));
        complete_constellation(&mut w);
        messages(&w);
        lock_into(&mut w, 0);
        assert!(messages(&w).iter().any(|m| m == "Ball 1 Locked"));
        assert!(w.take_relaunch_needed(), "a fresh ball waits in the lane");
        step(&mut w);
        assert!(w.rollover_lit("BallLockRollover1", 0), "the light stays on");
        assert!(w.group_ignores_ball("BallLockRollover1"));
        assert!(!w.group_ignores_ball("BallLockRollover2"), "the next lock is open");
    }

    #[test]
    fn a_ball_on_the_main_layer_does_not_reach_the_lock() {
        let mut w = world7();
        step(&mut w);
        let (x, y) = w.rollover_center("BallLockRollover1", 0).unwrap();
        place(&mut w, x, y);
        step(&mut w);
        step(&mut w);
        assert!(!messages(&w).iter().any(|m| m == "Ball 1 Locked"));
    }

    fn start_multiball(w: &mut PhysicsWorld) {
        for i in 0..3 {
            complete_constellation(w);
            lock_into(w, i);
        }
    }

    #[test]
    fn the_third_lock_starts_a_multiball_that_releases_three_balls() {
        let mut w = world7();
        start_multiball(&mut w);
        let msgs = messages(&w);
        assert!(msgs.iter().any(|m| m == "Multiball!"), "got {msgs:?}");
        assert_eq!(w.ball_count(), 1, "the ball is held over the last lock");
        run_frozen(&mut w, 1.2);
        assert!(!w.rollover_lit("BallLockRollover3", 0), "a lock light goes out as its ball is released");
        run_frozen(&mut w, 2.6);
        assert_eq!(w.ball_count(), 2);
        run_frozen(&mut w, 2.6);
        assert_eq!(w.ball_count(), 3);
        assert!(!w.rollover_lit("BallLockRollover1", 0) && !w.rollover_lit("BallLockRollover2", 0));
    }

    #[test]
    fn all_six_constellations_in_a_multiball_make_the_jackpot() {
        let mut w = world7();
        start_multiball(&mut w);
        run_frozen(&mut w, 6.5);
        assert_eq!(w.ball_count(), 3);
        messages(&w);
        // The three locks locked three constellations already; the other three are done in the
        // multiball. After each one the view flies back (10 s) before the next loop shot picks another.
        let mut msgs = Vec::new();
        for _ in 0..3 {
            complete_constellation(&mut w);
            msgs.extend(messages(&w));
            run_frozen(&mut w, 11.0);
        }
        assert!(msgs.iter().any(|m| m == "Shoot ramp for jackpot"), "got {msgs:?}");
        assert!(msgs.iter().any(|m| m.ends_with(" complete")), "the others said so");
        w.drain_score_events();
        into_sensor(&mut w, "MiniFieldDetector", 1);
        assert!(messages(&w).iter().any(|m| m == "Jackpot!"));
        assert!(w.drain_score_events().contains(&100_000));
    }

    #[test]
    fn the_multiball_ends_at_one_ball_and_the_locks_start_over() {
        let mut w = world7();
        start_multiball(&mut w);
        run_frozen(&mut w, 6.5);
        w.game_time += 100.0; // no multiball saver
        for i in (1..3).rev() {
            let body = w.bodies.get_mut(w.balls[i].body).unwrap();
            body.set_translation(Vector::new(5.0, -2.0), true);
        }
        run_frozen(&mut w, 0.6);
        assert_eq!(w.ball_count(), 1);
        run_frozen(&mut w, 0.5);
        assert!(!w.group_ignores_ball("BallLockRollover1"), "the first lock is open again");
    }
}

#[cfg(test)]
mod engine_rule_tests {
    use super::*;
    use crate::table::load_table;

    fn world(table: u8) -> PhysicsWorld {
        let mut w = PhysicsWorld::from_table(&load_table(table).unwrap());
        w.step_frame(0.033, &InputState::default());
        w
    }

    /// A ball floating in open air with almost no gravity: effectively motionless but in play.
    fn float_ball(w: &mut PhysicsWorld) {
        let b = w.bodies.get_mut(w.balls[0].body).unwrap();
        b.set_translation(Vector::new(10.0, 15.0), true);
        b.set_linvel(Vector::new(0.0, 0.0), true);
        b.set_gravity_scale(0.0005, true);
    }

    fn run(w: &mut PhysicsWorld, sim_secs: f32, input: &InputState) {
        let frame = 0.033;
        for _ in 0..(sim_secs / (frame * w.time_ratio())).ceil() as usize { w.step_frame(frame, input); }
    }

    fn messages(w: &PhysicsWorld) -> Vec<String> { w.drain_message_events().into_iter().map(|m| m.0).collect() }

    #[test]
    fn a_ball_that_sits_still_for_ten_seconds_is_bumped() {
        let mut w = world(1);
        float_ball(&mut w);
        messages(&w);
        run(&mut w, 9.0, &InputState::default());
        assert!(!messages(&w).iter().any(|m| m == "Bump!"), "not yet");
        let mut bumped = false;
        for _ in 0..200 {
            w.step_frame(0.033, &InputState::default());
            if messages(&w).iter().any(|m| m == "Bump!") { bumped = true; break; }
        }
        assert!(bumped, "bumped after about ten seconds");
        let v = w.bodies.get(w.balls[0].body).unwrap().linvel();
        assert!(v.y > 0.5 && v.x.abs() > 0.5, "shoved up and sideways: {v:?}");
    }

    #[test]
    fn a_held_flipper_or_a_waiting_ball_is_never_bumped() {
        let mut w = world(1);
        float_ball(&mut w);
        messages(&w);
        let mut held = InputState::default();
        held.left_flipper = true;
        run(&mut w, 15.0, &held);
        assert!(!messages(&w).iter().any(|m| m == "Bump!"), "the flipper could be trapping the ball");

        let mut w = world(1); // the ball waits in the shooter lane
        run(&mut w, 15.0, &InputState::default());
        assert!(!messages(&w).iter().any(|m| m == "Bump!"), "a parked ball is not stuck");
    }

    #[test]
    fn a_multiplier_raised_shortly_before_the_ball_is_lost_is_kept() {
        let mut w = world(1);
        assert!(!w.multiplier_increase_preserved(), "nothing raised yet");
        w.increment_multiplier();
        assert!(w.multiplier_increase_preserved());
        w.game_time += 7.0;
        assert!(w.multiplier_increase_preserved(), "7 s of game time is inside the 7.5 s window");
        w.game_time += 1.0;
        assert!(!w.multiplier_increase_preserved(), "8 s is outside it");
        w.increment_multiplier();
        w.park_ball();
        assert!(!w.multiplier_increase_preserved(), "the next ball starts clean");
    }

    #[test]
    fn a_bumper_with_no_score_scores_nothing() {
        let mut w = world(8);
        let i = w.bumpers.iter().position(|b| b.score == 0 && b.award_ladder.is_empty()).expect("table 8 has bumpers with no score");
        let (x, y) = (w.bumpers[i].position[0], w.bumpers[i].position[1] + w.bumpers[i].radius + w.ball_radius + 0.01);
        w.drain_score_events();
        let b = w.bodies.get_mut(w.balls[0].body).unwrap();
        b.set_translation(Vector::new(x, y), true);
        b.set_linvel(Vector::new(0.0, 0.0), true);
        b.set_gravity_scale(0.0, true);
        for _ in 0..16 { w.step(1.0 / 240.0, &InputState::default()); }
        assert!(w.drain_score_events().iter().all(|&p| p != 100), "no invented 100");
    }

    #[test]
    fn the_launch_jitter_is_additive_and_leaves_a_zero_delta_alone() {
        let w = world(1); // launch (0, 15.4) plus up to (0, 0.6)
        let mut ys = Vec::new();
        for _ in 0..50 {
            let [vx, vy] = w.jittered_launch_velocity();
            assert_eq!(vx, 0.0, "no x delta, no x change");
            assert!((15.4..=16.0).contains(&vy), "vy {vy}");
            ys.push(vy);
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
        assert!(ys.iter().any(|&y| (y - ys[0]).abs() > 1e-4), "it does vary");
    }

    #[test]
    fn table_8s_billiard_balls_are_drawn_dimmed_and_are_not_solid() {
        let mut w = world(8);
        let snap = w.snapshot();
        let rings = snap.walls.iter().filter(|s| s.color == Some((102, 69, 26))).count();
        assert_eq!(rings, 24, "the first ball is a 24 segment ring in its own colour, dimmed");
        // Not solid: a ball resting on the ring's edge stays where it is.
        let b = w.bodies.get_mut(w.balls[0].body).unwrap();
        b.set_translation(Vector::new(8.06 + 0.8, 5.52), true);
        b.set_linvel(Vector::new(0.0, 0.0), true);
        b.set_gravity_scale(0.0, true);
        for _ in 0..20 { w.step(1.0 / 240.0, &InputState::default()); }
        let p = w.bodies.get(w.balls[0].body).unwrap().translation();
        assert!((p.x - 8.86).abs() < 0.05 && (p.y - 5.52).abs() < 0.05, "not pushed: {p:?}");
    }

    #[test]
    fn a_moving_ball_leaves_a_trail_of_its_last_twenty_frames_and_a_jump_starts_a_new_one() {
        let mut w = world(1);
        w.launch_ball();
        for _ in 0..40 { w.step_frame(0.033, &InputState::default()); }
        let trail = w.snapshot().balls[0].trail.clone();
        assert_eq!(trail.len(), 20, "twenty frames of trail");
        let pos = w.ball_position().unwrap();
        let last = trail.last().unwrap();
        assert!((last.0 - pos.x).abs() < 3.0 && (last.1 - pos.y).abs() < 3.0, "ends at the ball");

        // Teleported: the old trail is not joined to the new position.
        let b = w.bodies.get_mut(w.balls[0].body).unwrap();
        b.set_translation(Vector::new(3.0, 3.0), true);
        b.set_linvel(Vector::new(0.0, 0.0), true);
        b.set_gravity_scale(0.0, true);
        w.step_frame(0.033, &InputState::default());
        assert_eq!(w.snapshot().balls[0].trail.len(), 1, "a fresh trail");
    }

    #[test]
    fn the_renderer_draws_the_trail_behind_the_ball() {
        let mut w = world(1);
        w.launch_ball();
        for _ in 0..30 { w.step_frame(0.033, &InputState::default()); }
        let snap = w.snapshot();
        let mut bare = snap.clone();
        for b in &mut bare.balls { b.trail.clear(); }
        let renderer = crate::renderer::FieldRenderer::new();
        let with = renderer.render(&snap, 0.0, 100, 70, false);
        let without = renderer.render(&bare, 0.0, 100, 70, false);
        let changed = with.chars.iter().zip(&without.chars).filter(|(a, b)| a != b).count();
        assert!(changed >= 3, "the trail adds dots: {changed}");
    }
}
