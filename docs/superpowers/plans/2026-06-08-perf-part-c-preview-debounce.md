# Performance Part C — Selection-Move Preview Debounce Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop spawning a `tmux resize-window` + `capture-pane` (500-line scrollback) on *every* `j`/`k` keystroke; capture the preview once the selection settles (~120 ms), so holding a navigation key no longer forks tmux per step or reflows the agent's window per step.

**Architecture:** Implements finding F4 of `docs/superpowers/specs/2026-06-08-performance-design.md`. Today every selection-move handler in `App` calls `update_preview()` inline — IO (tmux) inside `handle_key`. Part C decouples navigation (pure selection change) from the capture: the move handlers just change `self.selected`; the event loop in `src/main.rs` detects that the *selected session name* changed and, after a short debounce with no further move, calls `App::update_preview()` once. The instant feedback (list highlight) still moves every keystroke; only the right-pane preview capture is deferred and coalesced. `refresh()` already repopulates the selected session's preview each tick, so the debounce just makes preview-follows-selection feel snappy between ticks.

**Tech Stack:** Rust, ratatui/crossterm, tmux via `std::process::Command`.

**Spec reference:** §Finding F4 and §"Part C" of the spec. Acceptance: a burst of rapid `j`/`k` presses results in **one** `capture_scrollback`/`resize_window` (not one per step); a single move after a pause still updates the preview within ~120 ms; navigation feels at least as responsive.

---

## File Map

| File | Change |
|---|---|
| `src/app.rs` | Remove the 7 inline `self.update_preview()` calls from the selection-move handlers (navigation `j`/`k`/`↓`/`↑`/`g`, select-by-digit, `move_selected`); `update_preview()` itself stays (now called from the loop). Add a decoupling test. |
| `src/main.rs` | Debounce wiring in `run()`: a `preview_deadline: Option<Instant>`, armed when a key changed the selected session (and returned no `Action`); clamp the poll timeout so we wake at the deadline; service it by calling `app.update_preview()`; clear it after a `refresh` (which already captured the selected preview). New `PREVIEW_DEBOUNCE` const. |

---

## Task C1: app.rs — decouple navigation from preview capture (TDD)

**Files:** Modify `src/app.rs`; test in `src/app.rs` `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing test**

The test module already has a helper `fn named(name: &str) -> Session` (around `src/app.rs:2958`). Add to `mod tests`:

```rust
#[test]
fn navigation_defers_preview_capture() {
    // Moving the selection must NOT capture the preview inline (that IO is now
    // debounced in the event loop). The list selection still moves immediately;
    // only `app.preview` stays untouched until the loop calls update_preview().
    let mut app = App::new(Config::default());
    app.sessions = vec![named("a"), named("b")];
    app.preview = "stale".into();
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.selected, 1, "selection moved to the second session");
    assert_eq!(
        app.preview, "stale",
        "preview capture is deferred to the loop, not done inline on nav"
    );
}
```

- [ ] **Step 2: Run it to verify it FAILS**

Run: `cargo test --lib navigation_defers_preview_capture`
Expected: FAIL — today `j` calls `update_preview()`, which tries to capture the (nonexistent test) session and overwrites `app.preview` with `""`, so the `== "stale"` assert fails.

- [ ] **Step 3: Remove the inline `update_preview()` calls — the 4 navigation handlers**

In `handle_list_key` (`src/app.rs`), the `j`/`k`/`↓`/`↑` arms currently are:
```rust
            KeyCode::Char('j') if !ctrl => {
                self.select_next();
                self.update_preview();
            }
            KeyCode::Down => {
                self.select_next();
                self.update_preview();
            }
            KeyCode::Char('k') if !ctrl => {
                self.select_prev();
                self.update_preview();
            }
            KeyCode::Up => {
                self.select_prev();
                self.update_preview();
            }
```
Replace with (drop the `update_preview()` lines):
```rust
            KeyCode::Char('j') if !ctrl => self.select_next(),
            KeyCode::Down => self.select_next(),
            KeyCode::Char('k') if !ctrl => self.select_prev(),
            KeyCode::Up => self.select_prev(),
```

- [ ] **Step 4: Remove the `update_preview()` call — the `g` (top) handler**

The `g` arm currently is:
```rust
            KeyCode::Char('g') if !ctrl => {
                self.select_first();
                self.update_preview();
            }
```
Replace with:
```rust
            KeyCode::Char('g') if !ctrl => self.select_first(),
```

- [ ] **Step 5: Remove the `update_preview()` call — select-by-digit**

In `handle_select_session_key`, the digit arm currently is:
```rust
            KeyCode::Char(c @ '1'..='9') => {
                let pos = c as usize - '1' as usize;
                if pos < self.visible_indices().len() {
                    self.selected = pos;
                    self.update_preview();
                    self.mode = Mode::List;
                }
            }
```
Replace with (drop the `update_preview()` line only):
```rust
            KeyCode::Char(c @ '1'..='9') => {
                let pos = c as usize - '1' as usize;
                if pos < self.visible_indices().len() {
                    self.selected = pos;
                    self.mode = Mode::List;
                }
            }
```

- [ ] **Step 6: Remove the `update_preview()` call — `move_selected`**

In `move_selected`, the `if moved { … }` block ends with `self.update_preview();` (the last statement inside it). Remove that single line. Moving a session keeps the SAME session selected (its name doesn't change), so no re-capture is needed; the loop's name-change detection won't (and shouldn't) re-capture. The `if moved { … }` block keeps its other statements (re-deriving `self.order`/`self.project_order`, `self.dirty = true`).

- [ ] **Step 7: Run the test + full suite**

Run: `cargo test --lib navigation_defers_preview_capture`
Expected: PASS.
Run: `cargo test`
Expected: all pass. (`update_preview` is now called only from `src/main.rs`; within the lib it is unused, so expect a `dead_code`-style warning ONLY if it were private — it is `pub`, so no warning. If clippy flags anything, report it.)
Run: `cargo clippy --all-targets`
Expected: no new warnings.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs
git commit -m "perf(app): decouple selection moves from preview capture (F4)

Navigation handlers (j/k/up/down/g, select-by-digit, move_selected) no
longer call update_preview() inline — that tmux capture+resize is debounced
in the event loop now. Keystrokes stay pure selection changes.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task C2: main.rs — debounce the preview capture in run()

**Files:** Modify `src/main.rs` (`run()` + a new const).

- [ ] **Step 1: Add the debounce interval const**

Near `poll_timeout` (e.g. just above `fn run(`) in `src/main.rs`:
```rust
/// How long the selected session must stay settled before the loop captures its
/// preview — coalesces a burst of `j`/`k` into a single tmux capture+resize (F4).
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);
```

- [ ] **Step 2: Declare the deadline alongside the other loop state**

In `run()`, next to `let mut needs_redraw = true;`:
```rust
    // When the selection changes, capture the new preview after it settles.
    let mut preview_deadline: Option<Instant> = None;
```

- [ ] **Step 3: Arm the deadline when a key moved the selection**

In the `if let Event::Key(key) = ev { … }` block, the tail currently is:
```rust
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action, &mut install_rx)?;
                }
                // Persist split width / session order if a key changed them.
                if app.dirty {
                    app.snapshot_state().save();
                    app.dirty = false;
                }
```
Replace with:
```rust
                // Note the selected session before/after: a pure navigation key
                // (no Action) that changed it arms the preview-capture debounce.
                let prev_sel = app.selected_name();
                let action = app.handle_key(key);
                let nav_changed = action.is_none() && app.selected_name() != prev_sel;
                if let Some(action) = action {
                    handle_action(terminal, app, action, &mut install_rx)?;
                }
                // Persist split width / session order if a key changed them.
                if app.dirty {
                    app.snapshot_state().save();
                    app.dirty = false;
                }
                if nav_changed {
                    // Re-arm on each move so a held key coalesces to one capture.
                    preview_deadline = Some(Instant::now() + PREVIEW_DEBOUNCE);
                }
```

- [ ] **Step 4: Clamp the poll timeout so we wake at the deadline**

The poll line currently is:
```rust
        let timeout = poll_timeout(any_running, app.tmux_missing, last_refresh.elapsed(), refresh);
        if event::poll(timeout)? {
```
Replace with:
```rust
        let mut timeout = poll_timeout(any_running, app.tmux_missing, last_refresh.elapsed(), refresh);
        if let Some(deadline) = preview_deadline {
            // Don't sleep past a pending preview capture.
            let until = deadline.saturating_duration_since(Instant::now());
            timeout = timeout.min(until.max(Duration::from_millis(1)));
        }
        if event::poll(timeout)? {
```

- [ ] **Step 5: Service the deadline (after the event block, before the refresh block)**

Immediately AFTER the closing brace of the `if event::poll(timeout)? { … }` block and BEFORE the `if !app.tmux_missing && last_refresh.elapsed() >= refresh { … }` block, insert:
```rust
        // Selection settled long enough → capture its preview once.
        if let Some(deadline) = preview_deadline {
            if Instant::now() >= deadline {
                app.update_preview();
                preview_deadline = None;
                needs_redraw = true;
            }
        }
```

- [ ] **Step 6: Clear the deadline after a refresh (it already captured the selected preview)**

In the refresh-due block, right after `last_refresh = Instant::now();`:
```rust
            // refresh() just recaptured the selected session's preview, so any
            // pending debounce is redundant.
            preview_deadline = None;
```

- [ ] **Step 7: Build, test, clippy**

Run: `cargo build` → clean (no unused `Instant`/const warnings; `Instant` is already imported).
Run: `cargo test` → all pass.
Run: `cargo clippy --all-targets` → no new warnings.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "perf(loop): debounce preview capture on selection moves (F4)

A burst of j/k now coalesces to a single tmux capture_scrollback + resize
once the selection settles (~120ms), instead of one per keystroke. The list
highlight still moves instantly; the preview follows after the move settles
or on the next refresh tick.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task C3: Verify & manual smoke

- [ ] **Step 1: Full verification**

Run: `cargo test` → all green (report count).
Run: `cargo clippy --all-targets` → clean.
Run: `cargo build --release` → clean.

- [ ] **Step 2: Manual smoke (USER / interactive — do not automate)**

With several sessions open:
- Press `j`/`k` once and pause: the highlight moves instantly; the preview updates within ~120 ms.
- **Hold** `j` (or press rapidly) to scan the whole list: the highlight flies down smoothly; the preview does NOT flicker through every session — it settles on the final selection. Watching `top -o cpu` (or the children watch from Part B), tmux should NOT fork per keystroke during the scan.
- `g` (top), select-by-digit (`s` then a number): preview updates after settling.
- Moving a session (the `move_selected` key) keeps its preview (same session) — no flicker.
- Preview scroll (Ctrl+j/k, PageUp/Down, mouse wheel) and attach/detach still work.

- [ ] **Step 3 (optional): Record the keystroke-fork win**

While holding `j` across ~10 sessions, confirm via `top -l 8 -s 5 -o cpu -stats pid,command,cpu | grep -E 'tmux|git'` that there is no per-keystroke tmux burst — only the single settle capture. Note the qualitative result.

- [ ] **Step 4: Commit any recorded notes**

```bash
git add docs/superpowers/plans/2026-06-08-perf-part-c-preview-debounce.md
git commit -m "docs(plan): record Part C manual smoke result

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review notes

- **F4 coverage:** decouple nav from capture → C1 (remove the 7 inline calls);
  debounce + coalesce → C2 (loop `preview_deadline`).
- **Why loop-detected, not a flag:** comparing `selected_name()` before/after
  `handle_key` covers ALL selection changes (j/k/↓/↑/g, digit, filter-induced
  clamp) without touching each handler, and naturally skips `move_selected` (same
  session selected → no re-capture). Actions return `Some` and are excluded
  (they `refresh()`, which captures the preview itself).
- **No frozen preview:** if the user never pauses, `refresh()` (≤1.5 s) still
  recaptures the selected preview and clears the deadline; the 120 ms debounce
  just makes it feel immediate between ticks.
- **Architecture:** this REMOVES tmux IO from `handle_key` (the move handlers
  were the one place `App` shelled out mid-keypress), tightening the
  "App is pure; effects live in the loop" split.
- **Instant feedback preserved:** any key sets `needs_redraw = true`, so the list
  highlight repaints on every keystroke; only the preview capture is deferred.
- **Type consistency:** `preview_deadline: Option<Instant>`, `PREVIEW_DEBOUNCE:
  Duration`, `nav_changed = action.is_none() && app.selected_name() != prev_sel`,
  serviced via the existing `App::update_preview(&mut self)`.
- **Verify before done:** C1 has the decoupling unit test; C2 is loop wiring
  (build/clippy + manual smoke, like Part A's loop task — the debounce timing is
  Instant-based and validated by hand). Do not claim Part C complete until the
  held-`j` scan demonstrably stops forking tmux per keystroke.
```
