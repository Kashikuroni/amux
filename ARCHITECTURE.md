# Architecture

A short map of how `amux` is put together, for contributors.

## The core idea: a pure `App`, effects as `Action`s

The heart of the program is `App` (in `src/app.rs`). **`App` is pure — it performs
no IO.** It holds all state (the session list, selection, current mode, notes,
preview text, config) and turns key events into state changes. When something with
a side effect needs to happen — attach to a session, kill it, send text to an
agent, create a worktree — `App::handle_key` returns an [`Action`] describing the
effect rather than performing it.

The single place that performs effects is **`handle_action` in `src/main.rs`**,
the event-loop orchestrator. The loop is:

```
draw(app)  →  read key  →  app.handle_key(key) -> Option<Action>
           →  handle_action(action)  (the only IO: tmux, git, terminal handoff)
           →  if app.dirty { save state }
```

This split is the most important thing to understand:

- **`App` is trivially testable** — feed it `KeyEvent`s, assert on the resulting
  state or the returned `Action`. The ~180 unit tests rely on this; almost none of
  them touch tmux or the filesystem.
- **Rendering is read-only.** `src/ui/*` takes `&App` and draws; it never mutates
  state. (One documented exception: `App::preview_dims` is a `Cell` the renderer
  writes so the refresh logic can size the tmux window to the preview area.)

## Modes

`App.mode: Mode` is a state machine. Each variant either carries a small form
struct (`Create`, `Rename`, `Reply`, `Note`, …) or is fieldless (`List`, `Help`).
`handle_key` dispatches to a per-mode handler (`handle_list_key`,
`handle_create_key`, `handle_note_key`, …). Keys are normalized through
`latin_code` so hotkeys work on non-Latin keyboard layouts.

## tmux integration

- All sessions live on a **private tmux socket** (`-L cm`) so `amux` never touches
  the user's own tmux server, and are tagged with `@cm_*` user options so only
  managed sessions are listed. (The `cm` prefix is historical and intentional —
  don't "fix" it to `am`/`amux`.)
- `src/tmux.rs` wraps `tmux` subcommands. Every value (session name, path, branch,
  reply text) is passed as a separate `argv` entry — never interpolated into a
  shell string.
- `App::refresh` re-derives the session list from tmux each tick, diffs captured
  pane content to compute status (running / idle / waiting), and detects numbered
  prompts.

## Background work (off the UI thread)

Two things would otherwise block rendering, so they run on their own threads and
hand results back over channels:

- **Git** (`git::spawn_reader`) — reads branch + diff stat per session directory.
- **Claude usage** (`spawn_usage_poller` in `main.rs`) — polls the usage endpoint.

## Persistence

- Config: `~/.agent-multiplexer/config.toml` (`src/config.rs`).
- UI state + notes: `~/.agent-multiplexer/state.toml` (`src/state.rs`), saved
  whenever `app.dirty` is set.

## Module map

| Area | Files |
|------|-------|
| Core state & logic | `app.rs` |
| Event loop / effects | `main.rs` |
| tmux / git / clipboard | `tmux.rs`, `git.rs`, `clip.rs` |
| Pure helpers | `note.rs`, `editor.rs`, `browse.rs`, `timeutil.rs`, `usage.rs` |
| Config / persistence | `config.rs`, `state.rs` |
| Rendering | `ui/` (`sessions`, `preview`, `note`, `footer`, `header`, modals) |
| Theme | `theme.rs` |

[`Action`]: src/app.rs
