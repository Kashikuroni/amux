# UI bugfixes — preview width + mouse scroll

Date: 2026-06-03
Status: approved (design)

Two independent UI bugfixes for `am`, the first two items of a four-item list
(the remaining two — a plain non-agent terminal session, and an `N` shortcut
that opens the new-session form pre-filled from the selected project — are
deferred to their own design cycles).

## Bug #3 — Preview clips wide lines

### Problem
`src/ui/preview.rs` renders captured tmux pane content (`app.preview`) in the
content area (`rows[3]`) with a `Paragraph` that has no wrapping. The pane is
captured at the session's full width (the am terminal width at creation), but
the preview column is only ~40–55% of the body, so every line wider than the
column is clipped on the right and that text is lost.

### Fix
Enable soft-wrapping on the content `Paragraph`:

```rust
Paragraph::new(text).wrap(Wrap { trim: false })
```

`trim: false` keeps leading whitespace so wrapped agent output retains its
indentation/alignment.

### Scroll-anchor interaction
The preview is bottom-anchored so the latest output (and any pending prompt) is
always visible. Today the anchor is computed from logical line count:

```rust
let total = text.lines.len() as u16;
let bottom = total.saturating_sub(rows[3].height);
let scroll_y = bottom.saturating_sub(app.preview_scroll);
```

With wrapping enabled, a single logical line can occupy multiple display rows,
so `total` underestimates the real rendered height and the bottom-anchor drifts
(the newest output no longer sits flush at the bottom; manual scroll offsets are
also measured in display rows by ratatui, creating a mismatch).

**Resolution:** compute the anchor against the *wrapped display-row* count for
the current content width. Count display rows by wrapping each logical line to
`rows[3].width` (the content area width), reusing the wrapping logic already in
`ui/mod.rs` (`wrap_rows`) — promote it to a shared helper, or add a small
width-aware row-count function in the preview module. Use that wrapped total in
place of `text.lines.len()` when computing `bottom`.

Note: `Paragraph::scroll` offsets are in display rows when `wrap` is set, so the
existing `preview_scroll` (lines from bottom) continues to behave as a row
offset — acceptable; the key correctness requirement is that `scroll_y == 0`
(no manual scroll) lands the newest output at the bottom edge.

### Testing
Extend `src/ui/preview.rs` tests: render into a narrow `TestBackend` with a
preview line wider than the column and assert the overflow text appears on a
following row rather than being absent from the buffer.

## Bug #2 — Mouse wheel scrolls the session list

### Problem
`src/main.rs` never enables mouse reporting. In the alternate screen, terminals
default to "alternate scroll mode", auto-translating wheel up/down into ↑/↓
arrow key events. Those reach `App::handle_list_key` as `KeyCode::Up`/`Down`,
moving the selection — so an accidental wheel nudge changes the selected
session.

### Fix
Enable mouse capture and ignore mouse events, so the terminal emits mouse
escape sequences (decoded as `Event::Mouse`, which we drop) instead of arrow
keys:

- `init_terminal`: add `EnableMouseCapture` to the `execute!` alongside
  `EnterAlternateScreen, EnableBracketedPaste`.
- The attach re-entry block in `handle_action` (`Action::Attach`): mirror the
  same `EnableMouseCapture` when re-entering the alternate screen.
- `restore_terminal`: add `DisableMouseCapture` to the teardown `execute!`
  (before/with `DisableBracketedPaste, LeaveAlternateScreen`). This also runs
  before attaching, so the tmux session is handed a terminal with normal mouse
  behavior.
- Event loop in `run`: add a no-op arm for `Event::Mouse(_)` (mirroring the
  existing `Event::Paste` early-out style) so wheel/click events are discarded.

Import `EnableMouseCapture` / `DisableMouseCapture` from `crossterm::event`.

### Tradeoff (accepted)
Mouse capture disables the terminal's native click-drag text selection in the
`am` list view; in most terminals holding **Shift** while dragging still
selects. Attached tmux sessions are unaffected (capture is disabled before
hand-off). Accepted because the wheel must be dead app-wide.

### Testing
The `run` loop is not unit-tested; the added arm is trivial. Verify manually:
run `am`, scroll the wheel over the list, confirm the selection does not move.
The preview wrap change carries the automated coverage for this batch.

## Out of scope
- Plain non-agent terminal sessions (list item #1).
- `N` = new session pre-filled from the selected project (list item #4).
