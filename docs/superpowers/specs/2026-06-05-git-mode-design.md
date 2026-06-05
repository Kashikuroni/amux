# Git Mode — Design Spec (v0.4.0)

## Overview

A unified `Mode::Git` panel for git operations on sessions. Entered via `Ctrl+g`
(session-contextual) or `Ctrl+b` (global branch cleanup). Replaces ad-hoc git
operations scattered across session kill flow.

---

## Motivation

After an agent finishes work in a worktree, the user needs to:
1. Move the branch into the main workspace to run builds (env files, build
   artifacts live in the project root, not the worktree).
2. Test the branch, then delete it once merged.
3. Periodically clean up accumulated merged agent branches.

The existing `ConfirmDelete` toggle covers worktree removal but not the
"stay alive, move to root" case, and there is no branch deletion UI at all.

---

## Scope (v0.4.0)

Three git actions exposed through a single `Mode::Git`:

| Action | Trigger | Available on |
|---|---|---|
| Promote worktree | `Ctrl+g` | `⧉` worktree sessions |
| Delete branch | `Ctrl+g` | `⎇` normal branch sessions |
| Branch cleanup | `Ctrl+l` | Any session (global) |

---

## Data Model

### New types in `app.rs`

```rust
pub enum GitAction {
    Promote,
    DeleteBranch,
    BranchCleanup,
}

pub struct BranchItem {
    pub name: String,
    pub protected: bool,   // main / master / develop / dev
}

pub struct GitForm {
    pub session_name: String,
    pub branch: String,
    pub worktree_path: Option<String>,   // Some only for Promote
    pub has_stash: bool,                 // dirty worktree → stash needed
    pub action: GitAction,
    // BranchCleanup only:
    pub branches: Vec<BranchItem>,
    pub selected: std::collections::HashSet<usize>,
    pub cursor: usize,
}
```

### Mode extension

```rust
pub enum Mode {
    // … existing variants …
    Git(GitForm),
}
```

### New Action variants

```rust
pub enum Action {
    // … existing …
    PromoteWorktree { name: String },
    DeleteBranch    { name: String, branch: String },
    CleanupBranches { branches: Vec<String> },
}
```

---

## New git functions (`git.rs`)

| Function | Command | Notes |
|---|---|---|
| `is_dirty(dir) -> bool` | `git diff HEAD --quiet` | exit code ≠ 0 → dirty |
| `stash_push(dir)` | `git stash push -m "am-promote"` | returns error if nothing to stash |
| `stash_pop(dir)` | `git stash pop` | called after checkout in root |
| `delete_branch(repo_root, branch)` | `git branch -d <branch>` | `-d` not `-D`; fails on unmerged |
| `list_merged_branches(repo_root) -> Vec<String>` | `git branch --merged HEAD` | filters out current + protected names |

Protected branch names: `main`, `master`, `develop`, `dev`.

---

## UX Flows

### 1. Promote worktree (`Ctrl+g` on `⧉`)

**Modal:**

```
╭─ Git: promote worktree ──────────────────╮
│                                          │
│  Branch:   feature/my-agent              │
│  Worktree: /proj/.worktrees/feature/...  │
│                                          │
│  ⚠ Unstaged changes — will git stash    │
│     and restore after checkout           │
│                                          │
│  [Y] Promote   [N] Cancel                │
╰──────────────────────────────────────────╯
```

The `⚠` line is omitted when the worktree is clean.

**Execution sequence:**

1. `git::is_dirty(worktree_path)` → set `has_stash`
2. If dirty: `git::stash_push(worktree_path)`
3. `git::remove_worktree(repo_root, worktree_path)`
4. Build shell command:
   - clean:  `"cd <repo_root> && git checkout <branch>"`
   - dirty:  `"cd <repo_root> && git checkout <branch> && git stash pop"`
5. `tmux::send_keys(session, <command>)` — stash pop runs in the shell after
   checkout completes, no Rust-side timing needed
6. `app.refresh()` — git poller will pick up `⎇` icon on next tick

**Error handling:** Any step failure sets `app.error` with a contextual message
(e.g. "stash failed", "worktree removal failed — session left in worktree").
Execution stops at the first failure.

---

### 2. Delete branch (`Ctrl+g` on `⎇`)

**Modal:**

```
╭─ Git: delete branch ─────────────────────╮
│                                          │
│  Branch:  feature/my-agent               │
│  Session: cm-proj-feat                   │
│                                          │
│  Runs: git branch -d <branch>            │
│  Branch must be fully merged.            │
│                                          │
│  [Y] Delete   [N] Cancel                 │
╰──────────────────────────────────────────╯
```

**Execution:** `git::delete_branch(repo_root, branch)`. On error (unmerged),
`app.error` shows git's message verbatim. Session is not killed.

---

### 3. Branch cleanup (`Ctrl+l`)

**Modal:**

```
╭─ Git: branch cleanup ────────────────────╮
│  Merged into HEAD (5 branches):          │
│                                          │
│  [✓] feature/agent-auth                  │
│  [✓] fix/agent-typo                      │
│  [✓] feature/wip-test                    │
│  [✓] refactor/agent-ui                   │
│  [ ] main                    (protected) │  ← dim, not selectable
│                                          │
│  Space toggle · A select all · Y delete  │
╰──────────────────────────────────────────╯
```

Protected branches (`main`, `master`, `develop`, `dev`) are shown dim and cannot
be toggled. All non-protected branches are pre-selected by default.

**Execution:** `CleanupBranches` action iterates selected branch names and calls
`git::delete_branch` for each. Errors are collected and shown as a joined message
in `app.error` after the loop.

**Opening the modal:** `list_merged_branches(repo_root)` is called on the
currently focused session's repo root. If no session is focused or it is not a
git repo, `app.error` is set and mode stays `List`.

---

## Keybindings summary

| Key | Mode | Action |
|---|---|---|
| `Ctrl+g` | `Mode::List` (worktree session) | Open `Mode::Git(Promote)` |
| `Ctrl+g` | `Mode::List` (normal branch session) | Open `Mode::Git(DeleteBranch)` |
| `Ctrl+g` | `Mode::List` (non-git session) | No-op |
| `Ctrl+l` | `Mode::List` | Open `Mode::Git(BranchCleanup)` |
| `y` | `Mode::Git(Promote/Delete)` | Confirm + execute |
| `n` / `Esc` | `Mode::Git` | Cancel → `Mode::List` |
| `Space` | `Mode::Git(BranchCleanup)` | Toggle selection |
| `a` | `Mode::Git(BranchCleanup)` | Select all non-protected |
| `y` | `Mode::Git(BranchCleanup)` | Delete selected |

---

## UI rendering (`ui/modal_git.rs`)

New file following the pattern of `modal_kill.rs` and `modal_new.rs`. The modal
is rendered as a centered overlay. Branch cleanup uses a scrollable list widget
(same pattern as the session list) for the branch items.

---

## Out of scope (v0.4.0)

- `git branch -D` force delete
- Push/pull/remote sync
- Rebase/merge operations
- Stash list management
