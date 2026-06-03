# Session Notes / To-Do — Design

**Date:** 2026-06-03
**App:** `am` (agent multiplexer, Rust + ratatui)
**Status:** Approved design, pending implementation plan

## Summary

Add an Obsidian-style markdown note to the right-hand pane, shown **instead of**
the live preview when the user toggles into "notes mode". Each session has its
own note; there is also one global **Inbox** note. A note has two sub-modes:
**render** (markdown displayed, checkboxes toggleable, tasks selectable/copyable)
and **edit** (raw markdown text editing). Navigation through the session list is
unchanged — the note shown simply follows the selected session, exactly like the
preview does today.

This reuses the existing session-list navigation and the preview pane real
estate, so it introduces no new navigation concepts. The only genuinely new
pieces are markdown rendering with toggleable checkboxes and a small visual-
selection/clipboard flow.

## Goals

- A quick daily to-do ("what I want / need to do today") living inside `am`.
- Per-session notes, plus one global Inbox note.
- Two modes: edit raw markdown, and render markdown with toggleable checkboxes.
- Task progress (`done/total`) visible on the session card without opening the note.
- In render mode: select one or more tasks (vim-style `V`), toggle them, and copy
  them to the clipboard as a **numbered list** (`1. … 2. …`) — not as markdown
  `- [ ]` — so pasting into an agent chat yields clean items.
- Reply to the agent (`i`) without leaving notes mode.

## Non-goals (YAGNI)

- Full markdown (links, tables, code blocks, inline bold/italic). Render subset is
  tasks + headings + bullets only.
- Cross-platform clipboard. Target is macOS/tmux → `pbcopy`.
- Separate on-disk `.md` files. Notes persist inside `state.toml`.
- Tabs / a dedicated full-screen modal (an earlier idea, dropped in favor of the
  pane-integrated, list-driven approach).

## User-facing behavior

### View modes of the right pane

The right pane has a view mode: `Preview` (default, current behavior),
`SessionNote`, or `Inbox`.

- `t` — show the selected session's note (`SessionNote`); if already in
  `SessionNote`, return to `Preview`. Moving the list selection (`j/k`, `1-9`)
  updates the shown note, just like preview.
- `T` — show the global `Inbox` note; if already in `Inbox`, return to `Preview`.
- The two keys also switch between notes directly: pressing `T` while in
  `SessionNote` goes to `Inbox`, and `t` while in `Inbox` goes to `SessionNote`.
- All other List-mode keys keep working exactly as today, including `i` (reply to
  agent), while a note is shown.

### Focusing a note

- `Tab` (when the pane shows a note) — focus into the note → `Mode::Note`,
  starting in the **render** sub-mode.

### Render sub-mode

- `j/k` — move the task cursor between task lines (non-task lines are skipped).
- `space` — toggle done on the task under the cursor.
- `V` — begin visual selection (anchor at cursor); `j/k` extend the range.
  - `space` — toggle done on all selected tasks.
  - `y` — copy selected tasks to the clipboard as a numbered list, then exit
    selection.
  - `esc` — cancel selection.
- `e` — switch to the **edit** sub-mode.
- `esc` (no selection active) — return focus to the list; the pane stays on the
  note.

### Edit sub-mode

- Full multi-line text editing (reuses the existing `Reply` editor): typing,
  backspace, `ctrl+w` / `ctrl+u`, arrow movement.
- `Enter` inserts a newline (does **not** submit).
- `esc` — return to render (the note is re-parsed).

### Session card counter

- A dim `3/5` (done/total, **no icon**) appears on line 2 of the session card,
  only when that session's note contains at least one task (`total > 0`).
- Exact placement is tweakable during implementation.

## Data model & persistence

```
State (state.toml):
  inbox: String                      // global note (serde default = "")
  notes: BTreeMap<String, String>    // key = tmux session name → markdown text
```

- A note is just a markdown `String` (same as the `Reply` buffer). Nothing
  structured is stored on disk.
- New `State` fields use `#[serde(default)]`, so existing `state.toml` files load
  unchanged.
- `App::apply_state` / `App::snapshot_state` copy `inbox` and `notes`. Edits and
  checkbox toggles set `app.dirty = true`, so the existing autosave path persists
  them.
- **Kill a session → remove its note** (`notes.remove(name)`), including the
  worktree-kill path. A killed session's tasks are done or irrelevant.
- **Rename a session → move its note** (`notes.remove(old)` then
  `notes.insert(new, text)`), so the note stays attached and a stale/empty note
  doesn't appear under the old name.

## Architecture & modules

Decomposed for isolation; each unit has one purpose and is independently testable.

### 1. `src/note.rs` — pure note logic (no UI, no IO)

```rust
enum NoteLine {
    Task { done: bool, text: String },
    Heading { level: u8, text: String },
    Bullet(String),
    Text(String),
    Blank,
}

fn parse(buf: &str) -> Vec<NoteLine>;
fn counts(buf: &str) -> (u32, u32);                 // (done, total) for "3/5"
fn task_line_indices(buf: &str) -> Vec<usize>;      // task ordinal → buffer line index
fn toggle(buf: &mut String, task_ord: usize);       // flip [ ] <-> [x] of the Nth task
fn selected_as_numbered(buf: &str, ords: impl IntoIterator<Item = usize>) -> String;
                                                    // "1. text\n2. text" (strips "- [ ]")
```

`selected_as_numbered` copies every selected task's text regardless of done
state, renumbering from 1 in selection order.

Task syntax: a line matching `^\s*- \[( |x|X)\] (.*)` is a task; `x`/`X` = done.

### 2. `src/state.rs`

Add `inbox: String` and `notes: BTreeMap<String, String>` (both `#[serde(default)]`).

### 3. `src/app.rs`

- `enum RightPane { Preview, SessionNote, Inbox }` — a field on `App`.
- `enum Mode` gains `Note(NoteState)`, where `NoteState { sub: NoteSub, cursor:
  usize, anchor: Option<usize>, editor: <shared editor> }` and
  `enum NoteSub { Render, Edit }`.
- Fields on `App`: `inbox: String`, `notes: BTreeMap<String, String>`,
  `right_pane: RightPane`.
- `apply_state` / `snapshot_state` updated; kill handler removes the note; rename
  handler moves it.
- Key wiring: `t` / `T` in `handle_list_key` set `right_pane`; `Tab` (when a note
  is shown) enters `Mode::Note`; new `handle_note_key` drives render/edit/visual.
- The edit-mode editor reuses `ReplyForm`'s logic. Extract the multi-line editing
  buffer (char-indexed insert/backspace/word/line ops + cursor motion) into a
  shared editor unit so both `Reply` and `Note` use it (no duplication). Existing
  `Reply` editor tests are preserved against the extracted unit.

### 4. `src/clip.rs`

- `fn copy(text: &str)` — best-effort shell-out to `pbcopy`. The formatting lives
  in `note::selected_as_numbered` (pure, tested); this wrapper only pipes a string.

### 5. `src/ui/note.rs`

- Renders the note pane: a header (session name / `Inbox` · `3/5` · mode hint) and
  the body — either rendered markdown or the raw editor.
- Render: checkboxes `☐`/`☑`, done lines DIM/strikethrough, headings BOLD, bullets
  `•`, task-cursor and visual-selection highlight.
- Editor: reuses the `wrap_rows` / `cursor_rowcol` helpers (currently in
  `src/ui/mod.rs`) for wrapping and hardware-cursor placement.
- `src/ui/mod.rs`: when `right_pane` is a note, the body's right column draws
  `ui::note` instead of `preview`. `Mode::Note` does not overlay — it lives in the
  pane.
- `src/ui/sessions.rs`: draw the `3/5` counter on the card (via `note::counts` of
  `notes[name]`) when `total > 0`.
- `src/ui/footer.rs`: hints for the note-focused render/edit/visual states.

### Ephemeral state

Render cursor and visual anchor live on `App` (inside `NoteState`) and reset when
the shown note changes.

## Testing

- `note.rs`: parsing of each line kind; `counts`; `toggle` (correct line, flip is
  idempotent applied twice); `selected_as_numbered` (numbering, prefix stripping);
  `task_line_indices`.
- Persistence (app-level): `state.toml` round-trip of `notes` / `inbox`; kill
  removes the note; rename moves it to the new name.
- UI snapshots (existing `TestBackend` pattern): checkboxes render `☐`/`☑`; done
  lines are DIM; the `3/5` counter shows on the card; render cursor and visual
  selection are highlighted.
- Editor: covered by the existing `Reply` editor tests against the extracted
  shared editor unit.
- Clipboard: `pbcopy` shell-out is not unit-tested; the tested seam is
  `note::selected_as_numbered`.

## Open items / future enhancements

- Counter placement on the card may be adjusted after seeing it live.
- Inbox-note progress could also be surfaced somewhere in the chrome (deferred).
- Optional richer markdown (inline bold/italic) is explicitly out of scope for v1.
