use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    /// Per-table high scores; index 0 = table 1.
    #[serde(default)]
    pub high_scores:   [u64; 9],
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
}

fn default_true() -> bool { true }

impl Default for SavedState {
    fn default() -> Self {
        Self { high_scores: [0; 9], sound_enabled: true }
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
