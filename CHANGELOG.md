# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1] - 2026-06-09

### Added
- **What's New**: after an upgrade, a one-shot modal shows the changelog entries
  for every version since the one last run (skipped on a first install). Backed
  by the project `CHANGELOG.md` embedded in the binary.
- Help (`?`) is now tabbed — **Keys** and **Changelog** (`tab` to switch) — with
  the changelog rendered as Markdown (headings, `code`, **bold**, lists) and
  scrollable.
- **Recent** sessions: a second sidebar tab next to Current. A stopped/killed
  agent session is kept (last 20, persisted) and can be re-spawned with `enter`.
- Filtering (`/`) now matches a session's **path** (dir or live cwd), not just
  its name — both for live sessions and the Recent tab.

### Fixed
- Preview no longer freezes and the status no longer sticks at `idle` while an
  agent is working: status now reads a working-animation marker
  (`esc to interrupt`) with a frame-diff fallback, and every session is captured
  each tick (reverting the over-aggressive capture-gating from 0.5.0).
- Worktree promote (`ctrl+g`) no longer types `cd … && git checkout …` into a
  running agent's prompt: the agent is stopped, the git work runs, then it is
  respawned in the repo root. If removal fails, the agent is restored in place.
- Persisted session directories are kept in sync with each agent's live cwd, so
  cold-start restore points where the session actually is after a promote /
  return-to-root.
- What's New / Help scroll now reaches the bottom in the narrow modal (the cap
  used the wrapped row count instead of the logical line count).

### Changed
- Backfilled the changelog with the previously undocumented `0.2.0`–`0.4.1`
  releases.

## [0.5.0] - 2026-06-09

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

## [0.4.1] - 2026-06-05

### Fixed
- Maintenance release: packaging/CI fixes for the macOS release assets. No
  functional changes to the app.

## [0.4.0] - 2026-06-05

### Added
- Git operations panel: `Ctrl+g` on a worktree session **promotes** its branch
  into the repo root (stash → remove worktree → checkout → pop), and `Ctrl+l`
  opens branch **delete**/batch **cleanup** of merged branches.
- The create form gained a **worktree** checkbox that gates the branch/base
  pickers, so a new session can fork a branch + worktree in one flow.

### Fixed
- Escape single quotes when building the promote shell command (shell-injection
  hardening), and quote `repo_root`/branch consistently.
- `Ctrl+g` on a protected or non-git session now shows a clear error instead of
  doing nothing; re-check `is_dirty` at dispatch time so the stash decision
  matches the actual worktree state.

## [0.3.0] - 2026-06-04

### Added
- **Self-update**: a background check against GitHub Releases, download + verify
  (sha256), atomic binary swap, and `exec` restart — surfaced as an
  offer/progress modal, an update badge, and footer hints, triggered when idle.
- Mouse wheel scrolls the preview pane; mouse mode stays on during attach.

### Changed
- The preview subtitle shows the **project name** instead of the full path, with
  the age/project/branch row dimmed for hierarchy.

## [0.2.0] - 2026-06-04

### Added
- **Usage-log** modal (`Shift+L`): a full-screen log of recent OAuth
  (usage/profile) calls, scrollable, with copy.
- Create-form **branch picker** replacing the worktree toggle, with
  `hjkl`/arrow navigation, a Claude **model list + effort slider**, base-branch
  search, and `--model`/`--effort` passed through to the spawned command.
- Per-project **notes** on `T`.
- The `i` composer keeps a per-session **reply draft** whose lifetime is tied to
  its session (dropped on kill/rename/prune), with `Ctrl-y` copy and `Ctrl-x`
  clear.

### Fixed
- Usage poller backs off on the last fetch result (not cumulatively) with a
  clearer `401` message; persistent `429`/`401` errors resolved.
- `opencode` agent not found.
- Flush tick-dirtied state on exit so a quit right after a background tick
  doesn't lose changes.

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

[Unreleased]: https://github.com/kashikuroni/amux/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/kashikuroni/amux/compare/v0.1.0...v0.5.0
[0.1.0]: https://github.com/kashikuroni/amux/releases/tag/v0.1.0
