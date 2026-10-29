//! The screens off the main menu: Settings, High scores, and Help with each
//! table's rules (the original's preferences, high score panel and About screen).

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::menu::{draw_title, format_score_col, TABLE_NAMES};
use crate::persist::{SavedState, MAX_HIGH_SCORES};

// ── Settings ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    Effects,
    Music,
    ScoreAnimations,
    BallTrails,
    Zoom,
    GameSpeed,
    UnlimitedBalls,
    Back,
}

pub const SETTINGS: [Setting; 8] = [
    Setting::Effects, Setting::Music, Setting::ScoreAnimations, Setting::BallTrails,
    Setting::Zoom, Setting::GameSpeed, Setting::UnlimitedBalls, Setting::Back,
];

impl Setting {
    fn label(self) -> &'static str {
        match self {
            Setting::Effects => "Sound effects",
            Setting::Music => "Music",
            Setting::ScoreAnimations => "Score animations",
            Setting::BallTrails => "Ball trails",
            Setting::Zoom => "Zoom",
            Setting::GameSpeed => "Game speed",
            Setting::UnlimitedBalls => "Unlimited balls",
            Setting::Back => "Back",
        }
    }

    fn value(self, s: &SavedState) -> String {
        let on_off = |on: bool| if on { "[ ON ]" } else { "[OFF]" }.to_string();
        match self {
            Setting::Effects => on_off(s.sound_enabled),
            Setting::Music => on_off(s.music_enabled),
            Setting::ScoreAnimations => on_off(s.score_animations),
            Setting::BallTrails => on_off(s.ball_trails),
            Setting::Zoom => on_off(s.zoom),
            Setting::GameSpeed => format!("< {} >", s.game_speed.name()),
            Setting::UnlimitedBalls => on_off(s.unlimited_balls),
            Setting::Back => String::new(),
        }
    }

    fn about(self) -> &'static str {
        match self {
            Setting::Effects => "Dings, rollover notes, flippers, launches and messages.",
            Setting::Music => "The drums and pad that come in as you score.",
            Setting::ScoreAnimations => "Every score floats up from where it was won.",
            Setting::BallTrails => "Each ball leaves a fading trail behind it.",
            Setting::Zoom => "On: the table fills the width and follows the ball. Off: the whole table at once.",
            Setting::GameSpeed => "Slow, normal or fast, as in the original.",
            Setting::UnlimitedBalls => "The game never ends; the score shows a * and no high score is kept.",
            Setting::Back => "Back to the tables.",
        }
    }

    /// Changes the setting; `back` steps the game speed the other way.
    pub fn change(self, s: &mut SavedState, back: bool) {
        match self {
            Setting::Effects => s.sound_enabled = !s.sound_enabled,
            Setting::Music => s.music_enabled = !s.music_enabled,
            Setting::ScoreAnimations => s.score_animations = !s.score_animations,
            Setting::BallTrails => s.ball_trails = !s.ball_trails,
            Setting::Zoom => s.zoom = !s.zoom,
            Setting::GameSpeed => s.game_speed = if back { s.game_speed.next().next() } else { s.game_speed.next() },
            Setting::UnlimitedBalls => s.unlimited_balls = !s.unlimited_balls,
            Setting::Back => {}
        }
    }
}

fn styles() -> (Style, Style, Style) {
    (
        Style::default().fg(Color::White),
        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
        Style::default().fg(Color::DarkGray),
    )
}

/// The title art on top when there is room, the rest for the screen's box.
fn split(area: Rect) -> (Option<Rect>, Rect) {
    if area.height >= 16 + 14 {
        let c = Layout::default().direction(Direction::Vertical)
            .constraints([Constraint::Length(16), Constraint::Min(5)]).split(area);
        (Some(c[0]), c[1])
    } else {
        (None, area)
    }
}

fn boxed(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {title} "), Style::default().fg(Color::Yellow)))
        .style(Style::default().fg(Color::Rgb(60, 50, 10)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // the same 50-column body as the menu's list, centred
    let w = inner.width.min(56);
    Rect { x: inner.x + (inner.width - w) / 2, width: w, ..inner }
}

pub fn draw_settings(frame: &mut Frame, saved: &SavedState, selected: usize) {
    let (title, rest) = split(frame.area());
    if let Some(t) = title { draw_title(frame, t); }
    let body = boxed(frame, rest, "SETTINGS");
    let (normal, sel, dim) = styles();
    let mut lines = Vec::new();
    for (i, item) in SETTINGS.iter().enumerate() {
        let style = if i == selected { sel } else { normal };
        let value = item.value(saved);
        let gap = (body.width as usize).saturating_sub(4 + item.label().chars().count() + value.chars().count());
        lines.push(Line::from(Span::styled(
            format!("{} {}{}{} ", if i == selected { "▶" } else { " " }, item.label(), " ".repeat(gap), value),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(SETTINGS[selected].about(), Style::default().fg(Color::Rgb(200, 170, 60)))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("↑↓  Choose    Enter ←→  Change    Esc  Back", dim)));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}

// ── High scores ──────────────────────────────────────────────────────────────

pub fn draw_scores(frame: &mut Frame, saved: &SavedState, table: usize) {
    let (title, rest) = split(frame.area());
    if let Some(t) = title { draw_title(frame, t); }
    let body = boxed(frame, rest, "HIGH SCORES");
    let (normal, _, dim) = styles();
    let gold = Style::default().fg(Color::Rgb(200, 170, 60));
    let w = body.width as usize;
    let mut lines = vec![
        Line::from(Span::styled(format!("◀  {}. {}  ▶", table + 1, TABLE_NAMES[table]), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .alignment(Alignment::Center),
        Line::from(""),
    ];
    let scores = &saved.top_scores[table];
    if scores.is_empty() {
        lines.push(Line::from(Span::styled("No scores for this table yet", dim)).alignment(Alignment::Center));
        for _ in 1..MAX_HIGH_SCORES { lines.push(Line::from("")); }
    } else {
        for place in 0..MAX_HIGH_SCORES {
            let score = scores.get(place).map_or("".to_string(), |&s| format_score_col(s));
            let head = format!("  {}.", place + 1);
            let dots = "·".repeat(w.saturating_sub(head.len() + score.chars().count() + 2));
            lines.push(Line::from(vec![
                Span::styled(head, normal),
                Span::styled(if scores.get(place).is_some() { dots } else { String::new() }, dim),
                Span::styled(score, gold),
            ]));
        }
    }
    lines.push(Line::from(""));
    let last = saved.last_scores[table];
    if last > 0 {
        lines.push(Line::from(vec![Span::styled("  Last game  ", dim), Span::styled(format_score_col(last), normal)]));
    } else {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("←→  Table    Esc  Back", dim)));
    frame.render_widget(Paragraph::new(lines), body);
}

// ── Help ─────────────────────────────────────────────────────────────────────

/// Each table's rules, as the original's About screen gives them.
const RULES: [&str; 8] = [
    // 1 Classic
    "- Light all the cyan rollovers in the top or bottom lanes to advance the score multiplier. All points you earn are multiplied by it. Shooting the left ramp also advances the multiplier.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.
- Hitting any group of drop targets lights the next light in the centre of the field. When the large white centre light is on, shoot the left ramp to start multiball.",
    // 2 Round and Round
    "- Light all the cyan rollovers in the bottom lanes to advance the score multiplier. All points you earn are multiplied by it.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.
- The circles in the middle of the field count how many times you have hit all the left, upper and right drop targets. When each has been hit at least 3 times, shoot the red bumper in the upper right to start multiball.",
    // 3 Ringmaster
    "- Light all the cyan rollovers in the bottom lanes to advance the score multiplier. All points you earn are multiplied by it.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.
- Shooting either ramp turns the bumpers red, and they change back to blue over several seconds. Hit them meanwhile for extra points.
- Turn the middle rings red by hitting the bumpers and the middle sets of drop targets. When all the rings are red, shoot either ramp to start multiball.",
    // 4 Compound Interest
    "- Each bumper raises the score multiplier by 0.01x. Shooting the top drop targets or lighting all the lane rollovers raises that amount.
- Shoot all six bumpers to light the centre indicators. When all the indicators in a row are lit, shoot the right hole to lock the ball for multiball.
- Multiball starts when you lock three balls. During multiball the centre indicators light up as a ball crosses them. Light them all to score a jackpot.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.",
    // 5 Eye of Horus
    "- The central circle has twelve points around it, three each of four colours. Shooting a top ramp changes the colour of the ball and lets it light the points of that colour.
- When all twelve points are lit, hitting the edges of the rotating triangle removes them. Shoot the target in the middle of the triangle to start multiball.
- During multiball, light the twelve points again to score a jackpot, which grows each time.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.",
    // 6 Orbiters
    "- In the centre of the table five planets orbit a sun. When the ball rolls over a planet, it takes the planet's colour and the planet starts flashing.
- Shoot the matching target to activate the planet and make it solid. For the inner orange planet, hit the targets in the upper left. For the second, blue planet, light the rollovers in the upper right. For the other planets, shoot the ramp of the matching colour.
- Activate all five planets to start multiball. In multiball, activate all the planets again for a jackpot.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.
- The sun and planets have gravity and pull at balls that come close.",
    // 7 Ad Astra
    "- The centre of the table holds a star map. Shoot one of the ramps with the flashing green arrows to move the map to a constellation.
- The stars of the constellation appear in green. Roll the ball over them to light them; they turn yellow, and lines join the lit stars to draw the constellation.
- When the constellation is complete, shoot the right ramp with the flashing yellow arrow to lock the ball. Do this with two more constellations; locking the third ball starts multiball.
- In multiball, complete the remaining constellations and then shoot the right ramp for a jackpot.
- The game starts with ball savers in the left and right outlanes, which disappear when used. Hit all drop targets on the left or right side to restore them.",
    // 8 Pool Party
    "- Sink the pool balls by shooting the right targets, then sink the 8-ball by hitting the upper right target.
- Balls 1 to 7 stay sunk from ball to ball unless the right target bank is completed. If the bank is completed but the 8-ball target is not hit, all targets reset on the next ball.
- Each pool ball adds 7,000 points to the bonus. Each time balls 1 to 8 are sunk, a super bonus of 56,000 points is added to your bonus at the end of every later ball.
- After sinking the 8-ball, a one-time bonus collect is lit behind the 8-ball target.
- The bonus multiplier goes up to 5x by shooting the inline targets on the left. After all 4, a 50,000 point red target opens behind the drop targets.
- Completing the 4 rollover lanes raises the value of the spinner (up to 1,500 a spin) and of the 8-ball target (up to 60,000 a shot).
- Holding the left flipper up while launching the ball gives you one chance to shoot the 8-ball and collect the whole rack.",
];

const CONTROLS: &[(&str, &str)] = &[
    ("Space", "launch the ball; hold it to pull the plunger back, for a softer or a harder shot"),
    ("Z  ←  Left Shift", "left flipper"),
    ("X  →  Right Shift", "right flipper"),
    ("P", "pause"),
    ("H", "this help, for the table in play"),
    ("Esc  Q", "back to the menu"),
];

/// `text` broken at spaces into rows of at most `width` columns, every row
/// after the first indented by `indent`.
fn wrap(text: &str, width: usize, indent: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let len = row.chars().count();
        if len > 0 && len + 1 + word.chars().count() > width {
            rows.push(std::mem::take(&mut row));
            row = " ".repeat(indent);
        } else if len > 0 && !(rows.len() > 0 && len == indent) {
            row.push(' ');
        }
        row.push_str(word);
    }
    rows.push(row);
    rows
}

/// The help for `table` (0-based), wrapped to `width` columns.
fn help_lines(table: usize, release_keys: bool, width: usize) -> Vec<Line<'static>> {
    let head = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let key = Style::default().fg(Color::Cyan);
    let text = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);
    let para = |lines: &mut Vec<Line<'static>>, s: &str, indent: usize, style: Style| {
        for row in wrap(s, width, indent) { lines.push(Line::from(Span::styled(row, style))); }
    };
    const KEY_W: usize = 20;
    let mut lines = vec![Line::from(Span::styled("CONTROLS", head))];
    for &(k, what) in CONTROLS {
        let rows = wrap(what, width.saturating_sub(KEY_W).max(10), 0);
        for (i, row) in rows.into_iter().enumerate() {
            let k = if i == 0 { format!("  {k:<w$}", w = KEY_W - 2) } else { " ".repeat(KEY_W) };
            lines.push(Line::from(vec![Span::styled(k, key), Span::styled(row, text)]));
        }
    }
    if !release_keys {
        para(&mut lines, "  This terminal does not report key releases, so Space launches at once and Shift does not work the flippers.", 2, dim);
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("TABLE {}: {}", table + 1, TABLE_NAMES[table].to_uppercase()), head)));
    for rule in RULES[table].lines() {
        para(&mut lines, rule, 2, text);
    }
    lines.push(Line::from(""));
    para(&mut lines, "Every table: the multiplier halves when a ball is lost. A ball lost soon after its launch is saved once.", 0, text);
    lines.push(Line::from(""));
    para(&mut lines, "Tables and rules from Vector Pinball by Brian Nenninger; table 8 by Jack Morgan; sound and music by pdx.", 0, dim);
    lines
}

/// Draws the help for `table`, scrolled by `scroll` lines (clamped and returned).
pub fn draw_help(frame: &mut Frame, table: usize, scroll: u16, in_game: bool, release_keys: bool) -> u16 {
    let area = frame.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" HELP AND TABLE RULES ", Style::default().fg(Color::Yellow)))
        .style(Style::default().fg(Color::Rgb(60, 50, 10)));
    let inner = block.inner(area);
    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(block, area);
    let w = inner.width.min(72).saturating_sub(2);
    let body = Rect { x: inner.x + (inner.width.saturating_sub(w)) / 2, width: w, height: inner.height.saturating_sub(2), ..inner };
    let lines = help_lines(table, release_keys, w as usize);
    let scroll = scroll.min((lines.len() as u16).saturating_sub(body.height));
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body);
    let hint = if in_game { "↑↓  Scroll    ←→  Table    Esc  H  Back to the game" } else { "↑↓  Scroll    ←→  Table    Esc  Back" };
    let foot = Rect { y: inner.y + inner.height.saturating_sub(1), height: 1, ..body };
    frame.render_widget(Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))), foot);
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_changes_and_game_speed_goes_round_both_ways() {
        use crate::persist::GameSpeed;
        let mut s = SavedState::default();
        for item in SETTINGS {
            let before = format!("{:?}", (item.value(&s), s.game_speed));
            item.change(&mut s, false);
            if item != Setting::Back {
                assert_ne!(before, format!("{:?}", (item.value(&s), s.game_speed)), "{item:?}");
            }
        }
        assert_eq!(s.game_speed, GameSpeed::Fast);
        Setting::GameSpeed.change(&mut s, true);
        assert_eq!(s.game_speed, GameSpeed::Normal);
        Setting::GameSpeed.change(&mut s, true);
        assert_eq!(s.game_speed, GameSpeed::Slow);
    }

    #[test]
    fn wrapping_breaks_at_spaces_with_a_hanging_indent() {
        assert_eq!(wrap("- one two three four", 9, 2), vec!["- one two", "  three", "  four"]);
        assert_eq!(wrap("short", 40, 2), vec!["short"]);
        for row in wrap(RULES[7].lines().last().unwrap(), 30, 2) { assert!(row.chars().count() <= 30, "{row:?}"); }
    }

    #[test]
    fn every_table_has_its_rules() {
        for (i, rules) in RULES.iter().enumerate() {
            assert!(rules.lines().count() >= 3, "table {}", i + 1);
            assert!(rules.lines().all(|l| l.starts_with("- ")), "table {}", i + 1);
        }
    }
}

#[cfg(test)]
mod look {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn dump(t: &Terminal<TestBackend>) {
        let b = t.backend().buffer();
        for y in 0..b.area.height {
            let row: String = (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect();
            eprintln!("|{}|", row.trim_end());
        }
    }

    /// `cargo test look -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn screens() {
        let mut s = SavedState::default();
        for v in [41_250, 12_000, 88_900] { s.record(0, v); }
        let mut t = Terminal::new(TestBackend::new(80, 40)).unwrap();
        t.draw(|f| draw_settings(f, &s, 4)).unwrap(); dump(&t);
        t.draw(|f| draw_scores(f, &s, 0)).unwrap(); dump(&t);
        t.draw(|f| draw_scores(f, &s, 3)).unwrap(); dump(&t);
        t.draw(|f| { draw_help(f, 7, 0, true, true); }).unwrap(); dump(&t);
        t.draw(|f| { draw_help(f, 7, 99, false, false); }).unwrap(); dump(&t);
    }

    #[test]
    #[ignore]
    fn play() {
        use crate::{game::GameState, input::InputState, physics::PhysicsWorld, renderer::FieldRenderer, ui::{draw, DmdExtra, ViewOptions}};
        let table = crate::table::load_table(1).unwrap();
        let mut w = PhysicsWorld::from_table(&table);
        w.launch_ball();
        for _ in 0..40 { w.step_frame(0.033, &InputState::default()); }
        let game = GameState::new(3);
        let r = FieldRenderer::new();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        for whole in [true, false] {
            let view = ViewOptions { score_animations: true, ball_trails: true, whole_table: whole };
            let extra = DmdExtra { plunger: Some(0.6), idle: None };
            let cam = (w.ball_position().unwrap().y - 12.0).max(0.0);
            t.draw(|f| draw(f, &w, &game, &r, cam, false, false, view, &extra)).unwrap(); dump(&t);
        }
    }
}
