use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

#[derive(Default, Debug, Clone)]
pub struct InputState {
    pub left_flipper: bool,
    pub right_flipper: bool,
    pub plunger_held: bool,
    plunger_ticks: u32,
}

impl InputState {
    /// Returns the plunger charge in [0.0, 1.0]; peaks after ~1 second of holding.
    pub fn plunger_charge(&self) -> f32 {
        (self.plunger_ticks as f32 / 60.0).min(1.0)
    }

    /// Call once per game tick to advance plunger charging.
    pub fn tick(&mut self) {
        if self.plunger_held {
            self.plunger_ticks = (self.plunger_ticks + 1).min(60);
        } else {
            self.plunger_ticks = 0;
        }
    }

    /// Process a crossterm key event.  Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let pressed = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
        let released = key.kind == KeyEventKind::Release;

        match key.code {
            KeyCode::Char('z') | KeyCode::Left | KeyCode::Char('Z') => {
                if pressed { self.left_flipper = true; }
                if released { self.left_flipper = false; }
                true
            }
            KeyCode::Char('x') | KeyCode::Right | KeyCode::Char('X') => {
                if pressed { self.right_flipper = true; }
                if released { self.right_flipper = false; }
                true
            }
            KeyCode::Char(' ') => {
                if pressed { self.plunger_held = true; }
                if released { self.plunger_held = false; }
                true
            }
            _ => false,
        }
    }
}
