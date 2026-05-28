//! Dependency-free time helpers: age humanizer + local HH:MM via `date`.
use std::process::Command;

/// Human age from a duration in seconds: "0s","45s","3m","2h","5d".
pub fn humanize_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// Current local time as "HH:MM" via `date`. Empty string if it fails.
/// NOTE: forks a process — callers should cache (Task 11 wires this in main loop).
pub fn clock_hhmm() -> String {
    Command::new("date")
        .args(["+%H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Unix seconds now (for computing session age). 0 on failure.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(0), "0s");
        assert_eq!(humanize_age(45), "45s");
        assert_eq!(humanize_age(60), "1m");
        assert_eq!(humanize_age(185), "3m");
        assert_eq!(humanize_age(3600), "1h");
        assert_eq!(humanize_age(90_000), "1d");
        assert_eq!(humanize_age(-5), "0s");
    }
}
