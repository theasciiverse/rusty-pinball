mod dmd;
pub mod menu;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

use crate::{game::GameState, physics::PhysicsWorld, renderer::FieldRenderer};

pub fn draw_menu(frame: &mut Frame, state: &menu::MenuState, sound_enabled: bool) {
    let area = frame.area();
    menu::draw_menu(frame, area, state, sound_enabled);
}

const DMD_HEIGHT: u16 = 6;

pub fn draw(
    frame: &mut Frame,
    physics: &PhysicsWorld,
    game: &GameState,
    renderer: &FieldRenderer,
    camera_y: f32,
) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),            // play area — takes all remaining space
            Constraint::Length(DMD_HEIGHT), // DMD bar at bottom
        ])
        .split(area);

    draw_play_area(frame, chunks[0], physics, renderer, camera_y);
    dmd::draw_dmd(frame, chunks[1], game);
}

fn draw_play_area(
    frame: &mut Frame,
    area: Rect,
    physics: &PhysicsWorld,
    renderer: &FieldRenderer,
    camera_y: f32,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" RUSTTY-PIN ")
        .style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let snap     = physics.snapshot();
    let rendered = renderer.render(&snap, camera_y, inner.width, inner.height);

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
