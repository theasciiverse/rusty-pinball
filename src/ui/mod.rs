mod dmd;
pub mod menu;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{game::GameState, physics::{FieldSnapshot, PhysicsWorld}, renderer::FieldRenderer};

pub fn draw_menu(frame: &mut Frame, state: &menu::MenuState, settings: &menu::MenuSettings, high_scores: &[u64; 9]) {
    let area = frame.area();
    menu::draw_menu(frame, area, state, settings, high_scores);
}

/// What the play screen shows beyond the table itself.
#[derive(Clone, Copy)]
pub struct ViewOptions {
    pub score_animations: bool,
    pub ball_trails: bool,
}

const DMD_HEIGHT: u16 = 6;

pub fn draw(
    frame: &mut Frame,
    physics: &PhysicsWorld,
    game: &GameState,
    renderer: &FieldRenderer,
    camera_y: f32,
    paused: bool,
    debug_ids: bool,
    view: ViewOptions,
) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),            // play area: takes all remaining space
            Constraint::Length(DMD_HEIGHT), // DMD bar at bottom
        ])
        .split(area);

    let mut snap = physics.snapshot();
    if view.score_animations { snap.popups = game.popup_snapshots(); }
    if !view.ball_trails { for b in &mut snap.balls { b.trail.clear(); } }
    draw_play_area(frame, chunks[0], &snap, renderer, camera_y, debug_ids);
    dmd::draw_dmd(frame, chunks[1], game);

    if paused {
        draw_pause_overlay(frame, area);
    }
}

fn draw_pause_overlay(frame: &mut Frame, area: Rect) {
    // Centre a 26×7 box over the play area.
    let w: u16 = 26;
    let h: u16 = 7;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect { x, y, width: w.min(area.width), height: h.min(area.height) };

    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow).bg(Color::Black));

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ▐▌ PAUSED ▐▌",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Press P to resume",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );
}

fn draw_play_area(
    frame: &mut Frame,
    area: Rect,
    snap: &FieldSnapshot,
    renderer: &FieldRenderer,
    camera_y: f32,
    debug_ids: bool,
) {
    let title = if debug_ids { " RUSTY PINBALL [IDs] " } else { " RUSTY PINBALL " };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rendered = renderer.render(snap, camera_y, inner.width, inner.height, debug_ids);

    let cols = rendered.char_cols as usize;
    let rows = rendered.char_rows as usize;

    let text: Text = (0..rows).map(|cy| {
        let start = cy * cols;
        let row_chars  = &rendered.chars [start..start + cols];
        let row_colors = &rendered.colors[start..start + cols];

        // Group adjacent same-color characters into one Span each.
        let mut spans: Vec<Span> = Vec::new();
        let mut cur_color = Color::Reset;
        let mut cur_text  = String::new();

        for (&ch, &color) in row_chars.iter().zip(row_colors.iter()) {
            if color != cur_color {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(
                        std::mem::take(&mut cur_text),
                        Style::default().fg(cur_color),
                    ));
                }
                cur_color = color;
            }
            cur_text.push(ch);
        }
        if !cur_text.is_empty() {
            spans.push(Span::styled(cur_text, Style::default().fg(cur_color)));
        }

        Line::from(spans)
    }).collect::<Vec<_>>().into();

    frame.render_widget(Paragraph::new(text), inner);
}
