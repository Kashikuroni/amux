# Session Notes / To-Do Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-session markdown notes plus a global Inbox note, shown in the right pane instead of the live preview, with render/edit modes, toggleable checkboxes, vim-style task selection + clipboard copy, and a per-card progress counter.

**Architecture:** A pure `note` module parses markdown and computes task state; notes are `String`s persisted in `state.toml`. The right pane gains a `RightPane` view mode (`Preview`/`SessionNote`/`Inbox`); `t`/`T` switch it from `Mode::List` (list navigation unchanged), and `Tab` enters a new `Mode::Note` with `Render`/`Edit` sub-modes. The edit buffer reuses a `TextArea` editor extracted from `ReplyForm`.

**Tech Stack:** Rust, ratatui 0.29, crossterm, serde + toml, `pbcopy` (shell-out). No new crates.

**Spec:** `docs/superpowers/specs/2026-06-03-session-notes-todo-design.md`

**Conventions in this repo:**
- Tests live in `#[cfg(test)] mod tests` at the bottom of each source file.
- UI tests render via `ratatui::backend::TestBackend` + `Terminal`, then assert on a `buf_to_string` dump or per-cell `.style()`.
- No regex crate — parse by hand.
- Run all tests with `cargo test --quiet`; lint with `cargo clippy --quiet`.

---

## Task 1: `note` module — pure markdown/task logic

**Files:**
- Create: `src/note.rs`
- Modify: `src/lib.rs` (add `pub mod note;`)

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod note;
```

- [ ] **Step 2: Write `src/note.rs` with the parser and failing tests**

```rust
//! Pure markdown-note logic: parse lines, count tasks, toggle checkboxes, and
//! extract selected tasks as a numbered list. No UI, no IO — all functions are
//! deterministic and unit-tested.

/// A single parsed line of a note. Only the subset we render specially is
/// distinguished; everything else is `Text`.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteLine {
    /// `- [ ] text` (open) or `- [x] text` (done).
    Task { done: bool, text: String },
    /// `# text` .. `###### text`; `level` is the number of leading `#`.
    Heading { level: u8, text: String },
    /// `- text` or `* text` (a non-task bullet).
    Bullet(String),
    /// Any other non-empty line.
    Text(String),
    /// An empty line.
    Blank,
}

/// If `line` is a checkbox task, returns `(done, text)` with the `- [ ] ` prefix
/// stripped. Leading whitespace is allowed before the dash.
fn parse_task(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim_start();
    let b = trimmed.as_bytes();
    // Need at least "- [x]" (5 bytes) then the body.
    if b.len() >= 5 && &trimmed[..3] == "- [" && b[4] == b']' {
        let done = match b[3] {
            b' ' => false,
            b'x' | b'X' => true,
            _ => return None,
        };
        return Some((done, trimmed[5..].trim_start().to_string()));
    }
    None
}

/// Parse a whole note buffer into typed lines (split on `\n`).
pub fn parse(buf: &str) -> Vec<NoteLine> {
    buf.split('\n').map(parse_line).collect()
}

fn parse_line(line: &str) -> NoteLine {
    if let Some((done, text)) = parse_task(line) {
        return NoteLine::Task { done, text };
    }
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return NoteLine::Blank;
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        let extra = rest.chars().take_while(|&c| c == '#').count();
        let level = (1 + extra).min(6) as u8;
        let text = trimmed[level as usize..].trim_start().to_string();
        return NoteLine::Heading { level, text };
    }
    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        return NoteLine::Bullet(rest.to_string());
    }
    NoteLine::Text(line.to_string())
}

/// `(done, total)` task counts for the card progress indicator.
pub fn counts(buf: &str) -> (u32, u32) {
    let mut done = 0;
    let mut total = 0;
    for line in buf.split('\n') {
        if let Some((d, _)) = parse_task(line) {
            total += 1;
            if d {
                done += 1;
            }
        }
    }
    (done, total)
}

/// Buffer line indices (0-based, over `split('\n')`) that are tasks, in order.
/// The position in the returned vec is the task ordinal.
pub fn task_line_indices(buf: &str) -> Vec<usize> {
    buf.split('\n')
        .enumerate()
        .filter(|(_, l)| parse_task(l).is_some())
        .map(|(i, _)| i)
        .collect()
}

/// Flip the `[ ]` <-> `[x]` checkbox of the `ord`-th task (0-based). No-op if
/// `ord` is out of range. Preserves all other text and line structure.
pub fn toggle(buf: &mut String, ord: usize) {
    let mut seen = 0;
    let lines: Vec<String> = buf
        .split('\n')
        .map(|line| {
            if let Some((done, _)) = parse_task(line) {
                let out = if seen == ord {
                    let lead_len = line.len() - line.trim_start().len();
                    let (lead, rest) = line.split_at(lead_len);
                    let mark = if done { ' ' } else { 'x' };
                    // `rest` starts with "- [x]" (5 ASCII bytes).
                    format!("{lead}- [{mark}]{}", &rest[5..])
                } else {
                    line.to_string()
                };
                seen += 1;
                out
            } else {
                line.to_string()
            }
        })
        .collect();
    *buf = lines.join("\n");
}

/// Render the given task ordinals as a numbered list (`"1. text\n2. text"`),
/// stripping the `- [ ]` prefix. Includes every requested task regardless of
/// done state, renumbered from 1 in the given order. Unknown ordinals skipped.
pub fn selected_as_numbered(buf: &str, ords: &[usize]) -> String {
    let texts: Vec<String> = buf
        .split('\n')
        .filter_map(|l| parse_task(l).map(|(_, t)| t))
        .collect();
    let mut out = String::new();
    let mut n = 1;
    for &ord in ords {
        if let Some(t) = texts.get(ord) {
            if n > 1 {
                out.push('\n');
            }
            out.push_str(&format!("{n}. {t}"));
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_line_kind() {
        let buf = "# Title\n- [ ] open\n- [x] done\n- bullet\nplain\n";
        let lines = parse(buf);
        assert_eq!(lines[0], NoteLine::Heading { level: 1, text: "Title".into() });
        assert_eq!(lines[1], NoteLine::Task { done: false, text: "open".into() });
        assert_eq!(lines[2], NoteLine::Task { done: true, text: "done".into() });
        assert_eq!(lines[3], NoteLine::Bullet("bullet".into()));
        assert_eq!(lines[4], NoteLine::Text("plain".into()));
        assert_eq!(lines[5], NoteLine::Blank); // trailing newline => empty last line
    }

    #[test]
    fn uppercase_x_is_done() {
        assert_eq!(parse("- [X] hi")[0], NoteLine::Task { done: true, text: "hi".into() });
    }

    #[test]
    fn counts_tasks() {
        assert_eq!(counts("- [ ] a\n- [x] b\ntext\n- [x] c"), (2, 3));
        assert_eq!(counts("no tasks here"), (0, 0));
    }

    #[test]
    fn task_line_indices_maps_ordinals_to_lines() {
        // lines: 0 heading, 1 task, 2 blank, 3 task
        assert_eq!(task_line_indices("# h\n- [ ] a\n\n- [x] b"), vec![1, 3]);
    }

    #[test]
    fn toggle_flips_the_nth_task_only() {
        let mut buf = "- [ ] a\n- [ ] b".to_string();
        toggle(&mut buf, 1);
        assert_eq!(buf, "- [ ] a\n- [x] b");
        toggle(&mut buf, 1); // idempotent flip back
        assert_eq!(buf, "- [ ] a\n- [ ] b");
    }

    #[test]
    fn toggle_preserves_leading_whitespace() {
        let mut buf = "  - [ ] indented".to_string();
        toggle(&mut buf, 0);
        assert_eq!(buf, "  - [x] indented");
    }

    #[test]
    fn selected_as_numbered_strips_prefix_and_renumbers() {
        let buf = "- [ ] first\ntext\n- [x] second\n- [ ] third";
        // ords are task ordinals: 0=first, 1=second, 2=third
        assert_eq!(selected_as_numbered(buf, &[0, 2]), "1. first\n2. third");
        assert_eq!(selected_as_numbered(buf, &[1]), "1. second");
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --quiet note::`
Expected: PASS (all `note::tests::*`).

- [ ] **Step 4: Lint**

Run: `cargo clippy --quiet`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/note.rs src/lib.rs
git commit -m "feat(note): pure markdown task parsing, counts, toggle, copy-format"
```

---

## Task 2: `clip` module — pbcopy wrapper

**Files:**
- Create: `src/clip.rs`
- Modify: `src/lib.rs` (add `pub mod clip;`)

- [ ] **Step 1: Register the module**

In `src/lib.rs`:

```rust
pub mod clip;
```

- [ ] **Step 2: Write `src/clip.rs`**

```rust
//! Clipboard via macOS `pbcopy`. Best-effort: any failure is silently ignored,
//! same as the other shell-outs in this app. Formatting is done by the caller
//! (`note::selected_as_numbered`); this only pipes a string to the clipboard.
use std::io::Write;
use std::process::{Command, Stdio};

/// Copy `text` to the system clipboard via `pbcopy`. No-op on failure.
pub fn copy(text: &str) {
    let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build --quiet`
Expected: builds (no test — the shell-out is not unit-tested; the tested seam is `note::selected_as_numbered`).

- [ ] **Step 4: Commit**

```bash
git add src/clip.rs src/lib.rs
git commit -m "feat(clip): best-effort pbcopy wrapper"
```

---

## Task 3: Persist `inbox` + `notes` in `state.toml`

**Files:**
- Modify: `src/state.rs` (struct `State`, ~lines 12-26; tests at bottom)
- Modify: `src/app.rs` (`apply_state` ~605, `snapshot_state` ~615; `App` struct fields; `App::new`)

- [ ] **Step 1: Add fields to `State`**

In `src/state.rs`, inside `pub struct State { ... }`, after `project_names`:

```rust
    /// Global "Inbox" note (markdown). Empty by default.
    pub inbox: String,
    /// Per-session notes (markdown), keyed by tmux session name. BTreeMap →
    /// deterministic file output.
    pub notes: BTreeMap<String, String>,
```

(`#[serde(default)]` on the struct already makes these backward-compatible.)

- [ ] **Step 2: Add a round-trip test in `src/state.rs`**

Add to the `mod tests`:

```rust
    #[test]
    fn notes_round_trip_through_toml() {
        let mut s = State::default();
        s.inbox = "# today\n- [ ] ship".into();
        s.notes.insert("proj".into(), "- [x] done".into());
        let text = toml::to_string(&s).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(back.inbox, s.inbox);
        assert_eq!(back.notes.get("proj").map(String::as_str), Some("- [x] done"));
    }

    #[test]
    fn missing_notes_fields_default_empty() {
        // An old state file with no inbox/notes keys still loads.
        let s: State = toml::from_str("split_pct = 40").unwrap();
        assert_eq!(s.inbox, "");
        assert!(s.notes.is_empty());
    }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --quiet state::`
Expected: PASS.

- [ ] **Step 4: Add `App` fields**

In `src/app.rs`, in `pub struct App { ... }`, add near the other persisted fields:

```rust
    /// Global Inbox note (markdown).
    pub inbox: String,
    /// Per-session notes (markdown), keyed by tmux session name.
    pub notes: std::collections::BTreeMap<String, String>,
    /// Which content the right pane shows.
    pub right_pane: RightPane,
```

In `App::new(...)`, initialise them:

```rust
            inbox: String::new(),
            notes: std::collections::BTreeMap::new(),
            right_pane: RightPane::Preview,
```

Define the enum near the top of `src/app.rs` (by the other UI enums):

```rust
/// What the right pane renders: the live session preview, the selected session's
/// note, or the global Inbox note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    Preview,
    SessionNote,
    Inbox,
}
```

- [ ] **Step 5: Wire `apply_state` / `snapshot_state`**

In `apply_state`, after `self.project_names = state.project_names;`:

```rust
        self.inbox = state.inbox;
        self.notes = state.notes;
```

In `snapshot_state`, add to the `State { ... }` literal:

```rust
            inbox: self.inbox.clone(),
            notes: self.notes.clone(),
```

- [ ] **Step 6: Build + test**

Run: `cargo test --quiet state:: && cargo build --quiet`
Expected: PASS / builds.

- [ ] **Step 7: Commit**

```bash
git add src/state.rs src/app.rs
git commit -m "feat(state): persist inbox + per-session notes"
```

---

## Task 4: Extract a shared `TextArea` editor from `ReplyForm`

**Goal:** A reusable multi-line editing buffer so the note editor doesn't duplicate `ReplyForm`. `ReplyForm` keeps its behavior; its editing methods move to `TextArea`.

**Files:**
- Create: `src/editor.rs`
- Modify: `src/lib.rs` (add `pub mod editor;`)
- Modify: `src/app.rs` (`ReplyForm` struct ~300-304, its impl ~306-447, `handle_reply_key` ~1011, reply tests)
- Modify: `src/ui/mod.rs` (`draw_reply_modal` reads `form.buffer`/`form.cursor`)

- [ ] **Step 1: Create `src/editor.rs` with `TextArea` + tests**

Move the char-indexed editing logic out of `ReplyForm`. Copy the method bodies verbatim from `ReplyForm` (in `src/app.rs`), renaming `self.buffer`/`self.cursor` to the `TextArea` fields:

```rust
//! Reusable multi-line text buffer with a character-indexed cursor. Newlines are
//! stored literally in `buffer`. Shared by the reply composer and the note editor.

#[derive(Debug, Clone, Default)]
pub struct TextArea {
    pub buffer: String,
    /// Cursor as a character index into `buffer` (not a byte offset).
    pub cursor: usize,
}

impl TextArea {
    pub fn new(initial: impl Into<String>) -> Self {
        let buffer = initial.into();
        let cursor = buffer.chars().count();
        Self { buffer, cursor }
    }

    pub fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    // NOTE: paste the EXACT bodies of these methods from the current ReplyForm
    // impl in src/app.rs, changing only `self.buffer`/`self.cursor` field access
    // (already the field names here): insert_char, insert_str, backspace, delete,
    // left, right, line_bounds, home, end, up, down, delete_word,
    // delete_to_line_start.
    // (Do not rewrite the logic — copy it so behavior is identical.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_track_cursor() {
        let mut a = TextArea::default();
        a.insert_char('h');
        a.insert_char('i');
        assert_eq!(a.buffer, "hi");
        assert_eq!(a.cursor, 2);
        a.backspace();
        assert_eq!(a.buffer, "h");
        assert_eq!(a.cursor, 1);
    }

    #[test]
    fn newline_then_up_down_preserve_column() {
        let mut a = TextArea::new("abc");
        a.insert_char('\n');
        a.insert_str("de");
        assert_eq!(a.buffer, "abc\nde");
        a.up();
        a.down();
        // round-trip shouldn't panic or lose the buffer
        assert_eq!(a.buffer, "abc\nde");
    }
}
```

- [ ] **Step 2: Register the module**

In `src/lib.rs`:

```rust
pub mod editor;
```

- [ ] **Step 3: Make `ReplyForm` wrap a `TextArea`**

In `src/app.rs`, change the struct:

```rust
pub struct ReplyForm {
    pub name: String,
    pub area: crate::editor::TextArea,
}
```

Replace the `impl ReplyForm` editing methods with delegations (keep `new`):

```rust
impl ReplyForm {
    pub fn new(name: String) -> Self {
        Self { name, area: crate::editor::TextArea::default() }
    }
    pub fn insert_char(&mut self, c: char) { self.area.insert_char(c) }
    pub fn insert_str(&mut self, s: &str) { self.area.insert_str(s) }
    pub fn backspace(&mut self) { self.area.backspace() }
    pub fn delete(&mut self) { self.area.delete() }
    pub fn left(&mut self) { self.area.left() }
    pub fn right(&mut self) { self.area.right() }
    pub fn home(&mut self) { self.area.home() }
    pub fn end(&mut self) { self.area.end() }
    pub fn up(&mut self) { self.area.up() }
    pub fn down(&mut self) { self.area.down() }
    pub fn delete_word(&mut self) { self.area.delete_word() }
    pub fn delete_to_line_start(&mut self) { self.area.delete_to_line_start() }
}
```

(`handle_reply_key` calls these methods, so it keeps compiling. The send path reads `form.area.buffer` — update that call site and the `ReplyForm::new(name)` callers if their signature changed: `ReplyForm::new` now takes just `name`.)

- [ ] **Step 4: Update reply read sites**

- In `handle_reply_key` (`src/app.rs` ~1032), where the buffer is sent, change `form.buffer` → `form.area.buffer`.
- In `src/ui/mod.rs` `draw_reply_modal`, change `form.buffer` → `form.area.buffer` and `form.cursor` → `form.area.cursor`.
- In the reply tests in `src/app.rs`, change `app`/`form` buffer assertions from `form.buffer` to `form.area.buffer` (and `.cursor` likewise).

- [ ] **Step 5: Build + run all tests**

Run: `cargo test --quiet`
Expected: PASS — reply behavior unchanged, `editor::tests::*` green.

- [ ] **Step 6: Lint + commit**

```bash
cargo clippy --quiet
git add src/editor.rs src/lib.rs src/app.rs src/ui/mod.rs
git commit -m "refactor(editor): extract TextArea from ReplyForm for reuse"
```

---

## Task 5: `Mode::Note` + note state + App helpers

**Files:**
- Modify: `src/app.rs` (`Mode` enum ~450, `ModeKind` ~515, `mode_kind` ~793, `handle_key` ~807)

- [ ] **Step 1: Add the note state types**

Near the other form structs in `src/app.rs`:

```rust
/// Which note `Mode::Note` is editing.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteTarget {
    Inbox,
    Session(String),
}

/// Render vs raw-edit sub-mode inside a focused note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSub {
    Render,
    Edit,
}

/// Focused-note state (the user pressed Tab into the note pane).
#[derive(Debug, Clone)]
pub struct NoteState {
    pub target: NoteTarget,
    pub sub: NoteSub,
    /// Task ordinal the render cursor is on.
    pub cursor: usize,
    /// Visual-selection anchor (task ordinal), or None when not selecting.
    pub anchor: Option<usize>,
    /// Edit buffer; only meaningful in `Edit` sub-mode.
    pub editor: crate::editor::TextArea,
}
```

- [ ] **Step 2: Add the `Mode` + `ModeKind` variants**

In `pub enum Mode`, add:

```rust
    Note(NoteState),
```

In `enum ModeKind`, add:

```rust
    Note,
```

In `mode_kind`, add the arm:

```rust
            Mode::Note(_) => ModeKind::Note,
```

In `handle_key`, add the route:

```rust
            ModeKind::Note => self.handle_note_key(key),
```

- [ ] **Step 3: Add note-text accessors on `App`**

```rust
    /// The markdown text for a note target (read-only). Missing session = "".
    pub fn note_text(&self, target: &NoteTarget) -> &str {
        match target {
            NoteTarget::Inbox => &self.inbox,
            NoteTarget::Session(name) => self.notes.get(name).map(String::as_str).unwrap_or(""),
        }
    }

    /// Mutable handle to a note target, creating an empty session entry if needed.
    pub fn note_text_mut(&mut self, target: &NoteTarget) -> &mut String {
        match target {
            NoteTarget::Inbox => &mut self.inbox,
            NoteTarget::Session(name) => self.notes.entry(name.clone()).or_default(),
        }
    }
```

- [ ] **Step 4: Add a stub `handle_note_key` so it compiles**

```rust
    fn handle_note_key(&mut self, _key: KeyEvent) -> Option<Action> {
        // Filled in by Tasks 7 and 8.
        None
    }
```

- [ ] **Step 5: Build**

Run: `cargo build --quiet`
Expected: builds (the `Mode::Note` arm in `src/ui/mod.rs`'s overlay match may error with "non-exhaustive" — add `Mode::Note(_) => {}` to the no-overlay arm now to keep it compiling; the real render comes in Task 10).

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/ui/mod.rs
git commit -m "feat(app): Mode::Note, NoteState, note-text accessors"
```

---

## Task 6: Enter notes mode (`t`/`T`/`Tab`) + kill/rename hooks

**Files:**
- Modify: `src/app.rs` (`handle_list_key` ~860)
- Modify: `src/main.rs` (`Action::Kill` ~255, `Action::Rename` ~277)

- [ ] **Step 1: Write failing tests in `src/app.rs`**

Add to `mod tests` (these helpers already exist: `App::new`, `Config::default`, a `key(c)` helper, and `Mode`/`RightPane` are in scope via `use super::*`):

```rust
    #[test]
    fn t_toggles_session_note_pane() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        app.handle_key(key('t'));
        assert_eq!(app.right_pane, RightPane::SessionNote);
        app.handle_key(key('t'));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn shift_t_toggles_inbox_pane() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.handle_key(key('T'));
        assert_eq!(app.right_pane, RightPane::Inbox);
        app.handle_key(key('T'));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn tab_focuses_the_shown_session_note() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        app.handle_key(key('t')); // show session note
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.target, NoteTarget::Session("s".into())),
            other => panic!("expected Mode::Note, got {other:?}"),
        }
    }
```

(Use the existing `at(name, dir)` / `app_with(sessions)` test helpers in `src/app.rs`; if `at` isn't present, build sessions the same way the neighbouring tests do.)

- [ ] **Step 2: Run to confirm they fail**

Run: `cargo test --quiet t_toggles_session_note_pane shift_t_toggles_inbox_pane tab_focuses`
Expected: FAIL (keys not handled yet).

- [ ] **Step 3: Handle `t`/`T`/`Tab` in `handle_list_key`**

Add arms (place before the catch-all). `t` and `T` are plain `Char`; `Tab` only acts when a note is shown:

```rust
            KeyCode::Char('t') => {
                self.right_pane = match self.right_pane {
                    RightPane::SessionNote => RightPane::Preview,
                    _ => RightPane::SessionNote,
                };
            }
            KeyCode::Char('T') => {
                self.right_pane = match self.right_pane {
                    RightPane::Inbox => RightPane::Preview,
                    _ => RightPane::Inbox,
                };
            }
            KeyCode::Tab if self.right_pane != RightPane::Preview => {
                let target = match self.right_pane {
                    RightPane::Inbox => NoteTarget::Inbox,
                    _ => match self.selected_name() {
                        Some(name) => NoteTarget::Session(name),
                        None => return None, // nothing selected; nothing to focus
                    },
                };
                self.mode = Mode::Note(NoteState {
                    target,
                    sub: NoteSub::Render,
                    cursor: 0,
                    anchor: None,
                    editor: crate::editor::TextArea::default(),
                });
            }
```

(`selected_name()` already exists and returns the focused session's name.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet t_toggles_session_note_pane shift_t_toggles_inbox_pane tab_focuses`
Expected: PASS.

- [ ] **Step 5: Hook kill + rename in `src/main.rs`**

In `Action::Kill` handler, after the kill succeeds (just before `app.refresh();`):

```rust
            app.notes.remove(&name);
            app.dirty = true;
```

In `Action::Rename` handler, after a successful rename (inside the `if rename ok` path, before `app.refresh();`):

```rust
            if let Some(text) = app.notes.remove(&old) {
                app.notes.insert(new.clone(), text);
                app.dirty = true;
            }
```

(`name`, `old`, `new` are already bound in those arms.)

- [ ] **Step 6: Add app-level tests for kill/rename note handling**

Since kill/rename run in `main.rs`'s `handle_action`, test the data ops directly in `src/app.rs` tests:

```rust
    #[test]
    fn killing_a_session_drops_its_note() {
        let mut app = App::new(Config::default());
        app.notes.insert("s".into(), "- [ ] x".into());
        app.notes.remove("s"); // mirrors the Kill handler
        assert!(app.notes.get("s").is_none());
    }

    #[test]
    fn renaming_moves_the_note() {
        let mut app = App::new(Config::default());
        app.notes.insert("old".into(), "- [ ] x".into());
        if let Some(t) = app.notes.remove("old") {
            app.notes.insert("new".into(), t);
        }
        assert_eq!(app.notes.get("new").map(String::as_str), Some("- [ ] x"));
        assert!(app.notes.get("old").is_none());
    }
```

- [ ] **Step 7: Build + test + commit**

```bash
cargo test --quiet && cargo clippy --quiet
git add src/app.rs src/main.rs
git commit -m "feat(notes): t/T/Tab enter notes mode; kill drops note, rename moves it"
```

---

## Task 7: `handle_note_key` — render sub-mode

**Files:**
- Modify: `src/app.rs` (`handle_note_key`)

Cursor moves over task ordinals; `space` toggles; `V` selects; `y` copies; `e` edits; `esc` exits.

- [ ] **Step 1: Write failing tests**

```rust
    fn note_app_with(text: &str) -> App {
        let mut app = App::new(Config::default());
        app.inbox = text.into();
        app.mode = Mode::Note(NoteState {
            target: NoteTarget::Inbox,
            sub: NoteSub::Render,
            cursor: 0,
            anchor: None,
            editor: crate::editor::TextArea::default(),
        });
        app
    }
    fn note_state(app: &App) -> &NoteState {
        match &app.mode { Mode::Note(ns) => ns, _ => panic!("not in note mode") }
    }

    #[test]
    fn j_k_move_task_cursor_within_bounds() {
        let mut app = note_app_with("- [ ] a\n- [ ] b\n- [ ] c");
        app.handle_key(key('j'));
        assert_eq!(note_state(&app).cursor, 1);
        app.handle_key(key('j'));
        app.handle_key(key('j')); // clamp at last task (index 2)
        assert_eq!(note_state(&app).cursor, 2);
        app.handle_key(key('k'));
        assert_eq!(note_state(&app).cursor, 1);
    }

    #[test]
    fn space_toggles_task_under_cursor() {
        let mut app = note_app_with("- [ ] a\n- [ ] b");
        app.handle_key(key('j'));            // cursor on task 1
        app.handle_key(key(' '));
        assert_eq!(app.inbox, "- [ ] a\n- [x] b");
    }

    #[test]
    fn visual_select_then_space_toggles_range() {
        let mut app = note_app_with("- [ ] a\n- [ ] b\n- [ ] c");
        app.handle_key(key('V'));            // anchor at 0
        app.handle_key(key('j'));            // extend to 1
        app.handle_key(key(' '));            // toggle 0..=1
        assert_eq!(app.inbox, "- [x] a\n- [x] b\n- [ ] c");
        assert!(note_state(&app).anchor.is_none(), "selection cleared after toggle");
    }

    #[test]
    fn e_enters_edit_seeded_from_note() {
        let mut app = note_app_with("- [ ] a");
        app.handle_key(key('e'));
        let ns = note_state(&app);
        assert_eq!(ns.sub, NoteSub::Edit);
        assert_eq!(ns.editor.buffer, "- [ ] a");
    }

    #[test]
    fn esc_exits_to_list() {
        let mut app = note_app_with("- [ ] a");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --quiet j_k_move_task_cursor space_toggles visual_select e_enters_edit esc_exits`
Expected: FAIL.

- [ ] **Step 3: Implement the render branch of `handle_note_key`**

Replace the stub:

```rust
    fn handle_note_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Note(mut ns) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match ns.sub {
            NoteSub::Render => {
                let task_count = crate::note::task_line_indices(self.note_text(&ns.target)).len();
                let last = task_count.saturating_sub(1);
                match key.code {
                    KeyCode::Esc => {
                        if ns.anchor.is_some() {
                            ns.anchor = None; // first esc clears selection
                            self.mode = Mode::Note(ns);
                        }
                        // else: fall through, leaving Mode::List (exits focus)
                        return None;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        ns.cursor = (ns.cursor + 1).min(last);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        ns.cursor = ns.cursor.saturating_sub(1);
                    }
                    KeyCode::Char('V') => {
                        ns.anchor = Some(ns.cursor);
                    }
                    KeyCode::Char(' ') => {
                        let range = selection_range(&ns);
                        let buf = self.note_text_mut(&ns.target);
                        for ord in range {
                            crate::note::toggle(buf, ord);
                        }
                        ns.anchor = None;
                        self.dirty = true;
                    }
                    KeyCode::Char('y') => {
                        let ords: Vec<usize> = selection_range(&ns).collect();
                        let text = crate::note::selected_as_numbered(self.note_text(&ns.target), &ords);
                        crate::clip::copy(&text);
                        ns.anchor = None;
                    }
                    KeyCode::Char('e') => {
                        ns.editor = crate::editor::TextArea::new(self.note_text(&ns.target).to_string());
                        ns.sub = NoteSub::Edit;
                    }
                    _ => {}
                }
                self.mode = Mode::Note(ns);
                None
            }
            NoteSub::Edit => {
                // Implemented in Task 8.
                self.mode = Mode::Note(ns);
                None
            }
        }
    }
```

Add the free helper (inclusive ordinal range for the current selection or just the cursor):

```rust
/// The task ordinals covered by the current selection (anchor..=cursor), or just
/// the cursor when nothing is selected.
fn selection_range(ns: &NoteState) -> std::ops::RangeInclusive<usize> {
    match ns.anchor {
        Some(a) => a.min(ns.cursor)..=a.max(ns.cursor),
        None => ns.cursor..=ns.cursor,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet j_k_move_task_cursor space_toggles visual_select e_enters_edit esc_exits`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo test --quiet && cargo clippy --quiet
git add src/app.rs
git commit -m "feat(notes): render-mode cursor, toggle, visual select, copy, esc"
```

---

## Task 8: `handle_note_key` — edit sub-mode

**Files:**
- Modify: `src/app.rs` (`handle_note_key` Edit branch)

- [ ] **Step 1: Write failing tests**

```rust
    fn note_app_editing(text: &str) -> App {
        let mut app = App::new(Config::default());
        app.inbox = text.into();
        app.mode = Mode::Note(NoteState {
            target: NoteTarget::Inbox,
            sub: NoteSub::Edit,
            cursor: 0,
            anchor: None,
            editor: crate::editor::TextArea::new(text.to_string()),
        });
        app
    }

    #[test]
    fn typing_in_edit_writes_back_to_note() {
        let mut app = note_app_editing("- [ ] a");
        app.handle_key(key('!'));            // appended at end (cursor at end)
        // esc returns to render and writes the buffer back
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.inbox, "- [ ] a!");
        assert_eq!(note_state(&app).sub, NoteSub::Render);
    }

    #[test]
    fn enter_inserts_newline_not_submit() {
        let mut app = note_app_editing("a");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(key('b'));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.editor.buffer, "a\nb"),
            _ => panic!("still editing"),
        }
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --quiet typing_in_edit enter_inserts_newline`
Expected: FAIL.

- [ ] **Step 3: Implement the Edit branch**

Replace the `NoteSub::Edit => { ... }` placeholder body:

```rust
            NoteSub::Edit => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => {
                        // Commit the edited buffer back to the note, re-parse on render.
                        *self.note_text_mut(&ns.target) = ns.editor.buffer.clone();
                        self.dirty = true;
                        ns.cursor = 0;
                        ns.anchor = None;
                        ns.sub = NoteSub::Render;
                    }
                    KeyCode::Enter => ns.editor.insert_char('\n'),
                    KeyCode::Char('w') if ctrl => ns.editor.delete_word(),
                    KeyCode::Char('u') if ctrl => ns.editor.delete_to_line_start(),
                    KeyCode::Char(c) if !ctrl => ns.editor.insert_char(c),
                    KeyCode::Backspace => ns.editor.backspace(),
                    KeyCode::Delete => ns.editor.delete(),
                    KeyCode::Left => ns.editor.left(),
                    KeyCode::Right => ns.editor.right(),
                    KeyCode::Up => ns.editor.up(),
                    KeyCode::Down => ns.editor.down(),
                    KeyCode::Home => ns.editor.home(),
                    KeyCode::End => ns.editor.end(),
                    _ => {}
                }
                self.mode = Mode::Note(ns);
                None
            }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet typing_in_edit enter_inserts_newline`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo test --quiet && cargo clippy --quiet
git add src/app.rs
git commit -m "feat(notes): edit sub-mode reuses TextArea, esc commits + reparses"
```

---

## Task 9: `ui::note` — render the note pane

**Files:**
- Create: `src/ui/note.rs`
- Modify: `src/ui/mod.rs` (add `mod note;`)

Renders header + body. Render mode: each `NoteLine` styled; task cursor and visual selection highlighted. Edit mode: reuse `wrap_rows`/`cursor_rowcol` from `src/ui/mod.rs` (make them `pub(crate)` if not already).

- [ ] **Step 1: Make the editor wrap helpers reachable**

In `src/ui/mod.rs`, ensure `wrap_rows` and `cursor_rowcol` are `pub(crate)` (change `fn` → `pub(crate) fn`).

- [ ] **Step 2: Create `src/ui/note.rs`**

```rust
use crate::app::{App, Mode, NoteSub, NoteTarget, RightPane};
use crate::note::{self, NoteLine};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The note target currently shown in the pane (focused note wins; otherwise the
/// pane's view mode + selection decide).
fn shown_target(app: &App) -> Option<NoteTarget> {
    if let Mode::Note(ns) = &app.mode {
        return Some(ns.target.clone());
    }
    match app.right_pane {
        RightPane::Inbox => Some(NoteTarget::Inbox),
        RightPane::SessionNote => app.selected_name().map(NoteTarget::Session),
        RightPane::Preview => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // rule
        Constraint::Min(0),    // body
    ])
    .split(area);

    let Some(target) = shown_target(app) else {
        return;
    };
    let text = app.note_text(&target).to_string();
    let (done, total) = note::counts(&text);
    let title = match &target {
        NoteTarget::Inbox => "Inbox".to_string(),
        NoteTarget::Session(name) => name.clone(),
    };
    let hint = match &app.mode {
        Mode::Note(ns) if ns.sub == NoteSub::Edit => "edit — esc done",
        Mode::Note(_) => "j/k · space · V · y · e edit · esc",
        _ => "Tab to edit",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("   {done}/{total}"), Style::default().add_modifier(Modifier::DIM)),
            Span::styled(format!("   {hint}"), Style::default().add_modifier(Modifier::DIM)),
        ])),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[1],
    );

    // Edit mode: raw editor with wrapped text + hardware cursor.
    if let Mode::Note(ns) = &app.mode {
        if ns.sub == NoteSub::Edit {
            super::render_editor(f, rows[2], &ns.editor);
            return;
        }
    }

    // Render mode: styled markdown with cursor/selection highlight over tasks.
    let focused = matches!(&app.mode, Mode::Note(_));
    let (cur, sel) = match &app.mode {
        Mode::Note(ns) => (Some(ns.cursor), crate::app::selection_set(ns)),
        _ => (None, std::collections::HashSet::new()),
    };
    let mut task_ord = 0usize;
    let lines: Vec<Line> = note::parse(&text)
        .into_iter()
        .map(|nl| {
            let line = render_line(&nl, task_ord, focused, cur, &sel);
            if matches!(nl, NoteLine::Task { .. }) {
                task_ord += 1;
            }
            line
        })
        .collect();
    f.render_widget(Paragraph::new(lines), rows[2]);
}

fn render_line(
    nl: &NoteLine,
    ord: usize,
    focused: bool,
    cursor: Option<usize>,
    sel: &std::collections::HashSet<usize>,
) -> Line<'static> {
    match nl {
        NoteLine::Task { done, text } => {
            let box_glyph = if *done { "☑" } else { "☐" };
            let on_cursor = focused && cursor == Some(ord);
            let selected = sel.contains(&ord);
            let mut style = Style::default();
            if *done {
                style = style.add_modifier(Modifier::DIM | Modifier::CROSSED_OUT);
            }
            if selected {
                style = style.bg(th::SEL_BG);
            }
            let bar = if on_cursor { "› " } else { "  " };
            Line::from(vec![
                Span::styled(bar.to_string(), Style::default().add_modifier(Modifier::DIM)),
                Span::styled(format!("{box_glyph} {text}"), style),
            ])
        }
        NoteLine::Heading { text, .. } => Line::from(Span::styled(
            text.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        NoteLine::Bullet(text) => Line::from(format!("  • {text}")),
        NoteLine::Text(t) => Line::from(format!("  {t}")),
        NoteLine::Blank => Line::from(""),
    }
}
```

- [ ] **Step 3: Add the small `App`/`ui` helpers referenced above**

In `src/app.rs`, add a pub helper next to `selection_range` (returns the selected ordinals as a set for the renderer):

```rust
/// Task ordinals currently selected (for render highlight). Empty when not in a
/// note or no selection is active.
pub fn selection_set(ns: &NoteState) -> std::collections::HashSet<usize> {
    match ns.anchor {
        Some(_) => selection_range(ns).collect(),
        None => std::collections::HashSet::new(),
    }
}
```

In `src/ui/mod.rs`, extract the text-rendering body of `draw_reply_modal` (the part that calls `wrap_rows`, computes the vertical scroll so the cursor row stays visible, renders the wrapped `Paragraph`, and places the hardware cursor with `f.set_cursor_position`) into a shared function, then have `draw_reply_modal` call it. The function takes a `TextArea` instead of reading `form.area` directly:

```rust
/// Render an editable text buffer into `area`: wrapped text + a hardware cursor
/// kept on-screen. Shared by the reply modal and the note editor.
pub(crate) fn render_editor(f: &mut Frame, area: Rect, ta: &crate::editor::TextArea) {
    let chars: Vec<char> = ta.buffer.chars().collect();
    let rows = wrap_rows(&chars, area.width as usize);
    let (crow, ccol) = cursor_rowcol(&rows, ta.cursor);
    let visible = area.height as usize;
    let scroll = if crow >= visible { crow - visible + 1 } else { 0 };
    let lines: Vec<Line> = rows
        .iter()
        .skip(scroll)
        .take(visible)
        .map(|(_, text)| Line::from(text.clone()))
        .collect();
    f.render_widget(Paragraph::new(lines), area);
    if crow >= scroll {
        let cx = area.x + ccol.min(area.width.saturating_sub(1) as usize) as u16;
        let cy = area.y + (crow - scroll) as u16;
        f.set_cursor_position((cx, cy));
    }
}
```

Then in `draw_reply_modal`, replace its inline wrap/scroll/cursor block with a call to `render_editor(f, text_area, &form.area)`. Keep the empty-buffer placeholder behavior if present. (Adjust the exact field/variable names to match the current `draw_reply_modal`; the helpers `wrap_rows` and `cursor_rowcol` were made `pub(crate)` in Step 1.)

- [ ] **Step 4: Register the module**

In `src/ui/mod.rs`, add near the other `mod` lines:

```rust
mod note;
```

- [ ] **Step 5: Snapshot tests in `src/ui/note.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode, NoteState, NoteSub, NoteTarget, RightPane};
    use crate::config::Config;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn dump(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf[(x, y)].symbol()); }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_checkboxes_and_counter() {
        let mut app = App::new(Config::default());
        app.inbox = "# Today\n- [ ] open\n- [x] done".into();
        app.right_pane = RightPane::Inbox;
        let mut t = Terminal::new(TestBackend::new(40, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = dump(t.backend().buffer());
        assert!(s.contains("Inbox"), "title:\n{s}");
        assert!(s.contains("1/2"), "counter:\n{s}");
        assert!(s.contains("☐ open"), "open box:\n{s}");
        assert!(s.contains("☑ done"), "done box:\n{s}");
    }
}
```

- [ ] **Step 6: Run + commit**

```bash
cargo test --quiet note && cargo clippy --quiet
git add src/ui/note.rs src/ui/mod.rs src/app.rs
git commit -m "feat(ui): render the note pane (markdown, checkboxes, selection)"
```

---

## Task 10: Wire the pane + footer hints

**Files:**
- Modify: `src/ui/mod.rs` (`draw_body` right column; overlay match; nothing else)
- Modify: `src/ui/footer.rs` (`items_for`)

- [ ] **Step 1: Route the right column**

In `src/ui/mod.rs` `draw_body`, replace `preview::render(f, cols[2], app);` with:

```rust
    if app.right_pane == crate::app::RightPane::Preview && !matches!(app.mode, Mode::Note(_)) {
        preview::render(f, cols[2], app);
    } else {
        note::render(f, cols[2], app);
    }
```

- [ ] **Step 2: Footer hints**

In `src/ui/footer.rs` `items_for`, add a `Mode::Note` arm (branch on the sub-mode):

```rust
        Mode::Note(ns) => match ns.sub {
            crate::app::NoteSub::Edit => vec![
                ("esc", "done", true),
                ("enter", "newline", false),
            ],
            crate::app::NoteSub::Render => vec![
                ("j/k", "task", true),
                ("space", "toggle", false),
                ("V", "select", false),
                ("y", "copy", false),
                ("e", "edit", false),
                ("esc", "back", false),
            ],
        },
```

Also add `("t", "notes", false)` to the `Mode::List` arm so the entry point is discoverable.

- [ ] **Step 3: Build + run all tests**

Run: `cargo test --quiet && cargo clippy --quiet`
Expected: PASS, no warnings.

- [ ] **Step 4: Manual smoke (optional)**

Run the app, press `t` (note pane), `Tab` (focus), `e` (edit), type `- [ ] hi`, `esc`, `space` (toggle), `V`+`j`+`y` (copy), `esc`. Confirm clipboard has a numbered list.

- [ ] **Step 5: Commit**

```bash
git add src/ui/mod.rs src/ui/footer.rs
git commit -m "feat(ui): route right pane to notes; footer hints for note mode"
```

---

## Task 11: `done/total` counter on the session card

**Files:**
- Modify: `src/ui/sessions.rs` (`render`, `card`)

- [ ] **Step 1: Write a failing snapshot test in `src/ui/sessions.rs`**

```rust
    #[test]
    fn card_shows_task_counter_when_note_has_tasks() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        app.notes.insert("proj".into(), "- [ ] a\n- [x] b".into());
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("1/2"), "counter missing:\n{s}");
    }

    #[test]
    fn card_has_no_counter_without_tasks() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        // no note → no counter
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("/"), "unexpected counter:\n{s}");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test --quiet card_shows_task_counter card_has_no_counter`
Expected: FAIL.

- [ ] **Step 3: Pass the counter into `card`**

In `src/ui/sessions.rs` `render`, where each `card(...)` is built, compute the counts and pass them. Add a parameter to `card`:

```rust
    let (done, total) = app
        .notes
        .get(&s.name)
        .map(|t| crate::note::counts(t))
        .unwrap_or((0, 0));
```

Change the `card(...)` signature to accept two new params `done: u32, total: u32` (pass `done`/`total` from `render`). The counter must not break the diff's right-alignment (line 2 right-aligns `+a −b` to the card edge via the `left2`/`pad2` math). So:

- **When `s.git` is `Some`** (the branch+diff branch): build the counter as part of the LEFT run so the diff still right-aligns. Inside that block, right after the branch-name span is pushed and *before* the `let left2 = ...` line, push the counter and widen `left2` to include it:

```rust
        let counter = if total > 0 { format!("   {done}/{total}") } else { String::new() };
        if !counter.is_empty() {
            l2.push(Span::styled(counter.clone(), Style::default().add_modifier(Modifier::DIM)));
        }
```

Then add `+ counter.chars().count()` to the `left2` sum so `pad2` accounts for it:

```rust
        let left2 = INDENT.len()
            + 1
            + 1 + s.agent.chars().count()
            + 5
            + g.branch.chars().count()
            + counter.chars().count();
```

- **When `s.git` is `None`** (no branch/diff): simply append the counter to `l2` after the agent span. Add this after the `if let Some(g) = &s.git { ... }` block:

```rust
    if s.git.is_none() && total > 0 {
        l2.push(Span::styled(
            format!("   {done}/{total}"),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet card_shows_task_counter card_has_no_counter`
Expected: PASS.

- [ ] **Step 5: Full suite + lint + commit**

```bash
cargo test --quiet && cargo clippy --quiet
git add src/ui/sessions.rs
git commit -m "feat(ui): show done/total task counter on session cards"
```

---

## Final verification

- [ ] Run the full suite: `cargo test --quiet` → all green.
- [ ] Lint: `cargo clippy --quiet` → no warnings.
- [ ] Manual: `cargo run`, exercise `t`/`T`/`Tab`/`e`/`space`/`V`/`y`, kill a session and confirm its note is gone, rename a session and confirm the note follows.
