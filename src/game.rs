use crate::physics::PhysicsWorld;

const MAX_MULTIPLIER: u32 = 5;

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
}

impl GameState {
    pub fn new(ball_count: u8) -> Self {
        Self {
            score: 0,
            high_score: 0,
            balls_remaining: ball_count,
            phase: Phase::WaitingToLaunch,
            multiplier: 1,
        }
    }

    pub fn add_score(&mut self, points: u64) {
        self.score += points * self.multiplier as u64;
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
            // Multiplier is per-ball: a fresh ball always starts back at ×1.
            self.multiplier = 1;
        }

        // Accumulate score from contact events emitted by physics
        for pts in physics.drain_score_events() {
            self.add_score(pts);
        }

        for delta in physics.drain_multiplier_events() {
            self.apply_multiplier_delta(delta);
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
}
