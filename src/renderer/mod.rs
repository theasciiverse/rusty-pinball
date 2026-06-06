/// Renders the pinball field into a braille-pixel string via the `drawille` crate.
///
/// Coordinate mapping
/// ------------------
/// Game space:  origin at bottom-left, y increases upward.
/// Canvas space: origin at top-left, y increases downward (standard raster).
///
/// The viewport shows a vertical slice of the field that is `viewport_h`
/// game-units tall, starting at `camera_y` (game-space bottom of viewport).
/// Horizontal scale is always the full field width mapped to canvas width.

use drawille::Canvas;
use crate::physics::FieldSnapshot;

pub struct FieldRenderer;

impl FieldRenderer {
    pub fn new() -> Self { Self }

    /// Render the field into a multi-line braille string.
    ///
    /// `char_cols` / `char_rows` are the character dimensions of the play-area
    /// widget as reported by ratatui (i.e., the Rect size).
    pub fn render(
        &self,
        snap: &FieldSnapshot,
        camera_y: f32,
        char_cols: u16,
        char_rows: u16,
    ) -> String {
        // Braille canvas: each char cell = 2 dots wide × 4 dots tall
        let dot_w = char_cols as u32 * 2;
        let dot_h = char_rows as u32 * 4;

        if dot_w == 0 || dot_h == 0 { return String::new(); }

        let mut canvas = Canvas::new(dot_w, dot_h);

        // Game → canvas coordinate helpers (closures)
        let field_w = snap.field_width;
        let viewport_h = field_w * (dot_h as f32 / dot_w as f32); // preserve aspect

        let to_cx = |gx: f32| -> u32 {
            ((gx / field_w) * dot_w as f32).clamp(0.0, (dot_w - 1) as f32) as u32
        };
        let to_cy = |gy: f32| -> u32 {
            // flip y: game y=camera_y+viewport_h → dot_y=0, game y=camera_y → dot_y=dot_h
            let norm = (gy - camera_y) / viewport_h;
            let flipped = 1.0 - norm;
            (flipped * dot_h as f32).clamp(0.0, (dot_h - 1) as f32) as u32
        };

        // --- walls ---
        for w in &snap.walls {
            draw_line(&mut canvas, to_cx(w.x1), to_cy(w.y1), to_cx(w.x2), to_cy(w.y2));
        }

        // --- bumpers ---
        for b in &snap.bumpers {
            if let Some(outer) = b.outer_radius {
                draw_ellipse(&mut canvas, to_cx(b.x), to_cy(b.y),
                    (outer / field_w * dot_w as f32) as u32,
                    (outer / viewport_h * dot_h as f32) as u32,
                    dot_w, dot_h);
            }
            // Inner circle (filled if lit, otherwise just outline)
            let rx = (b.radius / field_w * dot_w as f32) as u32;
            let ry = (b.radius / viewport_h * dot_h as f32) as u32;
            if b.lit {
                fill_ellipse(&mut canvas, to_cx(b.x), to_cy(b.y), rx, ry, dot_w, dot_h);
            } else {
                draw_ellipse(&mut canvas, to_cx(b.x), to_cy(b.y), rx, ry, dot_w, dot_h);
            }
        }

        // --- flippers ---
        for fl in &snap.flippers {
            draw_line(&mut canvas, to_cx(fl.x1), to_cy(fl.y1), to_cx(fl.x2), to_cy(fl.y2));
        }

        // --- ball ---
        if let Some(b) = &snap.ball {
            let rx = ((snap.field_width * 0.025 / field_w) * dot_w as f32) as u32;
            let ry = ((snap.field_width * 0.025 / viewport_h) * dot_h as f32) as u32;
            let rx = rx.max(1);
            let ry = ry.max(1);
            fill_ellipse(&mut canvas, to_cx(b.x), to_cy(b.y), rx, ry, dot_w, dot_h);
        }

        canvas.frame()
    }
}

// ---------------------------------------------------------------------------
// Drawing primitives (Bresenham-based, operating on dot coordinates)
// ---------------------------------------------------------------------------

fn draw_line(c: &mut Canvas, x0: u32, y0: u32, x1: u32, y1: u32) {
    let (mut x0, mut y0) = (x0 as i32, y0 as i32);
    let (x1, y1) = (x1 as i32, y1 as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 { c.set(x0 as u32, y0 as u32); }
        if x0 == x1 && y0 == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x0 += sx; }
        if e2 <= dx { err += dx; y0 += sy; }
    }
}

fn draw_ellipse(c: &mut Canvas, cx: u32, cy: u32, rx: u32, ry: u32, max_x: u32, max_y: u32) {
    if rx == 0 || ry == 0 { return; }
    let (cx, cy) = (cx as i32, cy as i32);
    let (rx, ry) = (rx as i32, ry as i32);
    let steps = (rx.max(ry) * 8).max(16);
    let mut prev: Option<(i32, i32)> = None;
    for i in 0..=steps {
        let t = std::f64::consts::TAU * i as f64 / steps as f64;
        let x = cx + (rx as f64 * t.cos()) as i32;
        let y = cy + (ry as f64 * t.sin()) as i32;
        if let Some((px, py)) = prev {
            if px >= 0 && py >= 0 && x >= 0 && y >= 0
                && (px as u32) < max_x && (py as u32) < max_y
                && (x as u32) < max_x && (y as u32) < max_y {
                draw_line(c, px as u32, py as u32, x as u32, y as u32);
            }
        }
        prev = Some((x, y));
    }
}

fn fill_ellipse(c: &mut Canvas, cx: u32, cy: u32, rx: u32, ry: u32, max_x: u32, max_y: u32) {
    if rx == 0 || ry == 0 { c.set(cx, cy); return; }
    let (cx, cy) = (cx as i32, cy as i32);
    let (rx, ry) = (rx as i32, ry as i32);
    for dy in -ry..=ry {
        let frac = 1.0 - (dy as f64 / ry as f64).powi(2);
        let row_rx = (rx as f64 * frac.sqrt()) as i32;
        for dx in -row_rx..=row_rx {
            let x = cx + dx;
            let y = cy + dy;
            if x >= 0 && y >= 0 && (x as u32) < max_x && (y as u32) < max_y {
                c.set(x as u32, y as u32);
            }
        }
    }
}
