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
    persist,
    physics::PhysicsWorld,
    renderer::FieldRenderer,
    ui::menu::{MenuState, MenuAction},
};

const FRAME_MILLIS:  u64 = 33;

enum Screen {
    Menu,
    Playing {
        table_idx: u8,
        game:      GameState,
        physics:   PhysicsWorld,
        input:     InputState,
        camera_y:  f32,
    },
}

pub struct App {
    terminal:      Terminal<CrosstermBackend<io::Stdout>>,
    audio:         Option<AudioManager>,
    sound_enabled: bool,
    high_scores:   [u64; 9],
    renderer:      FieldRenderer,
    menu:          MenuState,
    screen:        Screen,
    paused:        bool,
    debug_ids:     bool,
}

impl App {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
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

        let saved = persist::load();
        let audio = AudioManager::try_new();
        if let Some(a) = &audio {
            // prime the audio system with the saved preference
            let _ = a; // set_enabled called below via the field
        }
        let mut app = Self {
            terminal,
            audio,
            sound_enabled: saved.sound_enabled,
            high_scores:   saved.high_scores,
            renderer:      FieldRenderer::new(),
            menu:          MenuState::new(),
            screen:        Screen::Menu,
            paused:        false,
            debug_ids:     false,
        };
        if let Some(a) = &mut app.audio {
            a.set_enabled(app.sound_enabled);
        }
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

        // Pause toggle — P key while playing (not on GameOver screen).
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

        // Debug object-ID overlay toggle — I key while playing.
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
                self.screen = Screen::Menu;
                self.paused = false;
                return Ok(false);
            }
        }

        if matches!(&self.screen, Screen::Menu) {
            return self.handle_menu_key(k);
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
                    MenuAction::ToggleSound   => {
                        self.sound_enabled = !self.sound_enabled;
                        if let Some(a) = &mut self.audio {
                            a.set_enabled(self.sound_enabled);
                        }
                        persist::save(&persist::SavedState {
                            sound_enabled: self.sound_enabled,
                            high_scores:   self.high_scores,
                        });
                    }
                    MenuAction::Exit          => return Ok(true),
                    MenuAction::None          => {}
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_play_key(&mut self, k: KeyEvent) {
        // GameOver restart must call start_table(&mut self), so handle it before
        // taking any other borrows from self.
        if k.kind == KeyEventKind::Press && k.code == KeyCode::Char(' ') {
            let is_game_over = matches!(&self.screen, Screen::Playing { game, .. } if game.phase == Phase::GameOver);
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
                if self.sound_enabled {
                    if let Some(a) = &mut self.audio { a.play(SoundTrigger::BallLaunch); }
                }
                return;
            }
        }

        let Screen::Playing { game, physics, input, .. } = &mut self.screen else { return };

        input.handle_key(k);

        if k.kind == KeyEventKind::Press && k.code == KeyCode::Char(' ') {
            match game.phase {
                Phase::WaitingToLaunch | Phase::BallLost => {
                    physics.launch_ball();
                    game.phase = Phase::Playing;
                    if self.sound_enabled {
                        if let Some(a) = &mut self.audio { a.play(SoundTrigger::BallLaunch); }
                    }
                }
                _ => {}
            }
        }

        if k.kind == KeyEventKind::Press && self.sound_enabled {
            let is_left  = matches!(k.code, KeyCode::Char('z') | KeyCode::Left
                | KeyCode::Modifier(ModifierKeyCode::LeftShift));
            let is_right = matches!(k.code, KeyCode::Char('x') | KeyCode::Right
                | KeyCode::Modifier(ModifierKeyCode::RightShift));
            if (is_left || is_right) && game.phase == Phase::Playing {
                if let Some(a) = &mut self.audio { a.play(SoundTrigger::FlipperUp); }
            }
        }
    }

    fn start_table(&mut self, n: u8) -> Result<()> {
        let table   = crate::table::load_table(n)?;
        let balls   = table.num_balls();
        let physics = PhysicsWorld::from_table(&table);
        let mut game = GameState::new(balls);
        game.high_score = self.high_scores[(n as usize).saturating_sub(1).min(8)];
        self.paused = false;
        self.screen = Screen::Playing {
            table_idx: n,
            game,
            physics,
            input:     InputState::default(),
            camera_y:  0.0,
        };
        Ok(())
    }

    fn tick(&mut self) {
        // Run physics and collect sound triggers in one borrow block, then
        // release the borrow before accessing self.audio.
        let (sound_events, ball_drained) =
            if let Screen::Playing { game, physics, input, camera_y, .. } = &mut self.screen
        {
            if self.paused { return; }
            input.tick();
            physics.step_frame(FRAME_MILLIS as f32 / 1000.0, input);
            let prev_phase = game.phase;
            game.sync(physics);

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

            let events: Vec<_> = physics.drain_sound_events().into_iter().collect();
            let drained = prev_phase == Phase::Playing && game.phase != Phase::Playing;
            if drained { physics.park_ball(); }
            (events, drained)
        } else {
            return;
        };

        if self.sound_enabled {
            if let Some(audio) = &mut self.audio {
                use crate::physics::PhysicsSound;
                for ev in sound_events {
                    match ev {
                        PhysicsSound::BumperHit => audio.play(SoundTrigger::BumperHit),
                    }
                }
                if ball_drained {
                    audio.play(SoundTrigger::BallDrain);
                }
            }
        }

        // Persist new high score when a ball is lost.
        if ball_drained {
            if let Screen::Playing { table_idx, game, .. } = &self.screen {
                let slot = (*table_idx as usize).saturating_sub(1).min(8);
                if game.high_score > self.high_scores[slot] {
                    self.high_scores[slot] = game.high_score;
                    persist::save(&persist::SavedState {
                        sound_enabled: self.sound_enabled,
                        high_scores:   self.high_scores,
                    });
                }
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        match &self.screen {
            Screen::Menu => {
                let menu          = &self.menu;
                let sound_enabled = self.sound_enabled;
                let high_scores   = self.high_scores;
                self.terminal.draw(|frame| {
                    crate::ui::draw_menu(frame, menu, sound_enabled, &high_scores);
                })?;
            }
            Screen::Playing { game, physics, camera_y, .. } => {
                let game      = game;
                let physics   = physics;
                let renderer  = &self.renderer;
                let camera_y  = *camera_y;
                let paused    = self.paused;
                let debug_ids = self.debug_ids;
                self.terminal.draw(|frame| {
                    crate::ui::draw(frame, physics, game, renderer, camera_y, paused, debug_ids);
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
