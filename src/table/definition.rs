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

    #[serde(deserialize_with = "de_numstr")]
    pub width: f32,

    #[serde(deserialize_with = "de_numstr")]
    pub height: f32,

    #[serde(default, deserialize_with = "de_numstr_opt")]
    pub gravity: Option<f32>,

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
}

impl TableDef {
    pub fn ball_radius(&self) -> f32 { self.ball_radius.unwrap_or(0.5) }
    pub fn gravity_magnitude(&self) -> f32 { self.gravity.unwrap_or(4.0) }
    pub fn num_balls(&self) -> u8 { self.numballs.unwrap_or(3) }
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
