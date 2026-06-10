/// Renders the pinball field into colored braille characters.
///
/// Coordinate system
/// -----------------
/// Game space:   origin bottom-left, y up.
/// Canvas space: origin top-left,    y down  (standard raster).
///
/// `to_cx` / `to_cy` return signed i32 so that elements outside the current
/// viewport produce negative or oversized values.  Every pixel write goes
/// through `set_pixel`, which silently ignores out-of-bounds coordinates —
/// this prevents off-screen elements from being clamped to the screen edge.

use ratatui::style::Color;
use crate::physics::FieldSnapshot;

// ── Palette ───────────────────────────────────────────────────────────────────

const COL_WALL:        Color = Color::Gray;
const COL_KICK_WALL:   Color = Color::LightGreen;
const COL_DROP_TARGET: Color = Color::Yellow;
const COL_BUMPER:      Color = Color::Blue;
const COL_BUMPER_LIT:  Color = Color::LightYellow;
const COL_FLIPPER:     Color = Color::Cyan;
const COL_BALL:        Color = Color::White;
const COL_BACKGROUND:  Color = Color::DarkGray;

// Layer-specific wall colors.  Index = layer number.
const COL_LAYER: [Color; 4] = [
    Color::Gray,           // layer 0 — main field
    Color::Green,          // layer 1 — left ramp
    Color::Magenta,        // layer 2 — outer orbit
    Color::Red,            // layer 3 — right ramp
];

// ── Public output type ────────────────────────────────────────────────────────

pub struct ColoredFrame {
    pub char_cols: u32,
    pub char_rows: u32,
    pub chars:  Vec<char>,
    pub colors: Vec<Color>,
}

// ── Renderer ──────────────────────────────────────────────────────────────────

pub struct FieldRenderer;

impl FieldRenderer {
    pub fn new() -> Self { Self }

    pub fn render(
        &self,
        snap: &FieldSnapshot,
        camera_y: f32,
        char_cols: u16,
        char_rows: u16,
    ) -> ColoredFrame {
        let char_cols = char_cols as u32;
        let char_rows = char_rows as u32;
        let dot_w = char_cols * 2;
        let dot_h = char_rows * 4;

        if dot_w == 0 || dot_h == 0 {
            return ColoredFrame { char_cols, char_rows, chars: Vec::new(), colors: Vec::new() };
        }

        let mut pixels = PixelBuf::new(dot_w, dot_h);
        let mut grid   = ColorGrid::new(char_cols, char_rows);

        let field_w    = snap.field_width;
        let viewport_h = field_w * (dot_h as f32 / dot_w as f32);

        // Signed coordinate helpers — no clamping; out-of-bounds dots are
        // dropped inside set_pixel without affecting on-screen elements.
        let to_cx = |gx: f32| -> i32 { ((gx / field_w) * dot_w as f32) as i32 };
        let to_cy = |gy: f32| -> i32 {
            let norm    = (gy - camera_y) / viewport_h;
            let flipped = 1.0 - norm;
            (flipped * dot_h as f32) as i32
        };

        // Draw in priority order (later = higher priority overwrites color).

        for w in &snap.walls {
            let color = match w.layer {
                None    => COL_WALL,
                Some(n) if n == snap.ball_layer => COL_LAYER[n.min(3) as usize],
                Some(_) => continue,  // inactive layer — not visible from below
            };
            draw_line(&mut pixels, &mut grid,
                to_cx(w.x1), to_cy(w.y1), to_cx(w.x2), to_cy(w.y2),
                color);
        }

        for kw in &snap.kick_walls {
            draw_line(&mut pixels, &mut grid,
                to_cx(kw.x1), to_cy(kw.y1), to_cx(kw.x2), to_cy(kw.y2),
                COL_KICK_WALL);
        }

        for dt in &snap.drop_targets {
            draw_line(&mut pixels, &mut grid,
                to_cx(dt.x1), to_cy(dt.y1), to_cx(dt.x2), to_cy(dt.y2),
                COL_DROP_TARGET);
        }

        for b in &snap.bumpers {
            let color = if b.lit { COL_BUMPER_LIT } else { COL_BUMPER };
            let cx = to_cx(b.x);
            let cy = to_cy(b.y);
            if let Some(outer) = b.outer_radius {
                let rx = (outer / field_w    * dot_w as f32) as u32;
                let ry = (outer / viewport_h * dot_h as f32) as u32;
                draw_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, color);
            }
            let rx = (b.radius / field_w    * dot_w as f32) as u32;
            let ry = (b.radius / viewport_h * dot_h as f32) as u32;
            if b.lit {
                fill_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, color);
            } else {
                draw_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, color);
            }
        }

        for fl in &snap.flippers {
            draw_line(&mut pixels, &mut grid,
                to_cx(fl.x1), to_cy(fl.y1), to_cx(fl.x2), to_cy(fl.y2),
                COL_FLIPPER);
        }

        if let Some(b) = &snap.ball {
            let rx = ((snap.field_width * 0.025 / field_w)    * dot_w as f32) as u32;
            let ry = ((snap.field_width * 0.025 / viewport_h) * dot_h as f32) as u32;
            fill_ellipse(&mut pixels, &mut grid,
                to_cx(b.x), to_cy(b.y), rx.max(1), ry.max(1), COL_BALL);
        }

        // Build output.
        let total = (char_cols * char_rows) as usize;
        let mut chars  = Vec::with_capacity(total);
        let mut colors = Vec::with_capacity(total);
        for cy in 0..char_rows {
            for cx in 0..char_cols {
                chars.push(pixels.braille_char(cx, cy));
                colors.push(grid.get(cx, cy));
            }
        }

        ColoredFrame { char_cols, char_rows, chars, colors }
    }
}

// ── Pixel buffer ──────────────────────────────────────────────────────────────

struct PixelBuf { data: Vec<bool>, width: u32, height: u32 }

impl PixelBuf {
    fn new(w: u32, h: u32) -> Self {
        Self { data: vec![false; (w * h) as usize], width: w, height: h }
    }

    /// Braille dot → bit mapping (Unicode standard):
    ///   col 0  col 1
    ///   dot1   dot4   → bit 0, 3
    ///   dot2   dot5   → bit 1, 4
    ///   dot3   dot6   → bit 2, 5
    ///   dot7   dot8   → bit 6, 7
    fn braille_char(&self, char_x: u32, char_y: u32) -> char {
        let dx = char_x * 2;
        let dy = char_y * 4;
        let b = |ox: u32, oy: u32| -> u32 {
            if dx + ox < self.width && dy + oy < self.height {
                self.data[((dy + oy) * self.width + (dx + ox)) as usize] as u32
            } else { 0 }
        };
        let bits = b(0,0)      | b(0,1) << 1 | b(0,2) << 2
                 | b(1,0) << 3 | b(1,1) << 4 | b(1,2) << 5
                 | b(0,3) << 6 | b(1,3) << 7;
        char::from_u32(0x2800 + bits).unwrap_or('⠀')
    }
}

// ── Color grid ────────────────────────────────────────────────────────────────

struct ColorGrid { data: Vec<Color>, cols: u32 }

impl ColorGrid {
    fn new(cols: u32, rows: u32) -> Self {
        Self { data: vec![COL_BACKGROUND; (cols * rows) as usize], cols }
    }
    fn get(&self, cx: u32, cy: u32) -> Color {
        let idx = (cy * self.cols + cx) as usize;
        if idx < self.data.len() { self.data[idx] } else { COL_BACKGROUND }
    }
}

// ── Core pixel writer (handles bounds, paints both buf and color grid) ────────

#[inline]
fn set_pixel(pixels: &mut PixelBuf, grid: &mut ColorGrid, x: i32, y: i32, color: Color) {
    if x < 0 || y < 0 { return; }
    let (ux, uy) = (x as u32, y as u32);
    if ux >= pixels.width || uy >= pixels.height { return; }
    pixels.data[(uy * pixels.width + ux) as usize] = true;
    let idx = (uy / 4 * grid.cols + ux / 2) as usize;
    if idx < grid.data.len() {
        grid.data[idx] = color;
    }
}

// ── Drawing primitives ────────────────────────────────────────────────────────

fn draw_line(
    pixels: &mut PixelBuf, grid: &mut ColorGrid,
    x0: i32, y0: i32, x1: i32, y1: i32, color: Color,
) {
    let (mut x, mut y) = (x0, y0);
    let dx =  (x1 - x).abs();
    let dy = -(y1 - y).abs();
    let sx = if x < x1 { 1_i32 } else { -1 };
    let sy = if y < y1 { 1_i32 } else { -1 };
    let mut err = dx + dy;
    loop {
        set_pixel(pixels, grid, x, y, color);
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
}

fn draw_ellipse(
    pixels: &mut PixelBuf, grid: &mut ColorGrid,
    cx: i32, cy: i32, rx: u32, ry: u32, color: Color,
) {
    if rx == 0 || ry == 0 { return; }
    let (rx, ry) = (rx as i32, ry as i32);
    let steps = (rx.max(ry) * 8).max(16);
    let mut prev: Option<(i32, i32)> = None;
    for i in 0..=steps {
        let t = std::f64::consts::TAU * i as f64 / steps as f64;
        let x = cx + (rx as f64 * t.cos()) as i32;
        let y = cy + (ry as f64 * t.sin()) as i32;
        if let Some((px, py)) = prev {
            draw_line(pixels, grid, px, py, x, y, color);
        }
        prev = Some((x, y));
    }
}

fn fill_ellipse(
    pixels: &mut PixelBuf, grid: &mut ColorGrid,
    cx: i32, cy: i32, rx: u32, ry: u32, color: Color,
) {
    if rx == 0 || ry == 0 {
        set_pixel(pixels, grid, cx, cy, color);
        return;
    }
    let (rx, ry) = (rx as i32, ry as i32);
    for dy in -ry..=ry {
        let frac   = 1.0 - (dy as f64 / ry as f64).powi(2);
        let row_rx = (rx as f64 * frac.sqrt()) as i32;
        for ddx in -row_rx..=row_rx {
            set_pixel(pixels, grid, cx + ddx, cy + dy, color);
        }
    }
}
