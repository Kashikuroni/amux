# Capture-pane fan-out reduction (activity-gating) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop forking `capture-pane` for every session every tick; capture only sessions whose tmux activity advanced, so idle ticks drop from `N+2` forks to ~1 and fork cost tracks *active* sessions, not total.

**Architecture:** Fold `#{session_activity}` (epoch seconds, free in the existing `list-sessions` fork) into `Session`. In `refresh()`, gate each `capture-pane` behind a pure `should_capture` rule (new session, activity advanced, or was `Running` last tick). Skipped sessions carry forward last tick's snapshot/prompt/status and preserve the preview. Mirrors the existing F3 git-gating (`should_read_git` / `git_last_enqueue`). The model stays stateless and self-healing — no long-lived process.

**Tech Stack:** Rust, tmux CLI (`-L cm` socket), existing `compute_status` / `content_hash` / `parse_prompt` helpers.

**Spec:** `docs/superpowers/specs/2026-06-09-capture-fanout-activity-gating-design.md`

---

## File Structure

- `src/tmux.rs` — add `activity: i64` to `Session`, extend `LIST_FORMAT`, parse it in `parse_line`. (Task 1)
- `src/app.rs` — `should_capture` + `status_when_idle` pure helpers (Tasks 2, 3); `last_activity` field + `#[cfg(test)] capture_count` + rewired `refresh()` capture loop (Task 4).
- Every `tmux::Session { … }` literal in `src/` (test fixtures) gains `activity: 0` (Task 1).
- `CHANGELOG.md` — `### Changed` note under `[Unreleased]` (Task 5).

---

## Task 1: `Session.activity` field + parse

**Files:**
- Modify: `src/tmux.rs:7-8` (`LIST_FORMAT`), `src/tmux.rs:20-37` (`Session`), `src/tmux.rs:45-79` (`parse_line`)
- Modify (mechanical, add `activity: 0,`): every `tmux::Session { … }` literal — authoritative list:
  `src/app.rs:3211, 3222, 3238, 3252, 5309, 5521, 5757`,
  `src/ui/mod.rs:414`,
  `src/ui/preview.rs:113, 139, 161, 186, 225, 262, 295, 327`,
  `src/ui/sessions.rs:544, 1072, 1115`,
  `src/ui/note.rs:145`
  (the compiler will name any missed site — see Step 4)
- Test: `src/tmux.rs` `mod tests`

- [ ] **Step 1: Write the failing test**

Add to `src/tmux.rs` `mod tests` (near the other `parse_sessions` tests, ~line 590):

```rust
#[test]
fn parses_session_activity() {
    // 9th tab-field is #{session_activity} (epoch secs). Older fixtures without
    // it default to 0.
    let out = "act\t/d\t1\t1\tclaude\t0\t\t/d\t1780955306";
    let s = &parse_sessions(out)[0];
    assert_eq!(s.activity, 1780955306);

    let no_activity = "old\t/d\t1\t1\tclaude\t0\t";
    assert_eq!(parse_sessions(no_activity)[0].activity, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib parses_session_activity`
Expected: FAIL — `no field 'activity' on type Session` (does not compile yet).

- [ ] **Step 3: Implement**

In `src/tmux.rs`, extend `LIST_FORMAT` (append one tab-field):

```rust
pub const LIST_FORMAT: &str =
    "#{session_name}\t#{session_path}\t#{session_created}\t#{@cm_managed}\t#{@cm_agent}\t#{session_attached}\t#{@cm_repo}\t#{pane_current_path}\t#{session_activity}";
```

Add the field to `Session` (after `worktree_repo`):

```rust
    /// Repo root if this session runs in a `cm`-created worktree; None otherwise.
    pub worktree_repo: Option<String>,
    /// `#{session_activity}` — epoch seconds of the session's last activity.
    /// Drives capture-gating: a pane is re-captured only when this advances.
    pub activity: i64,
```

In `parse_line`, widen the split and parse the new trailing field (it comes
*after* `cwd`, so parse `cwd` first, then `activity`):

```rust
fn parse_line(line: &str) -> Option<Session> {
    let mut f = line.splitn(9, '\t');
    let name = f.next()?.to_string();
    let dir = f.next()?.to_string();
    let created = f.next()?.trim().parse::<i64>().ok()?;
    let managed = f.next()?;
    let agent = f.next()?.to_string();
    if managed != "1" {
        return None;
    }
    let attached = f
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|n| n > 0)
        .unwrap_or(false);
    let worktree_repo = match f.next().map(str::trim) {
        Some(r) if !r.is_empty() => Some(r.to_string()),
        _ => None,
    };
    let cwd = match f.next().map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => dir.clone(),
    };
    let activity = f
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    Some(Session {
        name,
        dir,
        cwd,
        created,
        agent,
        status: Status::Idle,
        attached,
        git: None,
        worktree_repo,
        activity,
    })
}
```

Then add `activity: 0,` to every `tmux::Session { … }` test literal listed under **Files** above (each is a fixture; `0` is a fine inert default).

- [ ] **Step 4: Build to catch any missed literal, then run the test**

Run: `cargo build --tests 2>&1 | grep -A2 "missing.*activity" || echo OK`
Expected: `OK` (no missing-field errors). If any literal was missed, the
compiler names its `file:line` — add `activity: 0,` there and rebuild.

Run: `cargo test --lib parses_session_activity`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tmux.rs src/app.rs src/ui/mod.rs src/ui/preview.rs src/ui/sessions.rs src/ui/note.rs
git commit -m "feat(tmux): carry #{session_activity} on Session

Append session_activity (epoch secs) to LIST_FORMAT and parse it into a
new Session.activity field; old fixtures without the field default to 0.
Groundwork for capture-gating.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: `should_capture` pure helper

**Files:**
- Modify: `src/app.rs` (add free fn next to `compute_status`, ~line 2712)
- Test: `src/app.rs` `mod tests`

- [ ] **Step 1: Write the failing test**

Add to `src/app.rs` `mod tests`:

```rust
#[test]
fn should_capture_gates_on_activity_and_running() {
    use crate::tmux::Status;
    // New session (no last_seen): always capture (first observation).
    assert!(should_capture(100, None, None));
    // Activity advanced since last tick: capture.
    assert!(should_capture(101, Some(100), Some(&Status::Idle)));
    // Activity unchanged + was Idle: skip.
    assert!(!should_capture(100, Some(100), Some(&Status::Idle)));
    // Activity unchanged + was Running: conservative top-up — capture
    // (1s granularity can miss a sub-second burst).
    assert!(should_capture(100, Some(100), Some(&Status::Running)));
    // Activity unchanged + was Waiting: a blocked agent is quiet — skip.
    assert!(!should_capture(100, Some(100), Some(&Status::Waiting)));
    // Clock rollback (activity < last_seen) + Idle: not "advanced" — skip.
    assert!(!should_capture(99, Some(100), Some(&Status::Idle)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib should_capture_gates_on_activity_and_running`
Expected: FAIL — `cannot find function should_capture`.

- [ ] **Step 3: Implement**

Add next to `compute_status` in `src/app.rs`:

```rust
/// Whether to re-`capture-pane` a session this tick. Gate: capture only when
/// the pane may have changed — a new session, activity advanced since the last
/// tick, or it was `Running` last tick. The last clause is conservative:
/// `session_activity` has 1-second granularity, so a sub-second output burst
/// can leave the timestamp unchanged — but such a pane is always `Running`, so
/// it is always re-read. Idle/Waiting panes with unchanged activity are skipped.
pub fn should_capture(
    activity: i64,
    last_seen: Option<i64>,
    prev_status: Option<&crate::tmux::Status>,
) -> bool {
    match last_seen {
        None => true,                          // first observation
        Some(prev) if activity > prev => true, // output since last tick
        _ => prev_status == Some(&crate::tmux::Status::Running),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib should_capture_gates_on_activity_and_running`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): should_capture gate (activity + running top-up)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: `status_when_idle` pure helper

**Files:**
- Modify: `src/app.rs` (add free fn next to `should_capture`)
- Test: `src/app.rs` `mod tests`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn status_when_idle_maps_prompt_to_waiting() {
    use crate::tmux::Status;
    // Screen unchanged: a present prompt means the agent is still blocked.
    assert_eq!(status_when_idle(true), Status::Waiting);
    // No prompt on screen: idle.
    assert_eq!(status_when_idle(false), Status::Idle);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib status_when_idle_maps_prompt_to_waiting`
Expected: FAIL — `cannot find function status_when_idle`.

- [ ] **Step 3: Implement**

```rust
/// Status for a session whose capture was skipped this tick (its screen is
/// identical to the previous tick): `Waiting` if a prompt is still on screen,
/// else `Idle`. Reproduces what re-capturing unchanged content would yield —
/// `compute_status` returns `Idle` for an unchanged hash, and a present prompt
/// overlays `Waiting`.
pub fn status_when_idle(cached_prompt_present: bool) -> crate::tmux::Status {
    if cached_prompt_present {
        crate::tmux::Status::Waiting
    } else {
        crate::tmux::Status::Idle
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib status_when_idle_maps_prompt_to_waiting`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): status_when_idle maps skipped sessions

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Wire gating into `refresh()`

**Files:**
- Modify: `src/app.rs` — `App` struct (add `last_activity` near `git_last_enqueue` ~line 1008; add `#[cfg(test)] capture_count` near `preview_parse_count` ~line 940), `App::new` init (~lines 1008 and 1022), `refresh()` capture loop (~lines 2481-2503) and the prune site (~line 2554).
- Test: `src/app.rs` `mod tests` (real-tmux unit test, guarded by availability — same `isolate_socket` harness the integration tests use).

- [ ] **Step 1: Add the fields**

In the `App` struct, after `git_last_enqueue` (~line 1008's declaration — search for `git_last_enqueue:`):

```rust
    /// Last `#{session_activity}` seen per session (by name). Capture-gating
    /// reads this to skip panes that produced no output since the last tick.
    /// Pruned to live sessions each refresh, like `git_last_enqueue`.
    pub last_activity: HashMap<String, i64>,
```

Next to `preview_parse_count` (~line 940):

```rust
    /// Number of `capture-pane` forks issued by the last refresh cycle —
    /// test-only observability for capture-gating, so it adds nothing to
    /// release builds.
    #[cfg(test)]
    capture_count: std::cell::Cell<u64>,
```

In `App::new`'s `Self { … }`, next to `git_last_enqueue: HashMap::new(),`:

```rust
            last_activity: HashMap::new(),
```

and next to `preview_parse_count: std::cell::Cell::new(0),`:

```rust
            #[cfg(test)]
            capture_count: std::cell::Cell::new(0),
```

- [ ] **Step 2: Write the failing test**

Add to `src/app.rs` `mod tests`:

```rust
#[test]
fn refresh_skips_capture_for_quiet_sessions() {
    use crate::tmux::{self, Status};
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }
    // Throwaway socket, killed on drop — never touches the live `cm`.
    let _sock = tmux::isolate_socket(&format!("am_gate_{}", std::process::id()));
    let dir = std::env::temp_dir();
    let dir = dir.to_str().unwrap();
    let n1 = format!("am_gate_a_{}", std::process::id());
    let n2 = format!("am_gate_b_{}", std::process::id());
    struct Guard(Vec<String>);
    impl Drop for Guard {
        fn drop(&mut self) {
            for n in &self.0 {
                let _ = tmux::kill_session(n);
            }
        }
    }
    let _g = Guard(vec![n1.clone(), n2.clone()]);
    // Silent agents: `sleep` emits nothing, so activity never advances.
    tmux::new_session(&n1, dir, "sleep 60", "bash").expect("n1");
    tmux::new_session(&n2, dir, "sleep 60", "bash").expect("n2");
    // Let the initial pane draw settle so its activity is recorded by refresh 1.
    std::thread::sleep(std::time::Duration::from_millis(400));

    let mut app = App::new(crate::config::Config::default());

    // Refresh 1: both new → both captured.
    app.refresh();
    assert_eq!(app.capture_count.get(), 2, "first tick captures every session");
    assert_eq!(app.sessions.len(), 2, "both managed sessions listed");
    assert_eq!(app.snapshots.len(), 2, "snapshot recorded for each");

    // Refresh 2: both quiet (no output) and Idle → both skipped.
    let before = app.capture_count.get();
    app.refresh();
    assert_eq!(
        app.capture_count.get(),
        before,
        "quiet idle sessions must not be re-captured"
    );
    // Completeness invariant: skipped sessions stay in snapshots and stay Idle
    // (must not flip to Running for lack of a fresh snapshot).
    assert_eq!(app.snapshots.len(), 2, "skipped sessions kept in snapshots");
    for s in &app.sessions {
        assert_eq!(s.status, Status::Idle, "quiet session stays Idle");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib refresh_skips_capture_for_quiet_sessions -- --nocapture`
Expected: FAIL — second refresh still captures (count grows to 4), because
`refresh()` does not gate yet. (If tmux is unavailable the test no-ops; run it
where tmux exists.)

- [ ] **Step 4: Rewire the capture loop**

In `refresh()`, replace the existing capture loop (currently):

```rust
                let mut new_snaps = HashMap::new();
                let mut new_prompts = HashMap::new();
                let mut new_preview = None;
                for s in &mut sessions {
                    if let Ok(content) = crate::tmux::capture_pane(&s.name) {
                        let h = content_hash(&content);
                        s.status = compute_status(self.snapshots.get(&s.name).copied(), h);
                        new_snaps.insert(s.name.clone(), h);
                        let opts = parse_prompt(&content);
                        if !opts.is_empty() {
                            s.status = Status::Waiting;
                            new_prompts.insert(s.name.clone(), opts);
                        }
                        if selected_name.as_deref() == Some(s.name.as_str()) {
                            new_preview = Some(
                                crate::tmux::capture_scrollback(&s.name, 500).unwrap_or(content),
                            );
                        }
                    }
                }
```

with:

```rust
                // Prev status by name (last tick's view), snapshotted before the
                // loop so we don't borrow `self.sessions` while mutating maps.
                let prev_status: HashMap<String, Status> = self
                    .sessions
                    .iter()
                    .map(|s| (s.name.clone(), s.status.clone()))
                    .collect();
                let mut new_snaps = HashMap::new();
                let mut new_prompts = HashMap::new();
                let mut new_preview = None;
                for s in &mut sessions {
                    let last_seen = self.last_activity.get(&s.name).copied();
                    let gate = should_capture(s.activity, last_seen, prev_status.get(&s.name));
                    // Only fork `capture-pane` when the pane may have changed.
                    let captured = if gate {
                        #[cfg(test)]
                        self.capture_count.set(self.capture_count.get() + 1);
                        crate::tmux::capture_pane(&s.name).ok()
                    } else {
                        None
                    };
                    match captured {
                        Some(content) => {
                            let h = content_hash(&content);
                            s.status = compute_status(self.snapshots.get(&s.name).copied(), h);
                            new_snaps.insert(s.name.clone(), h);
                            let opts = parse_prompt(&content);
                            if !opts.is_empty() {
                                s.status = Status::Waiting;
                                new_prompts.insert(s.name.clone(), opts);
                            }
                            self.last_activity.insert(s.name.clone(), s.activity);
                            if selected_name.as_deref() == Some(s.name.as_str()) {
                                new_preview = Some(
                                    crate::tmux::capture_scrollback(&s.name, 500)
                                        .unwrap_or(content),
                                );
                            }
                        }
                        // Skipped (gated) OR capture failed: carry forward last
                        // tick's snapshot + prompt so `new_snaps` stays complete
                        // (an entry for every live session — else the next tick
                        // falsely sees a change), and keep the selected session's
                        // preview instead of blanking it.
                        None => {
                            if let Some(h) = self.snapshots.get(&s.name).copied() {
                                new_snaps.insert(s.name.clone(), h);
                            }
                            if let Some(opts) = self.prompts.get(&s.name).cloned() {
                                new_prompts.insert(s.name.clone(), opts);
                            }
                            s.status = status_when_idle(new_prompts.contains_key(&s.name));
                            if selected_name.as_deref() == Some(s.name.as_str()) {
                                new_preview = Some(self.preview.clone());
                            }
                        }
                    }
                }
```

Then add the `last_activity` prune. Find `self.snapshots = new_snaps;`
(~line 2554) and insert immediately after it:

```rust
                self.snapshots = new_snaps;
                // Prune activity bookkeeping for sessions that are gone (same
                // pattern as the git-cache prune above).
                let live_names: std::collections::HashSet<&str> =
                    sessions.iter().map(|s| s.name.as_str()).collect();
                self.last_activity
                    .retain(|k, _| live_names.contains(k.as_str()));
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib refresh_skips_capture_for_quiet_sessions -- --nocapture`
Expected: PASS — first refresh captures 2, second captures 0, snapshots stay
at 2, statuses stay `Idle`.

- [ ] **Step 6: Full regression run**

Run: `cargo test --lib && cargo test --test tmux_integration`
Expected: all PASS (no fixture or status regressions).

Run: `cargo clippy --all-targets 2>&1 | tail -5`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "perf(app): gate capture-pane behind session activity (F-fanout)

refresh() now captures a pane only when should_capture fires (new session,
activity advanced, or was Running last tick); quiet idle/waiting sessions
carry forward their snapshot, prompt, status, and (when selected) preview.
Idle ticks drop from N+2 forks to ~1; cost tracks active sessions, not
total. last_activity is pruned to live sessions each refresh.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: CHANGELOG + final verification

**Files:**
- Modify: `CHANGELOG.md` (`### Changed` under `[Unreleased]`)

- [ ] **Step 1: Add the changelog entry**

Under `## [Unreleased]` → `### Changed`, append a bullet:

```markdown
- Performance: the session poll no longer forks `tmux capture-pane` for every
  session every tick. A session's pane is re-captured only when its tmux
  activity advanced since the last tick (or it was running) — so an all-idle
  dashboard does one `list-sessions` fork per tick instead of one per session,
  and the cost scales with active sessions rather than total.
```

- [ ] **Step 2: Final full verification**

Run: `cargo test && cargo clippy --all-targets 2>&1 | tail -5`
Expected: all tests PASS, no clippy warnings.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: changelog for capture-pane activity-gating

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Manual smoke (USER, optional)

Build and run against the live `cm` server with a few real sessions:

```bash
cargo build --release
./target/release/amux
```

- With all agents idle, confirm the dashboard still updates statuses correctly
  (start an agent, watch it flip to running) and the preview of a *quiet*
  selected session does not blank out or flicker.
- Optionally measure fork rate (e.g. `sudo dtruss`/`fs_usage` on the amux pid,
  or `tmux -L cm` server CPU) before/after to confirm the idle drop.

---

## Self-Review (completed during planning)

- **Spec coverage:** signal (`#{session_activity}`) → Task 1; capture rule
  (3 clauses incl. Running top-up) → Task 2; idle status mapping → Task 3;
  `last_activity` field + refresh integration + completeness invariant +
  preview preservation + prune → Task 4; parse test, should_capture tests,
  status_when_idle tests, the completeness/skip integration test → Tasks 1-4;
  changelog → Task 5. Future control-mode upgrade + optional batching are
  spec'd as out-of-scope and have no tasks (correct).
- **Placeholders:** none — every code/step is concrete.
- **Type consistency:** `should_capture(i64, Option<i64>, Option<&Status>) -> bool`
  and `status_when_idle(bool) -> Status` are used identically in Task 4's loop;
  `Session.activity: i64` set in Task 1 is read in Task 4; `last_activity`,
  `capture_count` names match across struct/init/usage.
