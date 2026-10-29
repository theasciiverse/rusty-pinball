use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, ModifierKeyCode};

// Ticks a flipper hold lasts without a new key event before it retracts.
// At 30 fps this is ~100 ms, enough to bridge the gap between terminal
// key-repeat events on terminals that don't send Release events.
// On terminals that do (Kitty protocol), Release zeroes the counter immediately.
const FLIPPER_HOLD_TICKS: u8 = 3;

#[derive(Default, Debug, Clone)]
pub struct InputState {
    pub left_flipper:        bool,
    pub right_flipper:       bool,
    /// True only on the first tick after the left flipper transitions to active.
    pub left_just_pressed:   bool,
    /// True only on the first tick after the right flipper transitions to active.
    pub right_just_pressed:  bool,
    left_ticks:              u8,
    right_ticks:             u8,
}

impl InputState {
    /// Call once per game tick to decay the flipper counters. (The plunger,
    /// Space held before a launch, is the app's: it launches the ball.)
    pub fn tick(&mut self) {
        let prev_left  = self.left_flipper;
        let prev_right = self.right_flipper;
        if self.left_ticks  > 0 { self.left_ticks  -= 1; }
        if self.right_ticks > 0 { self.right_ticks -= 1; }
        self.left_flipper  = self.left_ticks  > 0;
        self.right_flipper = self.right_ticks > 0;
        self.left_just_pressed  = !prev_left  && self.left_flipper;
        self.right_just_pressed = !prev_right && self.right_flipper;
    }

    /// Process a crossterm key event.  Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let pressed  = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
        let released = key.kind == KeyEventKind::Release;

        match key.code {
            KeyCode::Char('z') | KeyCode::Left | KeyCode::Char('Z')
            | KeyCode::Modifier(ModifierKeyCode::LeftShift) => {
                if pressed  { self.left_ticks = FLIPPER_HOLD_TICKS; }
                if released { self.left_ticks = 0; }
                true
            }
            KeyCode::Char('x') | KeyCode::Right | KeyCode::Char('X')
            | KeyCode::Modifier(ModifierKeyCode::RightShift) => {
                if pressed  { self.right_ticks = FLIPPER_HOLD_TICKS; }
                if released { self.right_ticks = 0; }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, KeyModifiers};

    fn ev(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind, state: KeyEventState::NONE }
    }

    #[test]
    fn left_and_right_shift_work_the_flippers_and_release_drops_them() {
        let mut input = InputState::default();
        assert!(input.handle_key(ev(KeyCode::Modifier(ModifierKeyCode::LeftShift), KeyEventKind::Press)));
        input.tick();
        assert!(input.left_flipper && !input.right_flipper);

        assert!(input.handle_key(ev(KeyCode::Modifier(ModifierKeyCode::RightShift), KeyEventKind::Press)));
        input.tick();
        assert!(input.left_flipper && input.right_flipper);

        input.handle_key(ev(KeyCode::Modifier(ModifierKeyCode::LeftShift), KeyEventKind::Release));
        input.handle_key(ev(KeyCode::Modifier(ModifierKeyCode::RightShift), KeyEventKind::Release));
        input.tick();
        assert!(!input.left_flipper && !input.right_flipper);
    }
}
