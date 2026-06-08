# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Verify a session from the TUI: `v` runs its `.amux/verify.toml` contract in the
  background (a second `v` cancels), the status slot shows live progress and a
  `✓ verified` / `✗ failed: <gate>` verdict, and `V` opens a detail panel with
  per-gate results, the repro command, and the failure output.
- Sessions whose pane directory no longer resolves to a git repo now show their
  state instead of a blank: `no repo`, or `worktree removed · ctrl+r return to root`.
  Pressing `ctrl+r` returns the session to its project root (resuming Claude in place).
- `amux doctor [--clean]`: a CLI subcommand that surfaces tmux servers/sockets the
  dashboard hides (untagged, leaked, or stale) and, with `--clean`, removes the
  safe ones — never touching the live `cm` server or other users' tmux.
- `amux-verify`: a standalone workspace crate + CLI that runs the repo's
  verification contract (`.amux/verify.toml`) — ordered gates executed
  without a shell in a worktree, fail-fast cascade, per-gate timeout with
  process-group kill, `--json` verdict. Foundation for in-app verification
  (items 1–2 of the verification MVP).
- The `i` composer keeps a per‑session **draft**: Esc closes and saves, `i`
  restores it; the draft survives amux restarts and dies with its session.
  `Ctrl‑y` copies the whole message, `Ctrl‑x` clears it.
- `done/total` task counter on the project group header, fed by the project note.

### Changed
- `T` now opens the selected session's **project note** — keyed by the project
  root path, so it survives restarts and session kills — instead of the global
  Inbox. At startup, state entries of projects whose directory no longer exists
  are pruned (note: a project on an unmounted volume counts as deleted).
- Performance: the UI no longer redraws or wakes at a fixed ~12.5 fps when idle
  (event-driven render; the ANSI preview is parsed once per change, not per
  frame), git status is re-read only for sessions whose pane changed (or every
  few seconds) instead of every session every tick, and rapid `j`/`k` navigation
  debounces the preview capture instead of forking tmux per keystroke.
- Performance: the session poll no longer forks `tmux capture-pane` for every
  session every tick. A session's pane is re-captured only when its tmux
  activity advanced since the last tick (or it was running) — so an all-idle
  dashboard does one `list-sessions` fork per tick instead of one per session,
  and the cost scales with active sessions rather than total.

### Removed
- The global Inbox note; an existing `inbox` value in `state.toml` is ignored.

## [0.1.0] - 2026-06-03

Initial public release.

### Added
- Session dashboard for tmux‑backed AI agent sessions, grouped by project, with
  live running/idle/waiting status.
- Live ANSI preview of the selected session with scrollback (`Ctrl‑k/j`,
  `PgUp/PgDn`); one‑key attach/detach.
- Inline answering of agent numbered prompts (`1`–`9`) and free‑text replies (`i`).
- Git worktree sessions with color‑coded branch markers (`⎇` repo / `⧉` worktree)
  and a right‑aligned diff stat.
- Per‑session markdown notes plus a global Inbox, shown in place of the preview:
  render/edit modes, toggleable checkboxes, vim‑style task selection, copy as a
  numbered list, clear‑with‑confirmation, and a `done/total` counter on each card.
- Claude usage limits (5h / 7d) in the header, read from local Claude credentials.
- Keyboard‑layout‑independent hotkeys (works on non‑Latin layouts).
- Configuration via `~/.agent-multiplexer/config.toml`; persisted UI state and
  notes in `~/.agent-multiplexer/state.toml`.

[Unreleased]: https://github.com/kashikuroni/amux/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kashikuroni/amux/releases/tag/v0.1.0
