# Performance Part B — Refresh Fork Gating Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop `refresh` from re-forking `git` for every session every 1.5 s, and make it redraw only when visible state actually changed — cutting the fork/context-switch stream that dominates under many sessions.

**Architecture:** Implements finding F3 of `docs/superpowers/specs/2026-06-08-performance-design.md`. `App::refresh()` (Part A returns a `true` stub) becomes a real change-detector: it diffs the new session list / prompts / preview / clock / selected-age against the previous tick and returns whether a redraw is needed. Git work is gated — a session's dir is re-read only when its pane content hash changed or it hasn't been read within a coarse interval — and the git cache moves from *replace* to *merge + prune* so gating doesn't drop git info for un-requested dirs. App stays IO-free in spirit (refresh is the existing effect site). The event loop already does `if app.refresh() { needs_redraw = true }`, so no loop change is needed except a redraw guard while a Claude restart is pending.

**Tech Stack:** Rust, tmux/git via `std::process::Command`, background git reader thread (`src/git.rs`).

**Spec reference:** §Findings F3 and §"Part B" of the spec. Acceptance: idle/unchanged sessions enqueue **0 new git reads**; a changed pane re-reads; git info for unchanged sessions stays correct; existing tests green; measured git/tmux fork rate drops under a multi-session idle load.

**Field measurement (USER):** the fork stream is best seen with the children watch from the perf spec — `top -l 8 -s 5 -o cpu -stats pid,command,cpu | grep -E 'git|tmux|amux'` — watching whether `git`/`tmux` flicker every ~1.5 s. Capture before/after with several idle sessions open.

---

## File Map

| File | Change |
|---|---|
| `src/app.rs` | Add `git_last_enqueue` field + init; add `should_read_git` / `age_label_changed` free fns + `GIT_REFRESH_SECS` const (+tests); rewrite `refresh()` git block (gate + merge + prune) and change-detection (return real `bool`); update the Part A refresh test |
| `src/main.rs` | One redraw guard: redraw while a Claude restart is pending (so the `restarting` block's effects aren't hidden when `refresh()` returns false) |

---

## Task B1: app.rs — bookkeeping field + pure helpers (TDD)

**Files:** Modify `src/app.rs`; tests in `src/app.rs` `#[cfg(test)] mod tests`.

- [ ] **Step 1: Add the `git_last_enqueue` field to `App`**

In `struct App { … }`, next to `git_cache`:

```rust
    /// Per-dir (`cwd`) Unix time of the last git re-read request, so idle
    /// sessions aren't re-forked every tick (F3). Pruned to live sessions.
    git_last_enqueue: HashMap<String, i64>,
```

- [ ] **Step 2: Initialize it in `App::new`** (next to `git_cache: HashMap::new(),`):

```rust
            git_last_enqueue: HashMap::new(),
```

- [ ] **Step 3: Write failing tests** (add to `src/app.rs` `mod tests`):

```rust
#[test]
fn should_read_git_when_pane_changed() {
    // Pane content changed → always re-read, regardless of when last read.
    assert!(should_read_git(true, Some(1000), 1000, 5));
}

#[test]
fn should_read_git_first_time() {
    // Never enqueued (no timestamp) → read.
    assert!(should_read_git(false, None, 1000, 5));
}

#[test]
fn should_read_git_skips_recent_unchanged() {
    // Unchanged pane, read 2s ago, interval 5s → skip.
    assert!(!should_read_git(false, Some(1000), 1002, 5));
}

#[test]
fn should_read_git_refreshes_when_stale() {
    // Unchanged pane, but last read 5s ago (== interval) → coarse refresh.
    assert!(should_read_git(false, Some(1000), 1005, 5));
}

#[test]
fn age_label_changes_across_minute_boundary() {
    // created at t=0: 59s ("59s") → 61s ("1m") is a label change.
    assert!(age_label_changed(0, 59, 61));
}

#[test]
fn age_label_ticks_every_second_under_a_minute() {
    // Under a minute the label is second-granular, so it changes each tick.
    assert!(age_label_changed(0, 10, 11));
}

#[test]
fn age_label_stable_within_a_minute_bucket() {
    // 1m40s ("1m") → 1m41s ("1m") is NOT a label change.
    assert!(!age_label_changed(0, 100, 101));
}
```

- [ ] **Step 4: Run tests to verify they FAIL**

Run: `cargo test --lib should_read_git age_label`
Expected: FAIL — `cannot find function should_read_git` / `age_label_changed`.

- [ ] **Step 5: Implement the const + helpers**

Add near the other free functions at the bottom of `src/app.rs` (e.g. next to `compute_status`):

```rust
/// Coarse git re-read interval: even an unchanged pane gets its git re-read at
/// least this often, so changes that don't alter the pane (e.g. an external
/// commit) still surface. Pane-content changes re-read immediately.
pub const GIT_REFRESH_SECS: i64 = 5;

/// Whether to re-read git for a dir this tick: when its pane content changed,
/// or it hasn't been read within `interval_secs` (covers the first read and
/// periodic freshness). Pure — the refresh loop supplies the inputs.
pub fn should_read_git(
    pane_changed: bool,
    last_enqueue: Option<i64>,
    now: i64,
    interval_secs: i64,
) -> bool {
    pane_changed || last_enqueue.map_or(true, |t| now - t >= interval_secs)
}

/// Whether a session's humanized age label differs between two instants — used
/// so the preview header's age stays live without a per-frame redraw. Mirrors
/// `timeutil::humanize_age`'s buckets (seconds under a minute, then minutes…).
pub fn age_label_changed(created: i64, prev_now: i64, now: i64) -> bool {
    crate::timeutil::humanize_age(prev_now - created)
        != crate::timeutil::humanize_age(now - created)
}
```

- [ ] **Step 6: Run tests to verify they PASS**

Run: `cargo test --lib should_read_git age_label`
Expected: PASS (7 passed).

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "perf(app): git-gating bookkeeping + should_read_git/age_label_changed helpers (F3)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task B2: app.rs — gate git + real change detection in refresh()

**Files:** Modify `src/app.rs` (`refresh`, ~lines 2253–2342); update one Part A test; Modify `src/main.rs` (restart redraw guard).

- [ ] **Step 1: Update the Part A refresh test to the new semantics**

In `src/app.rs` `mod tests`, REPLACE the existing `refresh_signals_redraw` test with these two (the stub-always-true contract is gone):

```rust
#[test]
fn refresh_no_change_returns_false() {
    // A refresh that finds nothing new must not request a redraw (F3). Needs a
    // tmux binary to return an (empty) session list; skip if absent.
    if !crate::tmux::is_available() {
        return;
    }
    let mut app = App::new(Config::default());
    let _ = app.refresh(); // establish baseline (now_unix / clock / snapshots)
    assert!(!app.refresh(), "a no-op refresh must not request a redraw");
}

#[test]
fn refresh_redraws_on_clock_minute_change() {
    // Forcing a stale clock-minute makes refresh update the header clock, which
    // is a visible change → redraw. Works regardless of session count, and even
    // if tmux is absent (the Err path also returns true).
    let mut app = App::new(Config::default());
    app.last_clock_minute = 0;
    assert!(app.refresh(), "a clock-minute tick must request a redraw");
}
```

- [ ] **Step 2: Run them to verify the no-op test FAILS**

Run: `cargo test --lib refresh_no_change_returns_false refresh_redraws_on_clock_minute_change`
Expected: `refresh_no_change_returns_false` FAILS (refresh still returns the Part A `true` stub); `refresh_redraws_on_clock_minute_change` passes.

- [ ] **Step 3: Rewrite `refresh()`**

Replace the entire `refresh` method body. The current body is the `match crate::tmux::list_sessions() { Ok(mut sessions) => { … } Err(e) => … }` followed by a trailing `true`. Replace the whole method with:

```rust
    pub fn refresh(&mut self) -> bool {
        match crate::tmux::list_sessions() {
            Ok(mut sessions) => {
                let mut changed = false;
                let selected_name = self.selected_name();
                // Size the previewed window to the preview area before capturing,
                // so its scrollback reflows to the preview width (no doubled box).
                if let Some(name) = &selected_name {
                    self.fit_preview_window(name);
                }
                let prev_now = self.now_unix;
                self.now_unix = crate::timeutil::now_unix();
                // Re-fork `date` only when the minute actually changes; the new
                // clock string is a visible change.
                let minute = self.now_unix / 60;
                if minute != self.last_clock_minute {
                    self.clock = crate::timeutil::clock_hhmm();
                    self.last_clock_minute = minute;
                    changed = true;
                }
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
                            // A pending numbered prompt means the agent is blocked
                            // on the user; this overrides the pane-diff status.
                            s.status = Status::Waiting;
                            new_prompts.insert(s.name.clone(), opts);
                        }
                        if selected_name.as_deref() == Some(s.name.as_str()) {
                            // Preview keeps scrollback so it can be paged back.
                            new_preview = Some(
                                crate::tmux::capture_scrollback(&s.name, 500).unwrap_or(content),
                            );
                        }
                    }
                }
                // Git info: served from the background reader's cache when a
                // worker is attached (never blocks the UI), else read inline.
                // Read from `cwd` (the active pane's live path), not `dir`.
                if self.git_worker.is_some() {
                    // Apply newest results — MERGE (keep dirs we didn't re-request
                    // this tick), so gating below can't drop a session's git info.
                    let mut latest = None;
                    if let Some(w) = &self.git_worker {
                        while let Ok(map) = w.rx.try_recv() {
                            latest = Some(map);
                        }
                    }
                    if let Some(map) = latest {
                        for (dir, info) in map {
                            self.git_cache.insert(dir, info);
                        }
                    }
                    for s in &mut sessions {
                        s.git = self.git_cache.get(&s.cwd).cloned();
                    }
                    // Gate which dirs to re-read: pane changed, or not read within
                    // GIT_REFRESH_SECS (covers first read + periodic freshness).
                    let mut dirs: Vec<String> = Vec::new();
                    for s in &sessions {
                        let pane_changed =
                            new_snaps.get(&s.name) != self.snapshots.get(&s.name);
                        let last = self.git_last_enqueue.get(&s.cwd).copied();
                        if should_read_git(pane_changed, last, self.now_unix, GIT_REFRESH_SECS) {
                            self.git_last_enqueue.insert(s.cwd.clone(), self.now_unix);
                            dirs.push(s.cwd.clone());
                        }
                    }
                    dirs.sort();
                    dirs.dedup();
                    if !dirs.is_empty() {
                        if let Some(w) = &self.git_worker {
                            let _ = w.tx.send(dirs);
                        }
                    }
                    // Prune bookkeeping for sessions that are gone.
                    let live: std::collections::HashSet<&str> =
                        sessions.iter().map(|s| s.cwd.as_str()).collect();
                    self.git_cache.retain(|k, _| live.contains(k.as_str()));
                    self.git_last_enqueue.retain(|k, _| live.contains(k.as_str()));
                } else {
                    for s in &mut sessions {
                        s.git = crate::git::read(&s.cwd);
                    }
                }
                self.snapshots = new_snaps;
                let prompts_changed = self.prompts != new_prompts;
                self.prompts = new_prompts;
                let new_sessions =
                    apply_grouped_order(&self.project_order, &self.order, sessions);
                let sessions_changed = self.sessions != new_sessions;
                self.sessions = new_sessions;
                // A draft lives exactly as long as its session.
                self.prune_dead_drafts();
                self.clamp_selection();
                // Preview: Some(new) replaces; None clears (selection moved away).
                let new_preview_val = new_preview.unwrap_or_default();
                let preview_changed = new_preview_val != self.preview;
                self.preview = new_preview_val;
                changed = changed || sessions_changed || prompts_changed || preview_changed;
                // The preview header shows the selected session's humanized age;
                // redraw when that label would tick even if nothing else changed.
                if let Some(s) = self.selected_session() {
                    if age_label_changed(s.created, prev_now, self.now_unix) {
                        changed = true;
                    }
                }
                changed
            }
            Err(e) => {
                self.error = Some(e.to_string());
                true
            }
        }
    }
```

- [ ] **Step 4: Run the refresh tests + full suite**

Run: `cargo test --lib refresh_no_change_returns_false refresh_redraws_on_clock_minute_change`
Expected: both PASS.
Run: `cargo test`
Expected: all pass (the change is internal; existing tests that drive `handle_key` and rendering are unaffected; `prune_dead_drafts` still runs).

- [ ] **Step 5: Add the restart redraw guard in `src/main.rs`**

With `refresh()` now able to return `false`, the `restarting` resume block (which can set `app.error` or respawn panes) might run on a tick where `refresh()` reported no change — leaving its effects unpainted. Guard it: in `run()`, the block already sits under `if !app.tmux_missing && last_refresh.elapsed() >= refresh { if app.refresh() { needs_redraw = true } last_refresh = …; if !app.restarting.is_empty() { … } }`.

Change the `if !app.restarting.is_empty() {` line so entering the restart flow always forces a redraw. Replace:

```rust
            if !app.restarting.is_empty() {
```
with:
```rust
            if !app.restarting.is_empty() {
                // A pending restart mutates session state / errors as panes die
                // and respawn; always repaint while it's in flight.
                needs_redraw = true;
```

(Leave the rest of the block unchanged; this only adds the `needs_redraw = true;` as the first statement inside it.)

- [ ] **Step 6: Build, test, clippy**

Run: `cargo build` → clean.
Run: `cargo test` → all pass.
Run: `cargo clippy --all-targets` → no new warnings.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "perf(app): gate git re-reads + real change detection in refresh (F3)

refresh() now re-reads git only for sessions whose pane changed or that are
stale past GIT_REFRESH_SECS, merges (not replaces) the git cache so gating
can't drop info, prunes dead entries, and returns whether visible state
actually changed (sessions/prompts/preview/clock/selected-age). Idle
sessions stop re-forking git every tick. Loop repaints while a restart is
pending.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task B3: Verify & measure

- [ ] **Step 1: Full verification**

Run: `cargo test` → all green (report count).
Run: `cargo clippy --all-targets` → clean.
Run: `cargo build --release` → clean.

- [ ] **Step 2: Manual smoke (USER / interactive — do not automate)**

With several sessions open (mix of idle and one actively producing output):
- Idle sessions: git branch/diff in the list stays correct and does NOT flicker;
  the screen does not churn.
- Edit a file in one session's repo (or let an agent commit): within ~5 s its
  diff/branch updates (coarse refresh) — and immediately if its pane changes.
- Selected session's preview header age still ticks (seconds for a fresh
  session, minutes for older), and the header clock still advances.
- Restart-all-Claude (`u`) still shows progress and resumes.

- [ ] **Step 3: Measure git/tmux fork rate (USER)**

With ~5+ idle sessions, watch the children:
```sh
top -l 12 -s 5 -o cpu -stats pid,command,cpu | grep -E 'git|tmux|amux'
```
Expected after F3: `git` processes no longer appear every ~1.5 s for every
session — only on the ~5 s coarse sweep or when a pane changes. Compare against
the pre-B build (`62220ef`-based "old", or the Part-A HEAD) over the same
session set. Record:
- git forks/interval before (Part A) : `____`
- git forks/interval after (Part B)  : `____`
- idle %CPU of amux + tmux server     : `____` → `____`

- [ ] **Step 4: Record results + commit**

```bash
git add docs/superpowers/plans/2026-06-08-perf-part-b-refresh-gating.md
git commit -m "docs(plan): record Part B before/after fork measurements

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review notes

- **F3 coverage:** gating → `should_read_git` (B1) wired in `refresh` (B2);
  cache correctness → merge + prune (B2); change detection → `refresh -> bool`
  (B2) with clock/age handled so the UI doesn't freeze.
- **Why merge, not replace:** the git worker returns a map for *only the dirs it
  was sent*; with gating we send a subset, so `self.git_cache = map` (the Part A
  code) would erase git info for every un-requested session. Merge + prune keeps
  it correct and bounded.
- **Why the age/clock branches:** the preview header renders
  `humanize_age(now - created)` (second-granular under a minute) and the header
  renders `app.clock` (minute-granular). Without including these in `changed`,
  a no-op gate would freeze the age counter / clock. `age_label_changed` and the
  clock-minute check keep them live at the refresh cadence (~1.5 s) without the
  old fixed-rate redraw.
- **Spinner unaffected:** non-selected `Running` sessions animate via the loop's
  spinner logic (Part A), not `refresh`; `refresh` still flips their status
  (caught by `sessions_changed`).
- **No new must-use lint:** `refresh()`'s `bool` is consumed in `run()` and
  ignored at `handle_action` call-sites (plain `bool`, fine).
- **Type consistency:** `should_read_git(pane_changed, last_enqueue: Option<i64>,
  now: i64, interval_secs: i64) -> bool`, `age_label_changed(created, prev_now,
  now) -> bool`, `GIT_REFRESH_SECS: i64`, field `git_last_enqueue: HashMap<String,
  i64>` — used identically across tasks.
- **Verify before done:** B2/B3 run full `cargo test` + clippy; B3 re-measures
  the fork rate. Do not claim Part B complete until idle sessions demonstrably
  stop re-forking git every tick.
```
