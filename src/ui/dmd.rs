/// Dot-Matrix Display bar rendered at the bottom of the terminal.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{game::GameState, physics::FieldSnapshot};

const DOT: &str = "·";
const DOT_COLOR: Color = Color::Rgb(80, 55, 20);

pub fn draw_dmd(frame: &mut Frame, area: Rect, game: &GameState, snap: &FieldSnapshot) {
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
            Span::styled(format_score(game.score), score_style),
        ]),
        Line::from(vec![
            Span::styled("HI SCORE  ", label_style),
            Span::styled(format_score(game.high_score), hi_style),
        ]),
    ], cols[0].width, inner.height, dot_style);
    frame.render_widget(Paragraph::new(left_lines), cols[0]);

    // ── Centre column: ball indicators, plus letter-award progress (table 9) ──
    let mut centre_raw = vec![
        Line::from(vec![
            Span::styled("BALLS  ", label_style),
            Span::styled(ball_indicator(game.balls_remaining), Style::default().fg(Color::Red)),
        ]),
    ];
    if !snap.letter_groups.is_empty() {
        centre_raw.push(letter_awards_line(snap));
    }
    if let Some(line) = award_ladder_line(snap) {
        centre_raw.push(line);
    }
    if let Some(lit) = snap.jackpot_lit {
        centre_raw.push(jackpot_line(lit));
    }
    let centre_lines = dot_pad_lines(centre_raw, cols[1].width, inner.height, dot_style);
    frame.render_widget(Paragraph::new(centre_lines), cols[1]);

    // ── Right column: status right-justified row 0, hint right-justified last row ──
    let status_span = match game.phase {
        crate::game::Phase::WaitingToLaunch => Span::styled("PRESS SPACE", Style::default().fg(Color::Green)),
        crate::game::Phase::Playing if game.super_mode_ticks > 0 => Span::styled(
            format!("SUPER {}s", game.super_mode_seconds_remaining()),
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        crate::game::Phase::Playing         => Span::styled(format!("×{}", game.multiplier), score_style),
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

/// One line of space-separated words, each letter dim when unlit, bright
/// yellow when lit, bright magenta (fully lit / mid-celebration) when the
/// word is flashing complete.
fn letter_awards_line(snap: &FieldSnapshot) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (gi, group) in snap.letter_groups.iter().enumerate() {
        if gi > 0 {
            spans.push(Span::raw("  "));
        }
        for (ch, &lit) in group.word.chars().zip(group.lit.iter()) {
            let style = if group.flashing {
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
            } else if lit {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
    }
    Line::from(spans)
}

/// "AWARD  35,000" for the first bumper carrying an award ladder (there's
/// only ever one on a table today, but this doesn't assume it), bright
/// yellow while lit (just hit — the value shown is already the *next* rung).
fn award_ladder_line(snap: &FieldSnapshot) -> Option<Line<'static>> {
    let bump = snap.bumpers.iter().find(|b| b.next_award.is_some())?;
    let value_style = if bump.lit {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightYellow)
    };
    Some(Line::from(vec![
        Span::styled("AWARD  ", Style::default().fg(Color::DarkGray)),
        Span::styled(format_score(bump.next_award.unwrap()), value_style),
    ]))
}

/// "JACKPOT" dim when unearned, bright magenta and bold once lit.
fn jackpot_line(lit: bool) -> Line<'static> {
    let style = if lit {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Line::from(Span::styled("JACKPOT", style))
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

fn ball_indicator(remaining: u8) -> String {
    const MAX: u8 = 3;
    const FULL: &str = "●";
    const EMPTY: &str = "○";
    (0..MAX).map(|i| if i < remaining { FULL } else { EMPTY }).collect::<Vec<_>>().join(" ")
}
