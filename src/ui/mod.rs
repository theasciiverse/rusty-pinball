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

    let snap = physics.snapshot();
    let braille = renderer.render(&snap, camera_y, inner.width, inner.height);

    let text: Text = braille
        .lines()
        .map(|l| Line::from(Span::styled(l.to_owned(), Style::default().fg(Color::White))))
        .collect::<Vec<_>>()
        .into();

    frame.render_widget(Paragraph::new(text), inner);
}
