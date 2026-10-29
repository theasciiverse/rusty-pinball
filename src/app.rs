use std::time::{Duration, Instant};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyEvent, ModifierKeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use crate::{
    audio::{AudioManager, SoundTrigger},
    game::{GameState, Phase},
    input::InputState,
    persist::{self, SavedState},
    physics::PhysicsWorld,
    renderer::FieldRenderer,
    ui::{menu::{MenuState, MenuAction, TABLE_NAMES}, screens::{self, SETTINGS}},
};

const FRAME_MILLIS:  u64 = 33;
/// A launch (or restart) press this soon after a ball was lost is ignored, as the original
/// ignores touches for 750 ms, so a ball lost while mashing the key does not launch the next.
const END_BALL_DELAY: Duration = Duration::from_millis(750);

/// Frames (33 ms) Space may be held and still count as a tap: the table's own launch.
const PLUNGER_TAP_FRAMES: u32 = 6;
/// Frames of holding, after a tap's, that pull the plunger all the way back.
const PLUNGER_PULL_FRAMES: u32 = 30;
/// Launch strength, as a factor on the table's launch speed, from the first
/// moment the plunger is held to the full pull. A tap launches at 1.0, as does
/// half a pull. A soft shot may roll back into the lane, as on a real table.
const PLUNGER_SOFTEST: f32 = 0.85;
const PLUNGER_HARDEST: f32 = 1.15;
/// How long each line of the after-game cycle shows, as in the original's score view.
const IDLE_CYCLE: Duration = Duration::from_millis(3500);

/// How far the plunger is pulled after `frames` held, 0..1 (0 while it is still a tap).
fn plunger_pull(frames: u32) -> f32 {
    (frames.saturating_sub(PLUNGER_TAP_FRAMES) as f32 / PLUNGER_PULL_FRAMES as f32).min(1.0)
}

/// The launch strength for Space released after `frames` held.
fn plunger_strength(frames: u32) -> f32 {
    if frames < PLUNGER_TAP_FRAMES { return 1.0; }
    PLUNGER_SOFTEST + (PLUNGER_HARDEST - PLUNGER_SOFTEST) * plunger_pull(frames)
}

/// The lines the status cycles through after a game: game over, the last
/// score, then each high score, as the original's score view does.
fn idle_lines(last: u64, top: &[u64]) -> Vec<String> {
    let mut out = vec![String::new()];
    if last > 0 { out.push(format!("LAST SCORE {}", crate::ui::menu::format_score_col(last))); }
    for (i, &s) in top.iter().enumerate() {
        if s > 0 { out.push(format!("HIGH SCORE #{} {}", i + 1, crate::ui::menu::format_score_col(s))); }
    }
    out
}

/// A ball was lost when the game state left `Playing` in its own sync.
fn ball_lost_by_sync(before: Phase, after_sync: Phase) -> bool {
    before == Phase::Playing && after_sync != Phase::Playing
}

/// True while a press should still be ignored after a ball was lost at `lost_at`.
fn launch_locked(lost_at: Option<Instant>, now: Instant) -> bool {
    lost_at.map_or(false, |t| now.saturating_duration_since(t) < END_BALL_DELAY)
}

enum Screen {
    Menu,
    Settings { selected: usize },
    Scores { table: usize },
    /// The rules of `table` (0-based); `game` is the game it was opened from, if any.
    Help { table: usize, scroll: u16, game: Option<Box<Screen>> },
    Playing {
        table_idx: u8,
        game:      GameState,
        physics:   PhysicsWorld,
        input:     InputState,
        camera_y:  f32,
        /// Frames Space has been held for a launch, if it is.
        plunger:   Option<u32>,
        /// Whether this game's score has been recorded.
        recorded:  bool,
        game_over_at: Option<Instant>,
    },
}

pub struct App {
    terminal:      Terminal<CrosstermBackend<io::Stdout>>,
    audio:         Option<AudioManager>,
    saved:         SavedState,
    renderer:      FieldRenderer,
    menu:          MenuState,
    screen:        Screen,
    paused:        bool,
    debug_ids:     bool,
    ball_lost_at:  Option<Instant>,
    /// The terminal reports key releases (the kitty keyboard protocol), so
    /// Space can be held to pull the plunger.
    release_keys:  bool,
}

impl App {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let release_keys = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        // Request key-release events, and bare modifier keys (so Left/Right Shift
        // can work the flippers), on terminals that support the Kitty keyboard
        // protocol. Silently ignored on terminals that don't.
        let _ = execute!(stdout,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | crossterm::event::KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;

        let mut app = Self {
            terminal,
            audio:         AudioManager::try_new(),
            saved:         persist::load(),
            renderer:      FieldRenderer::new(),
            menu:          MenuState::new(),
            screen:        Screen::Menu,
            paused:        false,
            debug_ids:     false,
            ball_lost_at:  None,
            release_keys,
        };
        app.apply_audio();
        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
        let frame_dur = Duration::from_millis(FRAME_MILLIS);
        let mut last_tick = Instant::now();

        loop {
            if event::poll(Duration::from_millis(5))? {
                if let Event::Key(k) = event::read()? {
                    if self.handle_key(k)? {
                        break;
                    }
                }
            }

            if last_tick.elapsed() >= frame_dur {
                self.tick();
                last_tick = Instant::now();
                self.render()?;
            }
        }
        Ok(())
    }

    // Returns true when the app should quit entirely.
    fn handle_key(&mut self, k: KeyEvent) -> Result<bool> {
        let is_playing = matches!(&self.screen, Screen::Playing { .. });

        // Help for the table in play: pauses the game behind it.
        if is_playing && k.kind != KeyEventKind::Release
            && matches!(k.code, KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::F(1))
        {
            let table = match &self.screen { Screen::Playing { table_idx, .. } => *table_idx as usize - 1, _ => 0 };
            let game = std::mem::replace(&mut self.screen, Screen::Menu);
            self.paused = true;
            self.screen = Screen::Help { table, scroll: 0, game: Some(Box::new(game)) };
            return Ok(false);
        }

        // Pause toggle; P key while playing (not on GameOver screen).
        if is_playing && k.kind != KeyEventKind::Release
            && matches!(k.code, KeyCode::Char('p') | KeyCode::Char('P'))
        {
            let is_game_over = matches!(&self.screen,
                Screen::Playing { game, .. } if game.phase == Phase::GameOver);
            if !is_game_over {
                self.paused = !self.paused;
                return Ok(false);
            }
        }

        // Debug object-ID overlay toggle; I key while playing.
        if is_playing && k.kind != KeyEventKind::Release
            && matches!(k.code, KeyCode::Char('i') | KeyCode::Char('I'))
        {
            self.debug_ids = !self.debug_ids;
            return Ok(false);
        }

        // Escape from playing → back to menu (also clears pause).
        if is_playing {
            if k.kind != KeyEventKind::Release
                && (k.code == KeyCode::Esc || k.code == KeyCode::Char('q'))
            {
                // Leaving a game ends it, and it counts, as the original's End Game.
                self.record_game();
                self.screen = Screen::Menu;
                self.paused = false;
                return Ok(false);
            }
        }

        if matches!(&self.screen, Screen::Menu) {
            return self.handle_menu_key(k);
        }
        if matches!(&self.screen, Screen::Settings { .. } | Screen::Scores { .. } | Screen::Help { .. }) {
            if k.kind != KeyEventKind::Release { self.handle_screen_key(k); }
            return Ok(false);
        }

        self.handle_play_key(k);
        Ok(false)
    }

    fn handle_menu_key(&mut self, k: KeyEvent) -> Result<bool> {
        // With all-keys reporting on, terminals also send releases for ordinary
        // keys; acting on them would double-fire Enter and quit on a q release.
        if k.kind == KeyEventKind::Release {
            return Ok(false);
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(true),
            KeyCode::Up   | KeyCode::Char('k') => self.menu.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.menu.move_down(),
            KeyCode::Enter => {
                match self.menu.select() {
                    MenuAction::StartTable(n) => self.start_table(n)?,
                    MenuAction::Settings      => self.screen = Screen::Settings { selected: 0 },
                    MenuAction::Scores        => self.screen = Screen::Scores { table: self.menu.table },
                    MenuAction::Help          => self.screen = Screen::Help { table: self.menu.table, scroll: 0, game: None },
                    MenuAction::Exit          => return Ok(true),
                    MenuAction::None          => {}
                }
            }
            _ => {}
        }
        Ok(false)
    }

    /// Keys on the Settings, High scores and Help screens.
    fn handle_screen_key(&mut self, k: KeyEvent) {
        let tables = TABLE_NAMES.len();
        let back = matches!(k.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q'));
        match &mut self.screen {
            Screen::Settings { selected } => {
                let item = SETTINGS[*selected];
                match k.code {
                    _ if back => self.screen = Screen::Menu,
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1).min(SETTINGS.len() - 1),
                    KeyCode::Enter | KeyCode::Char(' ') if item == screens::Setting::Back => self.screen = Screen::Menu,
                    KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                        item.change(&mut self.saved, false);
                        self.apply_audio();
                        self.save_state();
                    }
                    KeyCode::Left | KeyCode::Char('h') if item != screens::Setting::Back => {
                        item.change(&mut self.saved, true);
                        self.apply_audio();
                        self.save_state();
                    }
                    _ => {}
                }
            }
            Screen::Scores { table } => match k.code {
                _ if back => self.screen = Screen::Menu,
                KeyCode::Left | KeyCode::Char('h') => *table = (*table + tables - 1) % tables,
                KeyCode::Right | KeyCode::Char('l') => *table = (*table + 1) % tables,
                _ => {}
            },
            Screen::Help { table, scroll, game } => match k.code {
                KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::F(1) | KeyCode::Enter if game.is_some() => {
                    if let Some(g) = game.take() { self.screen = *g; }
                }
                _ if back => {
                    self.screen = match game.take() { Some(g) => *g, None => Screen::Menu };
                }
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                KeyCode::PageDown => *scroll += 10,
                KeyCode::Left => { *table = (*table + tables - 1) % tables; *scroll = 0; }
                KeyCode::Right => { *table = (*table + 1) % tables; *scroll = 0; }
                _ => {}
            },
            _ => {}
        }
    }

    fn apply_audio(&mut self) {
        if let Some(a) = &mut self.audio {
            a.set_effects(self.saved.sound_enabled);
            a.set_music(self.saved.music_enabled);
        }
    }

    /// Records the game in play, once: its score as the last on its table, and
    /// among the five best if it beats one. Not with unlimited balls.
    fn record_game(&mut self) {
        let Screen::Playing { table_idx, game, recorded, .. } = &mut self.screen else { return };
        if *recorded || game.unlimited_balls || game.score == 0 { return; }
        *recorded = true;
        let slot = (*table_idx as usize).saturating_sub(1).min(8);
        self.saved.record(slot, game.score);
        self.save_state();
    }

    fn handle_play_key(&mut self, k: KeyEvent) {
        // GameOver restart must call start_table(&mut self), so handle it before
        // taking any other borrows from self.
        if k.kind == KeyEventKind::Press && k.code == KeyCode::Char(' ') {
            let is_game_over = matches!(&self.screen, Screen::Playing { game, .. } if game.phase == Phase::GameOver);
            if is_game_over && launch_locked(self.ball_lost_at, Instant::now()) { return; }
            if is_game_over {
                let table_n = match &self.screen {
                    Screen::Playing { table_idx, .. } => *table_idx,
                    _ => return,
                };
                let _ = self.start_table(table_n);
                if let Screen::Playing { game, physics, .. } = &mut self.screen {
                    physics.launch_ball();
                    game.phase = Phase::Playing;
                }
                return;
            }
        }

        let lost_at = self.ball_lost_at;
        let release_keys = self.release_keys;
        let Screen::Playing { game, physics, input, plunger, .. } = &mut self.screen else { return };

        input.handle_key(k);

        if k.code == KeyCode::Char(' ') {
            let ready = matches!(game.phase, Phase::WaitingToLaunch | Phase::BallLost)
                && !launch_locked(lost_at, Instant::now());
            match k.kind {
                // Where releases are reported Space pulls the plunger back and its
                // release launches; elsewhere it launches as it is pressed.
                KeyEventKind::Press if ready && release_keys => *plunger = Some(0),
                KeyEventKind::Press if ready => {
                    // The launch sound comes with the world's launch event.
                    physics.launch_ball();
                    game.phase = Phase::Playing;
                }
                KeyEventKind::Release => {
                    if let Some(frames) = plunger.take() {
                        if matches!(game.phase, Phase::WaitingToLaunch | Phase::BallLost) {
                            physics.launch_ball_with(plunger_strength(frames));
                            game.phase = Phase::Playing;
                        }
                    }
                }
                _ => {}
            }
        }

        if k.kind == KeyEventKind::Press {
            let is_left  = matches!(k.code, KeyCode::Char('z') | KeyCode::Left
                | KeyCode::Modifier(ModifierKeyCode::LeftShift));
            let is_right = matches!(k.code, KeyCode::Char('x') | KeyCode::Right
                | KeyCode::Modifier(ModifierKeyCode::RightShift));
            if (is_left || is_right) && game.phase == Phase::Playing {
                if let Some(a) = &mut self.audio { a.play(SoundTrigger::FlipperUp); }
            }
        }
    }

    fn save_state(&self) {
        persist::save(&self.saved);
    }

    fn start_table(&mut self, n: u8) -> Result<()> {
        let table   = crate::table::load_table(n)?;
        let balls   = table.num_balls();
        let mut physics = PhysicsWorld::from_table(&table);
        physics.set_game_speed(self.saved.game_speed.factor());
        let mut game = GameState::new(balls);
        game.high_score = self.saved.best((n as usize).saturating_sub(1).min(8));
        game.unlimited_balls = self.saved.unlimited_balls;
        self.paused = false;
        self.screen = Screen::Playing {
            table_idx: n,
            game,
            physics,
            input:     InputState::default(),
            camera_y:  0.0,
            plunger:   None,
            recorded:  false,
            game_over_at: None,
        };
        Ok(())
    }

    fn tick(&mut self) {
        // Run physics and collect sound triggers in one borrow block, then
        // release the borrow before accessing self.audio.
        let (sound_events, ball_drained): (Vec<SoundTrigger>, bool) =
            if let Screen::Playing { game, physics, input, camera_y, plunger, .. } = &mut self.screen
        {
            if self.paused { return; }
            input.tick();
            if let Some(frames) = plunger { *frames += 1; }
            physics.step_frame(FRAME_MILLIS as f32 / 1000.0, input);
            let prev_phase = game.phase;
            game.sync(physics);
            // Only the sync ends a ball. A ball lock or a ball that fell back into the
            // shooter lane below also leave `Playing`, but are not a lost ball (the
            // table's `ball_lost` rules, such as table 8's bonus, must not run for them).
            let drained = ball_lost_by_sync(prev_phase, game.phase);

            // A ball lock removed the last ball without it being lost: wait for
            // a relaunch, and do not cost a ball.
            if physics.take_relaunch_needed() && game.phase == Phase::Playing {
                game.phase = Phase::WaitingToLaunch;
            }

            let fh = physics.field_height();
            if let Some(pos) = physics.ball_position() {
                let target = (pos.y - fh * 0.40).clamp(0.0, fh);
                *camera_y += (target - *camera_y) * 0.15;
            }

            // Ball fell back into the shooter lane without clearing the gate. Only
            // with a single ball: re-parking would end a multiball.
            if game.phase == Phase::Playing && physics.ball_count() == 1 && physics.is_ball_in_dead_zone() {
                game.phase = Phase::WaitingToLaunch;
                physics.park_ball();
            }

            use crate::physics::PhysicsSound;
            let mut events: Vec<SoundTrigger> = physics.drain_sound_events().into_iter().map(|e| match e {
                PhysicsSound::Score => SoundTrigger::Score,
                PhysicsSound::Rollover => SoundTrigger::Rollover,
                PhysicsSound::Launch => SoundTrigger::BallLaunch,
            }).collect();
            if game.take_message_sound() { events.push(SoundTrigger::Message); }
            if drained {
                physics.ball_lost();
                physics.park_ball();
                // The original plays its startup sound when the game ends.
                if game.phase == Phase::GameOver { events.push(SoundTrigger::Start); }
            }
            (events, drained)
        } else {
            return;
        };

        if ball_drained { self.ball_lost_at = Some(Instant::now()); }

        // The audio manager plays what its two switches allow, and keeps the
        // music's schedule counting either way.
        if let Some(audio) = &mut self.audio {
            for ev in sound_events {
                audio.play(ev);
            }
        }

        // The game is over: record it, and start the after-game cycle.
        if ball_drained {
            let over = matches!(&self.screen, Screen::Playing { game, .. } if game.phase == Phase::GameOver);
            if over {
                self.record_game();
                if let Screen::Playing { game_over_at, .. } = &mut self.screen { *game_over_at = Some(Instant::now()); }
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        match &self.screen {
            Screen::Menu => {
                let menu          = &self.menu;
                let high_scores: [u64; 9] = std::array::from_fn(|i| self.saved.best(i));
                self.terminal.draw(|frame| {
                    crate::ui::draw_menu(frame, menu, &high_scores);
                })?;
            }
            Screen::Settings { selected } => {
                let (saved, selected) = (&self.saved, *selected);
                self.terminal.draw(|frame| screens::draw_settings(frame, saved, selected))?;
            }
            Screen::Scores { table } => {
                let (saved, table) = (&self.saved, *table);
                self.terminal.draw(|frame| screens::draw_scores(frame, saved, table))?;
            }
            Screen::Help { table, scroll, game } => {
                let (table, in_game, release_keys) = (*table, game.is_some(), self.release_keys);
                let mut shown = *scroll;
                self.terminal.draw(|frame| { shown = screens::draw_help(frame, table, shown, in_game, release_keys); })?;
                if let Screen::Help { scroll, .. } = &mut self.screen { *scroll = shown; }
            }
            Screen::Playing { table_idx, game, physics, camera_y, plunger, game_over_at, .. } => {
                let game      = game;
                let physics   = physics;
                let renderer  = &self.renderer;
                let camera_y  = *camera_y;
                let paused    = self.paused;
                let debug_ids = self.debug_ids;
                let view = crate::ui::ViewOptions {
                    score_animations: self.saved.score_animations,
                    ball_trails: self.saved.ball_trails,
                    whole_table: !self.saved.zoom,
                };
                let slot = (*table_idx as usize).saturating_sub(1).min(8);
                let idle = game_over_at.map(|t| {
                    let lines = idle_lines(self.saved.last_scores[slot], &self.saved.top_scores[slot]);
                    let i = (t.elapsed().as_millis() / IDLE_CYCLE.as_millis()) as usize % lines.len();
                    lines[i].clone()
                }).filter(|l| !l.is_empty());
                let extra = crate::ui::DmdExtra { plunger: plunger.map(plunger_pull), idle };
                self.terminal.draw(|frame| {
                    crate::ui::draw(frame, physics, game, renderer, camera_y, paused, debug_ids, view, &extra);
                })?;
            }
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(),
            crossterm::event::PopKeyboardEnhancementFlags
        );
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_launches_as_the_table_does_and_a_hold_pulls_from_soft_to_hard() {
        assert_eq!(plunger_strength(0), 1.0);
        assert_eq!(plunger_strength(PLUNGER_TAP_FRAMES - 1), 1.0);
        assert_eq!(plunger_pull(PLUNGER_TAP_FRAMES - 1), 0.0, "the meter stays empty during a tap");
        assert!((plunger_strength(PLUNGER_TAP_FRAMES) - PLUNGER_SOFTEST).abs() < 1e-6);
        let mid = plunger_strength(PLUNGER_TAP_FRAMES + PLUNGER_PULL_FRAMES / 2);
        assert!(mid > PLUNGER_SOFTEST && mid < PLUNGER_HARDEST);
        assert!((plunger_strength(PLUNGER_TAP_FRAMES + PLUNGER_PULL_FRAMES) - PLUNGER_HARDEST).abs() < 1e-6);
        assert!((plunger_strength(500) - PLUNGER_HARDEST).abs() < 1e-6, "held longer stays at the full pull");
    }

    #[test]
    fn after_a_game_the_status_cycles_last_score_then_high_scores() {
        let lines = idle_lines(1200, &[5000, 1200]);
        assert_eq!(lines, vec!["".to_string(), "LAST SCORE 1,200".into(), "HIGH SCORE #1 5,000".into(), "HIGH SCORE #2 1,200".into()]);
        assert_eq!(idle_lines(0, &[]), vec![String::new()], "only game over");
    }

    #[test]
    fn only_the_sync_leaving_play_is_a_lost_ball() {
        assert!(ball_lost_by_sync(Phase::Playing, Phase::BallLost));
        assert!(ball_lost_by_sync(Phase::Playing, Phase::GameOver));
        assert!(!ball_lost_by_sync(Phase::Playing, Phase::Playing));
        assert!(!ball_lost_by_sync(Phase::WaitingToLaunch, Phase::WaitingToLaunch));
    }

    #[test]
    fn a_press_just_after_a_ball_is_lost_is_ignored_for_three_quarters_of_a_second() {
        let lost = Instant::now();
        assert!(launch_locked(Some(lost), lost + Duration::from_millis(100)));
        assert!(launch_locked(Some(lost), lost + Duration::from_millis(749)));
        assert!(!launch_locked(Some(lost), lost + Duration::from_millis(750)));
        assert!(!launch_locked(None, lost), "no ball lost yet");
    }
}
