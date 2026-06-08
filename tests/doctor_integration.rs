use am::{doctor, tmux};
use std::process::Command;

/// Doctor must detect a leaked amux server (a non-cm socket with @cm_managed
/// sessions) and clean it — leaving no socket file. Skipped without tmux.
#[test]
fn detects_and_cleans_a_leaked_amux_server() {
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }
    // Resolve the socket dir and pick a unique throwaway socket name.
    let Some(dir) = doctor::socket_dir() else {
        eprintln!("skipping: no tmux socket dir");
        return;
    };
    let name = format!("cm_doctor_it_{}", std::process::id());
    let path = dir.join(&name);

    // Build a fake leaked amux server: a session tagged @cm_managed=1.
    let tmux_s = |args: &[&str]| {
        Command::new("tmux")
            .arg("-S")
            .arg(&path)
            .args(args)
            .status()
    };
    // Clean any leftover, then create + tag.
    let _ = tmux_s(&["kill-server"]);
    tmux_s(&["new-session", "-d", "-s", "ghost", "sleep 60"]).unwrap();
    tmux_s(&["set-option", "-t", "ghost", "@cm_managed", "1"]).unwrap();

    // Guard: ensure cleanup even if asserts fail.
    struct Guard(std::path::PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .arg("kill-server")
                .status();
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _g = Guard(path.clone());

    // scan() must classify our socket as LeakedAmux.
    let infos = doctor::scan();
    let mine = infos
        .iter()
        .find(|i| i.name == name)
        .expect("doctor saw the socket");
    assert_eq!(mine.class, doctor::SocketClass::LeakedAmux);
    assert!(mine.panes.has_cm_tags);

    // clean() must remove it and the file must be gone.
    let removed = doctor::clean(&infos);
    assert!(removed.contains(&name), "doctor cleaned our leaked socket");
    assert!(!path.exists(), "socket file removed");
}
