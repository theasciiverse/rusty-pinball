use std::time::{Duration, Instant};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

use crate::{
    audio::{AudioManager, SoundTrigger},
    game::{GameState, Phase},
    input::InputState,
    physics::PhysicsWorld,
    renderer::FieldRenderer,
    ui::menu::{MenuState, MenuAction},
};

const PHYSICS_DT:    f32 = 1.0 / 60.0;
const PHYSICS_STEPS: u32 = 2;
const FRAME_MILLIS:  u64 = 33;

enum Screen {
    Menu,
    Playing {
        _table:   u8,
        game:     GameState,
        physics:  PhysicsWorld,
        input:    InputState,
        camera_y: f32,
    },
}

pub struct App {
    terminal:      Terminal<CrosstermBackend<io::Stdout>>,
    audio:         Option<AudioManager>,
    sound_enabled: bool,
    renderer:      FieldRenderer,
    menu:          MenuState,
    screen:        Screen,
}

impl App {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        // Request key-release events on terminals that support the Kitty keyboard protocol.
        // Silently ignored on terminals that don't.
        let _ = execute!(stdout,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        );
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;

        Ok(Self {
            terminal,
            audio:         AudioManager::try_new(),
            sound_enabled: true,
            renderer:      FieldRenderer::new(),
            menu:          MenuState::new(),
            screen:        Screen::Menu,
        })
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
        // Escape from playing → back to menu
        if matches!(&self.screen, Screen::Playing { .. }) {
            if k.code == KeyCode::Esc || k.code == KeyCode::Char('q') {
                self.screen = Screen::Menu;
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
        let Screen::Playing { game, physics, input, .. } = &mut self.screen else { return };

        input.handle_key(k);

        if k.kind == KeyEventKind::Press
            && k.code == KeyCode::Char(' ')
            && game.phase == Phase::WaitingToLaunch
        {
            physics.launch_ball();
            game.phase = Phase::Playing;
            if self.sound_enabled {
                if let Some(a) = &mut self.audio { a.play(SoundTrigger::BallLaunch); }
            }
        }

        if k.kind == KeyEventKind::Press && self.sound_enabled {
            let is_left  = matches!(k.code, KeyCode::Char('z') | KeyCode::Left);
            let is_right = matches!(k.code, KeyCode::Char('x') | KeyCode::Right);
            if (is_left || is_right) && game.phase == Phase::Playing {
                if let Some(a) = &mut self.audio { a.play(SoundTrigger::FlipperUp); }
            }
        }

        if k.kind == KeyEventKind::Press && k.code == KeyCode::Enter {
            match game.phase {
                Phase::BallLost => {
                    physics.launch_ball();
                    game.phase = Phase::Playing;
                    if self.sound_enabled {
                        if let Some(a) = &mut self.audio { a.play(SoundTrigger::BallLaunch); }
                    }
                }
                Phase::GameOver => {
                    let n = game.balls_remaining;
                    *game = GameState::new(n);
                    physics.launch_ball();
                    game.phase = Phase::Playing;
                    if self.sound_enabled {
                        if let Some(a) = &mut self.audio { a.play(SoundTrigger::BallLaunch); }
                    }
                }
                _ => {}
            }
        }
    }

    fn start_table(&mut self, n: u8) -> Result<()> {
        let table   = crate::table::load_table(n)?;
        let balls   = table.num_balls();
        let physics = PhysicsWorld::from_table(&table);
        let game    = GameState::new(balls);
        self.screen = Screen::Playing {
            _table:   n,
            game,
            physics,
            input:    InputState::default(),
            camera_y: 0.0,
        };
        Ok(())
    }

    fn tick(&mut self) {
        // Run physics and collect sound triggers in one borrow block, then
        // release the borrow before accessing self.audio.
        let (sound_events, ball_drained) =
            if let Screen::Playing { game, physics, input, camera_y, .. } = &mut self.screen
        {
            input.tick();
            for _ in 0..PHYSICS_STEPS {
                physics.step(PHYSICS_DT, input);
            }
            let prev_phase = game.phase;
            game.sync(physics);

            let fh = physics.field_height();
            if let Some(pos) = physics.ball_position() {
                let target = (pos.y - fh * 0.40).clamp(0.0, fh);
                *camera_y += (target - *camera_y) * 0.15;
            }

            let events: Vec<_> = physics.drain_sound_events().into_iter().collect();
            let drained = prev_phase == Phase::Playing && game.phase != Phase::Playing;
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
    }

    fn render(&mut self) -> Result<()> {
        match &self.screen {
            Screen::Menu => {
                let menu          = &self.menu;
                let sound_enabled = self.sound_enabled;
                self.terminal.draw(|frame| {
                    crate::ui::draw_menu(frame, menu, sound_enabled);
                })?;
            }
            Screen::Playing { game, physics, camera_y, .. } => {
                let game      = game;
                let physics   = physics;
                let renderer  = &self.renderer;
                let camera_y  = *camera_y;
                self.terminal.draw(|frame| {
                    crate::ui::draw(frame, physics, game, renderer, camera_y);
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
