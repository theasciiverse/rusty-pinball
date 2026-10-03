use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    /// Per-table high scores; index 0 = table 1.
    #[serde(default)]
    pub high_scores:   [u64; 9],
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// Floating scores (the original's `showScoreAnimations`, on by default).
    #[serde(default = "default_true")]
    pub score_animations: bool,
    /// Ball trails (the original's `showBallTrails`, on by default).
    #[serde(default = "default_true")]
    pub ball_trails: bool,
    /// Play without a ball limit; no high scores are recorded.
    #[serde(default)]
    pub unlimited_balls: bool,
}

fn default_true() -> bool { true }

impl Default for SavedState {
    fn default() -> Self {
        Self { high_scores: [0; 9], sound_enabled: true, score_animations: true, ball_trails: true, unlimited_balls: false }
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
    serde_json::from_slice(&bytes).unwrap_or_default()
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
        let s: SavedState = serde_json::from_str(old).unwrap();
        assert_eq!(s.high_scores[0], 5);
        assert!(!s.sound_enabled);
        assert!(s.score_animations && s.ball_trails, "on by default, as in the original");
        assert!(!s.unlimited_balls);
    }
}
