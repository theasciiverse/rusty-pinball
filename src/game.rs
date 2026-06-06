use crate::physics::PhysicsWorld;

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
        }

        // Accumulate score from contact events emitted by physics
        for pts in physics.drain_score_events() {
            self.add_score(pts);
        }
    }
}
