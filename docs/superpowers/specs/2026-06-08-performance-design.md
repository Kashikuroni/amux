# Performance — Design Spec

## Overview

`amux` keeps the device warm during normal idle use. This spec is the
authoritative reference for the performance work: it documents the root cause,
each finding (with code evidence), the target behaviour, and the staged rollout.
Implementation plans (`docs/superpowers/plans/…`) are derived from this document,
one per part.

The single structural cause: **the app never idles.** The main loop redraws the
full UI at a fixed ~12.5 fps and shells out to `tmux`/`git` on a fixed interval,
regardless of whether anything changed or anyone is watching. The fix makes work
**event-driven**: redraw only when state changes, wake only when there is
something to animate, and re-parse/re-fork only when inputs change.

This is a static-analysis spec (no live profile was taken yet). The baseline
measurement in [Methodology](#methodology) MUST be captured before Part A so the
improvement is proven, not assumed.

---

## Motivation

Symptom (reported): the device heats noticeably during ordinary use, consistent
with sustained background CPU and frequent process spawning rather than a single
pegged core.

Confirmed from code (no busy-loop; `event::poll`, `mpsc::recv`, `thread::sleep`
are all placed correctly). The cost is *steady-state*:

- The render closure `ui::draw` runs in full every ≤80 ms (`src/main.rs:176`,
  `src/main.rs:203`) even when zero cells change.
- Inside each frame, the preview (up to 500 lines of ANSI scrollback) is
  re-parsed and re-wrapped from scratch (`src/ui/preview.rs:93-101`).
- Every `refresh` tick (default 1500 ms) forks `tmux` once per session plus a
  `git` pair per session via the worker (`src/app.rs:2176-2243`,
  `src/git.rs:60-73`).

A laptop sees this as ~single-digit-percent sustained CPU on one core plus
bursty fork/exec that prevents deep sleep — enough to keep fans audible.

---

## Methodology (capture BEFORE Part A)

The acceptance criterion for this work is **structural + measured**. Record a
baseline and re-measure after each part.

**Baseline protocol** (document results in the Part A plan):

1. Build release: `cargo build --release`.
2. Launch `amux` with a representative load: **≥1 session** for the warm case and
   a second run with **0 sessions** for the pure-idle case. Note session count.
3. Leave the screen completely static (no keypresses) for ≥30 s.
4. Capture, for the `amux` PID:
   - `%CPU` over the static window (`top -pid <pid> -l 30` on macOS, or Activity
     Monitor → sample).
   - Idle wakeups / context switches if available (Activity Monitor "Idle Wake
     Ups" column, or `powermetrics --samplers tasks` filtered to the pid).
5. Optional confirmation of the render hotspot:
   `cargo flamegraph --bin am` (debuginfo on) and inspect the share of
   `ansi_to_tui::IntoText` + `Paragraph::line_count` in the captured profile.

**Structural acceptance (the binding criterion):**

- With a fully static screen and **no `Running` session**, the app performs
  **0 `terminal.draw` calls/sec** and **0 process spawns/sec** between `refresh`
  ticks.
- A `refresh` tick that produces byte-identical session/preview state performs
  **0 `terminal.draw` calls** (no redraw on no-op refresh).
- The spinner animates (and only then drives ~12.5 fps wakeups) **only while at
  least one session is `Status::Running`**.

**Measured acceptance (the evidence):** idle `%CPU` and wakeups/sec after Part A
are materially lower than the recorded baseline, captured with the same protocol.
No fixed numeric threshold is mandated; the before/after pair is the proof.

---

## Current architecture (what we are changing)

The event loop, `src/main.rs:168-312` (abridged):

```rust
let tick = Duration::from_millis(80);                 // :176  spinner cadence
let mut last_refresh = Instant::now();
loop {
    app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
    // drain usage_rx / update_rx / install_rx (cheap, non-blocking)
    app.offer_update_if_idle();
    terminal.draw(|f| ui::draw(f, app))?;             // :203  UNCONDITIONAL full render
    if event::poll(tick)? {                           // :205  blocks ≤80 ms
        let ev = event::read()?;
        // … paste / mouse / key handling …
    }
    if !app.tmux_missing && last_refresh.elapsed() >= refresh {
        app.refresh();                                // :257  tmux + git fan-out
        last_refresh = Instant::now();
        // … restarting-session resume bookkeeping …
    }
    if app.should_quit { break; }
}
```

The preview render, `src/ui/preview.rs:92-104`:

```rust
let trimmed = app.preview.trim_end_matches(['\n', ' ', '\t']);
let text: Text = trimmed.into_text()                  // re-parse ALL ANSI every frame
    .unwrap_or_else(|_| Text::raw(trimmed.to_string()));
let para = Paragraph::new(text).wrap(Wrap { trim: false }).style(...);
let total = para.line_count(rows[3].width) as u16;    // re-wrap ALL lines every frame
let bottom = total.saturating_sub(rows[3].height);
let scroll_y = bottom.saturating_sub(app.preview_scroll);
f.render_widget(para.scroll((scroll_y, 0)), rows[3]);
```

`refresh`, `src/app.rs:2176-2243` (abridged): forks `tmux list-sessions` (1),
`tmux capture-pane` per session (N), `tmux capture-scrollback` for the selected
session (1), an optional `tmux resize-window`, and sends every session dir to the
git worker which runs `git symbolic-ref` + `git diff HEAD --shortstat` per dir
(2N) — `src/git.rs:60-73`.

What is already correct and MUST NOT regress:

- `App` is IO-free and pure; all effects live in `handle_action`/`refresh`
  (`ARCHITECTURE.md`). Keep this split.
- Background threads idle correctly: usage poller sleeps 300/600 s
  (`src/main.rs:84-85`), git worker blocks on `recv` and coalesces
  (`src/git.rs:88-110`).
- The ~180 unit tests drive `App` by feeding `KeyEvent`s and asserting on state /
  returned `Action`. New state added for these fixes must keep tests deterministic
  (no wall-clock reads inside `App` beyond the existing `now_unix` pattern).

---

## Findings

Each finding lists evidence, root cause, the target behaviour, the fix design,
and its own acceptance check. Severity: **F1/F2 High**, **F3 Medium**,
**F4 Medium**, **F5 Low**.

### F1 — ANSI preview re-parsed every frame (High)

**Evidence:** `src/ui/preview.rs:93-95` (`into_text()`) and `:101`
(`line_count`). Input is up to 500 lines of ANSI scrollback (`capture_scrollback(
name, 500)` at `src/app.rs:1256` and `src/app.rs:2210`).

**Root cause:** the parsed `Text` and its wrapped line count are recomputed on
every `ui::draw`, i.e. ~12.5×/sec, even when `app.preview` is byte-identical to
the previous frame. ratatui's double buffer only saves the terminal *write*, not
the widget-build cost.

**Target behaviour:** parse ANSI and compute the wrapped line count **once per
preview change**, not once per frame. A frame that re-renders an unchanged
preview does no ANSI work.

**Fix design:** introduce a cached, pre-parsed preview owned by `App`, rebuilt
only when the raw `preview` string changes.

```rust
// app.rs — new field on App
/// Cached parse of `preview`: (hash_of_raw, parsed Text, owned raw for fallback).
/// Rebuilt only when `preview` changes; the renderer borrows it read-only.
pub preview_cache: std::cell::RefCell<PreviewCache>,

pub struct PreviewCache {
    pub hash: u64,                 // content_hash(&preview) the cache was built for
    pub text: ratatui::text::Text<'static>,
}
```

The renderer asks the cache for the current preview, rebuilding on hash mismatch:

```rust
// app.rs
pub fn preview_text(&self) -> std::cell::Ref<'_, PreviewCache> {
    let h = content_hash(&self.preview);
    if self.preview_cache.borrow().hash != h {
        let trimmed = self.preview.trim_end_matches(['\n', ' ', '\t']);
        let text = trimmed.into_text()
            .unwrap_or_else(|_| Text::raw(trimmed.to_string()));
        *self.preview_cache.borrow_mut() = PreviewCache { hash: h, text };
    }
    self.preview_cache.borrow()
}
```

`preview::render` clones the cached `Text` (already the per-frame cost today) and
keeps the existing bottom-anchor math. `RefCell` mirrors the existing
`preview_dims: Cell` renderer-writes-through-&App pattern (`src/app.rs:869`), so
the "rendering is read-only" contract is preserved in spirit (no logical state
mutated; only a memo cache).

> Note: `content_hash` already exists (`src/app.rs:2364`) and is used for pane
> diffing — reuse it, do not add a second hasher.

**Acceptance:** a unit test renders the same `app.preview` twice and asserts the
cache hash is unchanged and `into_text` ran once (e.g. via a rebuild counter on
`PreviewCache`); changing `app.preview` flips the hash and rebuilds.

### F2 — Fixed 12.5 fps redraw + spinner wakeups at idle (High)

**Evidence:** `src/main.rs:176` (`tick = 80ms`), `:203` (unconditional draw),
`:181` (spinner advanced every iteration). `event::poll(tick)` blocks at most
80 ms, so the loop body runs ≥12.5×/sec with no input.

**Root cause:** the 80 ms cadence exists only to animate the braille spinner
(`src/spinner.rs`), but it is paid unconditionally — even with zero sessions, all
sessions idle, or the user away. Every wake does a full `ui::draw`.

**Target behaviour:**

- Redraw only when something changed (a dirty flag).
- Wake at the 80 ms spinner cadence **only while a session is `Running`**;
  otherwise block until the next event or the next `refresh` deadline.

**Fix design:** a `dirty` render flag + an adaptive poll timeout. (Note: `App`
already has a `dirty` field for *state persistence* at `src/app.rs:905` — do NOT
overload it. Use a separate loop-local `needs_redraw`.)

```rust
// main.rs run()
let spinner_tick = Duration::from_millis(80);
let mut needs_redraw = true;                 // draw once on entry
loop {
    // drain usage_rx / update_rx / install_rx; set needs_redraw if any applied
    let prev_frame = app.spinner_frame;
    app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
    let any_running = app.sessions.iter().any(|s| s.status == Status::Running);
    if any_running && app.spinner_frame != prev_frame { needs_redraw = true; }
    if app.offer_update_if_idle() { needs_redraw = true; }

    if needs_redraw {
        terminal.draw(|f| ui::draw(f, app))?;
        needs_redraw = false;
    }

    // Adaptive timeout: spinner cadence only while animating; otherwise sleep
    // until the next refresh is due (capped so refresh stays punctual).
    let until_refresh = refresh.saturating_sub(last_refresh.elapsed());
    let timeout = if any_running { spinner_tick.min(until_refresh) }
                  else { until_refresh };
    if event::poll(timeout)? {
        let ev = event::read()?;
        // … existing handling … then: needs_redraw = true;
    }

    if !app.tmux_missing && last_refresh.elapsed() >= refresh {
        if app.refresh() { needs_redraw = true; }   // see F3: refresh returns changed?
        last_refresh = Instant::now();
        // … restarting bookkeeping … set needs_redraw if it mutated visible state
    }
    if app.should_quit { break; }
}
```

Supporting change: `offer_update_if_idle` returns `bool` (did it open the modal),
so the loop knows to redraw. Currently returns `()` (`src/app.rs:1225`).

**Edge cases that MUST still redraw** (enumerate as tests where practical):
key/mouse/paste events; usage/update/install channel messages applied; a
`refresh` that changed sessions/preview/prompts; opening the update modal; the
`restarting` resume path mutating a session; terminal resize (crossterm delivers
`Event::Resize` through `event::read`, so it is covered by the event branch).

**Acceptance:** structural — with 0 `Running` sessions and no input, the loop
issues 0 draws over a 30 s static window (assert via a draw counter in an
integration harness, or reason it from the adaptive-timeout test). With a
`Running` session, spinner frames still advance at ~80 ms.

### F3 — Per-refresh tmux/git fork fan-out (Medium)

**Evidence:** `src/app.rs:2196` (`capture_pane` per session), `:2235-2237`
(every session dir sent to git worker each tick), `src/git.rs:60-73` (2 git forks
per dir). Default `refresh_interval_ms = 1500` (`src/config.rs:16`).

**Root cause:** every tick re-captures every pane and re-reads git for every
session, regardless of whether that session changed. `git diff HEAD --shortstat`
is the most expensive of these (spawns git, walks the index).

**Target behaviour:** spend forks only where state can have changed. Pane capture
stays per-tick (it is how status is detected), but git re-reads are gated on
actual pane change and/or a slower cadence.

**Fix design:**

1. `refresh` returns `bool` (any visible state changed) so the loop can gate
   redraw (ties into F2). "Changed" = sessions list, any status, any prompt, or
   the selected preview differs from the previous tick.
2. Gate git work by pane-content change. `refresh` already computes a per-session
   `content_hash` (`src/app.rs:2197`); only enqueue a dir to the git worker when
   that session's hash changed since the last git read, OR the session is new, OR
   a coarse git interval (e.g. ≥5 s) has elapsed for that dir. Track last-read
   hash/instant per dir.
3. Keep the git worker's coalescing (`src/git.rs:94-99`) — it already drops a
   backlog to the newest request.

> Boundary: do NOT reduce `capture-pane` frequency in this part — status
> detection depends on per-tick diffing. Collapsing N `capture-pane` forks into
> fewer tmux calls is a possible future optimization, explicitly **out of scope**
> here (see Non-goals).

**Acceptance:** with all sessions idle and unchanged across two consecutive
ticks, the git worker receives an **empty or unchanged** dir set (0 new git
forks). A session whose pane content changes between ticks does trigger a git
re-read for its dir. Existing git/tmux behaviour tests still pass.

### F4 — `capture_scrollback(500)` + `resize_window` on every j/k (Medium)

**Evidence:** `update_preview` (`src/app.rs:1249-1262`) calls `fit_preview_window`
(→ `tmux resize-window`, `src/app.rs:1276`) then `capture_scrollback(name, 500)`
(`src/app.rs:1256`). It is invoked on every selection move
(`src/app.rs:1289,1293,1297,1301,1375,1550` and `:1067`).

**Root cause:** holding `j`/`k` to scan the list spawns 1–2 tmux processes per
step and reflows the agent's tmux window each step (which also forces the agent
to re-render its own output).

**Target behaviour:** moving the selection is cheap; the full scrollback capture
and window resize happen once the selection settles, not on every intermediate
step.

**Fix design:** debounce the heavy capture. On a selection move, update the
lightweight selection state immediately and mark a `preview_dirty` deadline
(e.g. now + ~120 ms); perform `fit_preview_window` + `capture_scrollback` from the
loop only once no further move arrived before the deadline. Because `App` is
IO-free and time-free by contract, the debounce deadline lives in the **loop**
(`main.rs`), not in `App`: `App` exposes "selection changed, preview stale", the
loop coalesces rapid changes and triggers the capture.

Concretely: `select_next/prev` set `app.preview_stale = true` instead of calling
`update_preview` directly; the loop, when `preview_stale` and the debounce window
has elapsed with no new key, calls a new `app.refresh_preview()` (the
capture+fit half of today's `update_preview`). A single pending key in `j` burst
collapses to one capture.

> Compatibility: the instant-preview-on-selection UX (preview already shows the
> capture from the last `refresh` tick) is preserved because `refresh` keeps
> populating `app.preview` for the selected session; the debounced capture only
> sharpens scrollback depth.

**Acceptance:** an integration test simulates 5 rapid `j` presses within the
debounce window and asserts `capture_scrollback`/`resize_window` are invoked once
(via a tmux call spy / counter), not 5×. A single `j` followed by a pause still
captures.

### F5 — `line_count` re-wrap every frame (Low)

**Evidence:** `src/ui/preview.rs:101`.

**Root cause:** bottom-anchor scroll offset re-wraps all preview lines every
frame to get `total`.

**Resolution:** subsumed by F1 (cache the parsed `Text`) + F2 (don't redraw
unchanged frames). Cache the wrapped `line_count` per `(hash, width)` alongside
`PreviewCache` so even a forced redraw at an unchanged width skips the re-wrap.
No standalone work beyond extending `PreviewCache` with a `(width, total)` memo.

**Acceptance:** covered by the F1 cache test extended to assert `line_count` is
computed once across two same-width renders of identical preview.

---

## Staged rollout

Each part is independently shippable, testable, and committable. Parts are
ordered by impact; later parts depend on small hooks introduced earlier.

### Part A — Idle-cost elimination (F1 + F2 + F5)

The high-impact core. Delivers the structural acceptance: no redraw/fork at idle,
spinner-only wakeups while running, ANSI parsed once per preview change.

- F1: `PreviewCache` on `App`; `preview::render` consumes the cache.
- F5: extend `PreviewCache` with the `(width, total)` line-count memo.
- F2: `needs_redraw` flag + adaptive poll timeout; `offer_update_if_idle ->
  bool`; `refresh -> bool` (the F3 signature, introduced here as a stub that
  always returns `true`, refined in Part B).

Exit criteria: baseline captured (Methodology); re-measure shows lower idle CPU +
wakeups; structural assertions hold; full `cargo test` green; spinner still
animates for running sessions; all redraw edge-cases verified.

### Part B — Refresh fork gating (F3)

- `refresh` returns a real `bool` (visible state changed).
- Gate git-worker enqueues on per-session pane-hash change / new session / coarse
  git interval.

Exit criteria: idle ticks enqueue 0 new git forks; changed panes still update
git; tmux/git tests green; re-measure shows lower fork rate under a multi-session
idle load.

### Part C — Selection-move debounce (F4)

- `select_*` mark `preview_stale`; loop debounces and calls `refresh_preview`.

Exit criteria: rapid `j`/`k` burst collapses to one capture+resize; single move
after a pause still captures; navigation feels at least as responsive.

---

## Non-goals (explicitly out of scope)

- Rewriting input to a separate event-reader thread / async `select!` loop. The
  adaptive-timeout approach (Part A) achieves event-driven idle without it. (User
  decision: scope = the 5 review findings.)
- Collapsing N `capture-pane` forks into fewer tmux invocations. Possible later;
  status detection currently depends on per-session per-tick capture.
- Changing default `refresh_interval_ms`, the 500-line scrollback depth, or the
  spinner glyphs/cadence as a "fix" — these are symptom knobs, not root cause.
- Any change to the pure/IO-free `App` contract, the `cm-*`/`@cm_*` tmux naming,
  or persisted state schema.

---

## Risks & regression guards

| Risk | Guard |
|---|---|
| Missed redraw → stale screen after some state change | Enumerate redraw triggers in F2; one test per trigger (key, channel msg, refresh-changed, modal open, resize). Default to `needs_redraw = true` on any unhandled branch. |
| `RefCell`/`Cell` renderer writes break the "read-only render" contract | Cache is a pure memo of `preview`; no logical state derived from it. Mirrors existing `preview_dims: Cell`. Documented as such in code. |
| Debounce makes navigation feel laggy | Short window (~120 ms); selection highlight + last-tick preview update immediately; only scrollback-depth capture is deferred. |
| Git gating hides a real change (stale branch/diff) | Gate also fires on new session + coarse interval; pane-hash change covers agent activity. Falls back to periodic re-read. |
| Spinner stops animating when it should | `any_running` recomputed each loop; test that a `Running` session yields ~80 ms frame advance. |

---

## Verification protocol (per part)

1. `cargo test` — full suite green (≈180 tests).
2. `cargo clippy --all-targets` — no new warnings.
3. Re-run the [Methodology](#methodology) measurement; record before/after in the
   part's plan doc.
4. Manual smoke: create sessions, navigate, attach/detach, open modals, trigger
   a running agent — confirm no visual staleness and that the spinner animates
   only while a session runs.

---

## Open questions

- Coarse git re-read interval for F3: 5 s proposed — confirm against how quickly
  branch/diff staleness is acceptable in practice.
- Debounce window for F4: ~120 ms proposed — tune during Part C against feel.
