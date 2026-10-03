use crate::physics::{PhysicsWorld, ScorePopup};

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

/// How long a score floats (the original's `ScoreAnimation`: 2 s), in sync ticks.
const POPUP_TICKS: u32 = 2 * SYNC_HZ;
/// How far it rises over that time, in table units.
const POPUP_RISE: f32 = 3.0;
const MAX_POPUPS: usize = 12;

struct Popup {
    points: u64,
    x: f32,
    y: f32,
    age: u32,
}

pub struct GameState {
    popups: Vec<Popup>,
    pub score: u64,
    pub high_score: u64,
    pub balls_remaining: u8,
    /// The ball being played (1-based); an extra ball does not advance it.
    pub ball_number: u8,
    /// Extra balls won and not yet used (they are also counted in `balls_remaining`).
    pub extra_balls: u8,
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
    /// A message waiting to be shown: ticks until it appears, the text and its ms.
    pub pending_message: Option<(u32, String, u32)>,
    /// A message with its sound was just shown (the app plays the sound and clears this).
    message_sound_due: bool,
}

impl GameState {
    pub fn new(ball_count: u8) -> Self {
        Self {
            popups: Vec::new(),
            score: 0,
            high_score: 0,
            balls_remaining: ball_count,
            ball_number: 1,
            extra_balls: 0,
            phase: Phase::WaitingToLaunch,
            multiplier: 1,
            bonus_percent: 0,
            super_mode_ticks: 0,
            message: None,
            pending_message: None,
            message_sound_due: false,
        }
    }

    /// Shows `text` for `ms` of real time, replacing any current message, with the
    /// message sound (the original's default).
    pub fn show_message(&mut self, text: impl Into<String>, ms: u32) {
        self.show_message_quietly(text, ms);
        self.message_sound_due = true;
    }

    /// Like `show_message`, without the sound (the "Ball 2" and "Shoot Again" messages).
    pub fn show_message_quietly(&mut self, text: impl Into<String>, ms: u32) {
        let ticks = (ms * SYNC_HZ / 1000).max(1);
        self.message = Some((text.into(), ticks));
    }

    /// True once after a message with a sound was shown.
    pub fn take_message_sound(&mut self) -> bool {
        std::mem::take(&mut self.message_sound_due)
    }

    /// Shows `text` for `ms` after `delay_ms` (the original schedules its next-ball
    /// messages 1500 ms after the ball is lost).
    pub fn show_message_after(&mut self, delay_ms: u32, text: impl Into<String>, ms: u32) {
        self.pending_message = Some(((delay_ms * SYNC_HZ / 1000).max(1), text.into(), ms));
    }

    /// Counts the current message down by one tick, clearing it when it runs out.
    pub fn tick_message(&mut self) {
        if let Some((delay, _, _)) = &mut self.pending_message {
            *delay = delay.saturating_sub(1);
            if *delay == 0 {
                if let Some((_, text, ms)) = self.pending_message.take() { self.show_message_quietly(text, ms); }
            }
        }
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

    /// Adds `points` times the multiplier and returns what was added.
    pub fn add_score(&mut self, points: u64) -> u64 {
        // Rounded like the original's Math.round(points * multiplier).
        let added = (points * self.multiplier_percent() as u64 + 50) / 100;
        self.score += added;
        if self.score > self.high_score {
            self.high_score = self.score;
        }
        added
    }

    /// Floats `points` up from `(x, y)`; the newest `MAX_POPUPS` are kept.
    pub fn add_popup(&mut self, points: u64, x: f32, y: f32) {
        if points == 0 { return; }
        self.popups.push(Popup { points, x, y, age: 0 });
        if self.popups.len() > MAX_POPUPS { self.popups.remove(0); }
    }

    /// The scores floating now, as the renderer draws them. The colour follows the
    /// value like the original's: pink from 10,000, orange above 2,000, yellow
    /// above 500, else light green.
    pub fn popup_snapshots(&self) -> Vec<ScorePopup> {
        self.popups.iter().map(|p| ScorePopup {
            text: p.points.to_string(),
            x: p.x,
            y: p.y + POPUP_RISE * p.age as f32 / POPUP_TICKS as f32,
            progress: p.age as f32 / POPUP_TICKS as f32,
            color: match p.points {
                10_000.. => (0xFF, 0x00, 0x80),
                2_001..=9_999 => (0xFF, 0x40, 0x00),
                501..=2_000 => (0xFF, 0xFF, 0x00),
                _ => (0x80, 0xFF, 0x80),
            },
        }).collect()
    }

    pub fn sync(&mut self, physics: &PhysicsWorld) {
        if self.phase == Phase::Playing && physics.is_ball_drained() {
            self.ball_drained(physics.multiplier_increase_preserved());
        }

        // Accumulate score from contact events emitted by physics
        let at = physics.ball_position().map(|p| (p.x, p.y));
        for pts in physics.drain_score_events() {
            let added = self.add_score(pts);
            if let Some((x, y)) = at { self.add_popup(added, x, y); }
        }
        for p in &mut self.popups { p.age += 1; }
        self.popups.retain(|p| p.age < POPUP_TICKS);

        self.tick_message();
        // An extra ball is one more ball to play, in place of the one in play.
        for _ in 0..physics.drain_extra_ball_events() {
            self.balls_remaining = self.balls_remaining.saturating_add(1);
            self.extra_balls = self.extra_balls.saturating_add(1);
        }
        for pct in physics.drain_multiplier_percent_events() {
            self.bonus_percent = self.bonus_percent.saturating_add_signed(pct);
        }
        for factor in physics.drain_multiplier_scale_events() {
            self.scale_multiplier(factor);
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
    ///
    /// The multiplier follows the original's default: halved and rounded down,
    /// never below 1. If a script raised it within the last 7.5 seconds of game
    /// time (`preserve_increase`), that increase is taken off first and put
    /// back after, so it counts for the next ball. Super mode is per ball.
    pub fn ball_drained(&mut self, preserve_increase: bool) {
        let had_extra_ball = self.extra_balls > 0;
        self.balls_remaining = self.balls_remaining.saturating_sub(1);
        self.phase = if self.balls_remaining == 0 { Phase::GameOver } else { Phase::BallLost };

        let mut whole = (self.multiplier * 100 + self.bonus_percent) as f64 / 100.0;
        if preserve_increase { whole -= 1.0; }
        let mut next = (whole / 2.0).floor().max(1.0) as u32;
        if preserve_increase { next += 1; }
        self.multiplier = next;
        self.bonus_percent = 0;
        self.super_mode_ticks = 0;

        if had_extra_ball {
            self.extra_balls -= 1;
        } else {
            self.ball_number = self.ball_number.saturating_add(1);
        }
        if self.phase == Phase::GameOver {
            self.show_message("Game Over", 2500);
        } else if had_extra_ball {
            self.show_message_after(1500, "Shoot Again", 1500);
        } else {
            self.show_message_after(1500, format!("Ball {}", self.ball_number), 1500);
        }
    }

    /// Multiplies the whole multiplier by `percent / 100`, rounding down like the
    /// original's `(long) (multiplier * (100 + jackpot))`; the whole-number part
    /// stays and the percent part takes the difference.
    pub fn scale_multiplier(&mut self, percent: u64) {
        let total = (self.multiplier * 100 + self.bonus_percent) as u64;
        let scaled = (total * percent / 100).min(u32::MAX as u64) as u32;
        self.bonus_percent = scaled.saturating_sub(self.multiplier * 100);
    }

    /// Adjusts the multiplier by `delta`. As in the original there is no upper
    /// limit; it never drops below x1.
    pub fn apply_multiplier_delta(&mut self, delta: i32) {
        self.multiplier = (self.multiplier as i64 + delta as i64).clamp(1, u32::MAX as i64 / 2000) as u32;
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
        game.ball_drained(false);
        game.phase = Phase::Playing;
        game.ball_drained(false);
        game.phase = Phase::Playing;
        game.ball_drained(false);
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

        game.ball_drained(false);
        assert_eq!((game.multiplier, game.bonus_percent), (1, 0));
        assert_eq!(game.multiplier_text(), "x1");
    }

    #[test]
    fn a_three_ball_game_gives_exactly_three_balls() {
        let mut game = GameState::new(3);
        game.phase = Phase::Playing;
        game.ball_drained(false);
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 2), "after ball 1");
        game.phase = Phase::Playing;
        game.ball_drained(false);
        assert_eq!((game.phase, game.balls_remaining), (Phase::BallLost, 1), "after ball 2");
        game.phase = Phase::Playing;
        game.ball_drained(false);
        assert_eq!((game.phase, game.balls_remaining), (Phase::GameOver, 0), "the third drain ends the game");
    }

    #[test]
    fn a_ball_drain_halves_the_multiplier_rounding_down_and_ends_super_mode() {
        let mut game = GameState::new(3);
        game.multiplier = 5;
        game.super_mode_ticks = 100;
        game.ball_drained(false);
        assert_eq!((game.multiplier, game.super_mode_ticks), (2, 0), "5 halves to 2");
        game.ball_drained(false);
        assert_eq!(game.multiplier, 1, "2 halves to 1");
        game.ball_drained(false);
        assert_eq!(game.multiplier, 1, "never below 1");
    }

    #[test]
    fn a_ball_lost_just_after_a_multiplier_increase_keeps_the_increase() {
        let mut game = GameState::new(3);
        game.multiplier = 4; // just raised from 3
        game.ball_drained(true);
        assert_eq!(game.multiplier, 2, "(4 - 1) halved is 1, plus the increase back is 2");
        game.multiplier = 2; // 1 raised to 2
        game.ball_drained(true);
        assert_eq!(game.multiplier, 2, "kept");
        game.multiplier = 9;
        game.bonus_percent = 50;
        game.ball_drained(false);
        assert_eq!((game.multiplier, game.bonus_percent), (4, 0), "9.5 halves to 4.75, rounded down");
    }

    #[test]
    fn the_next_ball_is_announced_after_a_delay_and_an_extra_ball_says_shoot_again() {
        let mut game = GameState::new(3);
        game.ball_drained(false);
        assert_eq!(game.ball_number, 2);
        assert!(game.message.is_none(), "not at once");
        for _ in 0..(1500 * SYNC_HZ / 1000) { game.tick_message(); }
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Ball 2"));

        game.extra_balls = 1;
        game.balls_remaining += 1;
        game.ball_drained(false);
        assert_eq!((game.ball_number, game.extra_balls), (2, 0), "an extra ball does not advance the ball number");
        for _ in 0..(1500 * SYNC_HZ / 1000) { game.tick_message(); }
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Shoot Again"));

        game.ball_drained(false);
        assert_eq!(game.ball_number, 3);
        game.ball_drained(false);
        assert_eq!(game.phase, Phase::GameOver);
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Game Over"));
    }

    #[test]
    fn the_multiplier_has_no_upper_limit_and_never_drops_below_one() {
        let mut game = GameState::new(3);
        assert_eq!(game.multiplier, 1);

        for _ in 0..10 {
            game.apply_multiplier_delta(1);
        }
        assert_eq!(game.multiplier, 11, "as in the original it keeps growing");

        game.apply_multiplier_delta(-100);
        assert_eq!(game.multiplier, 1, "multiplier should never drop below x1");
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

    #[test]
    fn a_score_floats_up_for_two_seconds_in_its_value_colour_and_shows_the_multiplied_points() {
        let mut game = GameState::new(3);
        game.multiplier = 2;
        let added = game.add_score(600);
        game.add_popup(added, 10.0, 5.0);
        let snap = game.popup_snapshots();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].text, "1200", "the multiplied score");
        assert_eq!(snap[0].color, (0xFF, 0xFF, 0x00), "yellow above 500");
        assert_eq!((snap[0].x, snap[0].y), (10.0, 5.0));

        for (points, color) in [(100, (0x80, 0xFF, 0x80)), (3_000, (0xFF, 0x40, 0x00)), (50_000, (0xFF, 0x00, 0x80))] {
            let mut g = GameState::new(3);
            g.add_popup(points, 1.0, 1.0);
            assert_eq!(g.popup_snapshots()[0].color, color, "{points}");
        }

        // After a second it has risen half way, and after two it is gone.
        game.popups[0].age = POPUP_TICKS / 2;
        let half = game.popup_snapshots();
        assert!((half[0].y - 6.5).abs() < 0.01 && (half[0].progress - 0.5).abs() < 0.01);
        game.popups[0].age = POPUP_TICKS - 1;
        game.popups.iter_mut().for_each(|p| p.age += 1);
        game.popups.retain(|p| p.age < POPUP_TICKS);
        assert!(game.popup_snapshots().is_empty());
    }

    #[test]
    fn only_the_newest_scores_are_kept_and_a_zero_score_shows_nothing() {
        let mut game = GameState::new(3);
        game.add_popup(0, 1.0, 1.0);
        assert!(game.popup_snapshots().is_empty());
        for i in 1..=30 { game.add_popup(i * 100, 1.0, 1.0); }
        let snap = game.popup_snapshots();
        assert_eq!(snap.len(), MAX_POPUPS);
        assert_eq!(snap.last().unwrap().text, "3000");
    }

    #[test]
    fn a_message_has_its_sound_but_the_next_ball_message_does_not() {
        let mut game = GameState::new(3);
        assert!(!game.take_message_sound());
        game.show_message("Multiball!", 2000);
        assert!(game.take_message_sound(), "a message plays its sound once");
        assert!(!game.take_message_sound());

        game.ball_drained(false);
        for _ in 0..(1500 * SYNC_HZ / 1000) { game.tick_message(); }
        assert_eq!(game.message.as_ref().map(|m| m.0.as_str()), Some("Ball 2"));
        assert!(!game.take_message_sound(), "'Ball 2' is silent, as in the original");
    }
}
