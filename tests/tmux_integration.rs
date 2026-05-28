use cm::tmux::{self, Status};

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

    tmux::new_session(&name, dir, "bash").expect("new_session");

    let sessions = tmux::list_sessions().expect("list_sessions");
    let found = sessions.iter().find(|s| s.name == name).expect("session present");
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
