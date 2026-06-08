# amux-verify TUI integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run a session's `.amux/verify.toml` contract from the amux TUI with a key (`v`), show live progress + a pass/fail verdict in the session's status slot, cancel with a second `v`, and inspect failures in a detail modal (`V`).

**Architecture:** Reuse amux's background-worker pattern (`git::spawn_reader`): a new serial `verify.rs` thread runs the existing `amux-verify` crate off the UI thread and streams `VerdictMsg` events tagged by session name. The pure `App` holds per-session `VerificationState` (keyed by name, not a `Session` field), applies events in `refresh`, and renders the state in the card's status slot. Verdicts age out only after the agent has been Running ≥ 30 s (so brief status flips don't discard them).

**Tech Stack:** Rust, ratatui (TUI), std `mpsc` channels + threads, the `amux-verify` workspace crate.

**Spec:** `docs/superpowers/specs/2026-06-08-amux-verify-tui-integration-design.md`

**Package note:** the root package is `amux`; its **library crate is `am`** (so `main.rs` uses `am::app::…`, `am::verify::…`). The verifier crate's lib is `amux_verify`. Run root-crate tests with `cargo test -p amux`.

---

### Task 1: Dependency + `verify.rs` background worker

The verifier crate is a workspace member but the root crate does not depend on it yet. Add the dependency and a serial background worker that wraps `amux_verify::run`, plus a contract-loading helper.

**Files:**
- Modify: `Cargo.toml` (root `[dependencies]`)
- Create: `src/verify.rs`
- Modify: `src/lib.rs` (add `pub mod verify;`)
- Test: `src/verify.rs` (tests module)

- [ ] **Step 1: Add the path dependency**

In root `Cargo.toml`, under `[dependencies]` (after `ansi-to-tui = "7"`):

```toml
amux-verify = { path = "crates/amux-verify" }
```

- [ ] **Step 2: Declare the module**

In `src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod verify;
```

- [ ] **Step 3: Write the worker (and helper)**

Create `src/verify.rs`:

```rust
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
    Verifier { tx: req_tx, rx: res_rx }
}
```

- [ ] **Step 4: Write the worker tests**

Append to `src/verify.rs`:

```rust
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
        v.tx
            .send(VerifyRequest {
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
        v.tx
            .send(VerifyRequest {
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
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p amux --lib verify:: 2>&1 | tail -15`
Expected: 3 passed (the worker actually runs `true` and reports `passed == true`).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/lib.rs src/verify.rs
git commit -m "feat(verify): background verification worker over amux-verify"
```

---

### Task 2: `VerificationState` + `apply_verify_event` + App wiring

Add the per-session verification data to `App`, the event-application logic, and attach the worker at startup.

**Files:**
- Modify: `src/app.rs` (imports; `VerificationState`; App fields ~865-911; `App::new` init ~940-962; `attach_verifier`; `apply_verify_event`)
- Modify: `src/main.rs` (call `attach_verifier()` near `attach_git_worker()`, ~line 47)
- Test: `src/app.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/app.rs`:

```rust
fn done_verdict(passed: bool) -> amux_verify::Verdict {
    amux_verify::Verdict { task_id: None, passed, gates: vec![] }
}

#[test]
fn apply_verify_event_drives_running_then_done() {
    use amux_verify::VerdictMsg::*;
    let mut app = app_with_two_sessions();
    app.apply_verify_event("a", Started { total_gates: 2 });
    assert!(matches!(
        app.verification.get("a"),
        Some(VerificationState::Running { total: 2, done: 0, .. })
    ));
    app.apply_verify_event("a", GateStarted { index: 0, name: "build".into() });
    app.apply_verify_event(
        "a",
        GateFinished { index: 0, result: gate_result("build", amux_verify::GateStatus::Passed) },
    );
    match app.verification.get("a") {
        Some(VerificationState::Running { done, current, .. }) => {
            assert_eq!(*done, 1);
            assert_eq!(current, "build");
        }
        other => panic!("expected Running, got {other:?}"),
    }
    app.apply_verify_event("a", Finished { verdict: done_verdict(true) });
    assert!(matches!(app.verification.get("a"), Some(VerificationState::Done(_))));
}

#[test]
fn finished_is_ignored_without_a_running_entry() {
    // Models a cancelled run: state already removed, trailing Finished must not revive it.
    let mut app = app_with_two_sessions();
    app.apply_verify_event("a", amux_verify::VerdictMsg::Finished { verdict: done_verdict(true) });
    assert!(app.verification.get("a").is_none());
}

fn gate_result(name: &str, status: amux_verify::GateStatus) -> amux_verify::GateResult {
    amux_verify::GateResult {
        name: name.into(),
        status,
        exit_code: Some(0),
        duration_ms: 1,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        repro: name.into(),
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p amux --lib apply_verify_event_drives_running_then_done 2>&1 | tail -20`
Expected: compile error — `VerificationState`, `app.verification`, `apply_verify_event` undefined.

- [ ] **Step 3: Add imports**

Near the top of `src/app.rs`, ensure these are imported (add what's missing):

```rust
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
```

- [ ] **Step 4: Define `VerificationState`**

In `src/app.rs` (near the other public data types, e.g. just above `pub enum Action`):

```rust
/// A session's verification status, keyed by session name on `App` (not a
/// `Session` field — `Session` is rebuilt from tmux every refresh). Rendered in
/// the card's status slot.
#[derive(Debug, Clone)]
pub enum VerificationState {
    /// A run is in flight: `done`/`total` gates, `current` gate name (may be empty
    /// before the first GateStarted).
    Running { total: usize, done: usize, current: String },
    /// A finished verdict (passed or failed).
    Done(amux_verify::Verdict),
}
```

- [ ] **Step 5: Add the App fields**

In the `App` struct, beside `git_worker`/`restarting` (~865-911):

```rust
    /// Per-session verification state, keyed by session name.
    pub verification: HashMap<String, VerificationState>,
    /// In-flight cancel flags, keyed by session name (kept out of
    /// `VerificationState` so that stays pure data).
    pub verify_cancel: HashMap<String, Arc<AtomicBool>>,
    /// Unix-secs marking the start of a session's current uninterrupted Running
    /// spell — used to age out a verdict only after sustained (≥30 s) work.
    pub running_since: HashMap<String, i64>,
    pub verify_worker: Option<crate::verify::Verifier>,
```

In `App::new` (~940-962), initialise them:

```rust
            verification: HashMap::new(),
            verify_cancel: HashMap::new(),
            running_since: HashMap::new(),
            verify_worker: None,
```

- [ ] **Step 6: Add `attach_verifier` and `apply_verify_event`**

In `src/app.rs`, beside `attach_git_worker` (~2200):

```rust
    pub fn attach_verifier(&mut self) {
        self.verify_worker = Some(crate::verify::spawn_verifier());
    }

    /// Applies one verification event. Every non-`Started` event is applied only
    /// while a `Running` entry exists, so a cancelled run (whose state was
    /// removed) never revives into a `Done` verdict.
    pub fn apply_verify_event(&mut self, name: &str, msg: amux_verify::VerdictMsg) {
        use amux_verify::VerdictMsg::*;
        match msg {
            Started { total_gates } => {
                self.verification.insert(
                    name.to_string(),
                    VerificationState::Running { total: total_gates, done: 0, current: String::new() },
                );
            }
            GateStarted { name: gate, .. } => {
                if let Some(VerificationState::Running { current, .. }) = self.verification.get_mut(name) {
                    *current = gate;
                }
            }
            GateFinished { index, .. } => {
                if let Some(VerificationState::Running { done, .. }) = self.verification.get_mut(name) {
                    *done = index + 1;
                }
            }
            Finished { verdict } => {
                if matches!(self.verification.get(name), Some(VerificationState::Running { .. })) {
                    self.verification.insert(name.to_string(), VerificationState::Done(verdict));
                    self.verify_cancel.remove(name);
                }
            }
        }
    }
```

- [ ] **Step 7: Attach the worker at startup**

In `src/main.rs`, right after `app.attach_git_worker();` (~line 47):

```rust
    app.attach_verifier();
```

- [ ] **Step 8: Run the tests**

Run: `cargo test -p amux --lib apply_verify_event_drives_running_then_done finished_is_ignored_without_a_running_entry 2>&1 | tail -10`
Expected: both PASS. Then `cargo build -p amux 2>&1 | tail -3` — compiles.

- [ ] **Step 9: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(app): VerificationState + event application; attach verifier"
```

---

### Task 3: `Action::Verify`/`CancelVerify` + `v` key + execution

Bind `v` to start/cancel verification and execute it in `handle_action`.

**Files:**
- Modify: `src/app.rs` (`Action` enum ~800-810; `handle_list_key` — add `v` near the `u`/`Ctrl+R` arms ~1353)
- Modify: `src/main.rs` (new arms after `ReturnToRoot` ~line 530)
- Test: `src/app.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/app.rs`:

```rust
#[test]
fn v_starts_then_cancels_verification() {
    let mut app = app_with_two_sessions();
    app.selected = 0;
    // No state → Verify.
    assert_eq!(app.handle_key(key('v')), Some(Action::Verify { name: "a".into() }));
    // Running → CancelVerify.
    app.verification.insert(
        "a".into(),
        VerificationState::Running { total: 1, done: 0, current: String::new() },
    );
    assert_eq!(app.handle_key(key('v')), Some(Action::CancelVerify { name: "a".into() }));
    // Done → Verify (re-run).
    app.verification.insert("a".into(), VerificationState::Done(done_verdict(true)));
    assert_eq!(app.handle_key(key('v')), Some(Action::Verify { name: "a".into() }));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p amux --lib v_starts_then_cancels_verification 2>&1 | tail -20`
Expected: compile error — `Action::Verify`/`CancelVerify` undefined.

- [ ] **Step 3: Add the Action variants**

In `pub enum Action` (`src/app.rs`, before the closing brace):

```rust
    /// Verify a session's worktree against its `.amux/verify.toml`.
    Verify { name: String },
    /// Cancel the in-flight verification for a session.
    CancelVerify { name: String },
```

- [ ] **Step 4: Bind `v`**

In `handle_list_key`, after the `Ctrl+R` / `r` arms (~1353):

```rust
            // v: verify the selected session against its contract. A second v
            // while it runs cancels it. Plain letter (non-destructive: read-only
            // checks, itself cancellable) per the feature doc.
            KeyCode::Char('v') if !ctrl => {
                if let Some(name) = self.selected_name() {
                    let running = matches!(
                        self.verification.get(&name),
                        Some(VerificationState::Running { .. })
                    );
                    return Some(if running {
                        Action::CancelVerify { name }
                    } else {
                        Action::Verify { name }
                    });
                }
            }
```

- [ ] **Step 5: Run the key test**

Run: `cargo test -p amux --lib v_starts_then_cancels_verification 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Execute the actions in `main.rs`**

Add after the `Action::ReturnToRoot { .. }` arm (~line 530) in `handle_action`:

```rust
        Action::Verify { name } => {
            let dir = app.sessions.iter().find(|s| s.name == name).map(|s| s.dir.clone());
            match dir {
                None => app.error = Some(format!("session '{name}' not found")),
                Some(dir) => match am::verify::load_contract(std::path::Path::new(&dir)) {
                    Ok(contract) => {
                        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        app.verify_cancel.insert(name.clone(), cancel.clone());
                        app.verification.insert(
                            name.clone(),
                            am::app::VerificationState::Running {
                                total: contract.gates.len(),
                                done: 0,
                                current: String::new(),
                            },
                        );
                        if let Some(w) = &app.verify_worker {
                            let _ = w.tx.send(am::verify::VerifyRequest {
                                name: name.clone(),
                                dir: std::path::PathBuf::from(&dir),
                                contract,
                                cancel,
                            });
                        }
                    }
                    Err(e) => app.error = Some(e),
                },
            }
            app.refresh();
        }
        Action::CancelVerify { name } => {
            if let Some(c) = app.verify_cancel.remove(&name) {
                c.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            app.verification.remove(&name);
            app.refresh();
        }
```

- [ ] **Step 7: Build and run the suite**

Run: `cargo test -p amux 2>&1 | tail -8`
Expected: PASS (the `match action` is exhaustive again; key test green).

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(app): 'v' starts/cancels session verification"
```

---

### Task 4: Drain events + clear-on-resume + GC in `refresh`

Apply incoming events each refresh, age out stale verdicts after sustained Running, and GC vanished sessions.

**Files:**
- Modify: `src/app.rs` (`refresh` — call after `self.sessions = …` ~line 2276; new `poll_verifications` method)
- Test: `src/app.rs` (tests module)

- [ ] **Step 1: Write the failing tests**

Add to `#[cfg(test)] mod tests` in `src/app.rs`:

```rust
#[test]
fn verdict_clears_after_sustained_running_but_survives_flaky_flip() {
    let mut app = app_with_two_sessions(); // sessions "a","b"; status defaults Idle
    app.verification.insert("a".into(), VerificationState::Done(done_verdict(true)));

    // Running < 30s → kept (sets running_since = 1000).
    app.sessions[0].status = Status::Running;
    app.now_unix = 1000;
    app.poll_verifications();
    assert!(app.verification.contains_key("a"), "kept while running < 30s");

    // Flip back to Idle before 30s → running_since cleared, verdict survives.
    app.sessions[0].status = Status::Idle;
    app.now_unix = 1010;
    app.poll_verifications();
    assert!(app.verification.contains_key("a"), "flaky flip keeps verdict");

    // Sustained running ≥ 30s → cleared.
    app.sessions[0].status = Status::Running;
    app.now_unix = 1020; // running_since = 1020
    app.poll_verifications();
    app.now_unix = 1050; // +30s
    app.poll_verifications();
    assert!(!app.verification.contains_key("a"), "cleared after 30s sustained running");
}

#[test]
fn poll_verifications_gcs_vanished_sessions() {
    let mut app = app_with_two_sessions();
    app.verification.insert("ghost".into(), VerificationState::Done(done_verdict(true)));
    app.verify_cancel.insert("ghost".into(), std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
    app.running_since.insert("ghost".into(), 0);
    app.poll_verifications();
    assert!(!app.verification.contains_key("ghost"));
    assert!(!app.verify_cancel.contains_key("ghost"));
    assert!(!app.running_since.contains_key("ghost"));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p amux --lib poll_verifications_gcs_vanished_sessions 2>&1 | tail -15`
Expected: compile error — `poll_verifications` undefined.

- [ ] **Step 3: Implement `poll_verifications`**

In `src/app.rs` (as a method on `App`, near `refresh`):

```rust
    /// Per-refresh verification lifecycle: drain worker events, age out verdicts
    /// for sessions that genuinely resumed work (Running ≥ 30 s), and GC state
    /// for sessions that no longer exist.
    fn poll_verifications(&mut self) {
        // Drain events first (collect, then mutate — avoids borrowing the worker
        // while mutating `verification`).
        let mut events = Vec::new();
        if let Some(w) = &self.verify_worker {
            while let Ok(ev) = w.rx.try_recv() {
                events.push(ev);
            }
        }
        for (name, msg) in events {
            self.apply_verify_event(&name, msg);
        }

        // Clear-on-resume. Snapshot (name, is_running) so we don't hold a borrow
        // of `self.sessions` while mutating the maps. 30 s of *uninterrupted*
        // Running is the "agent genuinely resumed" signal; brief flips never
        // reach it.
        let snap: Vec<(String, bool)> = self
            .sessions
            .iter()
            .map(|s| (s.name.clone(), s.status == crate::tmux::Status::Running))
            .collect();
        for (name, is_running) in &snap {
            if *is_running {
                let since = *self.running_since.entry(name.clone()).or_insert(self.now_unix);
                if self.now_unix - since >= 30
                    && matches!(self.verification.get(name), Some(VerificationState::Done(_)))
                {
                    self.verification.remove(name);
                    self.running_since.remove(name);
                }
            } else {
                self.running_since.remove(name);
            }
        }

        // GC state for sessions that no longer exist.
        let live: std::collections::HashSet<&str> =
            self.sessions.iter().map(|s| s.name.as_str()).collect();
        self.verification.retain(|n, _| live.contains(n.as_str()));
        self.verify_cancel.retain(|n, _| live.contains(n.as_str()));
        self.running_since.retain(|n, _| live.contains(n.as_str()));
    }
```

- [ ] **Step 4: Call it from `refresh`**

In `src/app.rs` `refresh`, right after `self.sessions = apply_grouped_order(&self.project_order, &self.order, sessions);` (~line 2276):

```rust
                self.poll_verifications();
```

- [ ] **Step 5: Run the tests + suite**

Run: `cargo test -p amux --lib 2>&1 | tail -8`
Expected: PASS, including both new tests; no regressions.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): drain verify events, age out verdicts, GC in refresh"
```

---

### Task 5: Render verification in the status slot

Show the verification state in the card's line-1 status slot (replacing the agent status when present).

**Files:**
- Modify: `src/ui/sessions.rs` (`card` signature ~38-49; status tuple ~56-72; call site ~421-422)
- Test: `src/ui/sessions.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/ui/sessions.rs`. Reuse the module's existing single-card render pattern (render a `ListItem` into a `Buffer`, then `buf_to_string`). Build a `Session` literal like the module's other render tests.

```rust
fn verdict(passed: bool, fail_gate: &str) -> amux_verify::Verdict {
    let gates = if passed {
        vec![]
    } else {
        vec![amux_verify::GateResult {
            name: fail_gate.into(),
            status: amux_verify::GateStatus::Failed,
            exit_code: Some(1),
            duration_ms: 1,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            repro: fail_gate.into(),
        }]
    };
    amux_verify::Verdict { task_id: None, passed, gates }
}

#[test]
fn verification_states_render_in_status_slot() {
    let s = Session {
        name: "feat".into(),
        dir: "/repo".into(),
        cwd: "/repo".into(),
        created: 1,
        agent: "claude".into(),
        status: Status::Idle,
        attached: false,
        git: None,
        worktree_repo: None,
    };
    let mk = |vs: &crate::app::VerificationState| {
        let item = card(&s, 0, false, None, 80, 1, 0, 0, false, GitCardState::Loading, Some(vs));
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 80, 4));
        let list = ratatui::widgets::List::new(vec![item]);
        ratatui::widgets::Widget::render(list, buf.area, &mut buf);
        buf_to_string(&buf)
    };
    use crate::app::VerificationState::*;
    assert!(mk(&Running { total: 3, done: 1, current: "clippy".into() }).contains("clippy 1/3"));
    assert!(mk(&Done(verdict(true, ""))).contains("verified"));
    assert!(mk(&Done(verdict(false, "clippy"))).contains("failed: clippy"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p amux --lib verification_states_render_in_status_slot 2>&1 | tail -20`
Expected: compile error — `card` takes 10 args, not 11.

- [ ] **Step 3: Add the `verify` parameter**

In `src/ui/sessions.rs`, extend `card`'s signature (after `git_state: GitCardState`):

```rust
    git_state: GitCardState,
    verify: Option<&crate::app::VerificationState>,
) -> ListItem<'static> {
```

- [ ] **Step 4: Override the status tuple when verifying**

In `card`, the status tuple is currently built as `let (status_glyph, status_label, status_color) = if restarting { … } else { match s.status { … } };`. Wrap it so verification wins first, and make every arm yield `String` (the existing `restarting`/`status` arms use `&str` literals for the label — change those to `.to_string()` so all arms share the type):

```rust
    let (status_glyph, status_label, status_color) = if let Some(vs) = verify {
        match vs {
            crate::app::VerificationState::Running { total, done, current } => {
                let label = if current.is_empty() {
                    format!("verifying {done}/{total}")
                } else {
                    format!("{current} {done}/{total}")
                };
                (spinner::glyph(spinner_frame).to_string(), label, Color::Blue)
            }
            crate::app::VerificationState::Done(v) if v.passed => {
                ("✓".to_string(), "verified".to_string(), Color::Green)
            }
            crate::app::VerificationState::Done(v) => {
                let gate = v
                    .gates
                    .iter()
                    .find(|g| g.status != amux_verify::GateStatus::Passed)
                    .map(|g| g.name.as_str())
                    .unwrap_or("failed");
                ("✗".to_string(), format!("failed: {gate}"), Color::Red)
            }
        }
    } else if restarting {
        (
            spinner::glyph(spinner_frame).to_string(),
            "restarting".to_string(),
            Color::Yellow,
        )
    } else {
        match s.status {
            Status::Running => (
                spinner::glyph(spinner_frame).to_string(),
                "running".to_string(),
                Color::Blue,
            ),
            Status::Idle => (th::IDLE_MARK.to_string(), "idle".to_string(), Color::Red),
            Status::Waiting => (th::WAIT_MARK.to_string(), "waiting".to_string(), INDIGO),
        }
    };
```

(The rest of line 1 is unchanged: `status_width = 1 + 1 + status_label.chars().count()`, the padding math, and `Span::styled(format!(" {status_label}"), status_style)` all work with `status_label: String`.)

- [ ] **Step 5: Pass the state at the call site**

In `src/ui/sessions.rs`, where `card(...)` is invoked (~421-422), add the trailing argument:

```rust
        let git_state = crate::app::git_card_state(&app.git_cache, s);
        let verify = app.verification.get(&s.name);
        items.push(card(
            s,
            app.spinner_frame,
            pos == sel,
            prompt,
            content_width,
            pos + 1,
            done,
            total,
            restarting,
            git_state,
            verify,
        ));
```

- [ ] **Step 6: Run the test + suite**

Run: `cargo test -p amux --lib 2>&1 | tail -8`
Expected: PASS, including `verification_states_render_in_status_slot`. Existing card tests (which pass `None` for verify) are unaffected — but they now need the extra `None` argument; update each existing `card(...)` test call to pass a trailing `None` (search the test module for `card(` and add `, None`).

- [ ] **Step 7: Commit**

```bash
git add src/ui/sessions.rs
git commit -m "feat(ui): render verification status in the card status slot"
```

---

### Task 6: Detail modal (`V`)

A read-only modal listing gate results + failure details, opened with `V`.

**Files:**
- Modify: `src/app.rs` (`Mode` enum ~752-790; `ModeKind` enum ~821; `mode_kind` map ~1169-1184; `handle_key` routing ~1191; `V` key in `handle_list_key`)
- Create: `src/ui/modal_verify.rs`
- Modify: `src/ui/mod.rs` (`mod modal_verify;`; draw routing ~50-65)
- Modify: `src/ui/footer.rs` (`Mode::VerifyDetail` arm)
- Test: `src/ui/modal_verify.rs` (tests module); `src/app.rs` (key test)

- [ ] **Step 1: Write the failing key test**

Add to `#[cfg(test)] mod tests` in `src/app.rs`:

```rust
#[test]
fn shift_v_opens_detail_only_with_a_verdict() {
    let mut app = app_with_two_sessions();
    app.selected = 0;
    // No verdict → no-op.
    app.handle_key(key('V'));
    assert!(matches!(app.mode, Mode::List));
    // Done verdict → opens detail.
    app.verification.insert("a".into(), VerificationState::Done(done_verdict(false)));
    app.handle_key(key('V'));
    assert!(matches!(app.mode, Mode::VerifyDetail(ref n) if n == "a"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p amux --lib shift_v_opens_detail_only_with_a_verdict 2>&1 | tail -15`
Expected: compile error — `Mode::VerifyDetail` undefined.

- [ ] **Step 3: Add the mode**

In `src/app.rs` `pub enum Mode`, add a variant:

```rust
    /// Read-only verification detail for a session (gate results + failures).
    VerifyDetail(String),
```

In `enum ModeKind` (~821), add:

```rust
    VerifyDetail,
```

In `mode_kind()` (~1169-1184), add:

```rust
            Mode::VerifyDetail(_) => ModeKind::VerifyDetail,
```

In `handle_key`'s `match self.mode_kind()` (~1191, beside `ModeKind::Help`), add an any-key-closes arm:

```rust
            ModeKind::VerifyDetail => {
                self.mode = Mode::List;
                None
            }
```

- [ ] **Step 4: Bind `V`**

In `handle_list_key`, right after the `v` arm:

```rust
            // V: open the verification detail modal (gates + failure output).
            KeyCode::Char('V') => {
                if let Some(name) = self.selected_name() {
                    if matches!(self.verification.get(&name), Some(VerificationState::Done(_))) {
                        self.mode = Mode::VerifyDetail(name);
                    }
                }
            }
```

- [ ] **Step 5: Run the key test**

Run: `cargo test -p amux --lib shift_v_opens_detail_only_with_a_verdict 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Write the modal renderer + its test**

Create `src/ui/modal_verify.rs`. Mirror the centering used by `src/ui/modal_help.rs::render` (read it: a centered `Rect`, `Clear`, a bordered `Block`, then a `Paragraph` of `Line`s). Build the content as below:

```rust
use crate::app::{App, VerificationState};
use crate::theme as th;
use amux_verify::GateStatus;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, name: &str) {
    let Some(VerificationState::Done(verdict)) = app.verification.get(name) else {
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("verify · {name}"),
        Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for g in &verdict.gates {
        let (glyph, color) = match g.status {
            GateStatus::Passed => ("✓", Color::Green),
            GateStatus::Failed => ("✗", Color::Red),
            GateStatus::TimedOut => ("⏱", Color::Red),
            GateStatus::Skipped => ("⊘", Color::Reset),
        };
        let secs = g.duration_ms as f64 / 1000.0;
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::raw(format!("{:<14} ({secs:.1}s)", g.name)),
        ]));
    }
    for g in verdict.gates.iter().filter(|g| g.status != GateStatus::Passed) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("repro: {}", g.repro),
            Style::default().add_modifier(Modifier::DIM),
        )));
        for tail in g.stderr_tail.lines().chain(g.stdout_tail.lines()) {
            lines.push(Line::from(Span::styled(
                format!("  {tail}"),
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
    }

    // Centered box — same approach as modal_help::render.
    let area = centered(f.area(), 72, (lines.len() as u16 + 2).min(f.area().height));
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(" verify ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Centers a `w`×`h` rect inside `area` (clamped to `area`).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}
```

> If `modal_help.rs` already exposes a shared centering helper, use it instead of the local `centered` and drop the local one. Read `modal_help.rs` first and match the codebase's actual pattern (block style, title style).

Append a test to `src/ui/modal_verify.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn lists_gates_and_failure_details() {
        let mut app = App::new(Config::default());
        let verdict = amux_verify::Verdict {
            task_id: None,
            passed: false,
            gates: vec![amux_verify::GateResult {
                name: "clippy".into(),
                status: GateStatus::Failed,
                exit_code: Some(1),
                duration_ms: 1200,
                stdout_tail: String::new(),
                stderr_tail: "error: bad thing".into(),
                repro: "cargo clippy".into(),
            }],
        };
        app.verification.insert("feat".into(), VerificationState::Done(verdict));
        let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
        t.draw(|f| render(f, &app, "feat")).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("verify · feat"), "{s}");
        assert!(s.contains("clippy"), "{s}");
        assert!(s.contains("repro: cargo clippy"), "{s}");
        assert!(s.contains("error: bad thing"), "{s}");
    }
}
```

- [ ] **Step 7: Wire the renderer + footer**

In `src/ui/mod.rs`, add `mod modal_verify;` beside the other `mod modal_*;`, and a draw arm in the `match app.mode` (~50-65):

```rust
        Mode::VerifyDetail(name) => modal_verify::render(f, app, name),
```

In `src/ui/footer.rs` `items_for`, add an arm:

```rust
        Mode::VerifyDetail(_) => vec![("esc", "close", true)],
```

- [ ] **Step 8: Run tests + suite**

Run: `cargo test -p amux 2>&1 | tail -8`
Expected: PASS (modal test + key test + all else).

- [ ] **Step 9: Commit**

```bash
git add src/app.rs src/ui/modal_verify.rs src/ui/mod.rs src/ui/footer.rs
git commit -m "feat(ui): verification detail modal (V)"
```

---

### Task 7: Help, footer, changelog, final verification

**Files:**
- Modify: `src/ui/modal_help.rs` (Session group)
- Modify: `src/ui/footer.rs` (`Mode::List` items)
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Help entries**

In `src/ui/modal_help.rs`, in the `"Session"` group (beside `("d", "kill")`), add:

```rust
                ("v", "verify session"),
                ("V", "verify details"),
```

- [ ] **Step 2: Footer hint**

In `src/ui/footer.rs`, in the `Mode::List` items vec (beside `("d", "kill", false)`), add:

```rust
            ("v", "verify", false),
```

- [ ] **Step 3: Changelog**

In `CHANGELOG.md`, under `## [Unreleased]` → `### Added` (top of the list):

```markdown
- Verify a session from the TUI: `v` runs its `.amux/verify.toml` contract in the
  background (a second `v` cancels), the status slot shows live progress and a
  `✓ verified` / `✗ failed: <gate>` verdict, and `V` opens a detail panel with
  per-gate results, the repro command, and the failure output.
```

- [ ] **Step 4: Format**

Run: `cargo fmt -p amux`
Expected: no errors.

- [ ] **Step 5: Lint the workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: `Finished`, no warnings.

- [ ] **Step 6: Full test suite**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|error|FAILED"`
Expected: every line `test result: ok`; no `FAILED`.

- [ ] **Step 7: Commit**

```bash
git add src/ui/modal_help.rs src/ui/footer.rs CHANGELOG.md
git commit -m "docs: help/footer/changelog for session verification"
```

---

## Self-Review

**Spec coverage:**
- Dependency on `amux-verify` → Task 1. ✅
- `verify.rs` serial worker (`spawn_verifier`, request/result channels, skip-precancelled, `load_contract`) → Task 1. ✅
- `VerificationState` (Running/Done) keyed by name; `apply_verify_event` with cancel-ignore rule → Task 2. ✅
- `attach_verifier` at startup → Task 2. ✅
- `Action::Verify`/`CancelVerify`, `v` toggle, find+load contract / spawn / cancel → Task 3. ✅
- Drain in `refresh`; clear-on-resume (≥30 s sustained Running; flaky flip survives); GC → Task 4. ✅
- Status-slot render: `◐ <gate> done/total` / `✓ verified` / `✗ failed: <gate>` → Task 5. ✅
- `V` detail modal (`Mode::VerifyDetail`, gates + repro + output tail) → Task 6. ✅
- Help/footer/changelog → Task 7. ✅
- Tests: worker, event application, cancel-ignore, `v` toggle, clear-on-resume, GC, render states, modal → Tasks 1-6. ✅

**Deviation from spec (intentional):** verification badge colors are inline `Color::Green/Red/Blue`
in `ui/sessions.rs` (matching the existing `status_color` pattern there — the theme is otherwise
monochrome `Color::Reset`), rather than new `theme.rs` tokens. Same visual result, consistent with
the established status-color code.

**Placeholder scan:** none — every code step is complete. The one "read the existing file and mirror
its pattern" note (modal centering, Task 6 Step 6) points at a concrete existing function with full
content code provided; the centering helper is included inline as a fallback.

**Type consistency:** `VerificationState { Running{total,done,current}, Done(Verdict) }` is used
identically in Tasks 2-6. `apply_verify_event(&str, VerdictMsg)`, `poll_verifications(&mut self)`,
`Action::Verify/CancelVerify { name }`, `am::verify::{Verifier, VerifyRequest, load_contract,
spawn_verifier}`, and `card(.., git_state, verify)` signatures match across tasks. `amux_verify`
types (`Verdict{task_id,passed,gates}`, `GateResult{name,status,exit_code,duration_ms,stdout_tail,
stderr_tail,repro}`, `GateStatus`, `VerdictMsg`) match the crate exactly.

**Out of scope (per spec):** notes-as-checklist, verdict persistence, `config.toml` defaults,
test-adequacy gates, contract auto-scaffold, exploration mode, auto-trigger on idle.
