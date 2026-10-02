/// Strongly-typed mirror of the Vector-Pinball JSON table format.
///
/// The original Java app stores most numeric values as JSON strings
/// (e.g. `"0.5"`) but occasionally as bare numbers.  The `NumStr`
/// helper below handles both transparently.
use serde::{Deserialize, Deserializer};

// ---------------------------------------------------------------------------
// Helpers for the mixed string/number format used in the JSON files
// ---------------------------------------------------------------------------

/// Deserialise a field that may be either a JSON string or a JSON number.
fn de_numstr<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumStr { Num(f64), Str(String) }
    match NumStr::deserialize(d)? {
        NumStr::Num(n) => Ok(n as f32),
        NumStr::Str(s) => s.trim().parse::<f32>().map_err(serde::de::Error::custom),
    }
}

fn de_numstr_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumStr { Num(f64), Str(String) }
    match Option::<NumStr>::deserialize(d)? {
        None => Ok(None),
        Some(NumStr::Num(n)) => Ok(Some(n as f32)),
        Some(NumStr::Str(s)) => s.trim().parse::<f32>().map(Some)
            .map_err(serde::de::Error::custom),
    }
}

/// A `[x, y]` pair that accepts either strings or numbers in each slot.
fn de_xy<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 2], D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(d)?;
    if raw.len() != 2 { return Err(serde::de::Error::custom("expected [x,y]")); }
    let parse = |v: &serde_json::Value| -> Result<f32, _> {
        match v {
            serde_json::Value::Number(n) => Ok(n.as_f64().unwrap() as f32),
            serde_json::Value::String(s) => s.trim().parse::<f32>()
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("expected number or string")),
        }
    };
    Ok([parse(&raw[0])?, parse(&raw[1])?])
}

fn de_xy_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[f32; 2]>, D::Error> {
    Ok(match Option::<serde_json::Value>::deserialize(d)? {
        None => None,
        Some(v) => Some(de_xy(v).map_err(serde::de::Error::custom)?),
    })
}

/// A `[x1,y1,x2,y2]` quad that accepts mixed strings/numbers.
fn de_xxyy<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 4], D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(d)?;
    if raw.len() != 4 { return Err(serde::de::Error::custom("expected [x1,y1,x2,y2]")); }
    let parse = |v: &serde_json::Value| -> Result<f32, _> {
        match v {
            serde_json::Value::Number(n) => Ok(n.as_f64().unwrap() as f32),
            serde_json::Value::String(s) => s.trim().parse::<f32>()
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("expected number or string")),
        }
    };
    Ok([parse(&raw[0])?, parse(&raw[1])?, parse(&raw[2])?, parse(&raw[3])?])
}

fn de_xxyy_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[f32; 4]>, D::Error> {
    Ok(match Option::<serde_json::Value>::deserialize(d)? {
        None => None,
        Some(v) => Some(de_xxyy(v).map_err(serde::de::Error::custom)?),
    })
}

/// Vector-Pinball's `layer` property is a raw Box2D-world key and can be
/// negative (table 8 uses `-1` for purely decorative arcs/rollovers that no
/// ball ever occupies). Our layer system only assigns balls to layers 0-3,
/// so any negative value is mapped to this unreachable sentinel — the
/// element is built (matching the original always creating a physics body)
/// but, since no ball is ever on layer 31, it never collides and the
/// per-layer renderer (which only draws `Some(n)` when `n == ball_layer`)
/// never shows it.
const INERT_LAYER: u8 = 31;

fn de_layer_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u8>, D::Error> {
    Ok(match Option::<i64>::deserialize(d)? {
        None => None,
        Some(n) if n < 0 => Some(INERT_LAYER),
        Some(n) => Some(n as u8),
    })
}

fn de_vec_xy<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[f32; 2]>, D::Error> {
    let rows: Vec<Vec<serde_json::Value>> = Vec::deserialize(d)?;
    rows.iter().map(|row| {
        if row.len() != 2 { return Err(serde::de::Error::custom("expected [x,y]")); }
        let parse = |v: &serde_json::Value| -> Result<f32, _> {
            match v {
                serde_json::Value::Number(n) => Ok(n.as_f64().unwrap() as f32),
                serde_json::Value::String(s) => s.trim().parse::<f32>()
                    .map_err(serde::de::Error::custom),
                _ => Err(serde::de::Error::custom("expected number or string")),
            }
        };
        Ok([parse(&row[0])?, parse(&row[1])?])
    }).collect()
}

fn de_vec_xxyy<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[f32; 4]>, D::Error> {
    let rows: Vec<Vec<serde_json::Value>> = Vec::deserialize(d)?;
    rows.iter().map(|row| {
        if row.len() != 4 { return Err(serde::de::Error::custom("expected [x1,y1,x2,y2]")); }
        let parse = |v: &serde_json::Value| -> Result<f32, _> {
            match v {
                serde_json::Value::Number(n) => Ok(n.as_f64().unwrap() as f32),
                serde_json::Value::String(s) => s.trim().parse::<f32>()
                    .map_err(serde::de::Error::custom),
                _ => Err(serde::de::Error::custom("expected number or string")),
            }
        };
        Ok([parse(&row[0])?, parse(&row[1])?, parse(&row[2])?, parse(&row[3])?])
    }).collect()
}

// ---------------------------------------------------------------------------
// Top-level table definition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct TableDef {
    pub name: Option<String>,
    pub delegate: Option<String>,
    pub version: Option<u32>,

    /// Named numbers a table's delegate reads (e.g. table 2's rotating-bumper
    /// speeds). Values may be numbers or numeric strings.
    #[serde(default)]
    pub variables: std::collections::HashMap<String, serde_json::Value>,

    #[serde(deserialize_with = "de_numstr")]
    pub width: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub height: f32,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub gravity: Option<f32>,

    /// Sim seconds advanced per real second. The original Vector Pinball runs
    /// every table at ~2.3x; without it the ball feels like it is in water.
    #[serde(rename = "targetTimeRatio", default, deserialize_with = "de_numstr_opt")]
    pub target_time_ratio: Option<f32>,

    #[serde(default)]
    pub numballs: Option<u8>,

    #[serde(rename = "ballcolor")]
    pub ball_color: Option<[u8; 3]>,

    #[serde(rename = "ballradius", default, deserialize_with = "de_numstr_opt")]
    pub ball_radius: Option<f32>,

    #[serde(rename = "launchPosition", default, deserialize_with = "de_xy_opt")]
    pub launch_position: Option<[f32; 2]>,

    #[serde(rename = "launchVelocity", default, deserialize_with = "de_xy_opt")]
    pub launch_velocity: Option<[f32; 2]>,

    #[serde(rename = "launchVelocityRandomDelta", default, deserialize_with = "de_xy_opt")]
    pub launch_velocity_random_delta: Option<[f32; 2]>,

    #[serde(rename = "launchDeadZone", default, deserialize_with = "de_xxyy_opt")]
    pub launch_dead_zone: Option<[f32; 4]>,

    pub elements: Vec<ElementDef>,

    /// Not part of Vector-Pinball's format: an Asciiverse-original extension
    /// (currently only table 9 / Party Land) that spells a word by lighting
    /// one letter per named shot in `shots`, awarding `score` bonus points
    /// when all letters are lit, then resetting for replay.
    #[serde(rename = "letterAwards", default)]
    pub letter_awards: Vec<LetterAwardDef>,

    /// Not part of Vector-Pinball's format: an Asciiverse-original extension
    /// (currently only table 9 / Party Land). Lights once every word in
    /// `letter_awards` has been completed at least once since the ball
    /// started; the next hit on `shot` then collects `score` and un-lights
    /// it. Requires `letter_awards` to be non-empty to ever light.
    #[serde(default)]
    pub jackpot: Option<JackpotDef>,
}

/// Matches the ratio the original Vector Pinball tables use (2.25 to 2.3).
pub const DEFAULT_TIME_RATIO: f32 = 2.3;

impl TableDef {
    pub fn variable(&self, name: &str) -> Option<f32> {
        match self.variables.get(name)? {
            serde_json::Value::Number(n) => n.as_f64().map(|v| v as f32),
            serde_json::Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    pub fn ball_radius(&self) -> f32 { self.ball_radius.unwrap_or(0.5) }
    pub fn gravity_magnitude(&self) -> f32 { self.gravity.unwrap_or(4.0) }
    pub fn time_ratio(&self) -> f32 { self.target_time_ratio.unwrap_or(DEFAULT_TIME_RATIO) }
    pub fn num_balls(&self) -> u8 { self.numballs.unwrap_or(3) }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JackpotDef {
    /// Element `id` of the shot that collects the jackpot once lit.
    pub shot: String,
    pub score: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LetterAwardDef {
    pub word: String,
    /// Element `id`s, in letter order; hitting the shot with a given id
    /// lights the corresponding letter, in any order.
    pub shots: Vec<String>,
    #[serde(default)]
    pub score: Option<u64>,
}

// ---------------------------------------------------------------------------
// Element definitions — one variant per Java class
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "class")]
pub enum ElementDef {
    WallElement(WallDef),
    WallArcElement(WallArcDef),
    WallPathElement(WallPathDef),
    BumperElement(BumperDef),
    FlipperElement(FlipperDef),
    RolloverGroupElement(RolloverGroupDef),
    DropTargetGroupElement(DropTargetGroupDef),
    SensorElement(SensorDef),
    SpinnerElement(SpinnerDef),
    // Unknown element classes are silently skipped
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// WallElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct WallDef {
    /// [x1, y1, x2, y2]
    #[serde(default, deserialize_with = "de_xxyy_opt")]
    pub position: Option<[f32; 4]>,

    pub id: Option<String>,
    pub color: Option<serde_json::Value>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub kick: Option<f32>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub restitution: Option<f32>,

    pub kill: Option<bool>,
    pub disabled: Option<bool>,

    #[serde(rename = "retractWhenHit")]
    pub retract_when_hit: Option<bool>,

    /// Drawn (when the table's script shows it) but has no body: the ball
    /// passes straight through, e.g. table 5's centre-diagram lines.
    #[serde(rename = "ignoreBall", default)]
    pub ignore_ball: Option<bool>,

    pub score: Option<u64>,

    /// Physics/render layer (0 = main field; 1/2/3 = elevated ramps/orbits).
    /// None means universal — active on all layers.
    #[serde(default, deserialize_with = "de_layer_opt")]
    pub layer: Option<u8>,
}

// ---------------------------------------------------------------------------
// WallArcElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct WallArcDef {
    pub id: Option<String>,

    /// No body: the ball passes through (see `WallDef::ignore_ball`).
    #[serde(rename = "ignoreBall", default)]
    pub ignore_ball: Option<bool>,

    #[serde(deserialize_with = "de_xy")]
    pub center: [f32; 2],

    // Elliptical arcs supply xradius + yradius; circular arcs supply radius.
    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub xradius: Option<f32>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub yradius: Option<f32>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub radius: Option<f32>,

    #[serde(deserialize_with = "de_numstr")]
    pub minangle: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub maxangle: f32,

    pub segments: Option<u32>,
    pub color: Option<serde_json::Value>,

    #[serde(default, deserialize_with = "de_layer_opt")]
    pub layer: Option<u8>,

    #[serde(rename = "inactiveLayerColor")]
    pub inactive_layer_color: Option<serde_json::Value>,
}

impl WallArcDef {
    fn xr(&self) -> f32 { self.xradius.or(self.radius).unwrap_or(1.0) }
    fn yr(&self) -> f32 { self.yradius.or(self.radius).unwrap_or(1.0) }

    /// Generate the line-segment endpoints that approximate this arc.
    pub fn to_segments(&self) -> Vec<([f32; 2], [f32; 2])> {
        let n = self.segments.unwrap_or(16) as usize;
        let min_rad = self.minangle.to_radians();
        let max_rad = self.maxangle.to_radians();
        let step = (max_rad - min_rad) / n as f32;
        let pts: Vec<[f32; 2]> = (0..=n).map(|i| {
            let a = min_rad + step * i as f32;
            [
                self.center[0] + self.xr() * a.cos(),
                self.center[1] + self.yr() * a.sin(),
            ]
        }).collect();
        pts.windows(2).map(|w| (w[0], w[1])).collect()
    }
}

// ---------------------------------------------------------------------------
// WallPathElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct WallPathDef {
    pub id: Option<String>,

    /// No body: the ball passes through (see `WallDef::ignore_ball`).
    #[serde(rename = "ignoreBall", default)]
    pub ignore_ball: Option<bool>,

    #[serde(deserialize_with = "de_vec_xy")]
    pub positions: Vec<[f32; 2]>,
    pub color: Option<serde_json::Value>,

    #[serde(default, deserialize_with = "de_layer_opt")]
    pub layer: Option<u8>,

    #[serde(rename = "inactiveLayerColor")]
    pub inactive_layer_color: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// BumperElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct BumperDef {
    pub id: Option<String>,

    #[serde(deserialize_with = "de_xy")]
    pub position: [f32; 2],

    #[serde(deserialize_with = "de_numstr")]
    pub radius: f32,

    #[serde(rename = "outerRadius", default, deserialize_with = "de_numstr_opt")]
    pub outer_radius: Option<f32>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub kick: Option<f32>,

    pub color: Option<serde_json::Value>,

    #[serde(rename = "outerColor")]
    pub outer_color: Option<serde_json::Value>,

    pub score: Option<u64>,

    /// Not part of Vector-Pinball's format: an Asciiverse-original extension
    /// (currently only Party Land's Ghoul Skull) for an "advance the award"
    /// bumper. When present, each hit awards `award_ladder[rung]` instead of
    /// the flat `score` above, then advances `rung` (capping at the last
    /// entry, where it holds for the rest of the ball). Resets to the first
    /// rung on every new ball.
    #[serde(rename = "awardLadder", default)]
    pub award_ladder: Option<Vec<u64>>,

    /// Not part of Vector-Pinball's format: an Asciiverse-original extension
    /// (currently only Ghoul Skull). Only meaningful alongside `awardLadder`
    /// — the first hit that reaches the ladder's last rung (once per ball)
    /// starts a timed super mode.
    #[serde(rename = "triggersSuperMode", default)]
    pub triggers_super_mode: bool,
}

// ---------------------------------------------------------------------------
// FlipperElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct FlipperDef {
    pub id: Option<String>,

    /// Pivot point [x, y]
    #[serde(deserialize_with = "de_xy")]
    pub position: [f32; 2],

    /// Negative length means the pivot is at the right end (right flipper)
    #[serde(deserialize_with = "de_numstr")]
    pub length: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub minangle: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub maxangle: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub upspeed: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub downspeed: f32,
}

impl FlipperDef {
    pub fn is_right_flipper(&self) -> bool { self.length < 0.0 }
    pub fn abs_length(&self) -> f32 { self.length.abs() }
}

// ---------------------------------------------------------------------------
// RolloverGroupElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct RolloverGroupDef {
    pub id: Option<String>,
    pub rollovers: Vec<RolloverDef>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub radius: Option<f32>,

    #[serde(rename = "cycleOnFlipper")]
    pub cycle_on_flipper: Option<bool>,

    #[serde(rename = "ignoreBall")]
    pub ignore_ball: Option<bool>,

    pub color: Option<serde_json::Value>,
    pub score: Option<u64>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub reset: Option<f32>,

    /// Not part of Vector-Pinball's format: an Asciiverse-original extension
    /// (currently only Party Land's Puke Rollovers) — raises the game's
    /// score multiplier by this amount each time the group is completed
    /// (all rollovers lit), on top of the group's normal completion bonus.
    #[serde(rename = "multiplierStep", default)]
    pub multiplier_step: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RolloverDef {
    #[serde(deserialize_with = "de_xy")]
    pub position: [f32; 2],

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub radius: Option<f32>,

    pub color: Option<serde_json::Value>,
    pub score: Option<u64>,
}

// ---------------------------------------------------------------------------
// DropTargetGroupElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct DropTargetGroupDef {
    pub id: Option<String>,
    pub color: Option<serde_json::Value>,
    pub score: Option<u64>,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub reset: Option<f32>,

    /// Each entry is [x1, y1, x2, y2] for one target segment
    #[serde(default, deserialize_with = "de_vec_xxyy")]
    pub positions: Vec<[f32; 4]>,

    // Alternative to `positions` (used when it is empty or missing): lay
    // `numTargets` targets along a wall, as in the original. See `targets()`.
    #[serde(rename = "wallStart", default, deserialize_with = "de_xy_opt")]
    pub wall_start: Option<[f32; 2]>,
    /// Only gives the wall's direction.
    #[serde(rename = "wallEnd", default, deserialize_with = "de_xy_opt")]
    pub wall_end: Option<[f32; 2]>,
    /// Perpendicular distance from the wall to the targets; negative flips sides.
    #[serde(rename = "gapFromWall", default, deserialize_with = "de_numstr_opt")]
    pub gap_from_wall: Option<f32>,
    /// Distance from `wallStart` to the first target.
    #[serde(rename = "startDistanceAlongWall", default, deserialize_with = "de_numstr_opt")]
    pub start_distance_along_wall: Option<f32>,
    #[serde(rename = "targetWidth", default, deserialize_with = "de_numstr_opt")]
    pub target_width: Option<f32>,
    #[serde(rename = "gapBetweenTargets", default, deserialize_with = "de_numstr_opt")]
    pub gap_between_targets: Option<f32>,
    #[serde(rename = "numTargets", default, deserialize_with = "de_numstr_opt")]
    pub num_targets: Option<f32>,
}

impl DropTargetGroupDef {
    /// The target segments [x1, y1, x2, y2]: the explicit `positions` if there
    /// are any, otherwise generated along the wall exactly like the original's
    /// DropTargetGroupElement (target i spans `start + i * (width + gap)` to
    /// `+ width` along the wall, offset `gapFromWall` perpendicular to it).
    pub fn targets(&self) -> Vec<[f32; 4]> {
        if !self.positions.is_empty() {
            return self.positions.clone();
        }
        let (Some([sx, sy]), Some([ex, ey])) = (self.wall_start, self.wall_end) else { return Vec::new() };
        let n = self.num_targets.unwrap_or(0.0).max(0.0) as usize;
        let wall_angle = (ey - sy).atan2(ex - sx);
        let perp = wall_angle + std::f32::consts::FRAC_PI_2;
        let gap = self.gap_from_wall.unwrap_or(0.0);
        let (gx, gy) = (gap * perp.cos(), gap * perp.sin());
        let width = self.target_width.unwrap_or(0.0);
        let step = width + self.gap_between_targets.unwrap_or(0.0);
        let start = self.start_distance_along_wall.unwrap_or(0.0);
        (0..n).map(|i| {
            let a0 = start + i as f32 * step;
            let a1 = a0 + width;
            [
                sx + a0 * wall_angle.cos() + gx, sy + a0 * wall_angle.sin() + gy,
                sx + a1 * wall_angle.cos() + gx, sy + a1 * wall_angle.sin() + gy,
            ]
        }).collect()
    }
}

// ---------------------------------------------------------------------------
// SensorElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct SensorDef {
    pub id: Option<String>,

    /// [xmin, ymin, xmax, ymax]
    #[serde(default, deserialize_with = "de_xxyy_opt")]
    pub rect: Option<[f32; 4]>,

    /// When ball enters this sensor, switch it to this layer.
    #[serde(rename = "ballLayer")]
    pub ball_layer: Option<u8>,

    /// Only trigger the layer switch if ball is currently on this layer.
    #[serde(rename = "ballLayerFrom")]
    pub ball_layer_from: Option<u8>,
}

// ---------------------------------------------------------------------------
// SpinnerElement
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Deserialize)]
pub struct SpinnerDef {
    pub id: Option<String>,

    #[serde(deserialize_with = "de_xy")]
    pub position: [f32; 2],

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub radius: Option<f32>,

    pub color: Option<serde_json::Value>,
    pub score: Option<u64>,

    #[serde(rename = "minActivationSpeed", default, deserialize_with = "de_numstr_opt")]
    pub min_activation_speed: Option<f32>,

    #[serde(rename = "baseCyclesPerSecond", default, deserialize_with = "de_numstr_opt")]
    pub base_cycles_per_second: Option<f32>,

    #[serde(rename = "minCyclesPerSecond", default, deserialize_with = "de_numstr_opt")]
    pub min_cycles_per_second: Option<f32>,

    #[serde(rename = "speedDecayRate", default, deserialize_with = "de_numstr_opt")]
    pub speed_decay_rate: Option<f32>,
}
