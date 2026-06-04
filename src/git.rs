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

/// A background git reader. The UI sends the current session directories; the
/// worker shells out to `git` off the render thread and returns a `dir → GitInfo`
/// map, so a slow/large repo never stalls the UI loop.
pub struct GitReader {
    /// Send the current set of session directories to (re)read.
    pub tx: std::sync::mpsc::Sender<Vec<String>>,
    /// Receive the latest `dir → GitInfo` results.
    pub rx: std::sync::mpsc::Receiver<std::collections::HashMap<String, GitInfo>>,
}

/// Spawns the background git reader thread. It blocks on requests, coalesces to
/// the newest pending one (so a backlog can't build up behind a slow read),
/// dedups the directories, and reads each. Exits when the request sender drops.
pub fn spawn_reader() -> GitReader {
    use std::collections::HashMap;
    use std::sync::mpsc;
    let (req_tx, req_rx) = mpsc::channel::<Vec<String>>();
    let (res_tx, res_rx) = mpsc::channel::<HashMap<String, GitInfo>>();
    std::thread::spawn(move || {
        while let Ok(mut dirs) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                dirs = newer; // only the most recent request matters
            }
            dirs.sort();
            dirs.dedup();
            let mut map = HashMap::new();
            for dir in dirs {
                if let Some(info) = read(&dir) {
                    map.insert(dir, info);
                }
            }
            if res_tx.send(map).is_err() {
                break; // UI gone
            }
        }
    });
    GitReader {
        tx: req_tx,
        rx: res_rx,
    }
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

/// True if a *local* branch named `branch` already exists in the repo.
pub fn branch_exists(dir: &str, branch: &str) -> bool {
    git_out(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_some()
}

/// True if `wt_path` is currently registered as a worktree of this repo.
/// Paths are compared canonicalized so symlinked temp dirs (e.g. macOS
/// `/var` → `/private/var`) and trailing-slash differences don't cause misses.
pub fn is_registered_worktree(repo_root: &str, wt_path: &str) -> bool {
    let canon = |p: &str| {
        std::fs::canonicalize(p)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.trim_end_matches('/').to_string())
    };
    let Some(out) = git_out(repo_root, &["worktree", "list", "--porcelain"]) else {
        return false;
    };
    let target = canon(wt_path);
    out.lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| canon(p) == target)
}

/// Prepares a worktree at `wt_path` on a *new* branch `new_branch` forked from
/// `base`, then leaves a checked-out worktree there. Unlike the bare
/// [`add_worktree`], this resolves the two states that otherwise make
/// `git worktree add` fail with a cryptic error and silently abort session
/// creation:
///
/// - **Branch already taken** → returns a clear, user-facing error asking for a
///   different name (we never silently reuse or mutate an existing branch).
/// - **Leftover directory from a dead worktree** → prunes stale metadata and, if
///   the path is no longer a registered worktree, removes the orphaned directory
///   so creation can proceed. (An empty leftover dir is reused by git as-is.)
pub fn prepare_worktree(
    repo_root: &str,
    wt_path: &str,
    new_branch: &str,
    base: &str,
) -> std::io::Result<()> {
    if branch_exists(repo_root, new_branch) {
        return Err(std::io::Error::other(format!(
            "branch '{new_branch}' already exists — pick another name"
        )));
    }
    // Drop admin entries for worktrees whose directories are gone (best-effort).
    let _ = git_run(repo_root, &["worktree", "prune"]);
    // A non-empty directory at the target path blocks `git worktree add`. If git
    // no longer tracks it as a worktree, it's an orphan we can safely clear.
    let path = std::path::Path::new(wt_path);
    let non_empty = std::fs::read_dir(path)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    if non_empty && !is_registered_worktree(repo_root, wt_path) {
        std::fs::remove_dir_all(path)?;
    }
    add_worktree(repo_root, wt_path, new_branch, base)
}

/// [`prepare_worktree`]'s twin for an EXISTING branch: same stale-state
/// handling (prune + orphan-dir cleanup), but checks the branch *exists* and
/// runs `git worktree add` without `-b` so no new branch is created.
pub fn prepare_worktree_existing(
    repo_root: &str,
    wt_path: &str,
    branch: &str,
) -> std::io::Result<()> {
    if !branch_exists(repo_root, branch) {
        return Err(std::io::Error::other(format!(
            "branch '{branch}' does not exist"
        )));
    }
    let _ = git_run(repo_root, &["worktree", "prune"]);
    let path = std::path::Path::new(wt_path);
    let non_empty = std::fs::read_dir(path)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    if non_empty && !is_registered_worktree(repo_root, wt_path) {
        std::fs::remove_dir_all(path)?;
    }
    git_run(repo_root, &["worktree", "add", wt_path, branch])
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

/// Branch currently checked out in `dir`, or None (detached HEAD / not a repo).
pub fn current_branch(dir: &str) -> Option<String> {
    git_out(dir, &["symbolic-ref", "--short", "HEAD"])
}

/// Path of the worktree (including the main one) where `branch` is checked out,
/// or None if the branch is not checked out anywhere. Parses
/// `git worktree list --porcelain` stanzas: `worktree <path>` … `branch refs/heads/<name>`.
pub fn worktree_for_branch(repo_root: &str, branch: &str) -> Option<String> {
    let out = git_out(repo_root, &["worktree", "list", "--porcelain"])?;
    let needle = format!("branch refs/heads/{branch}");
    let mut current_path: Option<&str> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(p);
        } else if line == needle {
            return current_path.map(str::to_string);
        }
    }
    None
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
    fn branch_exists_detects_local_branch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("branchexists");
        let root = dir.to_str().unwrap();
        assert!(branch_exists(root, "main"), "main must exist");
        assert!(!branch_exists(root, "nope"), "missing branch is false");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_registered_worktree_tracks_live_worktree() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtreg");
        let root = dir.to_str().unwrap();
        let wt = dir.join(".worktrees").join("live");
        let wt_s = wt.to_str().unwrap();
        add_worktree(root, wt_s, "live", "main").expect("add");
        assert!(
            is_registered_worktree(root, wt_s),
            "a checked-out worktree must be recognized"
        );
        let ghost = dir.join(".worktrees").join("ghost");
        assert!(
            !is_registered_worktree(root, ghost.to_str().unwrap()),
            "an unknown path is not a registered worktree"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_rejects_existing_branch_clearly() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("prepdup");
        let root = dir.to_str().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "dup"])
            .output()
            .unwrap();
        let wt = dir.join(".worktrees").join("dup");
        let err = prepare_worktree(root, wt.to_str().unwrap(), "dup", "main")
            .expect_err("existing branch must error");
        assert!(
            err.to_string().contains("already exists"),
            "message must mention the branch already exists, got: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_clears_orphan_dir_and_creates() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("preporphan");
        let root = dir.to_str().unwrap();
        // Leftover NON-EMPTY directory from a dead worktree, branch does NOT exist.
        let wt = dir.join(".worktrees").join("feat");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("stale.txt"), "junk").unwrap();

        prepare_worktree(root, wt.to_str().unwrap(), "feat", "main")
            .expect("orphan dir must be cleared and worktree created");

        assert!(wt.join("f.txt").exists(), "base content checked out");
        assert!(!wt.join("stale.txt").exists(), "stale leftover removed");
        assert!(branch_exists(root, "feat"), "new branch created");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_reuses_empty_leftover_dir() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("prepempty");
        let root = dir.to_str().unwrap();
        // Empty leftover dir (the exact real-world state we observed).
        let wt = dir.join(".worktrees").join("feat");
        std::fs::create_dir_all(&wt).unwrap();

        prepare_worktree(root, wt.to_str().unwrap(), "feat", "main")
            .expect("empty leftover dir must not block creation");
        assert!(wt.join("f.txt").exists(), "base content checked out");
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

    #[test]
    fn current_branch_reports_checked_out_branch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("curbranch");
        let d = dir.to_str().unwrap();
        assert_eq!(current_branch(d).as_deref(), Some("main"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn current_branch_none_for_non_repo() {
        let dir = std::env::temp_dir().join(format!("cm_cb_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(current_branch(dir.to_str().unwrap()), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worktree_for_branch_finds_main_and_linked() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtfind");
        let root = dir.to_str().unwrap();
        let wt = dir.join(".worktrees").join("feat");
        let wt_s = wt.to_str().unwrap();
        add_worktree(root, wt_s, "feat", "main").expect("add");

        // The current branch lives in the main worktree (paths may differ by
        // symlink, e.g. /var vs /private/var — compare canonicalized).
        let canon = |p: &str| std::fs::canonicalize(p).unwrap();
        let main_path = worktree_for_branch(root, "main").expect("main found");
        assert_eq!(canon(&main_path), canon(root));
        // The linked worktree's branch resolves to its path.
        let feat_path = worktree_for_branch(root, "feat").expect("feat found");
        assert_eq!(canon(&feat_path), canon(wt_s));
        // A branch not checked out anywhere → None.
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "loose"])
            .output()
            .unwrap();
        assert_eq!(worktree_for_branch(root, "loose"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_existing_creates_for_existing_branch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtexist");
        let root = dir.to_str().unwrap();
        // An existing branch with NO worktree.
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "feat"])
            .output()
            .unwrap();
        let wt = dir.join(".worktrees").join("feat");
        prepare_worktree_existing(root, wt.to_str().unwrap(), "feat").expect("create");
        assert!(wt.join("f.txt").exists(), "branch content checked out");
        // No second branch appeared (no -b): still exactly main + feat.
        let branches = list_branches(root);
        assert_eq!(branches.len(), 2, "no extra branch created: {branches:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_existing_rejects_missing_branch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtmissing");
        let root = dir.to_str().unwrap();
        let wt = dir.join(".worktrees").join("nope");
        let err = prepare_worktree_existing(root, wt.to_str().unwrap(), "nope")
            .expect_err("missing branch must error");
        assert!(
            err.to_string().contains("does not exist"),
            "clear message, got: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_worktree_existing_clears_orphan_dir() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = temp_repo("wtorphan");
        let root = dir.to_str().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "feat"])
            .output()
            .unwrap();
        let wt = dir.join(".worktrees").join("feat");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("stale.txt"), "junk").unwrap();
        prepare_worktree_existing(root, wt.to_str().unwrap(), "feat")
            .expect("orphan cleared, worktree created");
        assert!(wt.join("f.txt").exists());
        assert!(!wt.join("stale.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
