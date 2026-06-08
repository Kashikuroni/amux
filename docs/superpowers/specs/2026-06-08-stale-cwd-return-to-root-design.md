# Design: graceful git state for a stale session cwd + return-to-root

**Date:** 2026-06-08
**Status:** Approved (brainstorming) — ready for plan
**Scope:** `am` TUI (root crate). No change to `crates/amux-verify`.

## Problem

A session's git branch is drawn from the *live pane working directory* (`Session.cwd`,
sourced from tmux `#{pane_current_path}`). `git::read(&cwd)` returns `Option<GitInfo>`;
the renderer shows the branch only `if let Some(g) = &s.git` (`ui/sessions.rs:120`).
When `cwd` does not resolve to a git repository, `s.git` is `None` and the card shows
**nothing** — no glyph, no branch, no explanation.

This happens in three real situations, observed live:

1. **cwd directory removed** — e.g. an external `git worktree remove` (or the
   `finishing-a-development-branch` cleanup) deletes the worktree out from under a live
   session. The pane still points into the deleted path; `git -C <gone>` fails outright
   (it cannot even `chdir`), so no branch — and no way for the user to recover from the UI.
2. **cwd is not a repository** — the pane sits in a plain directory that was never a repo
   (e.g. a non-git project). There is no root to return to.
3. **Reader has not answered yet** — the first ~100–250 ms after launch, before the
   background `GitReader` has reported. Transient.

Today all three render identically (blank). The app's *own* delete flow
(`Action::Kill` with `remove_worktree`, `main.rs:395`) kills the tmux session before
removing the worktree, so it never produces case 1 — case 1 only arises from **external**
removal.

## Goals

- Never show a silent blank where a branch belongs. Each non-resolving state reads as
  itself: *loading* vs *worktree removed* vs *no repo*.
- For a session whose worktree was removed but whose repo root is known, offer a
  one-key recovery that returns the pane to the project root — surfaced inline on the
  card as an English prompt with the keybinding, e.g. `worktree removed · return to root? c`.
- Recovery must preserve a running Claude session (resume it), not discard it.

## Non-goals

- Changing the app's own delete flow (it already kills the session — no orphan).
- Walking the filesystem upward to *discover* a root from a dead cwd (impossible —
  `git -C <gone>` fails). The root comes only from the stored `@cm_repo` tag.
- Auto-recovery. The cd is strictly user-triggered.
- Offering recovery for a genuinely non-repo cwd (case 2) — there is no root.

## Architecture

Three small, independent changes across the existing layers:

1. **Data layer** (`git.rs` reader + `app.rs` cache) — report a *definite verdict* per
   cwd so "loading" is distinguishable from "no repo".
2. **Render layer** (`ui/sessions.rs`) — a 4-way state on line 2 of the card.
3. **Action layer** (`app.rs` key handling + `main.rs` execution) — a contextual `c`
   key that returns the pane to root, reusing the proven Claude-restart pipeline.

### 1. Data layer — definite per-cwd verdict

Today the background reader inserts **only successful** reads into its result map
(`git.rs:103`, `map.insert(dir, info)`), and `app.git_cache` is
`HashMap<String, GitInfo>`. Absence of a key therefore conflates "not read yet" with
"read and not a repo", which would make the new prompt flicker on every card at startup.

**Change the map value to `Option<GitInfo>`** and insert an entry for **every** requested
directory:

- `git.rs` `spawn_reader`: `res_tx: Sender<HashMap<String, Option<GitInfo>>>`; the loop
  does `map.insert(dir.clone(), read(&dir))` for each dir (so `Some(info)` = repo,
  `None` = confirmed-not-a-repo). Channel request payload stays `Vec<String>` (cwd list).
- `app.rs`: `git_cache: HashMap<String, Option<GitInfo>>` (was `HashMap<String, GitInfo>`,
  app.rs:849). In `refresh()` (app.rs:2231–2232):
  - `s.git = self.git_cache.get(&s.cwd).cloned().flatten();`  // resolved info, or None
  - `s.git_checked = self.git_cache.get(&s.cwd).is_some();`   // has the reader answered?
- `tmux::Session` gains `pub git_checked: bool` (default `false` in `parse_line`,
  tmux.rs:68–78; set by `refresh()` as above). The inline (no-worker) path
  (app.rs:2240–2242) sets `s.git_checked = true` after each inline `read`.

`GitInfo` is unchanged. `git::read` is unchanged.

### 2. Render layer — 4-way card state

In `ui/sessions.rs` (the `if let Some(g) = &s.git` block, lines 120–177), replace the
binary present/absent with a 4-way match. The root for the prompt is
`crate::app::session_root(s)` (app.rs:2397 — returns the `@cm_repo` root trimmed of a
trailing slash when `worktree_repo` is set). The *offer* is gated on
`s.worktree_repo.is_some()` (true only for am-managed worktree sessions; `None` for a
plain non-repo dir such as the observed `mp-system-legacy`).

| State | Condition | Line-2 content |
|---|---|---|
| **Repo** | `s.git.is_some()` | branch glyph + branch + right-aligned `+a −d` (unchanged) |
| **Returnable** | `git.is_none() && git_checked && worktree_repo.is_some()` | DIM `worktree removed · return to root?` + key chip `c` (REVERSED+BOLD, matching the answer-button look at sessions.rs:194–197) |
| **No repo** | `git.is_none() && git_checked && worktree_repo.is_none()` | DIM `no repo` |
| **Loading** | `git.is_none() && !git_checked` | nothing (current behavior) |

Styling follows the repo's UI convention: chrome/text in `Reset` + `DIM`; the key chip is
the only accent, reusing the existing REVERSED+BOLD treatment of answer buttons (no new
color). The prompt text is truncated to fit the card width like other line-2 content; the
`+a −d` diff stat does not appear in non-Repo states (there is no git info).

This is a pure presentation change — no extra git reads on the UI thread.

### 3. Action layer — `c` returns the pane to root

**Key.** `c` is currently unbound in normal mode (`handle_normal_key`, app.rs:1285+).
Bind it so it fires **only** when the selected session is *Returnable*; otherwise it is a
no-op. It maps to `Action::ReturnToRoot { name: String, root: String }` where `root =
session_root(selected).to_string()`.

**Execution** (`main.rs`, new arm). Branch on whether the session runs Claude, mirroring
the `is_claude` filter used by `RestartAllClaude` (main.rs:475,
`s.agent.split_whitespace().next() == Some("claude")`):

- **Claude session** — reuse the existing restart pipeline for this one session so the
  Claude conversation is resumed in the new directory rather than lost:
  - `tmux::set_remain_on_exit(&name, true)` then `tmux::send_ctrl_c(&name)` (as
    `RestartAllClaude`, main.rs:483–491).
  - Record the session as restarting **with a cwd override = root** (see below).
  - The existing poll loop (main.rs:265–298) detects the dead pane, parses
    `claude --resume <uuid>` from the pane, and respawns it — passing the override
    directory so it becomes `tmux::respawn_pane(&name, &root, &cmd)` instead of `&s.dir`.
    Claude resumes its session in the project root; the cwd now resolves and the branch
    reappears on the next refresh.
- **Non-Claude (shell) session** — send the command directly:
  `tmux::send_text(&name, &format!("cd {}", shell_single_quote(&root)))`. `send_text`
  appends Enter (tmux.rs:254–256), so the `cd` executes. The path is wrapped in
  single quotes (with `'\''` escaping) so spaces and special characters are safe.

**Carrying the cwd override.** `App.restarting` is `HashMap<String, i64>` (name →
start unix-secs; app.rs:896). Replace the value with a small struct so the poll loop can
override the respawn directory:

```rust
pub struct RestartReq { pub started: i64, pub root: Option<String> }
pub restarting: HashMap<String, RestartReq>,
```

- `RestartAllClaude` (main.rs:491): `insert(name, RestartReq { started: now, root: None })`.
- `ReturnToRoot` (Claude branch): `insert(name, RestartReq { started: now, root: Some(root) })`.
- Poll loop (main.rs:267–286): `app.now_unix - req.started` for the 30 s timeout; the
  respawn directory becomes `req.root.clone().unwrap_or_else(|| s.dir.clone())`.

After the cd/respawn, the next `refresh()` re-reads the (now valid) cwd and the card
returns to the **Repo** state showing the root's branch.

## Help / footer

Add a help entry in `ui/modal_help.rs` (the keys list, ~line 22–32):
`("c", "return to root (stale cwd)")`. The footer (`ui/footer.rs:54`) is contextual; no
change required unless we want a hint there — out of scope for this pass.

## Edge cases

- **Reader race / first frame** — `git_checked == false` ⇒ *Loading* ⇒ blank, exactly as
  today. No prompt flicker.
- **cwd transiently changed between two refreshes** — one frame may misclassify; self-corrects
  next refresh (same transient as today's branch display).
- **Non-Claude pane that is mid-command** — `cd` is queued by the shell after the current
  command, standard shell behavior; user-triggered, so acceptable.
- **Claude already dead when `c` is pressed** — the restart pipeline's 30 s timeout
  (main.rs:268) clears the entry and leaves the dead pane for inspection, identical to a
  failed `u`.
- **Root itself is gone** — extremely unlikely (it's the main repo); the respawn/cd simply
  fails and surfaces via the existing error path. Not specially handled.
- **Non-worktree session with a deleted cwd** — `worktree_repo` is `None` ⇒ *No repo*
  (no offer). Its `s.dir`/project root is typically gone too, so there is no useful root.

## Testing

Unit tests, following the existing patterns:

- **Reader verdict** (`git.rs` tests): a non-repo directory yields a map entry of
  `Some(None)` (present, not-a-repo), a repo yields `Some(Some(info))`, and an
  unrequested dir is absent.
- **State mapping** (`app.rs` / `ui::sessions` tests): given `(git, git_checked,
  worktree_repo)` combinations, the card renders the four states correctly — assert the
  buffer contains `worktree removed`, `return to root?`, the `c` chip / `no repo` / branch
  / nothing as appropriate (mirrors existing buffer tests, e.g. sessions.rs:570+).
- **Key binding**: `c` on a selected *Returnable* session produces
  `Action::ReturnToRoot { root }` with the expected root; `c` on a *Repo* / *No repo* /
  *Loading* session is a no-op.
- **Restart override**: a `RestartReq { root: Some(_) }` in the poll loop respawns in the
  override directory; `RestartReq { root: None }` (the `u` path) respawns in `s.dir`
  (guard against a regression to the existing behavior).
- **Quoting**: `shell_single_quote` wraps and escapes a path containing a space and a
  single quote correctly.

## Out of scope

- A `respawn-pane`-based hard reset for the shell case (chosen mechanism is the
  least-destructive `cd`; the Claude case already respawns).
- Surfacing recovery in the footer or as a modal.
- Reacting automatically to external worktree removal (no filesystem watcher).
