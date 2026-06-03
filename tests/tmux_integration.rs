use am::tmux::{self, Status};
use std::process::Command;

/// Full round-trip against a real tmux server. Skipped if tmux is unavailable.
#[test]
fn new_list_rename_capture_kill_roundtrip() {
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }

    let name = format!("cm_it_{}", std::process::id());
    let renamed = format!("{name}_r");
    // Clean any leftovers from a previous failed run.
    let _ = tmux::kill_session(&name);
    let _ = tmux::kill_session(&renamed);

    struct SessionGuard(Vec<String>);
    impl Drop for SessionGuard {
        fn drop(&mut self) {
            for n in &self.0 {
                let _ = tmux::kill_session(n);
            }
        }
    }
    let _guard = SessionGuard(vec![name.clone(), renamed.clone()]);

    let dir = std::env::temp_dir();
    let dir = dir.to_str().unwrap();

    tmux::new_session(&name, dir, "bash", "bash").expect("new_session");

    let sessions = tmux::list_sessions().expect("list_sessions");
    let found = sessions
        .iter()
        .find(|s| s.name == name)
        .expect("session present");
    assert_eq!(found.agent, "bash");
    assert_eq!(found.status, Status::Idle);

    tmux::rename_session(&name, &renamed).expect("rename_session");
    let sessions = tmux::list_sessions().expect("list after rename");
    assert!(sessions.iter().any(|s| s.name == renamed));
    assert!(!sessions.iter().any(|s| s.name == name));

    let _capture = tmux::capture_pane(&renamed).expect("capture_pane");

    tmux::kill_session(&renamed).expect("kill_session");
    let sessions = tmux::list_sessions().expect("list after kill");
    assert!(!sessions.iter().any(|s| s.name == renamed));
}

/// End-to-end worktree path against real git + tmux: replicates what
/// `main::create_worktree_session` does (add_worktree → new_worktree_session),
/// confirms `list_sessions` surfaces `worktree_repo`, then removes the worktree.
/// Skipped if tmux or git is unavailable.
#[test]
fn worktree_session_reports_repo_and_cleans_up() {
    if !tmux::is_available() || Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: tmux or git not available");
        return;
    }

    // Build a throwaway git repo with one commit on `main`.
    let repo = std::env::temp_dir().join(format!("am_wt_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let repo_s = repo.to_str().unwrap().to_string();
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&repo_s)
            .args(args)
            .output()
            .unwrap();
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f.txt"), "a\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);

    let name = format!("am_wt_{}", std::process::id());
    let wt_path = repo.join(".worktrees").join("feature-x");
    let wt_str = wt_path.to_str().unwrap().to_string();

    // Cleanup guard: kill the session and remove the repo no matter what.
    struct Guard {
        name: String,
        repo: std::path::PathBuf,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = tmux::kill_session(&self.name);
            let _ = std::fs::remove_dir_all(&self.repo);
        }
    }
    let _guard = Guard {
        name: name.clone(),
        repo: repo.clone(),
    };

    // add_worktree + new_worktree_session = the create_worktree_session flow.
    am::git::add_worktree(&repo_s, &wt_str, "feature-x", "main").expect("add_worktree");
    assert!(wt_path.join("f.txt").exists(), "worktree checked out base");
    tmux::new_worktree_session(&name, &wt_str, "bash", "bash", &repo_s)
        .expect("new_worktree_session");

    // list_sessions must surface the @cm_repo tag as worktree_repo.
    let sessions = tmux::list_sessions().expect("list_sessions");
    let s = sessions
        .iter()
        .find(|s| s.name == name)
        .expect("worktree session present");
    assert_eq!(s.worktree_repo.as_deref(), Some(repo_s.as_str()));
    assert_eq!(s.dir.trim_end_matches('/'), wt_str.trim_end_matches('/'));

    // Kill, then remove the worktree (the kill-with-remove path).
    tmux::kill_session(&name).expect("kill_session");
    am::git::remove_worktree(&repo_s, &wt_str).expect("remove_worktree");
    assert!(!wt_path.exists(), "worktree dir removed");
}
