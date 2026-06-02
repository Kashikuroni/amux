# Worktree sessions — design

**Date:** 2026-06-02
**Status:** Approved, ready for implementation plan

## Problem

When creating a session, the agent runs in the chosen directory on its current
branch. Multiple agents working in the same repo step on each other's working
tree. We want an **optional** mode: create a fresh git worktree branched off a
chosen base branch, so the agent works in isolation on its own branch.

## Decisions

- **Worktree location:** `<repo_root>/.worktrees/<new_branch>` (inside the repo).
- **Base branch:** picker over existing branches; current `HEAD` first / default.
- **Toggle:** an optional step in the create form, **off by default**.
- **New branch name:** prefilled with the session name, editable.
- **`.gitignore`:** auto-append `.worktrees/` to the repo's `.gitignore` if absent.
- **Cleanup on kill:** the kill modal offers a toggle to also remove the worktree
  (only shown when the session has one). Off by default.
- **Tracking:** a tmux session option `@cm_repo=<repo_root>` marks a session as
  worktree-backed. The worktree path itself is the session's path.

## Data flow

```
CreateForm
  Name ─ Dir ─ [Worktree toggle] ─ Agent
                   on → Base branch (picker) + New branch (text)
                   off → unchanged 3-step flow
  ▼
Action::Create { name, dir, agent, worktree: Option<WorktreeSpec> }
  WorktreeSpec { base: String, new_branch: String }
  ▼
handle_action (main.rs), when worktree is Some:
  1. repo_root = git::repo_root(dir)                 (error if dir not a repo)
  2. git::ensure_gitignore(repo_root, ".worktrees/")
  3. wt_path = repo_root/.worktrees/<new_branch>
  4. git::add_worktree(repo_root, wt_path, new_branch, base)
  5. tmux::new_session(name, wt_path, agent)
  6. tag the session with @cm_repo=<repo_root>
  On any git error: surface via app.error, do not create the tmux session.
```

## Components

### `git.rs` — add mutating helpers

The module is currently read-only; new functions are explicitly mutating and
documented as such. They reuse the existing `git_out` / command-runner pattern.

- `list_branches(dir) -> Vec<String>`
  `git branch --format='%(refname:short)'`; the current branch (from
  `symbolic-ref --short HEAD`) is moved to the front. Empty vec if not a repo.
- `repo_root(dir) -> Option<String>`
  `git rev-parse --show-toplevel`.
- `add_worktree(repo_root, wt_path, new_branch, base) -> io::Result<()>`
  `git -C <repo_root> worktree add -b <new_branch> <wt_path> <base>`.
  Returns the stderr-bearing error on failure (existing branch, bad name, etc.).
- `remove_worktree(repo_root, wt_path) -> io::Result<()>`
  `git -C <repo_root> worktree remove <wt_path>` — **no** `--force`, so a dirty
  worktree surfaces an error rather than silently discarding work.
- `ensure_gitignore(repo_root, entry) -> io::Result<()>`
  Append `entry` on its own line to `<repo_root>/.gitignore` if not already
  present (exact-line match). Creates the file if missing. Best-effort but
  errors are returned so the caller can decide.

### `CreateForm` / `modal_new.rs`

New fields on `CreateForm`:
- `worktree: bool`
- `base_branches: Vec<String>`, `base_index: usize`
- `new_branch: String`

New `CreateField` variants: `Worktree`, `Base`, `Branch`.

Behavior:
- After `Dir`, a toggle step: "Create worktree (space)". When `dir` is not a git
  repo, the step renders disabled with a hint and cannot be turned on.
- When the toggle is on, two more rows appear:
  - **BASE** — branch picker, cycled with ←/→ (same idiom as the agent selector),
    populated from `git::list_branches(dir)`.
  - **NEW BRANCH** — text field, prefilled with the session name, editable.
- Tab/Enter navigation skips `Base`/`Branch` when the toggle is off.
- The step indicator becomes dynamic: `N of 3` (toggle off) or `N of 5` (on).
- The command preview shows the `git worktree add -b …` step before `tmux new …`.

Base branches are loaded when entering the worktree step (and when `dir`
changes while the toggle is on), mirroring how `dir_entries` is refreshed.

### Kill flow — `modal_kill.rs` / `Action::Kill`

- `tmux::Session` gains `worktree_repo: Option<String>`, parsed from a new
  `@cm_repo` field appended to `LIST_FORMAT`. An empty value parses to `None`.
- The kill modal shows a toggle "also remove worktree (space)" only when the
  selected session has `worktree_repo.is_some()`. Off by default.
- `Action::Kill` becomes `{ name: String, remove_worktree: bool }`.
- `handle_action`: kill the tmux session first; then, if `remove_worktree`, call
  `git::remove_worktree(repo, session.dir)`. A removal error is surfaced via
  `app.error` but the session stays killed (no resurrection).

## Error handling

- `dir` not a git repo: the worktree toggle is disabled in the form, so the
  worktree path is unreachable; defense-in-depth, `handle_action` re-checks
  `repo_root` and errors if absent.
- New branch already exists / invalid name: `git worktree add` fails; the error
  is shown in `app.error` and the create modal stays open for correction.
- Branch names with slashes (`feature/x`) nest under `.worktrees/`; git creates
  intermediate dirs and the `.worktrees/` ignore entry still covers them.
- Dirty worktree on removal: `remove_worktree` (no `--force`) errors; surfaced.

## Testing

- `git.rs`: `list_branches`, `add_worktree`, `remove_worktree`, `ensure_gitignore`
  against a temp repo (reusing the existing temp-repo test pattern).
- `tmux.rs`: `parse_line` populates `worktree_repo` from `@cm_repo`; empty → None.
- `app.rs`: form navigation with the toggle on vs off (field order, step count);
  `Action::Create` carries the `WorktreeSpec` when enabled; `Action::Kill`
  carries `remove_worktree`.
- `modal_new.rs`: render snapshot shows BASE / NEW BRANCH rows when toggled on.

## Out of scope (YAGNI)

- Checking out an existing branch into a worktree (only new branches).
- Configurable worktree root location.
- Deleting the branch itself on cleanup (only the worktree is removed).
- Listing/pruning orphaned worktrees from the UI.
