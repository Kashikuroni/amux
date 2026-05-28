//! Read-only git info for a directory (branch + uncommitted diff stat).
//! Never mutates the repo; non-repo dirs return None.
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub added: u32,
    pub removed: u32,
}

/// Parse `git diff --shortstat` output → (insertions, deletions).
/// Examples:
///   "" → (0,0)
///   " 3 files changed, 12 insertions(+), 4 deletions(-)" → (12,4)
///   " 1 file changed, 5 insertions(+)" → (5,0)
///   " 1 file changed, 2 deletions(-)" → (0,2)
pub fn parse_shortstat(s: &str) -> (u32, u32) {
    let mut added = 0;
    let mut removed = 0;
    for part in s.split(',') {
        let p = part.trim();
        if let Some(n) = p.split_whitespace().next().and_then(|w| w.parse::<u32>().ok()) {
            if p.contains("insertion") {
                added = n;
            } else if p.contains("deletion") {
                removed = n;
            }
        }
    }
    (added, removed)
}

fn git_out(dir: &str, args: &[&str]) -> Option<String> {
    // `LC_ALL=C` forces English so we can parse "insertion"/"deletion" reliably
    // regardless of the user's locale. `GIT_OPTIONAL_LOCKS=0` is safe for our
    // read-only commands and avoids stalling on a locked index.
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Branch + working-tree diff stat for `dir`, or None if not a git repo.
pub fn read(dir: &str) -> Option<GitInfo> {
    let branch = git_out(dir, &["symbolic-ref", "--short", "HEAD"])
        .or_else(|| git_out(dir, &["rev-parse", "--short", "HEAD"]))?;
    // `diff HEAD` includes both staged and unstaged changes (vs plain `diff`,
    // which is unstaged only). On a brand-new repo with no commits this is
    // simply empty, which the unwrap_or_default handles.
    let shortstat = git_out(dir, &["diff", "HEAD", "--shortstat"]).unwrap_or_default();
    let (added, removed) = parse_shortstat(&shortstat);
    Some(GitInfo { branch, added, removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_cases() {
        assert_eq!(parse_shortstat(""), (0, 0));
        assert_eq!(
            parse_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)"),
            (12, 4)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 5 insertions(+)"), (5, 0));
        assert_eq!(parse_shortstat(" 1 file changed, 2 deletions(-)"), (0, 2));
    }

    #[test]
    fn read_non_repo_is_none() {
        let dir = std::env::temp_dir().join(format!("cm_git_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read(dir.to_str().unwrap()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_repo_reports_branch_and_diff() {
        // Skip if git missing.
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("cm_git_repo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        // modify tracked file → uncommitted diff
        std::fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        // stage the modification — `diff HEAD` must still count it
        run(&["add", "f.txt"]);

        let info = read(d).expect("repo");
        assert_eq!(info.branch, "main");
        assert!(info.added >= 1, "expected at least one insertion, got {info:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
