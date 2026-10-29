/// Dot-Matrix Display bar rendered at the bottom of the terminal.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::game::GameState;
use super::DmdExtra;

const DOT: &str = "·";
const DOT_COLOR: Color = Color::Rgb(80, 55, 20);

pub fn draw_dmd(frame: &mut Frame, area: Rect, game: &GameState, extra: &DmdExtra) {
    let dot_style   = Style::default().fg(DOT_COLOR);
    let score_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let hi_style    = Style::default().fg(Color::Cyan);
    let label_style = Style::default().fg(Color::DarkGray);
    let hint_style  = Style::default().fg(Color::DarkGray);

    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(inner);

    // ── Left column: SCORE / HI SCORE ──────────────────────────────────────
    let left_lines = dot_pad_lines(vec![
        Line::from(vec![
            Span::styled("SCORE     ", label_style),
            Span::styled(format!("{}{}", format_score(game.score), if game.unlimited_balls { "*" } else { "" }), score_style),
        ]),
        Line::from(vec![
            Span::styled("HI SCORE  ", label_style),
            Span::styled(format_score(game.high_score), hi_style),
        ]),
    ], cols[0].width, inner.height, dot_style);
    frame.render_widget(Paragraph::new(left_lines), cols[0]);

    // ── Centre column: ball indicators ──
    let centre_raw = vec![
        Line::from(vec![
            Span::styled("BALLS  ", label_style),
            Span::styled(
                if game.unlimited_balls { unlimited_ball_indicator() } else { ball_indicator(game.balls_remaining) },
                Style::default().fg(Color::Red),
            ),
        ]),
    ];
    let centre_lines = dot_pad_lines(centre_raw, cols[1].width, inner.height, dot_style);
    frame.render_widget(Paragraph::new(centre_lines), cols[1]);

    // ── Right column: status right-justified row 0, hint right-justified last row ──
    let status_span = match game.phase {
        crate::game::Phase::WaitingToLaunch | crate::game::Phase::BallLost if extra.plunger.is_some() =>
            Span::styled(plunger_meter(extra.plunger.unwrap_or(0.0)), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        crate::game::Phase::GameOver if extra.idle.is_some() =>
            Span::styled(extra.idle.clone().unwrap_or_default(), hi_style),
        crate::game::Phase::WaitingToLaunch => Span::styled("PRESS SPACE", Style::default().fg(Color::Green)),
        crate::game::Phase::Playing         => Span::styled(game.multiplier_text().replacen('x', "×", 1), score_style),
        crate::game::Phase::BallLost        => Span::styled("BALL LOST  SPACE", Style::default().fg(Color::Red)),
        crate::game::Phase::GameOver        => Span::styled("GAME OVER  SPACE", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    };
    let hint_span = Span::styled("<esc> to menu", hint_style);
    let rw = cols[2].width as usize;
    let rh = inner.height as usize;

    let mut right_lines: Vec<Line> = Vec::with_capacity(rh.max(1));
    let sw = status_span.content.chars().count();
    right_lines.push(Line::from(vec![
        Span::styled(DOT.repeat(rw.saturating_sub(sw)), dot_style),
        status_span,
    ]));
    for row in 1..rh.saturating_sub(1) {
        // Row 1 carries the current game message ("Multiball!", ...), right-aligned.
        let message = if row == 1 { game.message.as_ref() } else { None };
        if let Some((text, _)) = message {
            let text: String = text.chars().take(rw).collect();
            let tw = text.chars().count();
            right_lines.push(Line::from(vec![
                Span::styled(DOT.repeat(rw.saturating_sub(tw)), dot_style),
                Span::styled(text, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]));
        } else {
            right_lines.push(Line::from(Span::styled(DOT.repeat(rw), dot_style)));
        }
    }
    if rh > 1 {
        let hw = hint_span.content.chars().count();
        right_lines.push(Line::from(vec![
            Span::styled(DOT.repeat(rw.saturating_sub(hw)), dot_style),
            hint_span,
        ]));
    }
    frame.render_widget(Paragraph::new(right_lines), cols[2]);
}

/// Append dots to a Line until it reaches `width` display columns.
fn dot_fill_line<'a>(line: Line<'a>, width: usize, dot_style: Style) -> Line<'a> {
    let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    let mut line = line;
    if width > used {
        line.spans.push(Span::styled(DOT.repeat(width - used), dot_style));
    }
    line
}

/// Fill each line to `width` with dots and pad to `height` rows with all-dot lines.
fn dot_pad_lines<'a>(lines: Vec<Line<'a>>, width: u16, height: u16, dot_style: Style) -> Vec<Line<'a>> {
    let w = width as usize;
    let mut out: Vec<Line<'a>> = lines.into_iter()
        .map(|l| dot_fill_line(l, w, dot_style))
        .collect();
    while out.len() < height as usize {
        out.push(Line::from(Span::styled(DOT.repeat(w), dot_style)));
    }
    out
}

fn format_score(s: u64) -> String {
    let s = s.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// The plunger pulled back: eight steps.
fn plunger_meter(pull: f32) -> String {
    let n = (pull.clamp(0.0, 1.0) * 8.0).round() as usize;
    format!("PLUNGER {}{}", "▮".repeat(n), "▯".repeat(8 - n))
}

/// Unlimited play: a row of balls that dwindle in the original; here five that never run out.
fn unlimited_ball_indicator() -> String { "● ● ● ● ●".to_string() }

fn ball_indicator(remaining: u8) -> String {
    const MAX: u8 = 3;
    const FULL: &str = "●";
    const EMPTY: &str = "○";
    let dots = (0..MAX).map(|i| if i < remaining { FULL } else { EMPTY }).collect::<Vec<_>>().join(" ");
    // More than three balls left (an extra ball was won): show the surplus.
    if remaining > MAX { format!("{dots} +{}", remaining - MAX) } else { dots }
}
