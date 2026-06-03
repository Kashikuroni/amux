# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/OWNER/amux/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/amux/releases/tag/v0.1.0
