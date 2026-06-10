use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

// Ticks a flipper hold lasts without a new key event before it retracts.
// At 30 fps this is ~100 ms — enough to bridge the gap between terminal
// key-repeat events on terminals that don't send Release events.
// On terminals that do (Kitty protocol), Release zeroes the counter immediately.
const FLIPPER_HOLD_TICKS: u8 = 3;

#[derive(Default, Debug, Clone)]
pub struct InputState {
    pub left_flipper:        bool,
    pub right_flipper:       bool,
    pub plunger_held:        bool,
    /// True only on the first tick after the left flipper transitions to active.
    pub left_just_pressed:   bool,
    /// True only on the first tick after the right flipper transitions to active.
    pub right_just_pressed:  bool,
    left_ticks:              u8,
    right_ticks:             u8,
    plunger_ticks:           u32,
}

impl InputState {
    /// Returns the plunger charge in [0.0, 1.0]; peaks after ~1 second of holding.
    pub fn plunger_charge(&self) -> f32 {
        (self.plunger_ticks as f32 / 60.0).min(1.0)
    }

    /// Call once per game tick to advance plunger charging and decay flipper counters.
    pub fn tick(&mut self) {
        let prev_left  = self.left_flipper;
        let prev_right = self.right_flipper;
        if self.left_ticks  > 0 { self.left_ticks  -= 1; }
        if self.right_ticks > 0 { self.right_ticks -= 1; }
        self.left_flipper  = self.left_ticks  > 0;
        self.right_flipper = self.right_ticks > 0;
        self.left_just_pressed  = !prev_left  && self.left_flipper;
        self.right_just_pressed = !prev_right && self.right_flipper;

        if self.plunger_held {
            self.plunger_ticks = (self.plunger_ticks + 1).min(60);
        } else {
            self.plunger_ticks = 0;
        }
    }

    /// Process a crossterm key event.  Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let pressed  = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
        let released = key.kind == KeyEventKind::Release;

        match key.code {
            KeyCode::Char('z') | KeyCode::Left | KeyCode::Char('Z') => {
                if pressed  { self.left_ticks = FLIPPER_HOLD_TICKS; }
                if released { self.left_ticks = 0; }
                true
            }
            KeyCode::Char('x') | KeyCode::Right | KeyCode::Char('X') => {
                if pressed  { self.right_ticks = FLIPPER_HOLD_TICKS; }
                if released { self.right_ticks = 0; }
                true
            }
            KeyCode::Char(' ') => {
                if pressed  { self.plunger_held = true; }
                if released { self.plunger_held = false; }
                true
            }
            _ => false,
        }
    }
}
