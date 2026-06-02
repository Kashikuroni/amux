# UI Bugfixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the preview from clipping wide lines (soft-wrap instead), and kill the mouse wheel app-wide so it no longer moves the session selection.

**Architecture:** Two independent, self-contained changes. (1) `src/ui/preview.rs`: add `Wrap { trim: false }` to the content `Paragraph` and bottom-anchor against the wrapped display-row count via ratatui's `Paragraph::line_count` (gated behind the `unstable-rendered-line-info` cargo feature). (2) `src/main.rs`: enable mouse capture so the terminal stops translating the wheel into arrow keys, and drop `Event::Mouse` in the loop.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tmux.

---

## File Structure

- `Cargo.toml` — enable ratatui's `unstable-rendered-line-info` feature (gives `Paragraph::line_count`).
- `src/ui/preview.rs` — wrap the content paragraph; recompute the bottom-anchor from wrapped row count; add a test for overflow text appearing on a following row.
- `src/main.rs` — enable/disable mouse capture in terminal setup/teardown and the attach re-entry; ignore `Event::Mouse` in the event loop.

---

## Task 1: Preview soft-wraps wide lines (with correct bottom-anchor)

**Files:**
- Modify: `Cargo.toml` (dependencies section)
- Modify: `src/ui/preview.rs:8` (imports), `src/ui/preview.rs:58-71` (render body)
- Test: `src/ui/preview.rs` (tests module)

- [ ] **Step 1: Enable the ratatui feature**

In `Cargo.toml`, change the ratatui dependency line:

```toml
ratatui = { version = "0.29", features = ["unstable-rendered-line-info"] }
```

(Replaces the current `ratatui = "0.29"`.)

- [ ] **Step 2: Verify it builds with the feature**

Run: `cargo build`
Expected: compiles successfully (the feature is additive; no code uses it yet).

- [ ] **Step 3: Write the failing test**

Add to the `tests` module in `src/ui/preview.rs` (after `renders_title_and_ansi_content`):

```rust
    #[test]
    fn wraps_wide_lines_instead_of_clipping() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(),
            dir: "~/work/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.now_unix = 0;
        // A line far wider than the 20-col backend; with wrapping the tail
        // ("zzz") must still appear in the buffer rather than being clipped.
        app.preview = "aaaaaaaaaaaaaaaaaaaa bbbbbbbbbb zzz".into();
        let mut t = Terminal::new(TestBackend::new(20, 12)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("zzz"), "wrapped tail must be visible:\n{s}");
    }
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --lib wraps_wide_lines_instead_of_clipping`
Expected: FAIL — without wrapping, "zzz" is clipped off the right edge and absent from the buffer.

- [ ] **Step 5: Add the `Wrap` import**

In `src/ui/preview.rs`, change line 8 from:

```rust
use ratatui::widgets::Paragraph;
```

to:

```rust
use ratatui::widgets::{Paragraph, Wrap};
```

- [ ] **Step 6: Wrap the content paragraph and fix the anchor**

Replace the content block (`src/ui/preview.rs:58-71`, from `let trimmed = ...` through the final `f.render_widget(...)` for `rows[3]`) with:

```rust
    let trimmed = app.preview.trim_end_matches(['\n', ' ', '\t']);
    let text: Text = trimmed
        .into_text()
        .unwrap_or_else(|_| Text::raw(trimmed.to_string()));
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(th::TEXT));
    // Bottom-anchor against the *wrapped* display-row count (one logical line
    // may wrap to several rows), then offset upward by the user's manual scroll.
    let total = para.line_count(rows[3].width) as u16;
    let bottom = total.saturating_sub(rows[3].height);
    let scroll_y = bottom.saturating_sub(app.preview_scroll);
    f.render_widget(para.scroll((scroll_y, 0)), rows[3]);
```

(`line_count` borrows `&para`, so it is called before `para.scroll(...)` moves it into `render_widget`.)

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test --lib wraps_wide_lines_instead_of_clipping`
Expected: PASS — the wrapped tail "zzz" now appears in the buffer.

- [ ] **Step 8: Run the full preview test module + clippy**

Run: `cargo test --lib preview:: && cargo clippy --all-targets -- -D warnings`
Expected: all preview tests pass (including the existing `renders_title_and_ansi_content`); no clippy warnings.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock src/ui/preview.rs
git commit -m "fix(ui): soft-wrap preview, anchor on wrapped row count

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Mouse wheel does nothing app-wide

**Files:**
- Modify: `src/main.rs:6-9` (imports), `src/main.rs:101` (init), `src/main.rs:123-127` (restore), `src/main.rs:158-163` (event loop), `src/main.rs:214-218` (attach re-entry)

- [ ] **Step 1: Import the mouse-capture commands**

In `src/main.rs`, the `crossterm::event` use block (lines 6-9) currently imports paste/keyboard items. Add `DisableMouseCapture, EnableMouseCapture` to it:

```rust
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
```

- [ ] **Step 2: Enable mouse capture in `init_terminal`**

In `init_terminal` (around line 101), add `EnableMouseCapture` to the `execute!`:

```rust
    execute!(out, EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture)?;
```

- [ ] **Step 3: Disable mouse capture in `restore_terminal`**

In `restore_terminal` (the `execute!` at lines 123-127), add `DisableMouseCapture`:

```rust
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
```

- [ ] **Step 4: Re-enable mouse capture on attach re-entry**

In `handle_action`, the `Action::Attach` arm re-enters the alternate screen after tmux detach (lines 214-218). Add `EnableMouseCapture`:

```rust
            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableMouseCapture
            )?;
```

- [ ] **Step 5: Ignore mouse events in the event loop**

In `run`, after the `if let Event::Paste(text) = &ev { ... continue; }` block (ends ~line 163) and before `if let Event::Key(key) = ev {`, add a no-op arm that discards mouse events:

```rust
            if let Event::Mouse(_) = &ev {
                continue; // wheel/click do nothing — kills accidental list scroll
            }
```

- [ ] **Step 6: Build and run the existing test suite**

Run: `cargo build && cargo test`
Expected: builds clean; all existing tests pass (no test targets this loop arm; the change is non-breaking).

- [ ] **Step 7: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings (e.g. confirm `EnableMouseCapture`/`DisableMouseCapture` are both used, so no unused-import warning).

- [ ] **Step 8: Manual verification**

Run: `cargo run`
- Scroll the mouse wheel over the session list → the selected session must NOT change.
- Press `j`/`k` and ↑/↓ → selection still moves (keyboard nav unaffected).
- Press Enter to attach to a session, then `Ctrl-q` to detach → returns to `am` cleanly with the wheel still inert.

- [ ] **Step 9: Commit**

```bash
git add src/main.rs
git commit -m "fix(ui): capture and ignore mouse so the wheel never scrolls the list

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Bug #3 (preview clips wide lines → soft-wrap) → Task 1 (Steps 5-6: `Wrap { trim: false }`).
- Bug #3 scroll-anchor interaction → Task 1 Step 6 (`line_count` for wrapped total). Resolved more simply than the spec's `wrap_rows`-promotion suggestion: ratatui's own `Paragraph::line_count` matches its wrap algorithm exactly, behind the `unstable-rendered-line-info` feature (Task 1 Step 1).
- Bug #3 testing → Task 1 Steps 3-4, 7-8.
- Bug #2 (mouse wheel → capture & ignore): init → Task 2 Step 2; restore → Step 3; attach re-entry → Step 4; event-loop drop → Step 5. Import → Step 1.
- Bug #2 testing (manual) → Task 2 Step 8.

**Placeholder scan:** none — every code step shows the exact code; every command shows expected output.

**Type consistency:** `Paragraph`, `Wrap { trim: false }`, `line_count(width: u16) -> usize`, `Event::Mouse(_)`, `EnableMouseCapture`/`DisableMouseCapture` are all real ratatui-0.29 / crossterm-0.28 APIs verified against the installed source.
