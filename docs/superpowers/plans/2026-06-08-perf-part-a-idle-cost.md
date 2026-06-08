# Performance Part A — Idle-Cost Elimination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the app from redrawing and waking at a fixed ~12.5 fps when idle, and stop re-parsing the ANSI preview every frame — so a static screen costs ≈0 CPU.

**Architecture:** Implements findings F1 (cache parsed-ANSI preview), F5 (memoize wrapped line count), and F2 (event-driven redraw + adaptive poll timeout) from the design spec `docs/superpowers/specs/2026-06-08-performance-design.md`. A `PreviewCache` on `App` parses ANSI only when `preview` changes; the main loop (`main.rs`) redraws only when a `needs_redraw` flag is set and blocks with an adaptive timeout that wakes at the 80 ms spinner cadence **only while a session is `Running`**, otherwise sleeps until the next refresh deadline. `App` stays IO-free; the cache is a pure memo (mirrors the existing `preview_dims: Cell` renderer-write pattern).

**Tech Stack:** Rust, Ratatui 0.29 (`unstable-rendered-line-info` feature, already enabled), Crossterm 0.28, `ansi_to_tui`.

**Spec reference:** This plan implements §Findings F1/F2/F5 and §"Part A" of the spec. Acceptance is **structural + measured** (§Methodology): baseline captured before code, re-measured after.

---

## File Map

| File | Change |
|---|---|
| `src/app.rs` | Add `PreviewCache` struct + `preview_cache`/`preview_parse_count` fields + inits; add `ensure_preview_cache`/`preview_text`/`preview_line_count`; change `offer_update_if_idle() -> bool`; change `refresh() -> bool` (Part A stub: always `true`); new imports |
| `src/ui/preview.rs` | Replace per-frame `into_text()` + `line_count()` with the cached `App::preview_text()` / `App::preview_line_count()` |
| `src/main.rs` | Add `poll_timeout` helper (+tests); rewrite `run()` to use `needs_redraw` + `poll_timeout` |

---

## Task 0: Capture the baseline (no code)

Per spec §Methodology — measure **before** touching code so the improvement is proven.

- [ ] **Step 1: Build release**

Run: `cargo build --release`
Expected: builds clean.

- [ ] **Step 2: Record idle CPU + wakeups, warm case**

Launch `./target/release/amux` with ≥1 session present. Leave the screen fully
static (no keypresses) for ≥30 s. In another terminal capture, for the `amux` PID:

Run: `top -l 6 -s 5 -pid "$(pgrep -x amux | head -1)" -stats pid,cpu,idlew`
(macOS: `idlew` = idle wakeups; or use Activity Monitor → the `amux` row → %CPU and
"Idle Wake Ups").

Record the numbers in this checkbox (edit the file):
- Sessions: `____`
- Idle %CPU (warm): `____`
- Idle wakeups/sec (warm): `____`

- [ ] **Step 3: Record idle CPU, zero-session case**

Repeat Step 2 with 0 sessions. Record:
- Idle %CPU (0 sessions): `____`

- [ ] **Step 4 (optional): Confirm the render hotspot**

Run: `cargo flamegraph --bin amux` (leave idle ~30 s, then quit). Note the combined
share of `ansi_to_tui` (`IntoText`) + `Paragraph::line_count` in the profile:
`____%`. This is the share Task 1–2 should remove.

---

## Task 1: app.rs — PreviewCache (F1) + line-count memo (F5)

**Files:**
- Modify: `src/app.rs`
- Test: `src/app.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Add imports**

At the top of `src/app.rs`, after the existing `use` lines, add:

```rust
use ansi_to_tui::IntoText;
use ratatui::text::Text;
use ratatui::widgets::{Paragraph, Wrap};
```

- [ ] **Step 2: Add the `PreviewCache` type**

Add near the other public structs in `src/app.rs` (e.g. just above `pub struct App {`):

```rust
/// Cached parse of `App::preview`. ANSI parsing of up to 500 scrollback lines is
/// expensive, so we do it once per preview change instead of once per frame.
/// A pure memo of `preview` — holds no logical state — so the "render is
/// read-only" contract is preserved (cf. the `preview_dims: Cell` pattern).
#[derive(Default)]
pub struct PreviewCache {
    /// `content_hash(&preview)` the cache was built for.
    hash: u64,
    /// Parsed, styled preview text (borrowed read-only by the renderer).
    pub text: Text<'static>,
    /// Memoized wrapped display-row count, keyed by render width: `(width, rows)`.
    line_count: Option<(u16, u16)>,
}
```

- [ ] **Step 3: Add fields to `App`**

In `struct App { … }`, add two fields (place them next to `preview_dims`):

```rust
    /// Parsed-ANSI memo of `preview`; rebuilt only when `preview` changes (F1/F5).
    preview_cache: std::cell::RefCell<PreviewCache>,
    /// Number of times the preview was (re)parsed — observability for tests.
    preview_parse_count: std::cell::Cell<u64>,
```

- [ ] **Step 4: Initialize the fields in `App::new`**

In `App::new`, next to `preview_dims: std::cell::Cell::new((0, 0)),`, add:

```rust
            preview_cache: std::cell::RefCell::new(PreviewCache::default()),
            preview_parse_count: std::cell::Cell::new(0),
```

- [ ] **Step 5: Write the failing tests**

Add to `src/app.rs` `mod tests`:

```rust
#[test]
fn preview_cache_parses_once_until_changed() {
    let mut app = App::new(Config::default());
    app.preview = "\u{1b}[32mhi\u{1b}[0m there".into();
    let _ = app.preview_text();
    let _ = app.preview_text();
    let _ = app.preview_line_count(40);
    assert_eq!(
        app.preview_parse_count.get(),
        1,
        "unchanged preview must be parsed exactly once"
    );
    app.preview = "completely different".into();
    let _ = app.preview_text();
    assert_eq!(
        app.preview_parse_count.get(),
        2,
        "changed preview must trigger one reparse"
    );
}

#[test]
fn preview_line_count_is_memoized_per_width() {
    let mut app = App::new(Config::default());
    app.preview = (0..10)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let a = app.preview_line_count(20);
    let b = app.preview_line_count(20);
    assert_eq!(a, b, "same width must return the same count");
    assert_eq!(
        app.preview_parse_count.get(),
        1,
        "two line-count calls must not reparse ANSI"
    );
}

#[test]
fn preview_text_returns_parsed_content() {
    let mut app = App::new(Config::default());
    app.preview = "\u{1b}[31mred\u{1b}[0m".into();
    let cache = app.preview_text();
    // The parsed text must contain the visible string with the escape stripped.
    let joined: String = cache
        .text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(joined.contains("red"), "parsed text missing content: {joined:?}");
    assert!(!joined.contains('\u{1b}'), "escape leaked: {joined:?}");
}
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test --lib preview_cache_parses_once_until_changed preview_line_count_is_memoized_per_width preview_text_returns_parsed_content`
Expected: FAIL — `no method named preview_text/preview_line_count found`.

- [ ] **Step 7: Implement the cache methods**

Add to `impl App` in `src/app.rs` (e.g. just below `update_preview`):

```rust
    /// Rebuild `preview_cache` if `preview` changed since the last parse.
    fn ensure_preview_cache(&self) {
        let h = content_hash(&self.preview);
        if self.preview_cache.borrow().hash == h {
            return;
        }
        let trimmed = self.preview.trim_end_matches(['\n', ' ', '\t']);
        let text = trimmed
            .into_text()
            .unwrap_or_else(|_| Text::raw(trimmed.to_string()));
        *self.preview_cache.borrow_mut() = PreviewCache {
            hash: h,
            text,
            line_count: None,
        };
        self.preview_parse_count
            .set(self.preview_parse_count.get() + 1);
    }

    /// Parsed preview text, rebuilt only on change. Borrowed read-only by the
    /// renderer (clones the `Text` into its `Paragraph`, as before).
    pub fn preview_text(&self) -> std::cell::Ref<'_, PreviewCache> {
        self.ensure_preview_cache();
        self.preview_cache.borrow()
    }

    /// Wrapped display-row count for `width`, memoized per `(hash, width)` so a
    /// redraw at an unchanged width skips the re-wrap pass (F5).
    pub fn preview_line_count(&self, width: u16) -> u16 {
        self.ensure_preview_cache();
        if let Some((w, total)) = self.preview_cache.borrow().line_count {
            if w == width {
                return total;
            }
        }
        let total = {
            let cache = self.preview_cache.borrow();
            Paragraph::new(cache.text.clone())
                .wrap(Wrap { trim: false })
                .line_count(width) as u16
        };
        self.preview_cache.borrow_mut().line_count = Some((width, total));
        total
    }
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test --lib preview_cache_parses_once_until_changed preview_line_count_is_memoized_per_width preview_text_returns_parsed_content`
Expected: PASS (3 passed).

- [ ] **Step 9: Commit**

```bash
git add src/app.rs
git commit -m "perf(app): cache parsed-ANSI preview + memoize line count (F1/F5)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: ui/preview.rs — consume the cache (F1/F5)

**Files:**
- Modify: `src/ui/preview.rs:89-104`
- Test: existing tests in `src/ui/preview.rs` must still pass (behavioural guard)

- [ ] **Step 1: Replace the per-frame parse + line-count**

In `src/ui/preview.rs`, replace the content block (currently lines 89-104, from
the `// Content:` comment through the final `f.render_widget(...)`) with:

```rust
    // Content: parsed ANSI is cached on App (rebuilt only when the preview
    // changes), so a static screen does no ANSI work. Drop trailing blank lines
    // (tmux pads the pane), then bottom-anchor so the newest output is visible.
    let total = app.preview_line_count(rows[3].width);
    let bottom = total.saturating_sub(rows[3].height);
    let scroll_y = bottom.saturating_sub(app.preview_scroll);
    let para = Paragraph::new(app.preview_text())
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(th::TEXT));
    f.render_widget(para.scroll((scroll_y, 0)), rows[3]);
```

> `App::preview_text()` returns an owned `Text<'static>` (the cache stays
> private), so call it directly inside `Paragraph::new` — no `.clone()` and no
> borrow held across `preview_line_count`. Compute `preview_line_count` first
> (it borrows the cache mutably to update its memo), then `preview_text`.

> The `use ansi_to_tui::IntoText;` import at the top of `preview.rs` is now
> unused. Remove that line to avoid an unused-import warning.

- [ ] **Step 2: Run the preview render tests**

Run: `cargo test --lib ui::preview`
Expected: PASS — `renders_title_and_ansi_content`, `wraps_wide_lines_instead_of_clipping`, `bottom_anchors_newest_line_when_wrapped_content_overflows`, and the limits tests all still pass (the cache is transparent to rendering).

- [ ] **Step 3: Run the full suite + clippy**

Run: `cargo test`
Expected: all pass (≈180 tests).
Run: `cargo clippy --all-targets`
Expected: no new warnings (in particular, no unused-import warning from the removed `IntoText`).

- [ ] **Step 4: Commit**

```bash
git add src/ui/preview.rs
git commit -m "perf(ui): render preview from App's ANSI cache instead of per-frame parse (F1/F5)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: app.rs — `offer_update_if_idle` and `refresh` return change signals (F2 hooks)

**Files:**
- Modify: `src/app.rs` (`offer_update_if_idle` ~line 1225; `refresh` ~line 2176)
- Test: `src/app.rs` `mod tests`

- [ ] **Step 1: Write the failing tests**

Add to `src/app.rs` `mod tests`:

```rust
#[test]
fn offer_update_returns_true_only_when_it_opens_the_modal() {
    let mut app = App::new(Config::default());
    // No pending update → nothing opens.
    assert!(!app.offer_update_if_idle(), "no update → false");
    // Pending update while idle in the list → opens once, returns true.
    // `upd_info()` is the existing test helper in this module (src/app.rs).
    app.update = Some(upd_info());
    assert!(app.offer_update_if_idle(), "first offer opens the modal → true");
    // Already prompted this run → no second open.
    assert!(!app.offer_update_if_idle(), "second call → false");
}

#[test]
fn refresh_signals_redraw() {
    // Part A: refresh always requests a redraw. (Part B refines to real change
    // detection.) With no tmux worker attached, refresh runs against an empty
    // session list and must still return true.
    let mut app = App::new(Config::default());
    assert!(app.refresh(), "refresh requests a redraw in Part A");
}
```

> `UpdateInfo { version, url }` (verified in `src/update.rs`); `upd_info()` already
> exists in the `app.rs` test module, so no new helper is needed.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib offer_update_returns_true_only_when_it_opens_the_modal refresh_signals_redraw`
Expected: FAIL — `offer_update_if_idle`/`refresh` return `()`, not `bool` (mismatched types).

- [ ] **Step 3: Change `offer_update_if_idle` to return `bool`**

In `src/app.rs`, replace the whole method:

```rust
    /// Opens the update offer when one is pending, the user is idle in the
    /// list, and we haven't asked yet this run. Returns true if it opened the
    /// modal (so the event loop knows to redraw).
    pub fn offer_update_if_idle(&mut self) -> bool {
        if self.update_prompted || !matches!(self.mode, Mode::List) {
            return false;
        }
        let Some(info) = self.update.clone() else {
            return false;
        };
        self.update_prompted = true;
        self.mode = Mode::ConfirmUpdate(UpdateModal { info, stage: None });
        true
    }
```

- [ ] **Step 4: Change `refresh` to return `bool` (Part A stub)**

In `src/app.rs`, change the signature line:

```rust
    pub fn refresh(&mut self) -> bool {
```

and add, immediately before the method's closing brace (after the
`match crate::tmux::list_sessions() { … }` block ends):

```rust
        // Part A (F2 hook): always request a redraw after a refresh. Part B (F3)
        // refines this to return whether visible state actually changed.
        true
```

> The existing statement call sites (`app.refresh();` in `main.rs` `handle_action`
> and in tests) compile unchanged — the returned `bool` is simply ignored.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib offer_update_returns_true_only_when_it_opens_the_modal refresh_signals_redraw`
Expected: PASS.

- [ ] **Step 6: Run the full suite**

Run: `cargo test`
Expected: all pass (the signature changes are source-compatible with existing callers).

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "perf(app): offer_update_if_idle/refresh return change signals for redraw gating (F2)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: main.rs — `poll_timeout` helper (F2 core)

**Files:**
- Modify: `src/main.rs`
- Test: `src/main.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Add to `src/main.rs` `mod tests`:

```rust
#[test]
fn poll_timeout_animates_while_running() {
    assert_eq!(
        poll_timeout(true, false, Duration::from_millis(10), Duration::from_millis(1500)),
        Duration::from_millis(80),
        "a running session animates the spinner at 80ms"
    );
}

#[test]
fn poll_timeout_idle_sleeps_until_next_refresh() {
    // 500ms into a 1500ms cycle → ~1000ms remaining.
    assert_eq!(
        poll_timeout(false, false, Duration::from_millis(500), Duration::from_millis(1500)),
        Duration::from_millis(1000)
    );
}

#[test]
fn poll_timeout_idle_does_not_wake_at_spinner_rate() {
    // The key idle property: wait far longer than the 80ms spinner tick.
    let t = poll_timeout(false, false, Duration::ZERO, Duration::from_millis(1500));
    assert!(t > Duration::from_millis(80), "idle must not wake at 80ms, got {t:?}");
}

#[test]
fn poll_timeout_floors_when_refresh_overdue() {
    // Overdue refresh must not yield a 0ms timeout (busy spin).
    let t = poll_timeout(false, false, Duration::from_millis(2000), Duration::from_millis(1500));
    assert_eq!(t, Duration::from_millis(1), "floored to 1ms, got {t:?}");
}

#[test]
fn poll_timeout_tmux_missing_waits_long() {
    let t = poll_timeout(true, true, Duration::ZERO, Duration::from_millis(1500));
    assert!(t >= Duration::from_secs(1), "static error screen waits long, got {t:?}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --bin amux poll_timeout`
Expected: FAIL — `cannot find function poll_timeout`.

- [ ] **Step 3: Implement `poll_timeout`**

Add to `src/main.rs` (e.g. just above `fn run(`):

```rust
/// How long the event loop should block in `event::poll` this iteration.
///
/// - A `Running` session animates the spinner → wake at the 80 ms frame cadence.
/// - Otherwise (idle) sleep until the next `refresh` is due, floored to 1 ms so a
///   just-due refresh doesn't busy-spin at a 0 ms timeout.
/// - `tmux_missing` shows a static error screen → wake rarely (only a keypress
///   matters).
fn poll_timeout(
    any_running: bool,
    tmux_missing: bool,
    since_refresh: Duration,
    refresh: Duration,
) -> Duration {
    const SPINNER_TICK: Duration = Duration::from_millis(80);
    const FLOOR: Duration = Duration::from_millis(1);
    if tmux_missing {
        return Duration::from_secs(1);
    }
    if any_running {
        return SPINNER_TICK;
    }
    refresh.saturating_sub(since_refresh).max(FLOOR)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin amux poll_timeout`
Expected: PASS (5 passed).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "perf(loop): add adaptive poll_timeout (spinner-rate only while running) (F2)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: main.rs — event-driven `run()` (F2 wiring)

**Files:**
- Modify: `src/main.rs:168-312` (the `run` function)

- [ ] **Step 1: Replace the body of `run`**

Replace the entire `run` function in `src/main.rs` with the version below. Changes
vs. the original: a `needs_redraw` flag gates `terminal.draw`; channel drains and
spinner/update/refresh transitions set it; `event::poll` uses `poll_timeout`; the
fixed `tick` is gone. The event-handling and `restarting`-resume blocks are
preserved verbatim.

```rust
fn run(
    terminal: &mut Term,
    app: &mut App,
    refresh: Duration,
    usage_rx: &mpsc::Receiver<am::usage::Account>,
    update_rx: &mpsc::Receiver<am::update::UpdateInfo>,
) -> io::Result<()> {
    let start = Instant::now();
    let mut last_refresh = Instant::now();
    let mut install_rx: Option<mpsc::Receiver<am::update::UpdateStage>> = None;
    // Draw once on entry; thereafter only when something changed (F2).
    let mut needs_redraw = true;
    loop {
        // Drain background channels; any applied message changes the view.
        while let Ok(acct) = usage_rx.try_recv() {
            if acct.usage.is_some() {
                app.usage = acct.usage;
                app.usage_error = None;
            } else if acct.usage_error.is_some() {
                app.usage_error = acct.usage_error;
            }
            if acct.plan.is_some() {
                app.plan = acct.plan;
            }
            needs_redraw = true;
        }
        while let Ok(info) = update_rx.try_recv() {
            app.update = Some(info);
            needs_redraw = true;
        }
        if let Some(rx) = &install_rx {
            while let Ok(stage) = rx.try_recv() {
                app.set_update_stage(stage);
                needs_redraw = true;
            }
        }

        // Advance the spinner; only a running session animates, and only an
        // actual frame change needs a redraw.
        let prev_frame = app.spinner_frame;
        app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
        let any_running = app
            .sessions
            .iter()
            .any(|s| s.status == am::tmux::Status::Running);
        if any_running && app.spinner_frame != prev_frame {
            needs_redraw = true;
        }
        if app.offer_update_if_idle() {
            needs_redraw = true;
        }

        if needs_redraw {
            terminal.draw(|f| ui::draw(f, app))?;
            needs_redraw = false;
        }

        let timeout = poll_timeout(any_running, app.tmux_missing, last_refresh.elapsed(), refresh);
        if event::poll(timeout)? {
            let ev = event::read()?;
            // Any consumed input may change the view.
            needs_redraw = true;
            if let Event::Paste(text) = &ev {
                if !app.tmux_missing {
                    app.handle_paste(text);
                }
                continue;
            }
            if let Event::Mouse(m) = &ev {
                match m.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        // Only scroll the preview when the cursor is over the right
                        // panel. The boundary is: x=2 margin + left-panel width.
                        let screen = terminal.size().unwrap_or_default();
                        let split_col = preview_boundary_col(screen.width, app.split_pct);
                        if m.column >= split_col {
                            if m.kind == MouseEventKind::ScrollUp {
                                app.preview_scroll_up(3);
                            } else {
                                app.preview_scroll_down(3);
                            }
                        }
                    }
                    _ => {} // clicks/moves ignored
                }
                continue;
            }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if app.tmux_missing {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    continue;
                }
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action, &mut install_rx)?;
                }
                // Persist split width / session order if a key changed them.
                if app.dirty {
                    app.snapshot_state().save();
                    app.dirty = false;
                }
            }
        }

        if !app.tmux_missing && last_refresh.elapsed() >= refresh {
            if app.refresh() {
                needs_redraw = true;
            }
            last_refresh = Instant::now();

            // For sessions awaiting resume: once claude exits, the pane goes
            // dead (kept by remain-on-exit) with the `claude --resume <uuid>`
            // hint in its content — parse it and respawn the pane with that
            // command. Time out after 30 s (something went wrong — the dead
            // pane is left for inspection; kill with `d` or retry `u`).
            if !app.restarting.is_empty() {
                let mut to_clear: Vec<String> = Vec::new();
                for (name, &started) in &app.restarting {
                    if app.now_unix - started > 30 {
                        let _ = tmux::set_remain_on_exit(name, false);
                        to_clear.push(name.clone());
                        continue;
                    }
                    // The hint is printed by claude on exit; wait until the
                    // pane is actually dead (also guards against respawning —
                    // and killing — a still-live claude).
                    if !tmux::pane_dead(name).unwrap_or(false) {
                        continue;
                    }
                    if let Ok(pane) = tmux::capture_pane(name) {
                        if let Some(cmd) = tmux::parse_resume_command(&pane) {
                            let dir = app
                                .sessions
                                .iter()
                                .find(|s| s.name == *name)
                                .map(|s| s.dir.clone())
                                .unwrap_or_default();
                            if let Err(e) = tmux::respawn_pane(name, &dir, &cmd) {
                                app.error = Some(format!("resume: {e}"));
                            }
                            let _ = tmux::set_remain_on_exit(name, false);
                            to_clear.push(name.clone());
                        }
                    }
                }
                for name in &to_clear {
                    app.restarting.remove(name);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    // Flush state changed by background ticks (e.g. pruned drafts): the in-loop
    // save only runs on a keypress, so a quit right after a tick would lose it.
    if app.dirty {
        app.snapshot_state().save();
        app.dirty = false;
    }
    Ok(())
}
```

- [ ] **Step 2: Build and run the full suite**

Run: `cargo build`
Expected: builds clean (no reference to the removed `tick` binding).
Run: `cargo test`
Expected: all pass (≈180 + the new tests).
Run: `cargo clippy --all-targets`
Expected: no new warnings.

- [ ] **Step 3: Manual smoke test (structural acceptance)**

Run: `cargo run` (with ≥1 session). Verify:
- Idle list (no running session): screen is static; in `top`/Activity Monitor the
  `amux` PID shows near-0 %CPU and few wakeups (vs. the Task 0 baseline).
- A session actively producing output shows the spinner animating smoothly.
- Navigation (`j`/`k`), opening modals (`n`, `?`, `d`), filter (`/`), preview
  scroll (PageUp/Down, mouse wheel), and attach/detach all redraw correctly — no
  stale frames.
- Resizing the terminal redraws immediately.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "perf(loop): redraw only on change + adaptive idle sleep (F2)

Idle list no longer redraws or wakes at the 80ms spinner cadence; the
spinner animates only while a session is Running. Combined with the ANSI
preview cache (F1/F5), a static screen does ~0 work.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Re-measure & record (proves the spec acceptance)

- [ ] **Step 1: Re-run the baseline protocol**

Repeat Task 0 Steps 2–3 on the release build (`cargo build --release`). Record:
- Idle %CPU (warm, after): `____`  (baseline was: `____`)
- Idle wakeups/sec (warm, after): `____`  (baseline: `____`)
- Idle %CPU (0 sessions, after): `____`  (baseline: `____`)

- [ ] **Step 2: Confirm structural acceptance (spec §Methodology)**

Verify and check off:
- [ ] Static screen, no `Running` session → essentially no draws/wakeups between refresh ticks (draws drop from ~12.5/sec to ~refresh-rate, i.e. ≤1/sec).
- [ ] Spinner animates only while a session is `Running`.
- [ ] After/before idle %CPU + wakeups are materially lower.

- [ ] **Step 3 (optional): Re-profile**

Run: `cargo flamegraph --bin amux`, leave idle ~30 s. Confirm `ansi_to_tui` /
`Paragraph::line_count` no longer appear in the idle profile.

- [ ] **Step 4: Commit the recorded results**

```bash
git add docs/superpowers/plans/2026-06-08-perf-part-a-idle-cost.md
git commit -m "docs(plan): record Part A before/after idle measurements

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review notes (carried from spec)

- **F1** → Task 1 (cache) + Task 2 (renderer consumes it). **F5** → Task 1
  (`preview_line_count` memo) + Task 2. **F2** → Task 3 (signals), Task 4
  (`poll_timeout`), Task 5 (loop wiring).
- **Out of scope here (later parts):** F3 refresh fork gating (`refresh` returns a
  real change bool — Part B), F4 selection-move debounce (Part C). The Part A
  `refresh -> bool` stub returns `true` so the loop is correct now and Part B only
  changes the return value, not the call sites.
- **Contract guard:** `PreviewCache` is a pure memo of `preview`; no logical state
  is derived from it (mirrors `preview_dims: Cell`). `App` performs no new IO.
- **Type consistency:** `preview_text() -> Ref<PreviewCache>`, `preview_line_count(
  width: u16) -> u16`, `ensure_preview_cache(&self)`, `offer_update_if_idle() ->
  bool`, `refresh() -> bool`, `poll_timeout(any_running, tmux_missing,
  since_refresh, refresh) -> Duration` — names used identically across tasks.
- **Verify before done:** Tasks 2/3/5 each run `cargo test`; Tasks 2/5 run
  `cargo clippy`; Task 6 re-measures. Do not claim Part A complete until the
  before/after numbers are recorded (spec acceptance is measured, not assumed).
```
