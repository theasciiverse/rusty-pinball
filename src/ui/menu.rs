use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub const TABLE_NAMES: [&str; 9] = [
    "Classic",
    "Round and Round",
    "Ringmaster",
    "Compound Interest",
    "Eye of Horus",
    "Orbiters",
    "Ad Astra",
    "Pool Party",
    "Party Land",
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

pub fn draw_menu(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool, high_scores: &[u64; 9]) {
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

fn draw_list(frame: &mut Frame, area: Rect, state: &MenuState, sound_enabled: bool, high_scores: &[u64; 9]) {
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

    // Arrow column (3 chars) sits to the left of the content column (50 chars).
    // Together they are 53 chars — the same as the PINBALL title art — so both
    // blocks share the same visual centre when centred on screen.
    const ARROW_W:   u16   = 3;
    const CONTENT_W: u16   = 50;
    const ROW_W:     usize = CONTENT_W as usize;
    // " {:>2}. " prefix = 1sp + 2-char-num + ". " = 5 chars before the name.
    const PREFIX_W:  usize = 5;

    let mut arrow_lines:   Vec<Line> = Vec::new();
    let mut content_lines: Vec<Line> = Vec::new();

    for (i, &name) in TABLE_NAMES.iter().enumerate() {
        let is_sel    = state.selected == i;
        let score_str = format_score_col(high_scores[i]);
        let dots      = "·".repeat(ROW_W
            .saturating_sub(PREFIX_W + name.chars().count() + score_str.chars().count()));

        let row_style = if is_sel { sel } else { normal };
        let sc_style  = if is_sel { sel } else { score_col };
        let dt_style  = if is_sel { sel } else { dim };

        arrow_lines.push(Line::from(Span::styled(
            if is_sel { "▶  " } else { "   " }, row_style,
        )));
        content_lines.push(Line::from(vec![
            Span::styled(format!(" {:>2}. {}", i + 1, name), row_style),
            Span::styled(dots, dt_style),
            Span::styled(score_str, sc_style),
        ]));
    }

    // Separator
    arrow_lines.push(Line::from(Span::styled("   ", dim)));
    content_lines.push(Line::from(Span::styled("─".repeat(ROW_W), dim)));

    // Sound / Exit
    for (idx, label) in [(ITEM_SOUND, sound_label), (ITEM_EXIT, "Exit")] {
        let is_sel = state.selected == idx;
        arrow_lines.push(Line::from(Span::styled(
            if is_sel { "▶  " } else { "   " },
            if is_sel { sel } else { normal },
        )));
        content_lines.push(Line::from(Span::styled(
            format!("     {:<width$}", label, width = ROW_W - 5),
            if is_sel { sel } else { normal },
        )));
    }

    // Hint
    arrow_lines.push(Line::from(""));
    content_lines.push(Line::from(""));
    arrow_lines.push(Line::from(""));
    content_lines.push(Line::from(Span::styled(
        format!("     {:<width$}", hint, width = ROW_W - 5),
        dim,
    )));

    // Place the arrow+content block centred in the inner area.
    let left_off = inner.width.saturating_sub(ARROW_W + CONTENT_W) / 2;
    let arrow_area = Rect {
        x: inner.x + left_off,
        y: inner.y,
        width: ARROW_W,
        height: inner.height,
    };
    let content_area = Rect {
        x: inner.x + left_off + ARROW_W,
        y: inner.y,
        width: CONTENT_W,
        height: inner.height,
    };

    frame.render_widget(Paragraph::new(arrow_lines),   arrow_area);
    frame.render_widget(Paragraph::new(content_lines), content_area);
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
