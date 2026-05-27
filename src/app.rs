use crate::tmux::Status;

/// Stable hash of pane content, used to detect changes between ticks.
pub fn content_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// First observation (no previous snapshot) is `Waiting`; changed → `Running`.
pub fn compute_status(prev: Option<u64>, current: u64) -> Status {
    match prev {
        Some(p) if p != current => Status::Running,
        _ => Status::Waiting,
    }
}

/// Expands a leading `~` using `$HOME`. Leaves other paths untouched.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{home}{rest}");
            }
        }
    }
    path.to_string()
}

/// Validates create-form input. `dir` is checked after tilde expansion.
pub fn validate_create(name: &str, dir: &str, existing: &[String]) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    if name.contains(':') || name.contains('.') {
        return Err("name cannot contain ':' or '.'".into());
    }
    if existing.iter().any(|n| n == name) {
        return Err(format!("session '{name}' already exists"));
    }
    let expanded = expand_tilde(dir);
    if !std::path::Path::new(&expanded).is_dir() {
        return Err(format!("directory not found: {expanded}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_waiting_on_first_observation() {
        assert_eq!(compute_status(None, 42), Status::Waiting);
    }

    #[test]
    fn status_is_running_when_content_changed() {
        assert_eq!(compute_status(Some(1), 2), Status::Running);
    }

    #[test]
    fn status_is_waiting_when_content_unchanged() {
        assert_eq!(compute_status(Some(7), 7), Status::Waiting);
    }

    #[test]
    fn expand_tilde_replaces_leading_home() {
        std::env::set_var("HOME", "/home/u");
        assert_eq!(expand_tilde("~/proj"), "/home/u/proj");
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    }

    #[test]
    fn validate_rejects_empty_and_duplicate_and_bad_name() {
        let existing = vec!["taken".to_string()];
        assert!(validate_create("", "/tmp", &existing).is_err());
        assert!(validate_create("taken", "/tmp", &existing).is_err());
        assert!(validate_create("a.b", "/tmp", &existing).is_err());
    }

    #[test]
    fn validate_rejects_missing_dir_and_accepts_existing() {
        let existing: Vec<String> = vec![];
        assert!(validate_create("ok", "/no/such/dir/xyz", &existing).is_err());
        assert!(validate_create("ok", "/tmp", &existing).is_ok());
    }
}
