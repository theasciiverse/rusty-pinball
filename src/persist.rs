use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Scores kept per table, as in the original.
pub const MAX_HIGH_SCORES: usize = 5;

/// The original's game speeds: a factor on the table's time ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

impl GameSpeed {
    pub fn factor(self) -> f32 {
        match self { GameSpeed::Slow => 0.85, GameSpeed::Normal => 1.0, GameSpeed::Fast => 1.2 }
    }
    pub fn next(self) -> Self {
        match self { GameSpeed::Slow => GameSpeed::Normal, GameSpeed::Normal => GameSpeed::Fast, GameSpeed::Fast => GameSpeed::Slow }
    }
    pub fn name(self) -> &'static str {
        match self { GameSpeed::Slow => "Slow", GameSpeed::Normal => "Normal", GameSpeed::Fast => "Fast" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    /// The best score per table, from before the top five were kept; read
    /// once to seed `top_scores`, then written as each table's best.
    #[serde(default)]
    pub high_scores:   [u64; 9],
    /// The five best scores per table, best first; index 0 = table 1.
    #[serde(default)]
    pub top_scores: [Vec<u64>; 9],
    /// The score of the last finished game per table.
    #[serde(default)]
    pub last_scores: [u64; 9],
    /// Sound effects (music has its own switch since 0.9.0).
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    #[serde(default = "default_true")]
    pub music_enabled: bool,
    /// Floating scores (the original's `showScoreAnimations`, on by default).
    #[serde(default = "default_true")]
    pub score_animations: bool,
    /// Ball trails (the original's `showBallTrails`, on by default).
    #[serde(default = "default_true")]
    pub ball_trails: bool,
    /// Play without a ball limit; no high scores are recorded.
    #[serde(default)]
    pub unlimited_balls: bool,
    /// Follow the ball across the full width (the original's `zoom`); off shows
    /// the whole table at once.
    #[serde(default = "default_true")]
    pub zoom: bool,
    #[serde(default)]
    pub game_speed: GameSpeed,
}

fn default_true() -> bool { true }

impl Default for SavedState {
    fn default() -> Self {
        Self {
            high_scores: [0; 9], top_scores: Default::default(), last_scores: [0; 9],
            sound_enabled: true, music_enabled: true, score_animations: true, ball_trails: true,
            unlimited_balls: false, zoom: true, game_speed: GameSpeed::Normal,
        }
    }
}

impl SavedState {
    /// Gives a state saved before the top five its single best scores as their lists.
    fn migrate(mut self) -> Self {
        for (list, &best) in self.top_scores.iter_mut().zip(&self.high_scores) {
            if list.is_empty() && best > 0 { list.push(best); }
        }
        self
    }

    /// The best score on table slot `slot`, 0 when none.
    pub fn best(&self, slot: usize) -> u64 {
        self.top_scores[slot].first().copied().unwrap_or(0)
    }

    /// Records a finished game on table slot `slot`: the last score always, and a
    /// place among the five best when it beats the lowest or there are fewer
    /// than five. Returns its place (0 = best), if it got one.
    pub fn record(&mut self, slot: usize, score: u64) -> Option<usize> {
        self.last_scores[slot] = score;
        if score == 0 { return None; }
        let list = &mut self.top_scores[slot];
        let place = list.iter().position(|&s| score > s).unwrap_or(list.len());
        if place >= MAX_HIGH_SCORES { return None; }
        list.insert(place, score);
        list.truncate(MAX_HIGH_SCORES);
        self.high_scores[slot] = list[0];
        Some(place)
    }
}

fn state_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    Some(base.join("rusty-pinball").join("state.json"))
}

pub fn load() -> SavedState {
    let path = match state_path() { Some(p) => p, None => return SavedState::default() };
    let bytes = match std::fs::read(&path) { Ok(b) => b, Err(_) => return SavedState::default() };
    serde_json::from_slice::<SavedState>(&bytes).unwrap_or_default().migrate()
}

pub fn save(state: &SavedState) {
    let path = match state_path() { Some(p) => p, None => return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(&path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_saved_before_the_new_options_loads_with_their_defaults() {
        let old = r#"{ "high_scores": [5,0,0,0,0,0,0,0,0], "sound_enabled": false }"#;
        let s: SavedState = serde_json::from_str::<SavedState>(old).unwrap().migrate();
        assert_eq!(s.best(0), 5, "the old best becomes the first of the five");
        assert!(s.top_scores[1].is_empty());
        assert!(!s.sound_enabled);
        assert!(s.music_enabled, "music was switched with the sound before; on unless set");
        assert!(s.score_animations && s.ball_trails && s.zoom, "on by default, as in the original");
        assert!(!s.unlimited_balls);
        assert_eq!(s.game_speed, GameSpeed::Normal);
    }

    #[test]
    fn the_five_best_are_kept_in_order_and_the_last_score_always() {
        let mut s = SavedState::default();
        for score in [300, 100, 500, 200, 400] {
            assert!(s.record(2, score).is_some());
        }
        assert_eq!(s.top_scores[2], vec![500, 400, 300, 200, 100]);
        assert_eq!(s.record(2, 50), None, "below all five");
        assert_eq!(s.last_scores[2], 50);
        assert_eq!(s.record(2, 450), Some(1));
        assert_eq!(s.top_scores[2], vec![500, 450, 400, 300, 200]);
        assert_eq!(s.record(2, 900), Some(0));
        assert_eq!(s.best(2), 900);
        assert_eq!(s.high_scores[2], 900);
        assert_eq!(s.record(2, 0), None, "an empty game takes no place");
    }

    #[test]
    fn game_speed_is_saved_by_name() {
        let s = SavedState { game_speed: GameSpeed::Fast, ..SavedState::default() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"game_speed\":\"fast\""));
        let back: SavedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.game_speed, GameSpeed::Fast);
    }
}
