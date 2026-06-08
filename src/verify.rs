//! Background verification worker: runs an `amux-verify` contract in a worktree
//! off the UI thread, streaming progress events tagged by session name.
//!
//! Mirrors `git::spawn_reader` — one thread, a request channel in, a result
//! channel out. Serial by design: gate commands are heavy (cargo builds), so a
//! second request queues behind the first rather than running in parallel.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use amux_verify::{run, Contract, RunOptions, VerdictMsg};

/// One verification request: verify `contract` in `dir`, tagging events with
/// `name`. `cancel` is shared with the controller so it can stop a running (or
/// still-queued) verification.
pub struct VerifyRequest {
    pub name: String,
    pub dir: PathBuf,
    pub contract: Contract,
    pub cancel: Arc<AtomicBool>,
}

/// Handle to the verification worker thread.
pub struct Verifier {
    pub tx: Sender<VerifyRequest>,
    pub rx: Receiver<(String, VerdictMsg)>,
}

/// Finds and loads the contract for a worktree dir, returning a user-facing
/// error string on failure (no contract, or an invalid one).
pub fn load_contract(dir: &Path) -> Result<Contract, String> {
    match amux_verify::find_contract(dir) {
        Some(path) => Contract::load(&path).map_err(|e| e.to_string()),
        None => Err(format!("no .amux/verify.toml in {}", dir.display())),
    }
}

/// Spawns the worker thread. It processes requests one at a time; a request
/// whose `cancel` is already set when it is dequeued is skipped without running.
/// Exits when the request sender drops.
pub fn spawn_verifier() -> Verifier {
    let (req_tx, req_rx) = mpsc::channel::<VerifyRequest>();
    let (res_tx, res_rx) = mpsc::channel::<(String, VerdictMsg)>();
    std::thread::spawn(move || {
        while let Ok(req) = req_rx.recv() {
            if req.cancel.load(Ordering::SeqCst) {
                continue; // cancelled before it ever started
            }
            let opts = RunOptions {
                default_timeout_s: 300,
                task_id: Some(req.name.clone()),
                stream_output: false,
            };
            let name = req.name.clone();
            let mut on_event = |msg: VerdictMsg| {
                let _ = res_tx.send((name.clone(), msg));
            };
            run(&req.dir, &req.contract, &opts, &req.cancel, &mut on_event);
        }
    });
    Verifier {
        tx: req_tx,
        rx: res_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_with_contract(tag: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("amux-verify-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".amux")).unwrap();
        std::fs::write(dir.join(".amux/verify.toml"), body).unwrap();
        dir
    }

    #[test]
    fn worker_runs_a_trivial_contract_to_a_passing_verdict() {
        let dir = temp_with_contract("worker", "[[gate]]\nname = \"ok\"\ncmd = \"true\"\n");
        let contract = load_contract(&dir).unwrap();
        let v = spawn_verifier();
        v.tx.send(VerifyRequest {
            name: "s1".into(),
            dir: dir.clone(),
            contract,
            cancel: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
        let mut passed = None;
        while let Ok((name, msg)) = v.rx.recv_timeout(Duration::from_secs(10)) {
            assert_eq!(name, "s1");
            if let VerdictMsg::Finished { verdict } = msg {
                passed = Some(verdict.passed);
                break;
            }
        }
        assert_eq!(passed, Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worker_skips_a_precancelled_request() {
        let dir = temp_with_contract("cancel", "[[gate]]\nname = \"ok\"\ncmd = \"true\"\n");
        let contract = load_contract(&dir).unwrap();
        let v = spawn_verifier();
        v.tx.send(VerifyRequest {
            name: "s1".into(),
            dir: dir.clone(),
            contract,
            cancel: Arc::new(AtomicBool::new(true)), // preset
        })
        .unwrap();
        // A skipped request emits nothing.
        assert!(v.rx.recv_timeout(Duration::from_millis(300)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_contract_reports_missing() {
        let dir = std::env::temp_dir().join(format!("amux-verify-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = load_contract(&dir).unwrap_err();
        assert!(err.contains("no .amux/verify.toml"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
