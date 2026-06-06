use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub const TABLE_NAMES: [&str; 8] = [
    "Classic",
    "Round and Round",
    "Ringmaster",
    "Compound Interest",
    "Eye of Horus",
    "Orbiters",
    "Ad Astra",
    "Pool Party",
];

const ITEM_SOUND: usize = TABLE_NAMES.len();
const ITEM_EXIT:  usize = TABLE_NAMES.len() + 1;

pub struct MenuState {
    pub selected: usize,
}

pub enum MenuAction {
    None,
    StartTable(u8),
    ToggleSound,
    Exit,
}

impl MenuState {
    pub fn new() -> Self { Self { selected: 0 } }
    pub fn item_count() -> usize { TABLE_NAMES.len() + 2 }

    pub fn move_up(&mut self) {
        if self.selected > 0 { self.selected -= 1; }
    }
    pub fn move_down(&mut self) {
        if self.selected < Self::item_count() - 1 { self.selected += 1; }
    }
    pub fn select(&self) -> MenuAction {
        if self.selected < TABLE_NAMES.len() {
            MenuAction::StartTable(self.selected as u8 + 1)
        } else if self.selected == ITEM_SOUND {
            MenuAction::ToggleSound
        } else {
            MenuAction::Exit
        }
    }
}

// ── ASCII title art ──────────────────────────────────────────────────────────

const RUSTY: [&str; 6] = [
    "██████╗ ██╗   ██╗███████╗████████╗██╗   ██╗",
    "██╔══██╗██║   ██║██╔════╝╚══██╔══╝╚██╗ ██╔╝",
    "██████╔╝██║   ██║███████╗   ██║    ╚████╔╝ ",
    "██╔══██╗██║   ██║╚════██║   ██║     ╚██╔╝  ",
    "██║  ██║╚██████╔╝███████║   ██║      ██║   ",
    "╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝      ╚═╝  ",
];

const PINBALL: [&str; 6] = [
    "██████╗ ██╗███╗   ██╗██████╗  █████╗ ██╗     ██╗     ",
    "██╔══██╗██║████╗  ██║██╔══██╗██╔══██╗██║     ██║     ",
    "██████╔╝██║██╔██╗ ██║██████╔╝███████║██║     ██║     ",
    "██╔═══╝ ██║██║╚██╗██║██╔══██╗██╔══██║██║     ██║     ",
    "██║     ██║██║ ╚████║██████╔╝██║  ██║███████╗███████╗",
    "╚═╝     ╚═╝╚═╝  ╚═══╝╚═════╝ ╚═╝  ╚═╝╚══════╝╚══════╝",
];

// ── Public draw entry point ───────────────────────────────────────────────────

pub fn draw_menu(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(16), // title block (6 + blank + 6 + flipper line + 2 border)
            Constraint::Min(5),     // menu list
        ])
        .split(area);

    draw_title(frame, chunks[0]);
    draw_list(frame, chunks[1], state, sound_enabled);
}

// ── Title block ───────────────────────────────────────────────────────────────

fn draw_title(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Rgb(80, 60, 0)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let orange = Style::default()
        .fg(Color::Rgb(255, 140, 0))
        .add_modifier(Modifier::BOLD);
    let gold = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Rgb(80, 60, 20));

    let mut lines: Vec<Line> = Vec::new();

    for row in RUSTY {
        lines.push(Line::from(Span::styled(row.to_owned(), orange)));
    }

    lines.push(Line::from(""));

    for row in PINBALL {
        lines.push(Line::from(Span::styled(row.to_owned(), gold)));
    }

    lines.push(Line::from(vec![
        Span::styled("  ╲──────╯", dim),
        Span::raw("         "),
        Span::styled("●", Style::default().fg(Color::Rgb(220, 180, 60))),
        Span::raw("         "),
        Span::styled("╰──────╱  ", dim),
    ]));

    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        inner,
    );
}

// ── Table / option list ───────────────────────────────────────────────────────

fn draw_list(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" SELECT TABLE ", Style::default().fg(Color::Yellow)))
        .style(Style::default().fg(Color::Rgb(60, 50, 10)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let normal = Style::default().fg(Color::White);
    let sel    = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let dim    = Style::default().fg(Color::DarkGray);

    let mut lines: Vec<Line> = Vec::new();

    for (i, &name) in TABLE_NAMES.iter().enumerate() {
        let style = if state.selected == i { sel } else { normal };
        let cursor = if state.selected == i { "▶" } else { " " };
        lines.push(Line::from(Span::styled(
            format!(" {}  {:>2}. {}", cursor, i + 1, name),
            style,
        )));
    }

    lines.push(Line::from(Span::styled(
        "    ────────────────────────",
        dim,
    )));

    let sound_label = if sound_enabled { "  Sound: [ ON ]" } else { "  Sound: [OFF]" };
    lines.push(Line::from(Span::styled(
        format!(" {}  {}", if state.selected == ITEM_SOUND { "▶" } else { " " }, sound_label),
        if state.selected == ITEM_SOUND { sel } else { normal },
    )));

    lines.push(Line::from(Span::styled(
        format!(" {}  Exit", if state.selected == ITEM_EXIT { "▶" } else { " " }),
        if state.selected == ITEM_EXIT { sel } else { normal },
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓  Navigate    Enter  Select",
        dim,
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}
