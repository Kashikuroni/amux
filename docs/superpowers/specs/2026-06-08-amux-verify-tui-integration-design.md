# Design: amux-verify TUI integration (verification MVP items 3–6)

**Date:** 2026-06-08
**Status:** Approved (brainstorming) — ready for plan
**Scope:** `amux` root crate (the TUI). Consumes the existing `amux-verify` crate; no change to that crate.
**Feature doc:** `docs/verification_feature.md` §5.2, §6 (this implements its in-app integration).

## Problem

The verifier (`amux-verify`) exists as a standalone crate + CLI (MVP items 1–2): it runs a
`.amux/verify.toml` contract's gates in a worktree and emits a `Verdict`. But there is no way
to run or see verification **from the amux TUI**. This milestone wires it in: press a key on a
session, run the contract in the background, and show progress + a pass/fail verdict on the
session card, with a detail view for failures.

This covers feature-doc §8 items 3–6 (trigger+key, background runner, state + event application,
status/badge render) plus the failure **detail panel** (§5.2) — the user opted to include it now.

## Goals

- A key (`v`) verifies the selected session in the background without blocking the UI; pressing
  `v` again while it runs cancels it.
- The session card shows verification status in its **status slot** (replacing the agent status
  while a verification is live or its verdict is current): `◐ <gate> 2/3` while running,
  `✓ verified` / `✗ failed: <gate>` when done.
- A completed verdict is **not** clobbered by the constant agent-status refresh, and is cleared
  only when the agent has genuinely resumed work (Running ≥ 30 s) — short "flaky" Running flips
  (e.g. Claude Code briefly flipping idle→running) never discard a verdict the user is reading.
- A detail modal (`V`) shows per-gate results and, for failures, the repro command + output tail.

## Non-goals (deferred per feature-doc "дальше наслаивается")

- Notes-as-checklist mapping, verdict persistence to `state.toml`/`.amux/verdicts/`, `config.toml`
  default gates/timeouts, test-adequacy gates, contract auto-scaffold, exploration mode.
- Auto-triggering verification on idle (manual `v` only — `idle` is an unreliable "done" signal).
- Parallel verification of multiple sessions (the worker is serial — cargo builds are heavy).

## Architecture

Five pieces, following amux's established patterns (pure `App` + `Action`/`handle_action`
effects + background worker thread feeding a channel, exactly like `git::spawn_reader` and the
Claude-restart pipeline):

1. **Dependency** — root crate depends on `amux-verify` (path).
2. **`verify.rs`** (new) — a persistent serial background worker; `spawn_verifier()`.
3. **`app.rs`** — `VerificationState` data per session; `Action::Verify`/`CancelVerify`; the `v`/`V`
   keys; event application; the clear-on-resume rule.
4. **`main.rs` / `handle_action`** — execute `Verify`/`CancelVerify`; (draining happens in `refresh`).
5. **`ui/sessions.rs` + `ui/` modal + `theme.rs`** — status-slot badge + the `V` detail modal.

### 1. Dependency

Root `Cargo.toml` `[dependencies]` gains:

```toml
amux-verify = { path = "crates/amux-verify" }
```

Used as `use amux_verify::{find_contract, Contract, RunOptions, Verdict, VerdictMsg, GateStatus, GateResult};`
(the lib crate name is `amux_verify`). `run` is called only inside the worker thread.

### 2. `verify.rs` — serial background worker

Mirrors `git::spawn_reader`: one thread, a request channel in, a tagged result channel out.

```rust
pub struct VerifyRequest {
    pub name: String,               // session name (tags result events)
    pub dir: std::path::PathBuf,    // worktree dir to verify in (= Session.dir)
    pub contract: amux_verify::Contract,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

pub struct Verifier {
    pub tx: std::sync::mpsc::Sender<VerifyRequest>,
    pub rx: std::sync::mpsc::Receiver<(String, amux_verify::VerdictMsg)>,
}

pub fn spawn_verifier() -> Verifier { /* thread: recv → run → forward events */ }
```

The worker loop:
- `recv()` a request (blocks; exits when the sender drops).
- If `cancel` is already set (cancelled before it started), skip it (do not run).
- Else call `amux_verify::run(&dir, &contract, &RunOptions::default-with-task_id(name),
  &cancel, &mut |msg| { let _ = res_tx.send((name.clone(), msg)); })`. `run` is blocking and
  emits `Started → GateStarted/GateFinished… → Finished`; each is forwarded tagged with `name`.
- Requests are processed **one at a time** (a second `v` while one runs queues behind it).

`RunOptions`: `default_timeout_s = 300`, `task_id = Some(session_name)`, `stream_output = false`.

### 3. `app.rs` — state, keys, events

**Per-session state** (pure data on `App`, NOT a new `Session` field — `Session` is rebuilt from
tmux each refresh and has 22 literals; keep verification keyed by session name on `App`):

```rust
pub enum VerificationState {
    Running { total: usize, done: usize, current: String },
    Done(amux_verify::Verdict),
}
// on App:
pub verification: HashMap<String, VerificationState>,
pub verify_cancel: HashMap<String, Arc<AtomicBool>>, // in-flight cancel flags, keyed by name
pub running_since: HashMap<String, i64>,             // unix-secs a session has been Running
pub verify_worker: Option<crate::verify::Verifier>,
```

`verify_cancel` is kept separate so `VerificationState` stays pure data. `attach_verifier()`
(called from `main`, beside `attach_git_worker()`) sets `verify_worker`.

**Keys** (`handle_list_key`):
- `v` (plain) → toggle: if the selected session has an active `Running` verification →
  `Action::CancelVerify { name }`; otherwise `Action::Verify { name }`.
- `V` (shift) → if the selected session has a `Done` verdict → open `Mode::VerifyDetail(name)`;
  else no-op.

(`v`/`V` are currently unbound. `v` is plain — it is non-destructive: it spawns read-only checks
and is itself cancellable — matching the feature doc's "клавиша v".)

**Event application** — a pure method `App::apply_verify_event(&mut self, name: &str, msg: VerdictMsg)`.
Every non-`Started` event is applied **only if `verification[name]` is currently `Running`** — if
it is absent (the run was cancelled, which clears it; see §4) the trailing events are ignored, so a
cancelled run never revives into a `Done` verdict:
- `Started { total_gates }` → `verification[name] = Running { total: total_gates, done: 0, current: String::new() }`.
- `GateStarted { name: gate, .. }` → if `Running`, set `current = gate`.
- `GateFinished { index, .. }` → if `Running`, set `done = index + 1`.
- `Finished { verdict }` → if `Running`, `verification[name] = Done(verdict)` and remove `verify_cancel[name]`; if absent, ignore.

**Draining + clear-on-resume** — in `App::refresh` (where the git worker is already drained):
- Drain `verify_worker.rx` into a local `Vec<(String, VerdictMsg)>` (immutable borrow of the
  worker field), then apply each via `apply_verify_event` (collect-then-mutate avoids the borrow
  conflict, same shape as the git-cache drain).
- Clear-on-resume, per session, using the raw tmux-derived `s.status`:
  - if `s.status == Running`: `running_since.entry(name).or_insert(self.now_unix)`; then if
    `self.now_unix - running_since[name] >= 30` and `verification[name]` is `Done` →
    remove `verification[name]` (and `running_since[name]`). 30 s of *sustained* Running is the
    "agent genuinely resumed" signal; brief flaky flips never reach it.
  - else (`Idle`/`Waiting`): `running_since.remove(name)`.
- GC: drop `verification`/`verify_cancel`/`running_since` entries for sessions that no longer
  exist (mirrors how stale state is pruned elsewhere).

### 4. `main.rs` / `handle_action`

- `Action::Verify { name }`: look up the session; `find_contract(&session.dir)` →
  `Contract::load`. On `None`/error → `app.error = Some("no .amux/verify.toml" | <load error>)`.
  On success: create `cancel = Arc::new(AtomicBool::new(false))`; insert into `verify_cancel`;
  set `verification[name] = Running { total: contract.gates.len(), done: 0, current: "" }`
  (so the badge shows immediately, before the first event); `verify_worker.tx.send(VerifyRequest{…})`.
- `Action::CancelVerify { name }`: `verify_cancel[name].store(true, SeqCst)` (the running gate is
  group-killed by the crate), then **immediately** `verification.remove(name)` and
  `verify_cancel.remove(name)` so the status slot reverts to the agent status at once. The worker
  still runs `run` to completion (skipped gates) and emits a trailing `Finished`, but with no
  `Running` entry that event is ignored (§3) — the cancelled run leaves no verdict. The user re-runs
  with `v`. (The worker's own `Arc<AtomicBool>` clone keeps the flag set, so removing the map entry
  does not un-cancel it.)
- Draining is in `refresh` (above), so no per-tick work is needed in the action handler.

### 5. Render — status slot, badge, detail modal

**Status slot** (`ui/sessions.rs`, line 1 — currently `status_glyph + status_label`). Compute a
verification override and, when present, render it *instead of* the agent status:
- `verification[name] == Running { current, done, total }` → `◐ {current} {done}/{total}`
  (spinner glyph from `spinner::glyph`, color = an "in progress" accent, e.g. blue/amber).
- `verification[name] == Done(v)` where `v.passed` → `✓ verified` (green).
- `Done(v)` where `!v.passed` → `✗ failed: {first non-passed gate name}` (red).
- otherwise → the existing agent status (`running`/`idle`/`waiting`), unchanged.

Right-edge alignment reuses the existing line-1 padding math (the verification label replaces
`status_label`; its display width is computed the same way). Colors live in `theme.rs`
(`VERIFY_OK` green, `VERIFY_FAIL` red, `VERIFY_RUN` accent), consistent with the repo's restrained
semantic-accent convention.

**Detail modal** (`Mode::VerifyDetail(String)` + a new `ui/modal_verify.rs`, modeled on
`ui/modal_help.rs`/`modal_git.rs`): a centered box titled `verify · <session>` listing each gate:
`✓/✗/⊘/⏱ <name>  (<duration>)` (passed/failed/skipped/timed-out), and below, for each non-passed
gate, its `repro` command and the tail (`stderr_tail`, then `stdout_tail` if non-empty), wrapped
to the box width. `Esc`/`q` closes (handled in the existing modal key dispatch). Footer gains
`("esc", "close", …)` for the mode.

## Error handling

- No contract / invalid contract on `v` → `app.error` (the existing error banner), no state change.
- Worker thread death (panic in `run`) → the result channel closes; the drain loop simply stops
  receiving for that session; the session keeps its last `Running` state. (Acceptable for MVP; the
  user can cancel/re-run. `run` is well-tested and infallible by contract.)
- `V` on a session without a `Done` verdict → no-op (nothing to show).
- Cancel of a queued-but-not-started request → `CancelVerify` clears `verification[name]`
  immediately (slot reverts at once); when the worker later dequeues that request it sees `cancel`
  set and skips running it (emitting nothing), so no events arrive.

## Testing

Pure `App` unit tests (no tmux/process):
- `apply_verify_event` sequence: `Started{3} → GateStarted{0,"build"} → GateFinished{0,…} →
  … → Finished{verdict}` drives `Running{done,total,current}` then `Done(verdict)`.
- `v` toggles: no state → `Verify`; `Running` present → `CancelVerify`; `Done` present → `Verify` (re-run).
- Cancel semantics: after `verification[name]` is removed, a trailing `Finished{verdict}` event is
  ignored (no `Done` revival).
- `V`: `Done` present → enters `Mode::VerifyDetail`; absent → no-op.
- Clear-on-resume: a `Done` verdict + `s.status == Running` for `< 30 s` is **kept**; at `≥ 30 s`
  it is cleared; a brief Running flip (set then cleared before 30 s) keeps the verdict.
- GC drops state for vanished sessions.

`verify.rs` worker test: `spawn_verifier()`, send a `VerifyRequest` for a temp dir containing a
trivial one-gate contract (`cmd = "true"`), assert the tagged event stream ends in
`Finished { verdict.passed == true }`; a cancelled request (cancel preset) yields no run.

Render tests (`ui/sessions.rs`): each status-slot state (`Running`/`verified`/`failed`/none)
produces the expected text; the `failed` badge names the failed gate. Modal test
(`ui/modal_verify.rs`): a failed verdict renders the gate list + repro + stderr tail.

## File structure

- `Cargo.toml` — add the path dependency.
- `src/verify.rs` (new) — `Verifier`, `VerifyRequest`, `spawn_verifier`.
- `src/app.rs` — `VerificationState`, the three maps + worker field, `attach_verifier`,
  `apply_verify_event`, `v`/`V` keys, `Action::Verify`/`CancelVerify`, refresh drain + clear-on-resume,
  `Mode::VerifyDetail`.
- `src/main.rs` — `attach_verifier()` call; `Verify`/`CancelVerify` action arms.
- `src/ui/sessions.rs` — status-slot verification override.
- `src/ui/modal_verify.rs` (new) + `ui/mod.rs` wiring + `ui/footer.rs` entry — the detail modal.
- `src/theme.rs` — verification colors.
- `src/lib.rs` — `pub mod verify;`.
