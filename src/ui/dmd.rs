/// Dot-Matrix Display bar rendered at the bottom of the terminal.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::game::GameState;

pub fn draw_dmd(frame: &mut Frame, area: Rect, game: &GameState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" DMD ")
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

    // Left column: score labels + values
    let score_style  = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let hi_style     = Style::default().fg(Color::Cyan);
    let label_style  = Style::default().fg(Color::DarkGray);

    let left = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("SCORE     ", label_style),
            Span::styled(format_score(game.score), score_style),
        ]),
        Line::from(vec![
            Span::styled("HI SCORE  ", label_style),
            Span::styled(format_score(game.high_score), hi_style),
        ]),
    ]);
    frame.render_widget(left, cols[0]);

    // Centre column: ball indicators
    let ball_dots = ball_indicator(game.balls_remaining);
    let centre = Paragraph::new(Line::from(vec![
        Span::styled("BALLS  ", label_style),
        Span::styled(ball_dots, Style::default().fg(Color::Red)),
    ]));
    frame.render_widget(centre, cols[1]);

    // Right column: multiplier / status + escape hint
    let status = match game.phase {
        crate::game::Phase::WaitingToLaunch => Span::styled("PRESS SPACE", Style::default().fg(Color::Green)),
        crate::game::Phase::Playing         => Span::styled(format!("×{}", game.multiplier), score_style),
        crate::game::Phase::BallLost        => Span::styled("BALL LOST",   Style::default().fg(Color::Red)),
        crate::game::Phase::GameOver        => Span::styled("GAME OVER",   Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    };
    let hint_style = Style::default().fg(Color::DarkGray);
    let right = Paragraph::new(vec![
        Line::from(status),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("<esc> to menu", hint_style)),
    ])
    .alignment(Alignment::Right);
    frame.render_widget(right, cols[2]);
}

fn format_score(s: u64) -> String {
    // Format with comma separators: 1234567 → "1,234,567"
    let s = s.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn ball_indicator(remaining: u8) -> String {
    // Use braille-filled circles for remaining balls, empty for lost ones
    const MAX: u8 = 3;
    const FULL: &str = "●";
    const EMPTY: &str = "○";
    (0..MAX).map(|i| if i < remaining { FULL } else { EMPTY }).collect::<Vec<_>>().join(" ")
}
