//! Persisted UI state — the bits the user arranges by hand and expects to find
//! again next launch: the left/right split width and the custom session order.
//! Stored as TOML at `~/.agent-multiplexer/state.toml`, separate from the
//! hand-edited `config.toml`. All IO is best-effort: a missing/garbage file or
//! an unwritable home just falls back to defaults and silently skips saving.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Left (sessions) pane width as a percentage of the body, if the user has
    /// resized it. `None` keeps the app default.
    pub split_pct: Option<u16>,
    /// Session names in the user's chosen order *within their project group*.
    /// Names not present are appended; names here but no longer present ignored.
    pub order: Vec<String>,
    /// Project root paths in the user's chosen group order. Same merge rules.
    pub project_order: Vec<String>,
    /// Display-name overrides for projects, keyed by project root path (stable
    /// against two projects sharing a folder name). Value is the shown name; the
    /// directory itself is never renamed. BTreeMap → deterministic file output.
    pub project_names: BTreeMap<String, String>,
    /// Global "Inbox" note (markdown). Empty by default.
    pub inbox: String,
    /// Per-session notes (markdown), keyed by tmux session name. BTreeMap →
    /// deterministic file output.
    pub notes: BTreeMap<String, String>,
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
        let mut names = BTreeMap::new();
        names.insert("/home/u/p".to_string(), "Backend".to_string());
        let s = State {
            split_pct: Some(55),
            order: vec!["a".into(), "b".into()],
            project_order: vec!["/home/u/p".into()],
            project_names: names,
            inbox: String::new(),
            notes: BTreeMap::new(),
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

    #[test]
    fn notes_round_trip_through_toml() {
        let mut s = State::default();
        s.inbox = "# today\n- [ ] ship".into();
        s.notes.insert("proj".into(), "- [x] done".into());
        let text = toml::to_string(&s).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(back.inbox, s.inbox);
        assert_eq!(
            back.notes.get("proj").map(String::as_str),
            Some("- [x] done")
        );
    }

    #[test]
    fn missing_notes_fields_default_empty() {
        // An old state file with no inbox/notes keys still loads.
        let s: State = toml::from_str("split_pct = 40").unwrap();
        assert_eq!(s.inbox, "");
        assert!(s.notes.is_empty());
    }
}
