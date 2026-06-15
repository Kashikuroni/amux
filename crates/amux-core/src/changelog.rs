//! The project changelog, embedded at compile time so the running binary always
//! carries the notes for its own version. Parsed into per-version [`Entry`]s for
//! the "What's New" modal (shown once after an upgrade) and the Help → Changelog
//! tab. Format is Keep a Changelog: `## [X.Y.Z] - date` headers with `### …`
//! subsections beneath.

/// The repo `CHANGELOG.md`, baked into the binary.
const RAW: &str = include_str!("../../../CHANGELOG.md");

/// The raw embedded changelog text.
pub fn raw() -> &'static str {
    RAW
}

/// One version section of the changelog.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Version string as written in the header, e.g. "0.5.0" or "Unreleased".
    pub version: String,
    /// Release date if present on the header line (`## [x] - 2026-06-09`).
    pub date: Option<String>,
    /// The body lines beneath the header (subsections + bullets), header excluded,
    /// with surrounding blank lines trimmed.
    pub body: String,
}

/// Splits Keep-a-Changelog text into per-version [`Entry`]s in file order
/// (newest first). A version header looks like `## [0.5.0] - 2026-06-09` or
/// `## [Unreleased]`. An entry with an empty body (e.g. a bare `[Unreleased]`)
/// is dropped, so it never shows as a blank section.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut cur: Option<(String, Option<String>, Vec<String>)> = None;
    let flush = |entries: &mut Vec<Entry>, cur: Option<(String, Option<String>, Vec<String>)>| {
        if let Some((version, date, lines)) = cur {
            let body = lines.join("\n").trim().to_string();
            if !body.is_empty() {
                entries.push(Entry {
                    version,
                    date,
                    body,
                });
            }
        }
    };
    for line in text.lines() {
        if let Some((version, date)) = parse_header(line) {
            flush(&mut entries, cur.take());
            cur = Some((version, date, Vec::new()));
        } else if let Some((_, _, lines)) = cur.as_mut() {
            lines.push(line.to_string());
        }
    }
    flush(&mut entries, cur.take());
    entries
}

/// Parses a `## [version] - date` header line, returning `(version, date)`.
/// `None` for any other line.
fn parse_header(line: &str) -> Option<(String, Option<String>)> {
    let rest = line.strip_prefix("## [")?;
    let (version, after) = rest.split_once(']')?;
    let date = after.trim().trim_start_matches('-').trim().to_string();
    let date = if date.is_empty() { None } else { Some(date) };
    Some((version.to_string(), date))
}

/// The version sections to show in "What's New" when the app boots: everything
/// newer than the `last` version seen, up to `current`. Empty on a first run
/// (`last == None`) or when the version is unchanged — both mean "no upgrade".
pub fn whats_new_on_upgrade(last: Option<&str>, current: &str, text: &str) -> Vec<Entry> {
    match last {
        Some(prev) if prev != current => since(text, prev),
        _ => Vec::new(),
    }
}

/// Entries strictly newer than `last_version` (a bare "X.Y.Z"), newest first.
/// Non-semver sections (e.g. `Unreleased`) are excluded — only released
/// versions count as "what's new".
pub fn since(text: &str, last_version: &str) -> Vec<Entry> {
    parse(text)
        .into_iter()
        .filter(|e| crate::update::is_newer(&format!("v{}", e.version), last_version))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Changelog

## [Unreleased]

## [0.6.0] - 2026-07-01

### Added
- Cool thing.

## [0.5.0] - 2026-06-09

### Changed
- Older thing.
";

    #[test]
    fn parse_splits_versions_skips_empty_unreleased() {
        let e = parse(SAMPLE);
        assert_eq!(e.len(), 2, "empty [Unreleased] is dropped");
        assert_eq!(e[0].version, "0.6.0");
        assert_eq!(e[0].date.as_deref(), Some("2026-07-01"));
        assert!(e[0].body.contains("Cool thing"));
        assert_eq!(e[1].version, "0.5.0");
        assert!(e[1].body.contains("Older thing"));
    }

    #[test]
    fn since_returns_only_newer_versions_newest_first() {
        let e = since(SAMPLE, "0.5.0");
        assert_eq!(e.len(), 1, "only versions strictly newer than 0.5.0");
        assert_eq!(e[0].version, "0.6.0");

        // Older baseline surfaces both released versions.
        let all = since(SAMPLE, "0.4.0");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].version, "0.6.0", "newest first");

        // Up to date: nothing new.
        assert!(since(SAMPLE, "0.6.0").is_empty());
    }

    #[test]
    fn whats_new_only_on_actual_upgrade() {
        // First run: nothing (no prior version recorded).
        assert!(whats_new_on_upgrade(None, "0.6.0", SAMPLE).is_empty());
        // Unchanged version: nothing.
        assert!(whats_new_on_upgrade(Some("0.6.0"), "0.6.0", SAMPLE).is_empty());
        // Upgraded: the new version's notes.
        let e = whats_new_on_upgrade(Some("0.5.0"), "0.6.0", SAMPLE);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].version, "0.6.0");
    }

    #[test]
    fn embedded_changelog_parses() {
        // The real CHANGELOG.md must parse, and carry every released version
        // (guards the backfilled 0.2–0.4.1 sections against accidental loss).
        let parsed = parse(raw());
        let versions: Vec<&str> = parsed.iter().map(|e| e.version.as_str()).collect();
        for v in ["0.5.0", "0.4.1", "0.4.0", "0.3.0", "0.2.0", "0.1.0"] {
            assert!(versions.contains(&v), "changelog missing {v}: {versions:?}");
        }
    }
}
