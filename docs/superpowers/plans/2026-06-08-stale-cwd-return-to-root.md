# Stale-cwd graceful git state + return-to-root — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a session's pane cwd no longer resolves to a git repository, the card stops showing a silent blank — it reads as *loading*, *no repo*, or *worktree removed · return to root? c* — and `c` returns the pane to the project root (resuming Claude in place).

**Architecture:** Three independent layers. (1) The background git reader reports a *definite verdict* per cwd (`Option<GitInfo>`) so "loading" is distinguishable from "no repo". (2) A pure classifier `git_card_state()` maps `(git_cache, session)` to a 4-way card state, rendered on line 2. (3) A contextual `c` key triggers `Action::ReturnToRoot`, which for a Claude session reuses the proven `u`-restart pipeline with a respawn-cwd override, and for a shell sends `cd '<root>'`.

**Tech Stack:** Rust, ratatui (TUI), tmux (session control via `send-keys`/`respawn-pane`), std mpsc channels.

**Spec:** `docs/superpowers/specs/2026-06-08-stale-cwd-return-to-root-design.md`

**Design note (decided during planning):** The card state is computed on the fly from `App.git_cache` at render time — NOT stored as a new `Session` field. Adding a field to `Session` would break all 22 `Session { … }` literals across the codebase; deriving from the cache (which the renderer already has via `app`) avoids that entirely.

---

### Task 1: Reader reports a definite verdict per cwd

The background reader currently inserts only *successful* reads, so an absent key conflates "not read yet" with "not a repo". Change the result map value to `Option<GitInfo>` and insert an entry for every requested directory. This change spans `git.rs` (the reader) and `app.rs` (the cache type + wiring); they must land together to compile.

**Files:**
- Modify: `src/git.rs` (`GitReader` struct ~line 78-83; `spawn_reader` ~line 88-115)
- Modify: `src/app.rs` (`git_cache` field line 849; `refresh()` git block lines 2220-2243)
- Test: `src/git.rs` (tests module, alongside existing reader/temp-repo tests)

- [ ] **Step 1: Write the failing reader test**

Add to the `#[cfg(test)] mod tests` in `src/git.rs`. It uses a fresh repo (via the existing `temp_repo` helper used by other tests in this file), a plain non-repo directory, and a path that does not exist — asserting each gets a *present* verdict (`Some(&…)`), with repo → `Some(&Some(_))` and the other two → `Some(&None)`.

```rust
#[test]
fn reader_reports_a_verdict_for_every_dir() {
    if Command::new("git").arg("--version").output().is_err() {
        return; // git not installed in this environment
    }
    let repo = temp_repo("reader-repo"); // existing helper: inits a git repo, returns PathBuf
    let repo_s = repo.to_str().unwrap().to_string();

    let plain = std::env::temp_dir().join(format!("reader-plain-{}", std::process::id()));
    std::fs::create_dir_all(&plain).unwrap();
    let plain_s = plain.to_str().unwrap().to_string();

    let gone_s = format!("/no/such/dir/reader-{}", std::process::id());

    let reader = spawn_reader();
    reader
        .tx
        .send(vec![repo_s.clone(), plain_s.clone(), gone_s.clone()])
        .unwrap();
    let map = reader.rx.recv().unwrap();

    assert!(matches!(map.get(&repo_s), Some(Some(_))), "repo → Some(Some)");
    assert_eq!(map.get(&plain_s), Some(&None), "non-repo dir → Some(None)");
    assert_eq!(map.get(&gone_s), Some(&None), "missing dir → Some(None)");

    let _ = std::fs::remove_dir_all(&plain);
    let _ = std::fs::remove_dir_all(&repo);
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p amux --lib git::tests::reader_reports_a_verdict_for_every_dir 2>&1 | tail -20`
Expected: compile error — `map.get(&plain_s)` is `Option<&GitInfo>`, not `Option<&Option<GitInfo>>` (the value type is still `GitInfo`).

- [ ] **Step 3: Change the reader to report `Option<GitInfo>`**

In `src/git.rs`, update the `GitReader.rx` type and the `spawn_reader` body. Replace the `rx` field:

```rust
pub struct GitReader {
    /// Send the current set of session directories to (re)read.
    pub tx: std::sync::mpsc::Sender<Vec<String>>,
    /// Receive the latest `dir → verdict` results. `Some(info)` = a repo,
    /// `None` = read but not a repo (or the dir is gone). An absent key means
    /// "not yet read".
    pub rx: std::sync::mpsc::Receiver<std::collections::HashMap<String, Option<GitInfo>>>,
}
```

In `spawn_reader`, change the result channel type and the per-dir insert:

```rust
    let (res_tx, res_rx) = mpsc::channel::<HashMap<String, Option<GitInfo>>>();
    std::thread::spawn(move || {
        while let Ok(mut dirs) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                dirs = newer; // only the most recent request matters
            }
            dirs.sort();
            dirs.dedup();
            let mut map = HashMap::new();
            for dir in dirs {
                // A verdict for every dir: Some = repo, None = not a repo / gone.
                let verdict = read(&dir);
                map.insert(dir, verdict);
            }
            if res_tx.send(map).is_err() {
                break; // UI gone
            }
        }
    });
```

- [ ] **Step 4: Update `app.rs` cache type and wiring**

In `src/app.rs`, change the field (line 849):

```rust
    pub git_cache: HashMap<String, Option<crate::git::GitInfo>>,
```

In `refresh()` (the worker branch, lines 2220-2238), keep `self.git_cache = map;` (types now match) and derive `s.git` by flattening:

```rust
                    if let Some(map) = latest {
                        self.git_cache = map;
                    }
                    for s in &mut sessions {
                        s.git = self.git_cache.get(&s.cwd).cloned().flatten();
                    }
```

In the inline (no-worker) branch (lines 2239-2243), also populate `git_cache` so the renderer can classify state uniformly:

```rust
                } else {
                    for s in &mut sessions {
                        s.git = crate::git::read(&s.cwd);
                        self.git_cache.insert(s.cwd.clone(), s.git.clone());
                    }
                }
```

- [ ] **Step 5: Run the reader test and the full lib suite**

Run: `cargo test -p amux --lib 2>&1 | tail -15`
Expected: PASS, including `reader_reports_a_verdict_for_every_dir`. No other test regresses (existing tests don't read `git_cache`'s value type).

- [ ] **Step 6: Commit**

```bash
git add src/git.rs src/app.rs
git commit -m "feat(git): reader reports a definite verdict per cwd (Option<GitInfo>)"
```

---

### Task 2: `git_card_state` classifier

A pure function mapping the cache verdict + session to a 4-way state. Used by both the renderer (Task 3) and the key handler (Task 5), so it lives in `app.rs` next to `session_root`/`is_worktree`.

**Files:**
- Modify: `src/app.rs` (add near `session_root`, ~line 2397; export the enum)
- Test: `src/app.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)] mod tests` in `src/app.rs`. It builds a `Session` (same literal shape as `app_with_two_sessions`) and a cache, asserting all four states.

```rust
#[test]
fn git_card_state_classifies_all_four_states() {
    use std::collections::HashMap;
    let mk = |cwd: &str, worktree_repo: Option<&str>| Session {
        name: "s".into(),
        dir: cwd.into(),
        cwd: cwd.into(),
        created: 1,
        agent: "claude".into(),
        status: Status::Idle,
        attached: false,
        git: None,
        worktree_repo: worktree_repo.map(|s| s.into()),
    };
    let info = crate::git::GitInfo { branch: "main".into(), added: 0, removed: 0 };

    // Loading: no cache entry.
    let empty: HashMap<String, Option<crate::git::GitInfo>> = HashMap::new();
    assert_eq!(git_card_state(&empty, &mk("/a", None)), GitCardState::Loading);

    // Repo: Some(Some(info)).
    let mut repo = HashMap::new();
    repo.insert("/a".to_string(), Some(info.clone()));
    assert_eq!(git_card_state(&repo, &mk("/a", None)), GitCardState::Repo);

    // Returnable: Some(None) + worktree_repo known.
    let mut gone = HashMap::new();
    gone.insert("/a".to_string(), None);
    assert_eq!(
        git_card_state(&gone, &mk("/a", Some("/repo"))),
        GitCardState::Returnable
    );

    // NoRepo: Some(None) + no worktree_repo.
    assert_eq!(git_card_state(&gone, &mk("/a", None)), GitCardState::NoRepo);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p amux --lib git_card_state_classifies_all_four_states 2>&1 | tail -20`
Expected: compile error — `git_card_state` and `GitCardState` are undefined.

- [ ] **Step 3: Implement the classifier**

In `src/app.rs`, just above `pub fn session_root` (line 2397):

```rust
/// How a session's git status should read on its card. Computed from the
/// background reader's verdict for the session's `cwd` (in `App::git_cache`):
/// an absent entry means the reader has not answered yet (Loading); `Some(info)`
/// is a live repo (Repo); `Some(None)` means the cwd is not a repo — either a
/// removed worktree we can return from (Returnable, when the repo root is known)
/// or a plain non-repo directory with nowhere to go (NoRepo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCardState {
    Repo,
    Returnable,
    NoRepo,
    Loading,
}

pub fn git_card_state(
    cache: &std::collections::HashMap<String, Option<crate::git::GitInfo>>,
    s: &Session,
) -> GitCardState {
    match cache.get(&s.cwd) {
        Some(Some(_)) => GitCardState::Repo,
        Some(None) => {
            if s.worktree_repo.is_some() {
                GitCardState::Returnable
            } else {
                GitCardState::NoRepo
            }
        }
        None => GitCardState::Loading,
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p amux --lib git_card_state_classifies_all_four_states 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): git_card_state classifier (Repo/Returnable/NoRepo/Loading)"
```

---

### Task 3: Render the 4 card states on line 2

Replace the binary present/absent branch on `s.git` with a match on `git_card_state`. The renderer already has `app` in scope where cards are built, so it computes the state and passes it into `card()`.

**Files:**
- Modify: `src/ui/sessions.rs` (`card` signature ~line 38-48; line-2 block lines 120-177; call site lines 392-402)
- Test: `src/ui/sessions.rs` (tests module, alongside existing buffer tests ~line 570)

- [ ] **Step 1: Write the failing render test**

Add to `#[cfg(test)] mod tests` in `src/ui/sessions.rs`. It renders a single card in the *Returnable* state and asserts the buffer contains the prompt text and the `c` key chip. (Mirror the existing buffer-test pattern in this module — e.g. `prompt_buttons_keep_a_blank_frame_row_below` — for how a `card` is rendered into a `Buffer`.)

```rust
#[test]
fn returnable_card_shows_return_to_root_hint() {
    let s = Session {
        name: "feat".into(),
        dir: "/repo/.worktrees/feat".into(),
        cwd: "/repo/.worktrees/feat".into(),
        created: 1,
        agent: "claude".into(),
        status: Status::Idle,
        attached: false,
        git: None,
        worktree_repo: Some("/repo".into()),
    };
    let item = card(
        &s,
        0,
        false,
        None,
        80,
        1,
        0,
        0,
        false,
        crate::app::GitCardState::Returnable,
    );
    let text = item_to_string(&item); // existing test helper that flattens a ListItem to a String
    assert!(text.contains("worktree removed"), "got: {text}");
    assert!(text.contains("return to root?"), "got: {text}");
    assert!(text.contains(" c "), "missing key chip, got: {text}");
}
```

> If this module has no `item_to_string` helper, render the `ListItem` into a `ratatui::buffer::Buffer` the same way the neighboring tests do and assert on the buffer's text content instead. Do not invent a helper that conflicts with an existing one — reuse what the module already uses.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p amux --lib returnable_card_shows_return_to_root_hint 2>&1 | tail -20`
Expected: compile error — `card` takes 9 args, not 10 (no `git_state` param yet).

- [ ] **Step 3: Add the `git_state` parameter to `card`**

In `src/ui/sessions.rs`, extend the signature (after `restarting: bool`):

```rust
#[allow(clippy::too_many_arguments)]
fn card(
    s: &Session,
    spinner_frame: usize,
    selected: bool,
    prompt: Option<&[String]>,
    width: u16,
    num: usize,
    done: u32,
    total: u32,
    restarting: bool,
    git_state: crate::app::GitCardState,
) -> ListItem<'static> {
```

- [ ] **Step 4: Branch line 2 on the state**

Replace the existing `if let Some(g) = &s.git { … }` block AND the following `if s.git.is_none() && total > 0 { … }` block (lines 120-177) with a single match. The *Repo* arm keeps the current branch+diff rendering verbatim; the other arms are new.

```rust
    use crate::app::GitCardState;
    match git_state {
        GitCardState::Repo => {
            // SAFETY: Repo state implies the cache held Some(info), so s.git is Some.
            if let Some(g) = &s.git {
                // --- existing branch + counter + right-aligned diff block, unchanged ---
                let (glyph, glyph_fg) = if crate::app::is_worktree(s) {
                    (th::WORKTREE, WORKTREE_FG)
                } else {
                    (th::BRANCH, BRANCH_FG)
                };
                l2.push(Span::styled(
                    format!("   {glyph} "),
                    Style::default().fg(glyph_fg).add_modifier(Modifier::BOLD),
                ));
                l2.push(Span::styled(
                    g.branch.clone(),
                    Style::default().fg(Color::Reset).add_modifier(Modifier::DIM),
                ));
                let counter = if total > 0 {
                    format!("   {done}/{total}")
                } else {
                    String::new()
                };
                if !counter.is_empty() {
                    l2.push(Span::styled(
                        counter.clone(),
                        Style::default().add_modifier(Modifier::DIM),
                    ));
                }
                let added = format!("+{}", g.added);
                let removed = format!("−{}", g.removed);
                let left2 = INDENT.len()
                    + 1
                    + 1 + s.agent.chars().count()
                    + 5
                    + g.branch.chars().count()
                    + counter.chars().count();
                let diff_width = added.chars().count() + 1 + removed.chars().count();
                let pad2 = (width as usize).saturating_sub(left2 + diff_width).max(1);
                l2.push(Span::raw(" ".repeat(pad2)));
                l2.push(Span::styled(added, Style::default().fg(Color::Green)));
                l2.push(Span::styled(
                    format!(" {removed}"),
                    Style::default().fg(Color::Red),
                ));
            }
        }
        GitCardState::Returnable => {
            l2.push(Span::styled(
                "   worktree removed · return to root? ".to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ));
            l2.push(Span::styled(
                " c ",
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
        }
        GitCardState::NoRepo => {
            l2.push(Span::styled(
                "   no repo".to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        GitCardState::Loading => {
            // Reader has not answered yet: keep line 2 as just the agent, and
            // (preserving prior behavior) append the task counter if any.
            if total > 0 {
                l2.push(Span::styled(
                    format!("   {done}/{total}"),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
        }
    }
```

- [ ] **Step 5: Pass the state at the call site**

In `src/ui/sessions.rs`, where `card(...)` is invoked (lines 392-402), compute and pass the state:

```rust
        let git_state = crate::app::git_card_state(&app.git_cache, s);
        items.push(card(
            s,
            app.spinner_frame,
            pos == sel,
            prompt,
            content_width,
            pos + 1,
            done,
            total,
            restarting,
            git_state,
        ));
```

- [ ] **Step 6: Run the render test + lib suite**

Run: `cargo test -p amux --lib 2>&1 | tail -15`
Expected: PASS, including `returnable_card_shows_return_to_root_hint`. Existing card tests (which build sessions with empty `git_cache`) still render *Loading* (blank line 2 / counter) exactly as before.

- [ ] **Step 7: Commit**

```bash
git add src/ui/sessions.rs
git commit -m "feat(ui): render stale-cwd card states (returnable / no repo / loading)"
```

---

### Task 4: `RestartReq` + shell quoting (behavior-preserving refactor)

Prepare the restart pipeline to accept a respawn-cwd override, without changing behavior yet, and add a path-quoting helper. Keeping this separate makes Task 5 a pure addition.

**Files:**
- Modify: `src/app.rs` (`restarting` field line 896; add `RestartReq` near it)
- Modify: `src/main.rs` (`RestartAllClaude` insert line 491; poll loop lines 265-298)
- Modify: `src/tmux.rs` (add `shell_single_quote` near `send_text` ~line 254)
- Test: `src/tmux.rs` (tests module)

- [ ] **Step 1: Write the failing quoting test**

Add to `#[cfg(test)] mod tests` in `src/tmux.rs`:

```rust
#[test]
fn shell_single_quote_wraps_and_escapes() {
    assert_eq!(shell_single_quote("/repo/main"), "'/repo/main'");
    assert_eq!(shell_single_quote("/has space"), "'/has space'");
    // A single quote closes, escapes, reopens: it's a → 'a'\''b'
    assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p amux --lib shell_single_quote_wraps_and_escapes 2>&1 | tail -15`
Expected: compile error — `shell_single_quote` is undefined.

- [ ] **Step 3: Implement `shell_single_quote`**

In `src/tmux.rs`, just above `send_text` (line 253):

```rust
/// Wraps a path in POSIX single quotes so it survives the shell verbatim when
/// sent via `send_text`. The only character that cannot appear inside single
/// quotes — `'` itself — is emitted as the standard `'\''` sequence.
pub fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
```

- [ ] **Step 4: Run the quoting test**

Run: `cargo test -p amux --lib shell_single_quote_wraps_and_escapes 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Introduce `RestartReq` and migrate the field**

In `src/app.rs`, add the struct just above the `App` struct's `restarting` field, and change the field type (line 896):

```rust
/// A session awaiting Claude restart. `root` overrides the respawn directory
/// (Some when returning to the project root from a removed worktree; None for a
/// plain `u` restart, which respawns in the session's own dir).
#[derive(Debug, Clone)]
pub struct RestartReq {
    pub started: i64,
    pub root: Option<String>,
}
```

```rust
    pub restarting: HashMap<String, RestartReq>,
```

- [ ] **Step 6: Update the two `main.rs` sites to compile (no behavior change)**

In `src/main.rs`, `RestartAllClaude` arm (line 491):

```rust
                } else {
                    app.restarting.insert(
                        name,
                        am::app::RestartReq { started: now, root: None },
                    );
                }
```

In the poll loop (lines 265-298), change the iterator binding and use the override for the respawn directory:

```rust
            if !app.restarting.is_empty() {
                let mut to_clear: Vec<String> = Vec::new();
                for (name, req) in &app.restarting {
                    if app.now_unix - req.started > 30 {
                        let _ = tmux::set_remain_on_exit(name, false);
                        to_clear.push(name.clone());
                        continue;
                    }
                    if !tmux::pane_dead(name).unwrap_or(false) {
                        continue;
                    }
                    if let Ok(pane) = tmux::capture_pane(name) {
                        if let Some(cmd) = tmux::parse_resume_command(&pane) {
                            let dir = req.root.clone().unwrap_or_else(|| {
                                app.sessions
                                    .iter()
                                    .find(|s| s.name == *name)
                                    .map(|s| s.dir.clone())
                                    .unwrap_or_default()
                            });
                            if let Err(e) = tmux::respawn_pane(name, &dir, &cmd) {
                                app.error = Some(format!("resume: {e}"));
                            }
                            let _ = tmux::set_remain_on_exit(name, false);
                            to_clear.push(name.clone());
                        }
                    }
                }
                for name in &to_clear {
                    app.restarting.remove(name);
                }
            }
```

> Note: `app.restarting` (immutable borrow in the `for`), `app.sessions` (immutable), and `app.error` (mutable) are disjoint fields, so this compiles under disjoint-borrow rules exactly as the original did.

- [ ] **Step 7: Verify no behavior regressed**

Run: `cargo test -p amux 2>&1 | tail -15`
Expected: PASS. The `u`-restart path now carries `root: None` and respawns in `s.dir` — identical to before.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/main.rs src/tmux.rs
git commit -m "refactor(restart): RestartReq carries a respawn-cwd override; add shell_single_quote"
```

---

### Task 5: `Action::ReturnToRoot` + `c` key + execution

Add the action, bind `c` to it only when the selected session is *Returnable*, and execute it: Claude sessions go through the restart pipeline with `root: Some(root)`; shells get a direct `cd`.

**Files:**
- Modify: `src/app.rs` (`Action` enum ~line 800-810; `handle_normal_key` ~line 1334)
- Modify: `src/main.rs` (new match arm next to `RestartAllClaude` ~line 495)
- Test: `src/app.rs` (tests module)

- [ ] **Step 1: Write the failing key-binding tests**

Add to `#[cfg(test)] mod tests` in `src/app.rs`:

```rust
#[test]
fn c_returns_to_root_for_returnable_session() {
    let mut app = app_with_two_sessions();
    app.sessions[0].worktree_repo = Some("/repo".into());
    app.git_cache.insert("/a".to_string(), None); // cwd "/a" → confirmed no repo
    app.selected = 0;
    let action = app.handle_key(key('c'));
    assert_eq!(
        action,
        Some(Action::ReturnToRoot {
            name: "a".into(),
            root: "/repo".into()
        })
    );
}

#[test]
fn c_is_a_noop_when_not_returnable() {
    let mut app = app_with_two_sessions(); // no git_cache entry → Loading
    app.selected = 0;
    assert_eq!(app.handle_key(key('c')), None);

    // Some(None) but no worktree_repo → NoRepo, still a no-op.
    app.git_cache.insert("/a".to_string(), None);
    assert_eq!(app.handle_key(key('c')), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p amux --lib c_returns_to_root_for_returnable_session c_is_a_noop_when_not_returnable 2>&1 | tail -20`
Expected: compile error — `Action::ReturnToRoot` is undefined.

- [ ] **Step 3: Add the `Action::ReturnToRoot` variant**

In `src/app.rs`, inside `pub enum Action` (before the closing brace at line 811):

```rust
    /// Return a session whose worktree was removed back to its repo root. For a
    /// Claude session this restarts it (resumed) in `root`; for a shell it sends
    /// `cd <root>`.
    ReturnToRoot {
        name: String,
        root: String,
    },
```

- [ ] **Step 4: Bind `c` in `handle_normal_key`**

In `src/app.rs`, add an arm after the `'u'` arm (line 1338). It fires only when the selected session classifies as *Returnable*:

```rust
            // c: return a removed-worktree session to its repo root. Only active
            // when the selected card shows the "return to root?" prompt.
            KeyCode::Char('c') => {
                if let Some(s) = self.selected_session() {
                    if git_card_state(&self.git_cache, s) == GitCardState::Returnable {
                        return Some(Action::ReturnToRoot {
                            name: s.name.clone(),
                            root: session_root(s).to_string(),
                        });
                    }
                }
            }
```

- [ ] **Step 5: Run the key tests to verify they pass**

Run: `cargo test -p amux --lib c_returns_to_root_for_returnable_session c_is_a_noop_when_not_returnable 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Execute the action in `main.rs`**

In `src/main.rs`, add a new arm after the `RestartAllClaude` arm (after line 495). It mirrors the per-session restart body for Claude, and sends `cd` for a shell.

```rust
        Action::ReturnToRoot { name, root } => {
            let is_claude = app
                .sessions
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.agent.split_whitespace().next() == Some("claude"))
                .unwrap_or(false);
            if is_claude {
                // Same pipeline as `u`, but the poll loop respawns in `root`:
                // remain-on-exit keeps the dead pane with the --resume hint,
                // Ctrl+C exits Claude, then it is respawned (resumed) in root.
                let now = app.now_unix;
                if let Err(e) = tmux::set_remain_on_exit(&name, true) {
                    app.error = Some(format!("return to root: {e}"));
                } else if let Err(e) = tmux::send_ctrl_c(&name) {
                    app.error = Some(format!("return to root: {e}"));
                    let _ = tmux::set_remain_on_exit(&name, false);
                } else {
                    app.restarting.insert(
                        name,
                        am::app::RestartReq { started: now, root: Some(root) },
                    );
                }
            } else {
                // Plain shell: run the cd directly.
                let cmd = format!("cd {}", tmux::shell_single_quote(&root));
                if let Err(e) = tmux::send_text(&name, &cmd) {
                    app.error = Some(format!("return to root: {e}"));
                }
            }
            app.refresh();
        }
```

- [ ] **Step 7: Build and run the full suite**

Run: `cargo test -p amux 2>&1 | tail -15`
Expected: PASS (match on `Action` is now exhaustive; key tests green).

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat(app): 'c' returns a removed-worktree session to its repo root"
```

---

### Task 6: Help entry, changelog, final verification

**Files:**
- Modify: `src/ui/modal_help.rs` (keys list ~line 22-32)
- Modify: `CHANGELOG.md` (Unreleased / Added)

- [ ] **Step 1: Add the help entry**

In `src/ui/modal_help.rs`, add to the shortcuts list (next to `("d", "kill")`, line 27):

```rust
                ("c", "return to root (stale cwd)"),
```

- [ ] **Step 2: Add a changelog bullet**

In `CHANGELOG.md`, under Unreleased → Added (top of the list):

```markdown
- Sessions whose pane directory no longer resolves to a git repo now show their
  state instead of a blank: `no repo`, or `worktree removed · return to root? c`.
  Pressing `c` returns the session to its project root (resuming Claude in place).
```

- [ ] **Step 3: Format**

Run: `cargo fmt -p amux`
Expected: no errors. (Use `-p amux` to avoid touching `amux-verify` formatting.)

- [ ] **Step 4: Lint the whole workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5`
Expected: `Finished` with no warnings.

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | grep -E "^test result|error|FAILED"`
Expected: every line `test result: ok`; no `FAILED`.

- [ ] **Step 6: Manual smoke (optional but recommended)**

Build the app and, with a session whose worktree was removed, confirm the card shows
`worktree removed · return to root? c` and that `c` returns it to root. (If no live
removed-worktree session is at hand, this is covered by the unit + render tests.)

- [ ] **Step 7: Commit**

```bash
git add src/ui/modal_help.rs CHANGELOG.md
git commit -m "docs: help entry + changelog for stale-cwd return-to-root"
```

---

## Self-Review

**Spec coverage:**
- Data-layer verdict (`Option<GitInfo>`, loading≠no-repo) → Task 1. ✅
- 4-way render states + wording + `c` chip → Task 3 (classifier in Task 2). ✅
- Returnable vs NoRepo gated on `worktree_repo.is_some()` → Task 2. ✅
- `c` key, contextual, → `Action::ReturnToRoot { name, root }`, `root = session_root` → Task 5. ✅
- Claude branch reuses `u` pipeline with respawn-cwd override; shell branch `cd '<root>'` → Tasks 4 (override + quoting) + 5 (handler). ✅
- `App.restarting` → `RestartReq { started, root }` → Task 4. ✅
- Help entry → Task 6. ✅
- Tests: reader verdict, state mapping, render states, key binding, restart override (preserved), quoting → Tasks 1-5. ✅
- Edge cases (reader race → Loading blank; non-worktree no-repo → NoRepo; Claude-dead → 30s timeout) handled by existing pipeline + classifier. ✅

**Placeholder scan:** none — every code step shows complete code.

**Type consistency:** `git_cache: HashMap<String, Option<GitInfo>>` (Task 1) is consumed by `git_card_state(cache, s)` (Task 2), the renderer (Task 3), and the `c` handler (Task 5) with the same type. `RestartReq { started: i64, root: Option<String> }` (Task 4) is constructed identically in `RestartAllClaude` (`root: None`) and `ReturnToRoot` (`root: Some`), and read in the poll loop. `GitCardState` variants are referenced consistently. `card()` gains exactly one param (`git_state`) used at its single call site.

**Out of scope (per spec):** respawn-based hard reset for the shell case; footer hint; filesystem watcher for external removal.
