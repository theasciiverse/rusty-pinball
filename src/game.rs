use crate::physics::PhysicsWorld;

const MAX_MULTIPLIER: u32 = 5;

// `sync()` runs once per rendered frame (app.rs's FRAME_MILLIS = 33ms), not
// once per physics substep — so "ticks" here means "sync calls", at ~30 Hz.
const SYNC_HZ: u32 = 30;
const SUPER_MODE_SECONDS: u32 = 15;
const SUPER_MODE_TICKS: u32 = SUPER_MODE_SECONDS * SYNC_HZ;
/// Stacks multiplicatively with the base multiplier while super mode is active.
const SUPER_MODE_FACTOR: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    WaitingToLaunch,
    Playing,
    BallLost,
    GameOver,
}

pub struct GameState {
    pub score: u64,
    pub high_score: u64,
    pub balls_remaining: u8,
    pub phase: Phase,
    pub multiplier: u32,
    /// Percent added on top of the whole-number multiplier by table scripts (table 4
    /// adds 1% per bumper hit); 137 here with `multiplier` 1 is a x2.37 multiplier.
    pub bonus_percent: u32,
    /// Ticks (sync calls) left in the timed super mode; 0 = inactive.
    pub super_mode_ticks: u32,
    /// The message being shown to the player and the ticks (sync calls, ~30 Hz,
    /// so real time like the original) it has left. One at a time; a new one
    /// replaces it.
    pub message: Option<(String, u32)>,
}

impl GameState {
    pub fn new(ball_count: u8) -> Self {
        Self {
            score: 0,
            high_score: 0,
            balls_remaining: ball_count,
            phase: Phase::WaitingToLaunch,
            multiplier: 1,
            bonus_percent: 0,
            super_mode_ticks: 0,
            message: None,
        }
    }

    /// Shows `text` for `ms` of real time, replacing any current message.
    pub fn show_message(&mut self, text: impl Into<String>, ms: u32) {
        let ticks = (ms * SYNC_HZ / 1000).max(1);
        self.message = Some((text.into(), ticks));
    }

    /// Counts the current message down by one tick, clearing it when it runs out.
    pub fn tick_message(&mut self) {
        if let Some((_, ticks)) = &mut self.message {
            *ticks = ticks.saturating_sub(1);
            if *ticks == 0 { self.message = None; }
        }
    }

    /// The base multiplier, doubled while super mode is active.
    pub fn effective_multiplier(&self) -> u32 {
        if self.super_mode_ticks > 0 { self.multiplier * SUPER_MODE_FACTOR } else { self.multiplier }
    }

    pub fn super_mode_seconds_remaining(&self) -> u32 {
        self.super_mode_ticks.div_ceil(SYNC_HZ)
    }

    /// The whole multiplier in percent (100 = x1), with super mode doubling and
    /// any bonus percent from table scripts.
    pub fn multiplier_percent(&self) -> u32 {
        (self.multiplier * 100 + self.bonus_percent) * if self.super_mode_ticks > 0 { SUPER_MODE_FACTOR } else { 1 }
    }

    /// "x3" for a whole multiplier, "x1.37" when a script has added a percentage.
    pub fn multiplier_text(&self) -> String {
        let pct = self.multiplier * 100 + self.bonus_percent;
        if pct % 100 == 0 { format!("x{}", pct / 100) } else { format!("x{}.{:02}", pct / 100, pct % 100) }
    }

    pub fn add_score(&mut self, points: u64) {
        // Rounded like the original's Math.round(points * multiplier).
        self.score += (points * self.multiplier_percent() as u64 + 50) / 100;
        if self.score > self.high_score {
            self.high_score = self.score;
        }
    }

    pub fn sync(&mut self, physics: &PhysicsWorld) {
        if self.phase == Phase::Playing && physics.is_ball_drained() {
            self.ball_drained();
        }

        // Accumulate score from contact events emitted by physics
        for pts in physics.drain_score_events() {
            self.add_score(pts);
        }

        self.tick_message();
        // An extra ball is one more ball to play, in place of the one in play.
        for _ in 0..physics.drain_extra_ball_events() {
            self.balls_remaining = self.balls_remaining.saturating_add(1);
        }
        for pct in physics.drain_multiplier_percent_events() {
            self.bonus_percent = self.bonus_percent.saturating_add_signed(pct);
        }
        for delta in physics.drain_multiplier_events() {
            self.apply_multiplier_delta(delta);
            if delta > 0 {
                self.show_message(format!("{}x Multiplier", self.multiplier), 1500);
            }
        }
        for (text, ms) in physics.drain_message_events() {
            self.show_message(text, ms);
        }

        for _ in 0..physics.drain_super_mode_events() {
            self.super_mode_ticks = SUPER_MODE_TICKS;
        }
        if self.super_mode_ticks > 0 {
            self.super_mode_ticks -= 1;
        }
    }

    /// The ball in play drained. `balls_remaining` counts the ball in play
    /// (it starts at the table's `numballs`), so the game ends when the last
    /// one is gone: a 3-ball game gives exactly 3 balls.
    pub fn ball_drained(&mut self) {
        self.balls_remaining = self.balls_remaining.saturating_sub(1);
        self.phase = if self.balls_remaining == 0 { Phase::GameOver } else { Phase::BallLost };
        // Multiplier and super mode are per-ball: a fresh ball always
        // starts back at ×1, mode off.
        self.multiplier = 1;
        self.bonus_percent = 0;
        self.super_mode_ticks = 0;
    }

    /// Adjusts the multiplier by `delta`, clamped to [1, MAX_MULTIPLIER] —
    /// it can never run away or drop below ×1.
    pub fn apply_multiplier_delta(&mut self, delta: i32) {
        self.multiplier = (self.multiplier as i32 + delta).clamp(1, MAX_MULTIPLIER as i32) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_lasts_its_real_time_then_clears_and_a_new_one_replaces_it() {
        let mut game = GameState::new(3);
        game.show_message("Multiball!", 2000);
        for _ in 0..(2000 * SYNC_HZ / 1000 - 1) { game.tick_message(); }
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Multiball!"), "still showing just before 2 s");
        game.tick_message();
        assert!(game.message.is_none(), "gone after 2 s");

        game.show_message("first", 3000);
        game.show_message("second", 3000);
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("second"));
    }

    #[test]
    fn an_extra_ball_adds_a_ball_to_play() {
        let mut game = GameState::new(3);
        game.balls_remaining = game.balls_remaining.saturating_add(1);
        game.phase = Phase::Playing;
        game.ball_drained();
        game.phase = Phase::Playing;
        game.ball_drained();
        game.phase = Phase::Playing;
        game.ball_drained();
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 1), "four balls in all, so three drains leave one");
    }

    #[test]
    fn a_percent_bonus_scales_scores_shows_as_a_fraction_and_resets_each_ball() {
        let mut game = GameState::new(3);
        assert_eq!(game.multiplier_text(), "x1");
        game.bonus_percent = 37;
        assert_eq!(game.multiplier_text(), "x1.37");
        game.add_score(1000);
        assert_eq!(game.score, 1370);
        game.add_score(5); // 6.85 rounds to 7
        assert_eq!(game.score, 1377);

        game.multiplier = 3;
        assert_eq!(game.multiplier_text(), "x3.37");
        game.super_mode_ticks = 1;
        assert_eq!(game.multiplier_percent(), 674, "super mode doubles the whole thing");

        game.ball_drained();
        assert_eq!((game.multiplier, game.bonus_percent), (1, 0));
        assert_eq!(game.multiplier_text(), "x1");
    }

    #[test]
    fn a_three_ball_game_gives_exactly_three_balls() {
        let mut game = GameState::new(3);
        game.phase = Phase::Playing;
        game.ball_drained();
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 2), "after ball 1");
        game.phase = Phase::Playing;
        game.ball_drained();
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 1), "after ball 2");
        game.phase = Phase::Playing;
        game.ball_drained();
        assert_eq!((game.phase, game.balls_remaining), (Phase::GameOver, 0), "the third drain ends the game");
    }

    #[test]
    fn a_ball_drain_resets_the_per_ball_multiplier_and_super_mode() {
        let mut game = GameState::new(3);
        game.multiplier = 4;
        game.super_mode_ticks = 100;
        game.ball_drained();
        assert_eq!((game.multiplier, game.super_mode_ticks), (1, 0));
    }

    #[test]
    fn multiplier_clamps_between_one_and_max() {
        let mut game = GameState::new(3);
        assert_eq!(game.multiplier, 1);

        for _ in 0..10 {
            game.apply_multiplier_delta(1);
        }
        assert_eq!(game.multiplier, MAX_MULTIPLIER, "multiplier should cap, not run away");

        game.apply_multiplier_delta(-100);
        assert_eq!(game.multiplier, 1, "multiplier should never drop below ×1");
    }

    #[test]
    fn add_score_scales_by_multiplier() {
        let mut game = GameState::new(3);
        game.apply_multiplier_delta(2); // now ×3
        game.add_score(1_000);
        assert_eq!(game.score, 3_000);
    }

    #[test]
    fn super_mode_doubles_the_effective_multiplier_while_active() {
        let mut game = GameState::new(3);
        game.apply_multiplier_delta(1); // base ×2
        assert_eq!(game.effective_multiplier(), 2, "no super mode yet");

        game.super_mode_ticks = SUPER_MODE_TICKS;
        assert_eq!(game.effective_multiplier(), 4, "super mode doubles the base multiplier");
        game.add_score(1_000);
        assert_eq!(game.score, 4_000);

        game.super_mode_ticks = 0;
        assert_eq!(game.effective_multiplier(), 2, "back to base once inactive");
    }

    #[test]
    fn super_mode_seconds_remaining_rounds_up() {
        let mut game = GameState::new(3);
        game.super_mode_ticks = SUPER_MODE_TICKS;
        assert_eq!(game.super_mode_seconds_remaining(), SUPER_MODE_SECONDS);

        game.super_mode_ticks = 1;
        assert_eq!(game.super_mode_seconds_remaining(), 1, "1 tick left should still show 1s, not 0s");

        game.super_mode_ticks = 0;
        assert_eq!(game.super_mode_seconds_remaining(), 0);
    }
}
