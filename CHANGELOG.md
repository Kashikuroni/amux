# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
