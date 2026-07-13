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
    /// Ticks (sync calls) left in the timed super mode; 0 = inactive.
    pub super_mode_ticks: u32,
}

impl GameState {
    pub fn new(ball_count: u8) -> Self {
        Self {
            score: 0,
            high_score: 0,
            balls_remaining: ball_count,
            phase: Phase::WaitingToLaunch,
            multiplier: 1,
            super_mode_ticks: 0,
        }
    }

    /// The base multiplier, doubled while super mode is active.
    pub fn effective_multiplier(&self) -> u32 {
        if self.super_mode_ticks > 0 { self.multiplier * SUPER_MODE_FACTOR } else { self.multiplier }
    }

    pub fn super_mode_seconds_remaining(&self) -> u32 {
        self.super_mode_ticks.div_ceil(SYNC_HZ)
    }

    pub fn add_score(&mut self, points: u64) {
        self.score += points * self.effective_multiplier() as u64;
        if self.score > self.high_score {
            self.high_score = self.score;
        }
    }

    pub fn sync(&mut self, physics: &PhysicsWorld) {
        if self.phase == Phase::Playing && physics.is_ball_drained() {
            if self.balls_remaining > 0 {
                self.balls_remaining -= 1;
                self.phase = Phase::BallLost;
            } else {
                self.phase = Phase::GameOver;
            }
            // Multiplier and super mode are per-ball: a fresh ball always
            // starts back at ×1, mode off.
            self.multiplier = 1;
            self.super_mode_ticks = 0;
        }

        // Accumulate score from contact events emitted by physics
        for pts in physics.drain_score_events() {
            self.add_score(pts);
        }

        for delta in physics.drain_multiplier_events() {
            self.apply_multiplier_delta(delta);
        }

        for _ in 0..physics.drain_super_mode_events() {
            self.super_mode_ticks = SUPER_MODE_TICKS;
        }
        if self.super_mode_ticks > 0 {
            self.super_mode_ticks -= 1;
        }
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
