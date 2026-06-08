use am::tmux::{self, Status};
use std::process::Command;

/// Full round-trip against a real tmux server. Skipped if tmux is unavailable.
#[test]
fn new_list_rename_capture_kill_roundtrip() {
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }

    // Run on a throwaway socket, killed on drop — never touches the live `cm`.
    let _sock = tmux::isolate_socket(&format!("cm_it_rt_{}", std::process::id()));

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

/// Restart lifecycle against a real tmux server: `remain-on-exit` must keep the
/// pane (and the session) alive after the agent process exits, the dead pane's
/// content must yield the `claude --resume <uuid>` hint, and `respawn_pane`
/// must bring the pane back to life — replicating what `u` does end to end.
/// Skipped if tmux is unavailable.
#[test]
fn restart_lifecycle_remain_on_exit_capture_respawn() {
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }

    // Run on a throwaway socket, killed on drop — never touches the live `cm`.
    let _sock = tmux::isolate_socket(&format!("cm_it_restart_{}", std::process::id()));

    let name = format!("cm_it_restart_{}", std::process::id());
    let _ = tmux::kill_session(&name);
    struct Guard(String);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = tmux::kill_session(&self.0);
        }
    }
    let _guard = Guard(name.clone());

    let dir = std::env::temp_dir();
    let dir = dir.to_str().unwrap();
    // A fake agent: prints the resume hint and exits on the first Ctrl+C —
    // mirroring claude's behavior without needing claude installed.
    let agent = "bash -c 'trap \"echo claude --resume f612324d-83b6-407a-9d74-d89ef7b91f70; exit 0\" INT; echo working; while true; do sleep 1; done'";
    tmux::new_session(&name, dir, agent, "claude").expect("new_session");

    // The restart flow: safety net on, then the double Ctrl+C.
    tmux::set_remain_on_exit(&name, true).expect("remain-on-exit on");
    tmux::send_ctrl_c(&name).expect("send_ctrl_c");

    // Wait for the process to die; the pane must survive as a dead pane.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !tmux::pane_dead(&name).unwrap_or(false) {
        assert!(
            std::time::Instant::now() < deadline,
            "pane never died after Ctrl+C"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let sessions = tmux::list_sessions().expect("list_sessions");
    assert!(
        sessions.iter().any(|s| s.name == name),
        "session must survive the agent exiting"
    );

    // The dead pane keeps the hint; parse it exactly like the poll tick does.
    let pane = tmux::capture_pane(&name).expect("capture dead pane");
    let cmd = tmux::parse_resume_command(&pane).expect("resume hint must parse");
    assert_eq!(cmd, "claude --resume f612324d-83b6-407a-9d74-d89ef7b91f70");

    // Respawn relaunches the pane (a sleeping placeholder stands in for claude).
    tmux::respawn_pane(&name, dir, "sleep 30").expect("respawn_pane");
    assert!(
        !tmux::pane_dead(&name).unwrap_or(true),
        "pane must be alive after respawn"
    );
    tmux::set_remain_on_exit(&name, false).expect("remain-on-exit off");
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

    // Run on a throwaway socket, killed on drop — never touches the live `cm`.
    let _sock = tmux::isolate_socket(&format!("am_wt_it_{}", std::process::id()));

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

/// Existing-branch worktree flow against real git + tmux: a branch that is not
/// checked out anywhere gets a worktree under .worktrees/<branch> (no new
/// branch created); a second session for the same branch reuses that worktree.
/// Skipped if tmux or git is unavailable.
#[test]
fn existing_branch_worktree_create_and_reuse() {
    if !tmux::is_available() || Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: tmux or git not available");
        return;
    }

    // Run on a throwaway socket, killed on drop — never touches the live `cm`.
    let _sock = tmux::isolate_socket(&format!("am_exwt_{}", std::process::id()));

    let repo = std::env::temp_dir().join(format!("am_exwt_{}", std::process::id()));
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
    git(&["branch", "feat"]); // exists, not checked out anywhere

    let name = format!("am_exwt_s_{}", std::process::id());
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

    // First resolution: branch has no worktree → prepare one (no -b).
    assert_eq!(am::git::worktree_for_branch(&repo_s, "feat"), None);
    let wt = repo.join(".worktrees").join("feat");
    let wt_s = wt.to_str().unwrap().to_string();
    am::git::prepare_worktree_existing(&repo_s, &wt_s, "feat").expect("prepare existing");
    assert!(wt.join("f.txt").exists(), "branch content checked out");
    tmux::new_worktree_session(&name, &wt_s, "bash", "bash", &repo_s)
        .expect("session in existing-branch worktree");
    let sessions = tmux::list_sessions().expect("list");
    let s = sessions.iter().find(|s| s.name == name).expect("present");
    assert_eq!(s.worktree_repo.as_deref(), Some(repo_s.as_str()));

    // Second resolution: the worktree is now registered → reuse its path.
    let found = am::git::worktree_for_branch(&repo_s, "feat").expect("registered now");
    assert_eq!(
        std::fs::canonicalize(&found).unwrap(),
        std::fs::canonicalize(&wt_s).unwrap(),
        "same worktree is reused"
    );
}
