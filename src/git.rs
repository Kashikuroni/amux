//! Read-only git info for a directory (branch + uncommitted diff stat).
//! Never mutates the repo; non-repo dirs return None.
use std::io::Write as _;
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
        if let Some(n) = p
            .split_whitespace()
            .next()
            .and_then(|w| w.parse::<u32>().ok())
        {
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
    Some(GitInfo {
        branch,
        added,
        removed,
    })
}

/// Runs a *mutating* git command in `dir`. Unlike `git_out`, returns the
/// stderr-bearing error on failure so the UI can show why it failed.
fn git_run(dir: &str, args: &[&str]) -> std::io::Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Creates a new worktree at `wt_path` on a new branch `new_branch` forked from `base`.
/// Fails if `new_branch` already exists.
pub fn add_worktree(
    repo_root: &str,
    wt_path: &str,
    new_branch: &str,
    base: &str,
) -> std::io::Result<()> {
    git_run(
        repo_root,
        &["worktree", "add", "-b", new_branch, wt_path, base],
    )
}

/// Removes the worktree at `wt_path`. No `--force`: a dirty worktree errors
/// instead of silently discarding work. The branch itself is left intact.
pub fn remove_worktree(repo_root: &str, wt_path: &str) -> std::io::Result<()> {
    git_run(repo_root, &["worktree", "remove", wt_path])
}

/// Appends `entry` as its own line to `<repo_root>/.gitignore` if not already
/// present (exact-line match). Creates the file when missing.
pub fn ensure_gitignore(repo_root: &str, entry: &str) -> std::io::Result<()> {
    let path = std::path::Path::new(repo_root).join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l == entry) {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.write_all(entry.as_bytes())?;
    f.write_all(b"\n")?;
    Ok(())
}

/// Absolute path of the repository containing `dir`, or None if `dir` is not in a repo.
pub fn repo_root(dir: &str) -> Option<String> {
    git_out(dir, &["rev-parse", "--show-toplevel"])
}

/// Local branch names for `dir`, with the current branch first.
/// Empty when `dir` is not a git repo.
pub fn list_branches(dir: &str) -> Vec<String> {
    let Some(out) = git_out(dir, &["branch", "--format=%(refname:short)"]) else {
        return Vec::new();
    };
    let mut branches: Vec<String> = out
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if let Some(cur) = git_out(dir, &["symbolic-ref", "--short", "HEAD"]) {
        if let Some(i) = branches.iter().position(|b| *b == cur) {
            branches.remove(i);
            branches.insert(0, cur);
        }
    }
    branches
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

    /// Builds a temp git repo on branch `main` with one commit. Returns the dir path.
    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cm_wt_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(d)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn repo_root_resolves_from_subdir() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("root");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let root = repo_root(sub.to_str().unwrap()).expect("root");
        // macOS temp dir is symlinked (/var -> /private/var); compare canonicalized.
        assert_eq!(
            std::fs::canonicalize(&root).unwrap(),
            std::fs::canonicalize(&dir).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_branches_puts_current_first() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("branches");
        let d = dir.to_str().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(d)
            .args(["branch", "feature"])
            .output()
            .unwrap();
        let branches = list_branches(d);
        assert_eq!(branches.first().map(String::as_str), Some("main"));
        assert!(branches.iter().any(|b| b == "feature"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_then_remove_worktree() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wt");
        let root = dir.to_str().unwrap();
        let wt = dir.join(".worktrees").join("feature-x");
        let wt_s = wt.to_str().unwrap();

        add_worktree(root, wt_s, "feature-x", "main").expect("add");
        assert!(
            wt.join("f.txt").exists(),
            "worktree checked out base content"
        );
        let branches = list_branches(root);
        assert!(branches.iter().any(|b| b == "feature-x"));

        remove_worktree(root, wt_s).expect("remove");
        assert!(!wt.exists(), "worktree dir gone after remove");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_worktree_existing_branch_errors() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtdup");
        let root = dir.to_str().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "dup"])
            .output()
            .unwrap();
        let wt = dir.join(".worktrees").join("dup");
        let err = add_worktree(root, wt.to_str().unwrap(), "dup", "main");
        assert!(err.is_err(), "creating an existing branch must fail");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_gitignore_appends_once() {
        let dir = std::env::temp_dir().join(format!("cm_ign_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_str().unwrap();
        ensure_gitignore(root, ".worktrees/").unwrap();
        ensure_gitignore(root, ".worktrees/").unwrap(); // idempotent
        let body = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(body.matches(".worktrees/").count(), 1);
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
            Command::new("git")
                .arg("-C")
                .arg(d)
                .args(args)
                .output()
                .unwrap();
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
        assert!(
            info.added >= 1,
            "expected at least one insertion, got {info:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
