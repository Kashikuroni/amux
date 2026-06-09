//! Persisted UI state — the bits the user arranges by hand and expects to find
//! again next launch: the left/right split width and the custom session order.
//! Stored as TOML at `~/.agent-multiplexer/state.toml`, separate from the
//! hand-edited `config.toml`. All IO is best-effort: a missing/garbage file or
//! an unwritable home just falls back to defaults and silently skips saving.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-session data persisted across restarts. Enough to recreate a session
/// after a computer reboot: the working directory, the agent, and (for Claude
/// Code) the last known `--resume <uuid>` command.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedSession {
    pub dir: String,
    pub agent: String,
    /// For Claude Code: the `claude --resume <uuid>` command from the last
    /// clean shutdown / restart. `None` for fresh or non-Claude sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cmd: Option<String>,
}

/// A recently-stopped agent session, kept so the user can re-spawn it from the
/// "Recent" tab. Same recreate-able payload as [`PersistedSession`] plus the
/// session name (recents is an ordered list, not a name-keyed map).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentSession {
    pub name: String,
    pub dir: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cmd: Option<String>,
}

/// Maximum number of recently-stopped sessions retained (newest first). Pushing
/// past this evicts the oldest.
pub const MAX_RECENTS: usize = 20;

/// Record a stopped session at the front of `recents`: any existing entry with
/// the same name is removed first (so a re-stopped session moves to the front
/// without duplicating), then the list is truncated to [`MAX_RECENTS`].
pub fn push_recent(recents: &mut Vec<RecentSession>, entry: RecentSession) {
    recents.retain(|e| e.name != entry.name);
    recents.insert(0, entry);
    recents.truncate(MAX_RECENTS);
}

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
    /// Per-project notes (markdown), keyed by project root path — the same
    /// stable key `project_names` uses, so a note survives amux restarts and
    /// the death of every session of its project.
    pub project_notes: BTreeMap<String, String>,
    /// Per-session notes (markdown), keyed by tmux session name. BTreeMap →
    /// deterministic file output.
    pub notes: BTreeMap<String, String>,
    /// In-progress reply drafts (the `i` composer), keyed by tmux session
    /// name. A draft lives exactly as long as its session.
    pub drafts: BTreeMap<String, String>,
    /// Agent sessions to restore on cold start (e.g. after a computer reboot),
    /// keyed by tmux session name. Shell/terminal sessions are excluded.
    pub sessions: BTreeMap<String, PersistedSession>,
    /// Recently-stopped agent sessions (newest first, capped at [`MAX_RECENTS`])
    /// the user can re-spawn from the "Recent" tab. Maintained via [`push_recent`].
    pub recents: Vec<RecentSession>,
    /// App version seen on the last run, so an upgrade can show "What's New"
    /// exactly once. `None` on a first-ever launch (no modal then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_version: Option<String>,
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

    /// Drops entries keyed by a project root whose directory no longer exists
    /// (per `exists`) — notes, display names, and ordering of dead projects.
    /// Returns true when anything was removed, so the caller can re-save.
    pub fn prune_missing_projects(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.project_notes.len() + self.project_names.len() + self.project_order.len();
        self.project_notes.retain(|root, _| exists(root));
        self.project_names.retain(|root, _| exists(root));
        self.project_order.retain(|root| exists(root));
        before != self.project_notes.len() + self.project_names.len() + self.project_order.len()
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
            project_notes: BTreeMap::new(),
            notes: BTreeMap::new(),
            drafts: BTreeMap::new(),
            sessions: BTreeMap::new(),
            recents: Vec::new(),
            last_version: Some("0.5.0".into()),
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
        let s = State {
            project_notes: BTreeMap::from([("/p".to_string(), "# today\n- [ ] ship".to_string())]),
            notes: BTreeMap::from([("proj".to_string(), "- [x] done".to_string())]),
            drafts: BTreeMap::from([("sess".to_string(), "half-written".to_string())]),
            ..Default::default()
        };
        let text = toml::to_string(&s).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(
            back.project_notes.get("/p").map(String::as_str),
            Some("# today\n- [ ] ship")
        );
        assert_eq!(
            back.notes.get("proj").map(String::as_str),
            Some("- [x] done")
        );
        assert_eq!(
            back.drafts.get("sess").map(String::as_str),
            Some("half-written")
        );
    }

    #[test]
    fn missing_notes_fields_default_empty() {
        // An old state file with no project_notes/notes keys still loads.
        let s: State = toml::from_str("split_pct = 40").unwrap();
        assert!(s.project_notes.is_empty());
        assert!(s.notes.is_empty());
    }

    #[test]
    fn old_state_with_inbox_key_still_loads() {
        // Pre-project-notes files carried a global `inbox` string; serde drops
        // unknown keys, so the file loads and the old text is discarded.
        let s: State = toml::from_str("inbox = \"- [ ] old\"\nsplit_pct = 40").unwrap();
        assert_eq!(s.split_pct, Some(40));
        assert!(s.project_notes.is_empty());
    }

    #[test]
    fn prune_drops_entries_for_missing_roots() {
        let mut s = State {
            project_order: vec!["/alive".into(), "/dead".into()],
            project_names: BTreeMap::from([
                ("/alive".to_string(), "A".to_string()),
                ("/dead".to_string(), "D".to_string()),
            ]),
            project_notes: BTreeMap::from([
                ("/alive".to_string(), "- [ ] keep".to_string()),
                ("/dead".to_string(), "- [ ] gone".to_string()),
            ]),
            ..Default::default()
        };
        assert!(s.prune_missing_projects(|root| root == "/alive"));
        assert_eq!(s.project_order, vec!["/alive".to_string()]);
        assert!(s.project_names.contains_key("/alive"));
        assert!(!s.project_names.contains_key("/dead"));
        assert!(s.project_notes.contains_key("/alive"));
        assert!(!s.project_notes.contains_key("/dead"));
    }

    #[test]
    fn prune_reports_no_change_when_all_roots_exist() {
        let mut s = State {
            project_notes: BTreeMap::from([("/p".to_string(), "x".to_string())]),
            ..Default::default()
        };
        assert!(!s.prune_missing_projects(|_| true));
        assert!(s.project_notes.contains_key("/p"));
    }

    #[test]
    fn persisted_session_roundtrips_through_toml() {
        let s = State {
            sessions: BTreeMap::from([
                (
                    "work".to_string(),
                    PersistedSession {
                        dir: "/home/u/work".to_string(),
                        agent: "claude".to_string(),
                        resume_cmd: Some(
                            "claude --resume 12345678-1234-1234-1234-1234567890ab".to_string(),
                        ),
                    },
                ),
                (
                    "side".to_string(),
                    PersistedSession {
                        dir: "/home/u/side".to_string(),
                        agent: "codex".to_string(),
                        resume_cmd: None,
                    },
                ),
            ]),
            ..Default::default()
        };
        let toml = toml::to_string(&s).unwrap();
        let back: State = toml::from_str(&toml).unwrap();
        assert_eq!(back.sessions.get("work").unwrap().agent, "claude");
        assert_eq!(
            back.sessions.get("work").unwrap().resume_cmd.as_deref(),
            Some("claude --resume 12345678-1234-1234-1234-1234567890ab")
        );
        assert_eq!(back.sessions.get("side").unwrap().agent, "codex");
        assert!(back.sessions.get("side").unwrap().resume_cmd.is_none());
    }

    #[test]
    fn old_state_without_sessions_key_still_loads() {
        let s: State = toml::from_str("split_pct = 40").unwrap();
        assert!(s.sessions.is_empty());
    }

    fn rec(name: &str) -> RecentSession {
        RecentSession {
            name: name.to_string(),
            dir: format!("/d/{name}"),
            agent: "claude".to_string(),
            resume_cmd: None,
        }
    }

    #[test]
    fn push_recent_dedups_by_name_newest_first_capped() {
        let mut r: Vec<RecentSession> = Vec::new();
        for i in 0..25 {
            push_recent(&mut r, rec(&format!("s{i}")));
        }
        assert_eq!(r.len(), MAX_RECENTS, "capped at the limit");
        assert_eq!(r[0].name, "s24", "newest is first");
        assert_eq!(
            r.last().unwrap().name,
            "s5",
            "oldest beyond the cap evicted"
        );

        // Re-pushing an existing name moves it to front without duplicating,
        // and doesn't grow the list.
        push_recent(&mut r, rec("s10"));
        assert_eq!(r[0].name, "s10", "re-pushed entry jumps to front");
        assert_eq!(
            r.iter().filter(|e| e.name == "s10").count(),
            1,
            "no duplicate entry"
        );
        assert_eq!(r.len(), MAX_RECENTS, "still capped");
    }

    #[test]
    fn recents_roundtrip_through_toml() {
        let s = State {
            recents: vec![rec("a"), rec("b")],
            ..Default::default()
        };
        let back: State = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.recents.len(), 2);
        assert_eq!(back.recents[0].name, "a");
    }
}
