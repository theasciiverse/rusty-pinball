/// Renders the pinball field into colored braille characters.
///
/// Coordinate system
/// -----------------
/// Game space:   origin bottom-left, y up.
/// Canvas space: origin top-left,    y down  (standard raster).
///
/// `to_cx` / `to_cy` return signed i32 so that elements outside the current
/// viewport produce negative or oversized values.  Every pixel write goes
/// through `set_pixel`, which silently ignores out-of-bounds coordinates,
/// this prevents off-screen elements from being clamped to the screen edge.

use ratatui::style::Color;
use crate::physics::{blend, FieldSnapshot, RolloverSnapshot, Shape};

// ── Palette ───────────────────────────────────────────────────────────────────

const COL_WALL:        Color = Color::Gray;
const COL_KICK_WALL:   Color = Color::LightGreen;
const COL_DROP_TARGET: Color = Color::Yellow;
const COL_BUMPER:      Color = Color::Blue;
const COL_BUMPER_LIT:  Color = Color::LightYellow;
const COL_FLIPPER:     Color = Color::Cyan;
const COL_BALL:        Color = Color::White;
const COL_BACKGROUND:  Color = Color::DarkGray;
const COL_DEBUG_LABEL: Color = Color::White;

// Layer-specific wall colors.  Index = layer number.
const COL_ROLLOVER_LIT:   Color = Color::Cyan;
const COL_ROLLOVER_UNLIT: Color = Color::Rgb(0, 80, 80);
const COL_SPINNER:        Color = Color::Rgb(90, 70, 0);
const COL_SPINNER_FLASH:  Color = Color::LightYellow;

const COL_LAYER: [Color; 5] = [
    Color::Gray,           // layer 0: main field
    Color::Green,          // layer 1: left ramp
    Color::Magenta,        // layer 2: outer orbit
    Color::Red,            // layer 3: right ramp
    Color::Yellow,         // layer 4: table 7's ball lock lane
];

/// Converts a table-JSON RGB triple to a ratatui `Color`.
fn rgb((r, g, b): (u8, u8, u8)) -> Color { Color::Rgb(r, g, b) }

/// A colour as RGB, the named ones as xterm draws them, for a fade.
fn rgb_of(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Gray => (229, 229, 229),
        Color::Green => (0, 205, 0),
        Color::Magenta => (205, 0, 205),
        Color::Red => (205, 0, 0),
        Color::Yellow => (205, 205, 0),
        Color::Cyan => (0, 205, 205),
        _ => (192, 192, 192),
    }
}

/// A dimmed variant of an explicit element color, used for "unlit" rollover
/// states; mirrors the ~31% brightness ratio between the built-in
/// `COL_ROLLOVER_LIT` / `COL_ROLLOVER_UNLIT` pair.
fn dim((r, g, b): (u8, u8, u8)) -> Color { Color::Rgb(r / 3, g / 3, b / 3) }

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
        whole_table: bool,
        char_cols: u16,
        char_rows: u16,
        debug_ids: bool,
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
        let mut labels: Vec<(i32, i32, String)> = Vec::new();

        // Dots per table unit. Following the ball, the table fills the width and
        // scrolls with `camera_y`; the whole table fits it in both directions,
        // centred, and does not scroll (the original's zoom setting, off).
        let (scale, off_x, off_y, camera_y) = if whole_table {
            let s = (dot_w as f32 / snap.field_width).min(dot_h as f32 / snap.field_height);
            (s, (dot_w as f32 - snap.field_width * s) / 2.0, (dot_h as f32 - snap.field_height * s) / 2.0, 0.0)
        } else {
            (dot_w as f32 / snap.field_width, 0.0, 0.0, camera_y)
        };
        // Table units to dots, for a radius in either direction (dots are square).
        let field_w    = dot_w as f32 / scale;
        let viewport_h = dot_h as f32 / scale;

        // Signed coordinate helpers; no clamping; out-of-bounds dots are
        // dropped inside set_pixel without affecting on-screen elements.
        let to_cx = |gx: f32| -> i32 { (off_x + gx * scale) as i32 };
        let to_cy = |gy: f32| -> i32 { (dot_h as f32 - off_y - (gy - camera_y) * scale) as i32 };
        // Dot coords → char-cell coords (2 dots per column, 4 per row).
        let label_at = |labels: &mut Vec<(i32, i32, String)>, dot_x: i32, dot_y: i32, text: &str| {
            if debug_ids {
                labels.push((dot_x.div_euclid(2), dot_y.div_euclid(4), text.to_string()));
            }
        };

        // Draw in priority order (later = higher priority overwrites color).

        // Script shapes (table 7's star map) go underneath everything else.
        for shape in &snap.shapes {
            match *shape {
                Shape::Line { x1, y1, x2, y2, color } => {
                    draw_line(&mut pixels, &mut grid, to_cx(x1), to_cy(y1), to_cx(x2), to_cy(y2), rgb(color));
                }
                Shape::Circle { x, y, radius, color } => {
                    let rx = ((radius / field_w)    * dot_w as f32) as u32;
                    let ry = ((radius / viewport_h) * dot_h as f32) as u32;
                    fill_ellipse(&mut pixels, &mut grid, to_cx(x), to_cy(y), rx.max(1), ry.max(1), rgb(color));
                }
            }
        }

        // Ball trails go under the elements: dots that shrink and fade towards the tail,
        // at most 30% of the ball's brightness (the original's alpha 0.3 * size fraction).
        for b in &snap.balls {
            let base = b.color.or(snap.ball_color).map(rgb).unwrap_or(COL_BALL);
            let (br, bg, bb) = match base { Color::Rgb(r, g, b) => (r, g, b), Color::White => (255, 255, 255), _ => (200, 200, 200) };
            let n = b.trail.len();
            for (i, &(tx, ty)) in b.trail.iter().enumerate() {
                let fraction = (i + 1) as f32 / (n + 1) as f32;
                let shade = |v: u8| (v as f32 * 0.3 * fraction) as u8;
                let rx = ((snap.ball_radius * fraction / field_w)    * dot_w as f32) as u32;
                let ry = ((snap.ball_radius * fraction / viewport_h) * dot_h as f32) as u32;
                fill_ellipse(&mut pixels, &mut grid, to_cx(tx), to_cy(ty), rx.max(1), ry.max(1),
                    Color::Rgb(shade(br), shade(bg), shade(bb)));
            }
        }

        for w in &snap.walls {
            let color = match w.layer {
                None    => w.color.map(rgb).unwrap_or(COL_WALL),
                Some(n) => {
                    let own = w.color.map(rgb).unwrap_or(COL_LAYER[n.min(4) as usize]);
                    let f = if n == 0 { 1.0 } else { snap.layer_fade[n.min(31) as usize] };
                    // Another layer: the original's faint outline, if the table gives one,
                    // fading to the wall's own colour over a quarter of a second.
                    match (f, w.inactive) {
                        (f, _) if f >= 1.0 => own,
                        (f, None) if f <= 0.0 => continue,
                        (f, inactive) => rgb(blend(inactive.unwrap_or((0, 0, 0)), rgb_of(own), f)),
                    }
                }
            };
            draw_line(&mut pixels, &mut grid,
                to_cx(w.x1), to_cy(w.y1), to_cx(w.x2), to_cy(w.y2),
                color);
            if let Some(text) = &w.label {
                label_at(&mut labels, to_cx(w.x1), to_cy(w.y1), text);
            }
        }

        for kw in &snap.kick_walls {
            draw_line(&mut pixels, &mut grid,
                to_cx(kw.x1), to_cy(kw.y1), to_cx(kw.x2), to_cy(kw.y2),
                kw.color.map(rgb).unwrap_or(COL_KICK_WALL));
            if let Some(text) = &kw.label {
                label_at(&mut labels, to_cx(kw.x1), to_cy(kw.y1), text);
            }
        }

        for dt in &snap.drop_targets {
            draw_line(&mut pixels, &mut grid,
                to_cx(dt.x1), to_cy(dt.y1), to_cx(dt.x2), to_cy(dt.y2),
                dt.color.map(rgb).unwrap_or(COL_DROP_TARGET));
            if let Some(text) = &dt.label {
                label_at(&mut labels, to_cx(dt.x1), to_cy(dt.y1), text);
            }
        }

        for b in &snap.bumpers {
            // A hit flashes the bumper in the inverse of its colour, as in the original.
            let flash = |c: Option<(u8, u8, u8)>| match c {
                Some((r, g, b)) => Color::Rgb(255 - r, 255 - g, 255 - b),
                None => COL_BUMPER_LIT,
            };
            let outer_color = if b.lit { flash(b.outer_color.or(b.color)) } else { b.outer_color.or(b.color).map(rgb).unwrap_or(COL_BUMPER) };
            let inner_color = if b.lit { flash(b.color) } else { b.color.map(rgb).unwrap_or(COL_BUMPER) };
            let cx = to_cx(b.x);
            let cy = to_cy(b.y);
            if let Some(outer) = b.outer_radius {
                let rx = (outer / field_w    * dot_w as f32) as u32;
                let ry = (outer / viewport_h * dot_h as f32) as u32;
                draw_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, outer_color);
            }
            let rx = (b.radius / field_w    * dot_w as f32) as u32;
            let ry = (b.radius / viewport_h * dot_h as f32) as u32;
            if b.lit {
                fill_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, inner_color);
            } else {
                draw_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, inner_color);
            }
            label_at(&mut labels, cx, cy, &b.label);
        }

        for group in &snap.rollover_groups {
            for rv in &group.rollovers {
                draw_rollover(&mut pixels, &mut grid, rv, field_w, viewport_h, dot_w, dot_h, &to_cx, &to_cy);
                label_at(&mut labels, to_cx(rv.x), to_cy(rv.y), &rv.label);
            }
        }

        for sp in &snap.spinners {
            let cx = to_cx(sp.x);
            let cy = to_cy(sp.y);
            let rx = ((sp.radius / field_w)    * dot_w as f32) as u32;
            let ry = ((sp.radius / viewport_h) * dot_h as f32) as u32;
            let rx = rx.max(1);
            let ry = ry.max(1);
            if sp.flash_on {
                fill_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, COL_SPINNER_FLASH);
            } else {
                draw_ellipse(&mut pixels, &mut grid, cx, cy, rx, ry, COL_SPINNER);
            }
            label_at(&mut labels, cx, cy, &sp.label);
        }

        for fl in &snap.flippers {
            draw_line(&mut pixels, &mut grid,
                to_cx(fl.x1), to_cy(fl.y1), to_cx(fl.x2), to_cy(fl.y2),
                fl.color.map(rgb).unwrap_or(COL_FLIPPER));
            label_at(&mut labels, to_cx(fl.x1), to_cy(fl.y1), &fl.label);
        }

        // Every ball on the field (more than one during multiball), at the table's
        // ball radius. The original's spin dot, a quarter of the radius at half the
        // radius out, turning with the ball, is cut out of it: a braille cell has
        // one colour, so a dot of another colour would not show.
        for b in &snap.balls {
            let r = snap.ball_radius * scale;
            let (cx, cy) = (to_cx(b.x), to_cy(b.y));
            fill_ellipse(&mut pixels, &mut grid,
                cx, cy, (r as u32).max(1), (r as u32).max(1), b.color.or(snap.ball_color).map(rgb).unwrap_or(COL_BALL));
            if r >= 3.0 {
                let (sx, sy) = (b.x + snap.ball_radius / 2.0 * b.angle.cos(), b.y + snap.ball_radius / 2.0 * b.angle.sin());
                clear_disc(&mut pixels, to_cx(sx), to_cy(sy), (r / 4.0).max(0.8));
            }
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

        // Scores floating up from where they were won, over everything but the debug labels.
        for popup in &snap.popups {
            // The original fades them out over the second half of their life.
            let fade = if popup.progress > 0.5 { 1.0 - (popup.progress - 0.5) * 2.0 } else { 1.0 };
            let shade = |v: u8| (v as f32 * fade) as u8;
            let color = Color::Rgb(shade(popup.color.0), shade(popup.color.1), shade(popup.color.2));
            let (dot_x, dot_y) = (to_cx(popup.x), to_cy(popup.y));
            let row = dot_y.div_euclid(4);
            if row < 0 || row as u32 >= char_rows { continue; }
            let width = popup.text.chars().count() as i32;
            let start = (dot_x.div_euclid(2) - width / 2).clamp(0, (char_cols as i32 - width).max(0));
            for (i, ch) in popup.text.chars().enumerate() {
                let col = start + i as i32;
                if col < 0 || col as u32 >= char_cols { continue; }
                let idx = (row as u32 * char_cols + col as u32) as usize;
                chars[idx] = ch;
                colors[idx] = color;
            }
        }

        // Overlay debug ID labels last, directly onto the character grid,
        // braille sub-pixels can't render text, so labels overwrite whatever
        // was drawn at that char cell.
        if debug_ids {
            for (label_cx, label_cy, text) in labels {
                if label_cy < 0 || label_cy as u32 >= char_rows { continue; }
                let row = label_cy as u32;
                for (i, ch) in text.chars().enumerate() {
                    let col = label_cx + i as i32;
                    if col < 0 { continue; }
                    let col = col as u32;
                    if col >= char_cols { break; }
                    let idx = (row * char_cols + col) as usize;
                    chars[idx] = ch;
                    colors[idx] = COL_DEBUG_LABEL;
                }
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

// ── Rollover helper ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_rollover(
    pixels: &mut PixelBuf, grid: &mut ColorGrid,
    rv: &RolloverSnapshot,
    field_w: f32, viewport_h: f32, dot_w: u32, dot_h: u32,
    to_cx: &impl Fn(f32) -> i32,
    to_cy: &impl Fn(f32) -> i32,
) {
    let cx = to_cx(rv.x);
    let cy = to_cy(rv.y);
    let rx = ((rv.radius / field_w)    * dot_w as f32) as u32;
    let ry = ((rv.radius / viewport_h) * dot_h as f32) as u32;
    let rx = rx.max(1);
    let ry = ry.max(1);
    let color = match (rv.lit, rv.color) {
        (true, Some(c))  => rgb(c),
        (true, None)     => COL_ROLLOVER_LIT,
        (false, Some(c)) => dim(c),
        (false, None)    => COL_ROLLOVER_UNLIT,
    };
    if rv.lit {
        fill_ellipse(pixels, grid, cx, cy, rx, ry, color);
    } else {
        draw_ellipse(pixels, grid, cx, cy, rx, ry, color);
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

/// Turns off the dots within `r` of a point (the ball's spin dot).
fn clear_disc(pixels: &mut PixelBuf, cx: i32, cy: i32, r: f32) {
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if ((dx * dx + dy * dy) as f32) > r * r { continue; }
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 || x as u32 >= pixels.width || y as u32 >= pixels.height { continue; }
            pixels.data[(y as u32 * pixels.width + x as u32) as usize] = false;
        }
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
