# Worktree Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional "create git worktree" mode to session creation so an agent works on its own branch in `<repo>/.worktrees/<branch>`, isolated from the main working tree.

**Architecture:** The create form gains a toggle + base-branch picker + new-branch field. On submit, `Action::Create` carries an optional `WorktreeSpec`; `handle_action` resolves the repo root, ensures `.gitignore`, runs `git worktree add`, then starts the tmux session in the worktree path tagged with `@cm_repo`. The kill flow reads that tag to offer worktree removal. All git mutation lives in `git.rs`.

**Tech Stack:** Rust, ratatui (TUI), crossterm, tmux CLI, git CLI.

---

## File Structure

- `src/git.rs` — add mutating helpers: `repo_root`, `list_branches`, `add_worktree`, `remove_worktree`, `ensure_gitignore`. (Currently read-only; new fns documented as mutating.)
- `src/tmux.rs` — add `@cm_repo` to `LIST_FORMAT`, `worktree_repo: Option<String>` on `Session`, parse it, and a `new_worktree_session` that tags `@cm_repo`.
- `src/app.rs` — `CreateForm` worktree fields + `CreateField::{Worktree,Base,Branch}` + navigation; `WorktreeSpec`; `Action::Create` gains `worktree`; `Action::Kill` becomes a struct; new `KillForm`; `Mode::ConfirmDelete(KillForm)`.
- `src/ui/modal_new.rs` — render the toggle / BASE / NEW BRANCH rows + dynamic step indicator + updated command preview.
- `src/ui/modal_kill.rs` — render the "also remove worktree" toggle when present.
- `src/main.rs` — `handle_action` wiring for worktree create + kill removal.

Tests live inline (`#[cfg(test)] mod tests`) in each module, matching the existing convention.

---

## Task 1: `git.rs` — repo_root + list_branches

**Files:**
- Modify: `src/git.rs`

- [ ] **Step 1: Write failing tests**

Add to the existing `#[cfg(test)] mod tests` in `src/git.rs`. Reuse the temp-repo
setup pattern from `read_repo_reports_branch_and_diff` (init, config user, commit).

```rust
    /// Builds a temp git repo on branch `main` with one commit. Returns (dir, path-string-owner).
    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cm_wt_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn repo_root_resolves_from_subdir() {
        if Command::new("git").arg("--version").output().is_err() { return; }
        let dir = temp_repo("root");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let root = repo_root(sub.to_str().unwrap()).expect("root");
        // macOS temp dir is symlinked (/var -> /private/var); compare canonicalized.
        assert_eq!(
            std::fs::canonicalize(&root).unwrap(),
            std::fs::canonicalize(&dir).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_branches_puts_current_first() {
        if Command::new("git").arg("--version").output().is_err() { return; }
        let dir = temp_repo("branches");
        let d = dir.to_str().unwrap();
        Command::new("git").arg("-C").arg(d).args(["branch", "feature"]).output().unwrap();
        let branches = list_branches(d);
        assert_eq!(branches.first().map(String::as_str), Some("main"));
        assert!(branches.iter().any(|b| b == "feature"));
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib git::tests::repo_root_resolves_from_subdir git::tests::list_branches_puts_current_first`
Expected: compile error / FAIL — `repo_root` and `list_branches` not found.

- [ ] **Step 3: Implement both functions**

Add after `read` in `src/git.rs`:

```rust
/// Absolute path of the repository containing `dir`, or None if `dir` is not in a repo.
pub fn repo_root(dir: &str) -> Option<String> {
    git_out(dir, &["rev-parse", "--show-toplevel"])
}

/// Local branch names for `dir`, with the current branch first.
/// Empty when `dir` is not a git repo.
pub fn list_branches(dir: &str) -> Vec<String> {
    let Some(out) = git_out(dir, &["branch", "--format=%(refname:short)"]) else {
        return Vec::new();
    };
    let mut branches: Vec<String> =
        out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if let Some(cur) = git_out(dir, &["symbolic-ref", "--short", "HEAD"]) {
        if let Some(i) = branches.iter().position(|b| *b == cur) {
            branches.remove(i);
            branches.insert(0, cur);
        }
    }
    branches
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib git::tests::repo_root_resolves_from_subdir git::tests::list_branches_puts_current_first`
Expected: PASS (or quietly returns if git is absent).

- [ ] **Step 5: Commit**

```bash
git add src/git.rs
git commit -m "feat(git): repo_root + list_branches helpers"
```

---

## Task 2: `git.rs` — add_worktree, remove_worktree, ensure_gitignore

**Files:**
- Modify: `src/git.rs`

- [ ] **Step 1: Write failing tests**

Add to `mod tests` in `src/git.rs` (uses `temp_repo` from Task 1):

```rust
    #[test]
    fn add_then_remove_worktree() {
        if Command::new("git").arg("--version").output().is_err() { return; }
        let dir = temp_repo("wt");
        let root = dir.to_str().unwrap();
        let wt = dir.join(".worktrees").join("feature-x");
        let wt_s = wt.to_str().unwrap();

        add_worktree(root, wt_s, "feature-x", "main").expect("add");
        assert!(wt.join("f.txt").exists(), "worktree checked out base content");
        // branch was created
        let branches = list_branches(root);
        assert!(branches.iter().any(|b| b == "feature-x"));

        remove_worktree(root, wt_s).expect("remove");
        assert!(!wt.exists(), "worktree dir gone after remove");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_worktree_existing_branch_errors() {
        if Command::new("git").arg("--version").output().is_err() { return; }
        let dir = temp_repo("wtdup");
        let root = dir.to_str().unwrap();
        Command::new("git").arg("-C").arg(root).args(["branch", "dup"]).output().unwrap();
        let wt = dir.join(".worktrees").join("dup");
        let err = add_worktree(root, wt.to_str().unwrap(), "dup", "main");
        assert!(err.is_err(), "creating an existing branch must fail");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_gitignore_appends_once() {
        let dir = std::env::temp_dir().join(format!("cm_ign_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_str().unwrap();
        ensure_gitignore(root, ".worktrees/").unwrap();
        ensure_gitignore(root, ".worktrees/").unwrap(); // idempotent
        let body = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(body.matches(".worktrees/").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib git::tests::add_then_remove_worktree git::tests::add_worktree_existing_branch_errors git::tests::ensure_gitignore_appends_once`
Expected: compile error — functions not found.

- [ ] **Step 3: Implement the three functions**

Add to `src/git.rs`. Add `use std::io::Write as _;` near the top (alongside the
existing `use std::process::Command;`). Add a mutating command runner mirroring
`git_out`:

```rust
/// Runs a *mutating* git command in `dir`. Unlike `git_out`, returns the
/// stderr-bearing error on failure so the UI can show why it failed.
fn git_run(dir: &str, args: &[&str]) -> std::io::Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Creates a new worktree at `wt_path` on a new branch `new_branch` forked from `base`.
/// Fails if `new_branch` already exists.
pub fn add_worktree(repo_root: &str, wt_path: &str, new_branch: &str, base: &str) -> std::io::Result<()> {
    git_run(repo_root, &["worktree", "add", "-b", new_branch, wt_path, base])
}

/// Removes the worktree at `wt_path`. No `--force`: a dirty worktree errors
/// instead of silently discarding work. The branch itself is left intact.
pub fn remove_worktree(repo_root: &str, wt_path: &str) -> std::io::Result<()> {
    git_run(repo_root, &["worktree", "remove", wt_path])
}

/// Appends `entry` as its own line to `<repo_root>/.gitignore` if not already
/// present (exact-line match). Creates the file when missing.
pub fn ensure_gitignore(repo_root: &str, entry: &str) -> std::io::Result<()> {
    let path = std::path::Path::new(repo_root).join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.write_all(entry.as_bytes())?;
    f.write_all(b"\n")?;
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib git::tests::add_then_remove_worktree git::tests::add_worktree_existing_branch_errors git::tests::ensure_gitignore_appends_once`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/git.rs
git commit -m "feat(git): add/remove worktree + ensure_gitignore"
```

---

## Task 3: `tmux.rs` — @cm_repo on Session

**Files:**
- Modify: `src/tmux.rs`

- [ ] **Step 1: Write failing tests**

Update the existing `parses_managed_session` expectation and add a new test.
The `LIST_FORMAT` gains a 7th field, so existing fixtures need the extra column.
Add to `mod tests` in `src/tmux.rs`:

```rust
    #[test]
    fn parses_worktree_repo() {
        let out = "wt\t/r/.worktrees/x\t1\t1\tclaude\t0\t/r";
        let s = &parse_sessions(out)[0];
        assert_eq!(s.worktree_repo.as_deref(), Some("/r"));
    }

    #[test]
    fn empty_worktree_repo_is_none() {
        let out = "plain\t/d\t1\t1\tclaude\t0\t";
        let s = &parse_sessions(out)[0];
        assert_eq!(s.worktree_repo, None);
    }
```

Also update the existing `parses_managed_session` fixture string to add a
trailing `\t` (empty repo field) and assert `worktree_repo` is `None`:

```rust
        let out = "proj-a\t/home/u/proj-a\t1716800000\t1\tclaude\t0\t";
        // ... existing asserts ...
        assert_eq!(sessions[0].worktree_repo, None);
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib tmux::tests`
Expected: compile error — `worktree_repo` field missing.

- [ ] **Step 3: Implement**

In `src/tmux.rs`:

Extend `LIST_FORMAT` (add the repo field at the end):
```rust
pub const LIST_FORMAT: &str =
    "#{session_name}\t#{session_path}\t#{session_created}\t#{@cm_managed}\t#{@cm_agent}\t#{session_attached}\t#{@cm_repo}";
```

Add to `struct Session` (after `git`):
```rust
    /// Repo root if this session runs in a `cm`-created worktree; None otherwise.
    pub worktree_repo: Option<String>,
```

In `parse_line`, change the splitn count from 6 to 7 and read the field. The
`attached` field is currently read via `f.next()` after `managed` check; keep
order: name, dir, created, managed, agent, attached, repo. Rewrite the tail:

```rust
fn parse_line(line: &str) -> Option<Session> {
    let mut f = line.splitn(7, '\t');
    let name = f.next()?.to_string();
    let dir = f.next()?.to_string();
    let created = f.next()?.trim().parse::<i64>().ok()?;
    let managed = f.next()?;
    let agent = f.next()?.to_string();
    if managed != "1" {
        return None;
    }
    let attached = f
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|n| n > 0)
        .unwrap_or(false);
    let worktree_repo = match f.next().map(str::trim) {
        Some(r) if !r.is_empty() => Some(r.to_string()),
        _ => None,
    };
    Some(Session {
        name,
        dir,
        created,
        agent,
        status: Status::Idle,
        attached,
        git: None,
        worktree_repo,
    })
}
```

- [ ] **Step 4: Fix other `Session { ... }` literals**

Building now fails wherever a `Session` is constructed without `worktree_repo`.
Add `worktree_repo: None,` to each. Find them:

Run: `grep -rn "git: None," src tests`
Expected sites: `src/app.rs` test helpers (the two sessions in
`app_with_two_sessions`, ~lines 1160/1169) and any in `tests/`. Add the field to
each literal.

- [ ] **Step 5: Run tests, verify they pass**

Run: `cargo test --lib tmux::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tmux.rs src/app.rs
git commit -m "feat(tmux): track @cm_repo as Session.worktree_repo"
```

---

## Task 4: `tmux.rs` — new_worktree_session

**Files:**
- Modify: `src/tmux.rs`

- [ ] **Step 1: Implement (no unit test — needs a live tmux server)**

`new_session` already tags `@cm_managed` and `@cm_agent`. Add a variant that
also tags `@cm_repo`. Refactor to avoid duplication: keep `new_session` as the
no-worktree path and add `new_worktree_session`.

Add to `src/tmux.rs`:

```rust
/// Like `new_session`, but also tags the session with `@cm_repo=<repo_root>` so
/// the UI knows it runs in a worktree (enables worktree-aware kill).
pub fn new_worktree_session(name: &str, dir: &str, agent: &str, repo_root: &str) -> io::Result<()> {
    new_session(name, dir, agent)?;
    if let Err(e) = run(&["set-option", "-t", name, "@cm_repo", repo_root]) {
        let _ = run(&["kill-session", "-t", name]);
        return Err(e);
    }
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add src/tmux.rs
git commit -m "feat(tmux): new_worktree_session tags @cm_repo"
```

---

## Task 5: `CreateForm` worktree state + fields

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Write failing tests**

Add to `mod tests` in `src/app.rs`:

```rust
    #[test]
    fn worktree_off_skips_base_and_branch() {
        let mut form = CreateForm::new("claude", &[]);
        // Name -> Dir -> Worktree(off) -> Agent
        form.field = CreateField::Worktree;
        assert!(!form.worktree);
        form.advance(); // toggle off -> straight to Agent
        assert_eq!(form.field, CreateField::Agent);
    }

    #[test]
    fn worktree_on_visits_base_and_branch() {
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Worktree;
        form.toggle_worktree(); // turn on
        assert!(form.worktree);
        form.advance();
        assert_eq!(form.field, CreateField::Base);
        form.advance();
        assert_eq!(form.field, CreateField::Branch);
        form.advance();
        assert_eq!(form.field, CreateField::Agent);
    }

    #[test]
    fn step_count_grows_with_worktree() {
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.total_steps(), 3);
        form.worktree = true;
        assert_eq!(form.total_steps(), 5);
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib app::tests::worktree_off_skips_base_and_branch app::tests::worktree_on_visits_base_and_branch app::tests::step_count_grows_with_worktree`
Expected: compile error — variants/methods missing.

- [ ] **Step 3: Implement**

In `src/app.rs`:

Extend the enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateField {
    Name,
    Dir,
    Worktree,
    Base,
    Branch,
    Agent,
}
```

Add fields to `CreateForm`:
```rust
    pub worktree: bool,
    pub base_branches: Vec<String>,
    pub base_index: usize,
    pub new_branch: String,
```

Initialize in `CreateForm::new` (after `agent_index: 0,`):
```rust
            worktree: false,
            base_branches: Vec::new(),
            base_index: 0,
            new_branch: String::new(),
```

Replace `next_field` with a worktree-aware version and add helpers. The
`current_mut` match must also handle the new variants (`Base` is a picker, not a
text field — route it to `new_branch`'s sibling carefully). Update `current_mut`:

```rust
    fn current_mut(&mut self) -> &mut String {
        match self.field {
            CreateField::Name => &mut self.name,
            CreateField::Dir => &mut self.dir,
            CreateField::Branch => &mut self.new_branch,
            CreateField::Agent => &mut self.agent,
            // Worktree (toggle) and Base (picker) have no text buffer; route to a
            // throwaway so callers that always have a buffer still compile. These
            // fields are never reached as text inputs (handled before current_mut).
            CreateField::Worktree | CreateField::Base => &mut self.agent,
        }
    }

    fn next_field(&self) -> CreateField {
        match self.field {
            CreateField::Name => CreateField::Dir,
            CreateField::Dir => CreateField::Worktree,
            CreateField::Worktree if self.worktree => CreateField::Base,
            CreateField::Worktree => CreateField::Agent,
            CreateField::Base => CreateField::Branch,
            CreateField::Branch => CreateField::Agent,
            CreateField::Agent => CreateField::Name,
        }
    }

    /// Advance focus to the next field (used by Tab/Enter and tests).
    pub fn advance(&mut self) {
        self.field = self.next_field();
        if self.field == CreateField::Dir {
            self.refresh_dir_entries();
        }
    }

    /// Toggle the worktree option. On enabling, load branches and prefill the
    /// new-branch name from the session name (only if still empty).
    pub fn toggle_worktree(&mut self) {
        self.worktree = !self.worktree;
        if self.worktree {
            self.base_branches = crate::git::list_branches(&expand_tilde(&self.dir));
            self.base_index = 0;
            if self.new_branch.is_empty() {
                self.new_branch = self.name.trim().to_string();
            }
        }
    }

    /// Move the base-branch selection by `delta` (wraps). No-op if no branches.
    pub fn cycle_base(&mut self, delta: isize) {
        let n = self.base_branches.len() as isize;
        if n == 0 {
            return;
        }
        self.base_index = (((self.base_index as isize + delta) % n + n) % n) as usize;
    }

    /// Total number of steps shown in the `N of M` indicator.
    pub fn total_steps(&self) -> usize {
        if self.worktree { 5 } else { 3 }
    }
```

Update `step` to number the new fields:
```rust
    pub fn step(&self) -> usize {
        match self.field {
            CreateField::Name => 1,
            CreateField::Dir => 2,
            CreateField::Worktree => 3,
            CreateField::Base => 3,
            CreateField::Branch => 4,
            CreateField::Agent => self.total_steps(),
        }
    }
```

Note: the existing `step_tracks_focused_field` test asserts `Agent` => 3. With
no worktree, `total_steps()` is 3, so `Agent => 3` still holds — that test stays
green.

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib app::tests`
Expected: PASS (including the pre-existing `step_tracks_focused_field`).

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): CreateForm worktree fields + navigation"
```

---

## Task 6: WorktreeSpec + Action::Create carries it

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Write failing test**

Add to `mod tests` in `src/app.rs`. This drives the form to submit with worktree on:

```rust
    #[test]
    fn create_action_carries_worktree_spec() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.name = "iso".into();
        form.dir = "/tmp".into(); // exists as a dir on CI/macOS/Linux
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.base_index = 0;
        form.new_branch = "iso-branch".into();
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            Some(Action::Create { worktree: Some(spec), .. }) => {
                assert_eq!(spec.base, "main");
                assert_eq!(spec.new_branch, "iso-branch");
            }
            other => panic!("expected Create with worktree, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --lib app::tests::create_action_carries_worktree_spec`
Expected: compile error — `WorktreeSpec` / `worktree` field missing.

- [ ] **Step 3: Implement**

In `src/app.rs`, add the spec type (near `Action`):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct WorktreeSpec {
    pub base: String,
    pub new_branch: String,
}
```

Change the `Action::Create` variant:
```rust
    Create {
        name: String,
        dir: String,
        agent: String,
        worktree: Option<WorktreeSpec>,
    },
```

In `handle_create_key`, where `Action::Create` is returned (the `Enter` arm on
the Agent step, ~line 804), build the spec:
```rust
                        Ok(()) => {
                            self.error = None;
                            let worktree = if form.worktree {
                                Some(WorktreeSpec {
                                    base: form
                                        .base_branches
                                        .get(form.base_index)
                                        .cloned()
                                        .unwrap_or_default(),
                                    new_branch: form.new_branch.trim().to_string(),
                                })
                            } else {
                                None
                            };
                            return Some(Action::Create {
                                name: form.name.trim().to_string(),
                                dir: expand_tilde(&form.dir),
                                agent: form.agent.clone(),
                                worktree,
                            });
                        }
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test --lib app::tests::create_action_carries_worktree_spec`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): Action::Create carries optional WorktreeSpec"
```

---

## Task 7: Create-form key handling for new fields

**Files:**
- Modify: `src/app.rs` (`handle_create_key`)

- [ ] **Step 1: Write failing test**

```rust
    #[test]
    fn space_toggles_worktree_on_worktree_step() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.name = "s".into();
        form.field = CreateField::Worktree;
        app.mode = Mode::Create(form);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        match &app.mode {
            Mode::Create(f) => {
                assert!(f.worktree);
                assert_eq!(f.new_branch, "s"); // prefilled from name
            }
            _ => panic!("still in create mode"),
        }
    }

    #[test]
    fn left_right_cycles_base_branch() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.worktree = true;
        form.base_branches = vec!["main".into(), "dev".into()];
        form.field = CreateField::Base;
        app.mode = Mode::Create(form);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        match &app.mode {
            Mode::Create(f) => assert_eq!(f.base_index, 1),
            _ => panic!(),
        }
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib app::tests::space_toggles_worktree_on_worktree_step app::tests::left_right_cycles_base_branch`
Expected: FAIL — space does nothing / base_index unchanged.

- [ ] **Step 3: Implement**

In `handle_create_key`, the existing code has a `Dir`-specific block, then a
general match for "Name / agent steps". Insert dedicated handling for the new
fields. After the `Dir` block (before the "Name / agent steps" match), add:

```rust
        // Worktree toggle step.
        if form.field == CreateField::Worktree {
            match key.code {
                KeyCode::Esc => return None,
                KeyCode::Char(' ') => form.toggle_worktree(),
                KeyCode::Tab | KeyCode::Enter => form.advance(),
                _ => {}
            }
            self.mode = Mode::Create(form);
            return None;
        }

        // Base-branch picker step.
        if form.field == CreateField::Base {
            match key.code {
                KeyCode::Esc => return None,
                KeyCode::Left => form.cycle_base(-1),
                KeyCode::Right => form.cycle_base(1),
                KeyCode::Tab | KeyCode::Enter => form.advance(),
                _ => {}
            }
            self.mode = Mode::Create(form);
            return None;
        }
```

The existing general match already handles `Branch` and `Agent` as text fields
via `current_mut()` (Branch routes to `new_branch`). But its `Tab`/`Enter` arms
use `form.next_field()` / advance to dir — confirm they call the worktree-aware
`next_field`. Replace the `KeyCode::Tab` arm body in the general match with
`form.advance();` and the non-Agent `Enter` arm's `form.field = form.next_field(); form.refresh_dir_entries();` with `form.advance();` (advance already refreshes dir entries when landing on Dir).

The `Enter`-on-Agent arm (submit) is unchanged from Task 6.

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib app::tests`
Expected: PASS (all create-flow tests).

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): key handling for worktree/base/branch steps"
```

---

## Task 8: Render worktree rows in modal_new

**Files:**
- Modify: `src/ui/modal_new.rs`

- [ ] **Step 1: Write failing test**

Add to `mod tests` in `src/ui/modal_new.rs`:

```rust
    #[test]
    fn new_modal_shows_worktree_rows_when_enabled() {
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.new_branch = "feature-x".into();
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("B A S E"), "BASE label");
        assert!(s.contains("B R A N C H"), "BRANCH label");
        assert!(s.contains("feature-x"));
        assert!(s.contains("of 5"), "dynamic step total");
    }

    #[test]
    fn new_modal_hides_worktree_rows_by_default() {
        let form = CreateForm::new("claude", &["claude".into()]);
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("B A S E"));
        assert!(s.contains("of 3"));
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib modal_new`
Expected: FAIL — labels absent, "of 5" absent.

- [ ] **Step 3: Implement**

In `src/ui/modal_new.rs`:

Change the step indicator (currently `format!("{} of 3", form.step())`) to:
```rust
        let step = Line::from(Span::styled(
            format!("{} of {}", form.step(), form.total_steps()),
            Style::default().fg(th::DIM),
        ));
```

After the DIRECTORY section / validation rows (after the `y += 1;` that follows
the picker block, before the AGENT label), insert a WORKTREE section. Use the
existing `label`, `input_box`, and `row` helpers. The toggle renders as a
checkbox line; when on, render BASE (segmented picker like the agent selector)
and BRANCH (input_box). Adjust `BASE_ROWS` upward to reserve space — bump the
const so the panel is tall enough when expanded:

```rust
/// Fixed content rows (everything except the variable-height picker and the
/// optional worktree rows).
const BASE_ROWS: u16 = 18;
/// Extra rows when the worktree toggle is on (label+toggle + BASE + BRANCH).
const WORKTREE_ROWS: u16 = 5;
```

And in `render`, fold the extra height in:
```rust
    let wt_extra = if form.worktree { WORKTREE_ROWS } else { 0 };
    let h = (BASE_ROWS + want_picker + wt_extra + 4).min(full.height);
```

Insert the section (mirror the AGENT segmented selector idiom for BASE):
```rust
    // WORKTREE (optional)
    if let Some(r) = row(x, y, w, bottom) {
        let mark = if form.worktree { "[x]" } else { "[ ]" };
        let line = Line::from(vec![
            Span::styled(format!("{mark} "), Style::default().fg(th::AMBER)),
            Span::styled("Create worktree", Style::default().fg(th::TEXT_BOLD)),
            Span::styled("   space to toggle", Style::default().fg(th::DIM)),
        ]);
        let lbl = if form.field == CreateField::Worktree {
            label("WORKTREE", th::AMBER)
        } else {
            label("WORKTREE", th::MUTED)
        };
        f.render_widget(Paragraph::new(Line::from(lbl)), r);
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        let mark = if form.worktree { "[x]" } else { "[ ]" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(th::AMBER)),
                Span::styled("Create worktree", Style::default().fg(th::TEXT_BOLD)),
                Span::styled("   space to toggle", Style::default().fg(th::DIM)),
            ])),
            r,
        );
    }
    y += 1;
    if form.worktree {
        // BASE picker (segmented, cycled with ← →)
        if let Some(r) = row(x, y, w, bottom) {
            f.render_widget(Paragraph::new(Line::from(label("BASE", th::MUTED))), r);
        }
        y += 1;
        if let Some(r) = row(x, y, w, bottom) {
            let mut seg: Vec<Span> = Vec::new();
            for (i, b) in form.base_branches.iter().enumerate() {
                if i > 0 {
                    seg.push(Span::raw("   "));
                }
                let sel = i == form.base_index;
                let st = if sel {
                    Style::default().bg(th::AMBER).fg(th::BG).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(th::MUTED)
                };
                seg.push(Span::styled(format!(" {b} "), st));
            }
            if form.base_branches.is_empty() {
                seg.push(Span::styled(" (no branches) ", Style::default().fg(th::DIM)));
            }
            f.render_widget(
                Paragraph::new(Line::from(seg)).style(Style::default().bg(th::BG_SUNKEN)),
                r,
            );
        }
        y += 1;
        // NEW BRANCH input
        if let Some(r) = row(x, y, w, bottom) {
            f.render_widget(Paragraph::new(Line::from(label("BRANCH", th::MUTED))), r);
        }
        y += 1;
        if let Some(r) = row(x, y, w, bottom) {
            input_box(f, r, &form.new_branch, form.field == CreateField::Branch);
        }
        y += 1;
    }
```

(Remove the stray first `if let Some(r)` block that only computes `line`/`lbl`
without rendering — keep just the rendered version above. The plan shows both for
clarity; implement only the rendering blocks.)

Add `use crate::app::CreateField;` to the imports at the top (the first `use`
already pulls several `crate::app` items — add `CreateField` there).

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib modal_new`
Expected: PASS (both new tests and the pre-existing `new_modal_shows_header_labels_and_agent_segments`).

- [ ] **Step 5: Commit**

```bash
git add src/ui/modal_new.rs
git commit -m "feat(ui): render worktree toggle/base/branch in new-session modal"
```

---

## Task 9: KillForm + worktree-aware kill action

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Write failing tests**

```rust
    #[test]
    fn kill_without_worktree_yields_plain_kill() {
        let mut app = app_with_two_sessions(); // sessions have worktree_repo: None
        app.handle_key(key('d'));
        let action = app.handle_key(key('y'));
        match action {
            Some(Action::Kill { name, remove_worktree }) => {
                assert_eq!(name, "a");
                assert!(!remove_worktree);
            }
            other => panic!("expected Kill, got {other:?}"),
        }
    }

    #[test]
    fn kill_toggles_and_removes_worktree() {
        let mut app = app_with_two_sessions();
        app.sessions[0].worktree_repo = Some("/repo".into());
        app.selected = 0;
        app.handle_key(key('d'));
        // space toggles "also remove worktree"
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        let action = app.handle_key(key('y'));
        match action {
            Some(Action::Kill { name, remove_worktree }) => {
                assert_eq!(name, "a");
                assert!(remove_worktree);
            }
            other => panic!("expected Kill, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib app::tests::kill_without_worktree_yields_plain_kill app::tests::kill_toggles_and_removes_worktree`
Expected: compile error — `Action::Kill` is a tuple variant; `KillForm` missing.

- [ ] **Step 3: Implement**

Add `KillForm`:
```rust
#[derive(Debug, Clone)]
pub struct KillForm {
    pub name: String,
    /// repo root + worktree path when the session is worktree-backed.
    pub worktree: Option<(String, String)>,
    pub remove_worktree: bool,
}
```

Change `Mode::ConfirmDelete(String)` → `Mode::ConfirmDelete(KillForm)`.

Change `Action::Kill(String)` →
```rust
    Kill {
        name: String,
        remove_worktree: bool,
    },
```

(The `remove_worktree` flag is enough for `handle_action`, which re-derives the
repo/path from the live session — see Task 11. Keeping the path in `KillForm` is
for rendering; the action stays minimal.)

Update the `'d'` handler (~line 647) to build a `KillForm` from the selected
session:
```rust
            KeyCode::Char('d') => {
                if let Some(s) = self.selected_session() {
                    let worktree = s
                        .worktree_repo
                        .clone()
                        .map(|repo| (repo, s.dir.clone()));
                    self.mode = Mode::ConfirmDelete(KillForm {
                        name: s.name.clone(),
                        worktree,
                        remove_worktree: false,
                    });
                }
            }
```

Rewrite `handle_confirm_key`:
```rust
    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::ConfirmDelete(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match latin_code(key.code) {
            KeyCode::Char('y') => {
                return Some(Action::Kill {
                    name: form.name.clone(),
                    remove_worktree: form.worktree.is_some() && form.remove_worktree,
                })
            }
            KeyCode::Char(' ') if form.worktree.is_some() => {
                form.remove_worktree = !form.remove_worktree;
                self.mode = Mode::ConfirmDelete(form);
            }
            KeyCode::Char('n') | KeyCode::Esc => {}
            _ => self.mode = Mode::ConfirmDelete(form),
        }
        None
    }
```

Update `mode_kind` match arm `Mode::ConfirmDelete(_) => ModeKind::ConfirmDelete`
— unchanged (still `_`). Update the pre-existing `kill_flow` test (~line 1299)
that asserts `Action::Kill("a".into())` to the new struct form:
`assert_eq!(action, Some(Action::Kill { name: "a".into(), remove_worktree: false }));`

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib app::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): KillForm + worktree-aware Kill action"
```

---

## Task 10: Render kill toggle + fix ui/mod dispatch

**Files:**
- Modify: `src/ui/modal_kill.rs`, `src/ui/mod.rs`

- [ ] **Step 1: Update the render dispatch in `ui/mod.rs`**

The match arm `Mode::ConfirmDelete(name) => { ... modal_kill::render(f, name, s); }`
now holds a `KillForm`. Change it to pass the form:
```rust
        Mode::ConfirmDelete(form) => {
            let s = app.sessions.iter().find(|s| s.name == form.name);
            modal_kill::render(f, form, s);
        }
```

- [ ] **Step 2: Write failing test in modal_kill**

```rust
    #[test]
    fn kill_modal_shows_worktree_toggle_when_present() {
        use crate::app::KillForm;
        let form = KillForm {
            name: "wt".into(),
            worktree: Some(("/repo".into(), "/repo/.worktrees/wt".into())),
            remove_worktree: true,
        };
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &form, None)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("remove worktree"));
        assert!(s.contains("[x]"));
    }
```

- [ ] **Step 3: Run test, verify it fails**

Run: `cargo test --lib modal_kill`
Expected: compile error — `render` signature still `(f, name, session)`.

- [ ] **Step 4: Implement**

Change `render` signature to `pub fn render(f: &mut Frame, form: &crate::app::KillForm, session: Option<&Session>)`.
Replace internal `name` uses with `form.name`. Before the final hint line, when
`form.worktree.is_some()`, insert a toggle line:
```rust
    if form.worktree.is_some() {
        let mark = if form.remove_worktree { "[x]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} ", ), Style::default().fg(th::AMBER)),
            Span::styled("also remove worktree", Style::default().fg(th::TEXT)),
            Span::styled("   space to toggle", Style::default().fg(th::DIM)),
        ]));
        lines.push(Line::from(""));
    }
```

Update the existing `kill_modal_shows_name_and_warning` test to build a `KillForm`
and call the new signature:
```rust
        let form = crate::app::KillForm { name: "project-a".into(), worktree: None, remove_worktree: false };
        t.draw(|f| render(f, &form, None)).unwrap();
```

- [ ] **Step 5: Run tests, verify they pass**

Run: `cargo test --lib modal_kill`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ui/modal_kill.rs src/ui/mod.rs
git commit -m "feat(ui): kill modal worktree-removal toggle"
```

---

## Task 11: Wire handle_action in main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement Create branch**

Replace the `Action::Create { name, dir, agent }` arm (~line 190) with one that
handles the optional worktree. No unit test (needs live tmux + git); verified
manually in Task 12.

```rust
        Action::Create { name, dir, agent, worktree } => {
            let result = match worktree {
                None => tmux::new_session(&name, &dir, &agent),
                Some(spec) => create_worktree_session(&name, &dir, &agent, &spec),
            };
            if let Err(e) = result {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
```

Add a helper function in `main.rs` (above or below `handle_action`):
```rust
/// Creates a git worktree under `<repo>/.worktrees/<branch>` then starts a tmux
/// session in it. Any git step failing aborts before the session is created.
fn create_worktree_session(
    name: &str,
    dir: &str,
    agent: &str,
    spec: &cm::app::WorktreeSpec,
) -> io::Result<()> {
    let repo = cm::git::repo_root(dir)
        .ok_or_else(|| io::Error::other(format!("not a git repo: {dir}")))?;
    cm::git::ensure_gitignore(&repo, ".worktrees/")?;
    let wt_path = std::path::Path::new(&repo)
        .join(".worktrees")
        .join(&spec.new_branch);
    let wt_str = wt_path.to_string_lossy().to_string();
    cm::git::add_worktree(&repo, &wt_str, &spec.new_branch, &spec.base)?;
    tmux::new_worktree_session(name, &wt_str, agent, &repo)
}
```

Note: check how `main.rs` refers to crate modules. If it uses `mod`/`use crate::`
rather than the `cm::` lib path, match that — `grep -n "^use\|^mod" src/main.rs`
and use the same form (e.g. `use cm::{app::..., git, tmux};` already present →
reuse those names, dropping the `cm::` prefix as appropriate).

- [ ] **Step 2: Implement Kill branch**

Replace the `Action::Kill(name)` arm (~line 196). The action no longer carries
the repo/path, so look up the session before killing to capture them:

```rust
        Action::Kill { name, remove_worktree } => {
            // Capture worktree info before the session disappears.
            let wt = if remove_worktree {
                app.sessions
                    .iter()
                    .find(|s| s.name == name)
                    .and_then(|s| s.worktree_repo.clone().map(|repo| (repo, s.dir.clone())))
            } else {
                None
            };
            if let Err(e) = tmux::kill_session(&name) {
                app.error = Some(e.to_string());
            } else if let Some((repo, path)) = wt {
                if let Err(e) = cm::git::remove_worktree(&repo, &path) {
                    app.error = Some(format!("session killed, worktree not removed: {e}"));
                }
            }
            app.refresh();
        }
```

- [ ] **Step 3: Verify it builds + full test suite**

Run: `cargo build && cargo test`
Expected: builds clean; all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire worktree create + kill in handle_action"
```

---

## Task 12: Lint, integration check, manual smoke test

**Files:** none (verification only)

- [ ] **Step 1: Clippy + fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: no warnings, formatting clean. Fix any issues, then re-commit with
`git commit -m "chore: clippy/fmt for worktree feature"`.

- [ ] **Step 2: Full test suite (incl. integration)**

Run: `cargo test`
Expected: PASS. Check `tests/tmux_integration.rs` — if it constructs `Session`
literals or asserts `LIST_FORMAT`/`Action` shapes, update them for the new
`worktree_repo` field and `Action::Kill`/`Action::Create` shapes.

- [ ] **Step 3: Manual smoke test (requires tmux + a git repo)**

Run the app against a real repo:
```bash
cargo run
```
Then:
1. Press `n`, fill name + a real git repo dir, Tab to WORKTREE, press space to
   enable, pick a base branch with ←/→, confirm the prefilled branch name,
   Tab to AGENT, Enter.
2. Verify a session appears; attach and confirm `git branch` shows the new
   branch and `pwd` is `<repo>/.worktrees/<branch>`.
3. Confirm `<repo>/.gitignore` now contains `.worktrees/`.
4. Back in cm, press `d` on that session, press space ("also remove worktree"),
   then `y`. Verify the session is gone and `<repo>/.worktrees/<branch>` is
   removed (`git worktree list` no longer shows it).

- [ ] **Step 4: Final commit (if any fixes)**

```bash
git add -A
git commit -m "test: integration + manual fixes for worktree sessions"
```

---

## Self-Review Notes

- **Spec coverage:** location `.worktrees/<branch>` (T11), base picker (T5/T7/T8),
  toggle off-by-default (T5), prefill branch name (T5/T7), auto `.gitignore`
  (T2/T11), kill toggle (T9/T10), `@cm_repo` tracking (T3/T4), error handling
  surfaced via `app.error` (T11) — all mapped.
- **Type consistency:** `WorktreeSpec { base, new_branch }`, `KillForm { name,
  worktree, remove_worktree }`, `Action::Create { ..., worktree }`, `Action::Kill
  { name, remove_worktree }`, `Session.worktree_repo`, `new_worktree_session`,
  `add_worktree/remove_worktree/ensure_gitignore/repo_root/list_branches` — used
  consistently across tasks.
- **Known follow-up:** branch names with `/` create nested dirs under
  `.worktrees/` — covered by the single `.worktrees/` ignore entry; no extra work.
