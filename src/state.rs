//! Persisted UI state — the bits the user arranges by hand and expects to find
//! again next launch: the left/right split width and the custom session order.
//! Stored as TOML at `~/.agent-multiplexer/state.toml`, separate from the
//! hand-edited `config.toml`. All IO is best-effort: a missing/garbage file or
//! an unwritable home just falls back to defaults and silently skips saving.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Left (sessions) pane width as a percentage of the body, if the user has
    /// resized it. `None` keeps the app default.
    pub split_pct: Option<u16>,
    /// Session names in the user's chosen display order. Names not present are
    /// appended (in tmux order); names here but no longer present are ignored.
    pub order: Vec<String>,
}

fn state_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        Path::new(&home)
            .join(".agent-multiplexer")
            .join("state.toml"),
    )
}

impl State {
    /// Reads the saved state, or returns defaults if absent/unreadable/invalid.
    pub fn load() -> State {
        let Some(path) = state_path() else {
            return State::default();
        };
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Writes the state to disk, creating the directory if needed. Best-effort:
    /// any failure (no HOME, unwritable dir) is silently ignored.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = toml::to_string(self) {
            let _ = std::fs::write(&path, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_toml() {
        let s = State {
            split_pct: Some(55),
            order: vec!["a".into(), "b".into()],
        };
        let toml = toml::to_string(&s).unwrap();
        let back: State = toml::from_str(&toml).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s: State = toml::from_str("").unwrap();
        assert_eq!(s.split_pct, None);
        assert!(s.order.is_empty());
    }
}
