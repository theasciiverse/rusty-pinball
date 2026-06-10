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

pub fn draw_menu(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool, high_scores: &[u64; 8]) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(16), // title block (6 + blank + 6 + flipper line + 2 border)
            Constraint::Min(5),     // menu list
        ])
        .split(area);

    draw_title(frame, chunks[0]);
    draw_list(frame, chunks[1], state, sound_enabled, high_scores);
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

fn draw_list(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool, high_scores: &[u64; 8]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" SELECT TABLE ", Style::default().fg(Color::Yellow)))
        .style(Style::default().fg(Color::Rgb(60, 50, 10)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let normal    = Style::default().fg(Color::White);
    let sel       = Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD);
    let dim       = Style::default().fg(Color::DarkGray);
    let score_col = Style::default().fg(Color::Rgb(200, 170, 60));

    let sound_label = if sound_enabled { "Sound: [ ON ]" } else { "Sound: [OFF]" };
    let hint        = "↑↓  Navigate    Enter  Select";

    // Fixed layout widths so the score column is always right-aligned.
    let longest_name = TABLE_NAMES.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    // prefix = "▶  1. " = 6 chars (cursor + 2 spaces + 2-digit number + dot + space)
    let prefix_w = 6;
    let score_w  = 11; // "999,999,999" = 11 chars
    let gap      = 2;
    let row_w    = prefix_w + longest_name + gap + score_w;

    let mut lines: Vec<Line> = Vec::new();

    for (i, &name) in TABLE_NAMES.iter().enumerate() {
        let is_sel = state.selected == i;
        let cursor = if is_sel { "▶" } else { " " };
        let score_str = format_score_col(high_scores[i]);
        let dots_count = row_w
            .saturating_sub(prefix_w + name.chars().count() + score_str.chars().count());
        let dots = "·".repeat(dots_count);

        let row_style = if is_sel { sel } else { normal };
        let sc_style  = if is_sel { sel } else { score_col };
        let dt_style  = if is_sel { sel } else { dim };

        lines.push(Line::from(vec![
            Span::styled(format!("{}  {:>2}. {}", cursor, i + 1, name), row_style),
            Span::styled(dots, dt_style),
            Span::styled(score_str, sc_style),
        ]));
    }

    lines.push(Line::from(Span::styled("─".repeat(row_w), dim)));

    lines.push(Line::from(vec![
        Span::styled(
            format!("{}  {:<width$}", if state.selected == ITEM_SOUND { "▶" } else { " " }, sound_label, width = row_w - 3),
            if state.selected == ITEM_SOUND { sel } else { normal },
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}  {:<width$}", if state.selected == ITEM_EXIT { "▶" } else { " " }, "Exit", width = row_w - 3),
            if state.selected == ITEM_EXIT { sel } else { normal },
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{:<width$}", hint, width = row_w),
        dim,
    )));

    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

fn format_score_col(score: u64) -> String {
    if score == 0 {
        return "──────────".to_owned();
    }
    // Insert thousands separators.
    let s = score.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { out.push(','); }
        out.push(ch);
    }
    out.chars().rev().collect()
}
