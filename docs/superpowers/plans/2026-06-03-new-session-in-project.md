# New Session In Project (`N`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `N` (Shift+N) to open the new-session form pre-filled with the selected session's project path + agent, streamlined so the user only enters a name and optionally configures a worktree.

**Architecture:** Extend `CreateForm` with a `prefilled` flag and a `for_project` constructor; branch the step machine (`next_field`/`step`/`total_steps`) on it so the flow is `Name → Worktree → [Base → Branch]` (Dir and Agent steps skipped). Extract the existing create-submit assembly into one `build_create_action` helper, then wire prefilled submit onto the Worktree/Branch steps. Add the `N` key arm and a footer hint. The modal renderer is unchanged (it already reads `step()`/`total_steps()`).

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tmux.

---

## File Structure

- `src/app.rs` — `CreateForm.prefilled` field, `CreateForm::for_project`, prefilled branches in `next_field`/`step`/`total_steps`, the `build_create_action` helper, prefilled submit wiring in `handle_create_key`, the `N` arm in `handle_list_key`, plus unit tests.
- `src/ui/footer.rs` — one extra List-mode hint.
- `src/ui/modal_new.rs` — render test only (no code change).

---

## Task 1: Form model — `prefilled` flag, `for_project`, step machine

**Files:**
- Modify: `src/app.rs` — `CreateForm` struct (~lines 20-34), `CreateForm::new` (~lines 36-60), `next_field` (~lines 72-82), `total_steps` (~lines 124-131), `step` (~lines 190-199)
- Test: `src/app.rs` tests module

- [ ] **Step 1: Add the `prefilled` field to the struct**

In the `pub struct CreateForm { ... }` block, add a field after `new_branch`:

```rust
    pub new_branch: String,
    /// True when opened pre-filled for an existing project (`N`): `dir` and
    /// `agent` are fixed and the flow only walks Name → Worktree → [Base → Branch].
    pub prefilled: bool,
```

- [ ] **Step 2: Initialize `prefilled` in `CreateForm::new`**

In `CreateForm::new`, the returned `Self { ... }` ends with `new_branch: String::new(),`. Add:

```rust
            new_branch: String::new(),
            prefilled: false,
        }
```

- [ ] **Step 3: Run the build to confirm the struct still compiles**

Run: `cargo build`
Expected: compiles (the new field is set everywhere `CreateForm` is constructed — only `new` constructs it so far).

- [ ] **Step 4: Write failing tests for the constructor and step machine**

Add to the `tests` module in `src/app.rs` (near the other create-form tests):

```rust
    #[test]
    fn for_project_prefills_dir_agent_and_starts_at_name() {
        let form = CreateForm::for_project("/home/u/proj", "claude", &["codex".into()]);
        assert!(form.prefilled);
        assert_eq!(form.dir, "/home/u/proj"); // not under $HOME → unchanged by collapse_home
        assert_eq!(form.agent, "claude");
        assert_eq!(form.agent_index, 0); // project agent pre-selected
        assert_eq!(form.field, CreateField::Name);
    }

    #[test]
    fn prefilled_flow_skips_dir_and_agent() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        // Name → Worktree (Dir skipped).
        form.advance();
        assert_eq!(form.field, CreateField::Worktree);
        // Worktree off → wraps back to Name (Agent skipped).
        form.advance();
        assert_eq!(form.field, CreateField::Name);
    }

    #[test]
    fn prefilled_flow_with_worktree_walks_base_then_branch() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        form.field = CreateField::Worktree;
        form.worktree = true;
        form.advance();
        assert_eq!(form.field, CreateField::Base);
        form.advance();
        assert_eq!(form.field, CreateField::Branch);
        form.advance();
        assert_eq!(form.field, CreateField::Name); // wrap
    }

    #[test]
    fn prefilled_step_indicator_counts() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        assert_eq!(form.total_steps(), 2); // name + worktree
        assert_eq!(form.step(), 1); // on Name
        form.field = CreateField::Worktree;
        assert_eq!(form.step(), 2);
        form.worktree = true;
        assert_eq!(form.total_steps(), 4); // name, worktree, base, branch
        form.field = CreateField::Branch;
        assert_eq!(form.step(), 4);
    }
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test --lib for_project_prefills prefilled_flow prefilled_step`
Expected: FAIL — `for_project` does not exist (compile error) and the prefilled branches aren't implemented.

- [ ] **Step 6: Add the `for_project` constructor**

In `impl CreateForm`, right after the `new` constructor (after its closing `}`), add:

```rust
    /// New-session form pre-filled for an existing project: `dir` and `agent`
    /// are fixed, so the streamlined flow only walks Name → Worktree → [Base →
    /// Branch]. `new(project_agent, ...)` already puts `project_agent` first in
    /// `agent_choices` and selects it (index 0), so the agent is pre-chosen.
    pub fn for_project(project_dir: &str, project_agent: &str, presets: &[String]) -> Self {
        let mut f = CreateForm::new(project_agent, presets);
        f.dir = collapse_home(project_dir);
        f.prefilled = true;
        f.field = CreateField::Name;
        f
    }
```

- [ ] **Step 7: Add the `prefilled` branch to `next_field`**

At the very top of `fn next_field(&self) -> CreateField {`, before the existing `match self.field { ... }`, insert:

```rust
        if self.prefilled {
            return match self.field {
                CreateField::Name => CreateField::Worktree,
                CreateField::Worktree if self.worktree => CreateField::Base,
                CreateField::Worktree => CreateField::Name,
                CreateField::Base => CreateField::Branch,
                CreateField::Branch => CreateField::Name,
                CreateField::Dir | CreateField::Agent => CreateField::Name,
            };
        }
```

- [ ] **Step 8: Add the `prefilled` branch to `step` and `total_steps`**

At the top of `fn total_steps(&self) -> usize {`, before the existing body:

```rust
        if self.prefilled {
            return if self.worktree { 4 } else { 2 };
        }
```

At the top of `pub fn step(&self) -> usize {`, before the existing `match`:

```rust
        if self.prefilled {
            return match self.field {
                CreateField::Name => 1,
                CreateField::Worktree => 2,
                CreateField::Base => 3,
                CreateField::Branch => 4,
                _ => 1,
            };
        }
```

(Note `total_steps`'s existing body uses an `if/else` returning a value, and `step`'s existing body is a `match` returned directly. Insert the prefilled guard as an early `return` above each so the existing code is untouched.)

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test --lib for_project_prefills prefilled_flow prefilled_step`
Expected: PASS (all four tests).

- [ ] **Step 10: Full lib tests + clippy**

Run: `cargo test --lib && cargo clippy --all-targets -- -D warnings`
Expected: all existing tests still pass; no clippy warnings.

- [ ] **Step 11: Commit**

```bash
git add src/app.rs
git commit -m "feat(create): prefilled CreateForm model for project-scoped new session

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Submit — extract `build_create_action`, wire prefilled submit

**Files:**
- Modify: `src/app.rs` — add `build_create_action` (near `validate_create`, ~line 1499); `handle_create_key` Worktree block (~lines 1016-1025) and the Name/Agent/Branch Enter branch (~lines 1061-1097)
- Test: `src/app.rs` tests module

- [ ] **Step 1: Write failing submit tests**

Add to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn prefilled_submit_without_worktree_creates_in_project() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::for_project(&dir, "claude", &[]);
        form.name = "sess".into();
        form.field = CreateField::Worktree; // worktree off
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match act {
            Some(Action::Create { name, dir: d, agent, worktree }) => {
                assert_eq!(name, "sess");
                assert_eq!(d, dir);
                assert_eq!(agent, "claude");
                assert_eq!(worktree, None);
            }
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }

    #[test]
    fn prefilled_submit_with_worktree_carries_spec() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::for_project(&dir, "claude", &[]);
        form.name = "sess".into();
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.base_index = 0;
        form.new_branch = "feat".into();
        form.field = CreateField::Branch;
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match act {
            Some(Action::Create { worktree: Some(spec), .. }) => {
                assert_eq!(spec.base, "main");
                assert_eq!(spec.new_branch, "feat");
            }
            other => panic!("expected Action::Create with worktree, got {other:?}"),
        }
    }

    #[test]
    fn non_prefilled_still_submits_on_agent_step() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sess".into();
        form.dir = dir.clone();
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match act {
            Some(Action::Create { name, dir: d, .. }) => {
                assert_eq!(name, "sess");
                assert_eq!(d, dir);
            }
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib prefilled_submit non_prefilled_still_submits`
Expected: the two `prefilled_submit_*` tests FAIL (Enter on Worktree/Branch advances instead of submitting); `non_prefilled_still_submits_on_agent_step` likely PASSES already (it guards the refactor — keep it).

- [ ] **Step 3: Add the `build_create_action` helper**

In `src/app.rs`, immediately above `pub fn validate_create(...)` (~line 1498), add:

```rust
/// Builds the create action from a completed form, or an error string if the
/// name/dir fail validation. Shared by the Agent-step submit (non-prefilled)
/// and the Worktree/Branch submit (prefilled) so the assembly lives in one place.
pub fn build_create_action(form: &CreateForm, existing: &[String]) -> Result<Action, String> {
    validate_create(&form.name, &form.dir, existing)?;
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
    Ok(Action::Create {
        name: form.name.trim().to_string(),
        dir: expand_tilde(&form.dir),
        agent: form.agent.clone(),
        worktree,
    })
}
```

- [ ] **Step 4: Route the Worktree step's Enter through prefilled submit**

In `handle_create_key`, the Worktree-step block currently reads:

```rust
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
```

Replace it with:

```rust
        if form.field == CreateField::Worktree {
            match key.code {
                KeyCode::Esc => return None,
                KeyCode::Char(' ') => form.toggle_worktree(),
                KeyCode::Tab => form.advance(),
                KeyCode::Enter => {
                    if form.prefilled && !form.worktree {
                        let existing: Vec<String> =
                            self.sessions.iter().map(|s| s.name.clone()).collect();
                        match build_create_action(&form, &existing) {
                            Ok(action) => {
                                self.error = None;
                                return Some(action);
                            }
                            Err(e) => {
                                self.error = Some(e);
                                self.mode = Mode::Create(form);
                                return None;
                            }
                        }
                    } else {
                        form.advance();
                    }
                }
                _ => {}
            }
            self.mode = Mode::Create(form);
            return None;
        }
```

- [ ] **Step 5: Route the Agent/Branch Enter through the helper**

In `handle_create_key`, the final text-field `match key.code { ... }` has an `Enter` branch that currently builds the action inline only for the Agent step. Replace that whole `KeyCode::Enter => { ... }` branch with:

```rust
            KeyCode::Enter => {
                let submit = form.field == CreateField::Agent
                    || (form.prefilled && form.field == CreateField::Branch);
                if submit {
                    let existing: Vec<String> =
                        self.sessions.iter().map(|s| s.name.clone()).collect();
                    match build_create_action(&form, &existing) {
                        Ok(action) => {
                            self.error = None;
                            return Some(action);
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.mode = Mode::Create(form);
                            return None;
                        }
                    }
                } else {
                    // Non-submit step → advance (handles Dir refresh when needed).
                    form.advance();
                }
            }
```

- [ ] **Step 6: Run the submit tests to verify they pass**

Run: `cargo test --lib prefilled_submit non_prefilled_still_submits`
Expected: PASS (all three).

- [ ] **Step 7: Full lib tests + clippy**

Run: `cargo test --lib && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass (including the existing reply/create tests); no clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs
git commit -m "feat(create): submit prefilled form from worktree/branch step

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Wire `N` key, footer hint, modal render test

**Files:**
- Modify: `src/app.rs` — `handle_list_key` (add `N` arm after the `'n'` arm, ~line 798)
- Modify: `src/ui/footer.rs` — List-mode hints (~line 42)
- Test: `src/app.rs` tests module; `src/ui/modal_new.rs` tests module

- [ ] **Step 1: Write failing tests for the `N` key**

Add to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn shift_n_opens_prefilled_form_for_project() {
        let mut app = app_with(vec![at("s", "/home/u/proj")]);
        app.selected = 0;
        app.handle_key(key('N'));
        match &app.mode {
            Mode::Create(form) => {
                assert!(form.prefilled);
                assert_eq!(form.dir, "/home/u/proj");
                assert_eq!(form.agent, "claude"); // `at` sets agent = "claude"
                assert_eq!(form.field, CreateField::Name);
            }
            other => panic!("expected Create mode, got {other:?}"),
        }
    }

    #[test]
    fn shift_n_is_noop_without_sessions() {
        let mut app = App::new(Config::default());
        app.handle_key(key('N'));
        assert!(matches!(app.mode, Mode::List));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib shift_n_opens shift_n_is_noop`
Expected: `shift_n_opens_prefilled_form_for_project` FAILS (mode stays `List`); `shift_n_is_noop_without_sessions` PASSES (no `N` handler yet → already a no-op, kept as a guard).

- [ ] **Step 3: Add the `N` arm in `handle_list_key`**

In `handle_list_key`, the existing lowercase arm is:

```rust
            KeyCode::Char('n') => {
                self.error = None;
                self.mode = Mode::Create(CreateForm::new(
                    &self.config.default_agent,
                    &self.config.agent_presets,
                ));
            }
```

Immediately after that arm, add:

```rust
            // Shift+N: new session pre-filled from the selected session's project
            // (path + agent), streamlined to name + worktree. No-op if nothing
            // is selected.
            KeyCode::Char('N') => {
                if let Some(s) = self.selected_session() {
                    let dir = session_root(s).to_string();
                    let agent = s.agent.clone();
                    self.error = None;
                    self.mode = Mode::Create(CreateForm::for_project(
                        &dir,
                        &agent,
                        &self.config.agent_presets,
                    ));
                }
            }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib shift_n_opens shift_n_is_noop`
Expected: PASS (both).

- [ ] **Step 5: Add the footer hint**

In `src/ui/footer.rs`, the `Mode::List => vec![ ... ]` starts with `("n", "new", true),`. Insert the new hint right after it:

```rust
        Mode::List => vec![
            ("n", "new", true),
            ("⇧N", "new in proj", false),
            ("↵", "attach", false),
```

(Leave the rest of the list unchanged.)

- [ ] **Step 6: Add the modal render test**

In `src/ui/modal_new.rs` tests module, add (note: focus uses a background band that `buf_to_string` cannot observe, so this asserts visible text/counts, not styling):

```rust
    #[test]
    fn prefilled_modal_shows_two_steps_and_project_values() {
        let form = CreateForm::for_project("/home/u/proj", "claude", &["claude".into()]);
        let mut t = Terminal::new(TestBackend::new(80, 30)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("of 2"), "streamlined step total:\n{s}");
        assert!(s.contains("proj"), "project path on directory row:\n{s}");
        assert!(s.contains("claude"), "project agent shown:\n{s}");
        assert!(s.contains("name"), "name row present:\n{s}");
    }
```

`CreateForm` and `for_project` come through the existing `use crate::app::{...}` import line at the top of `modal_new.rs`; if `CreateForm` is not already imported in the test module, the `use super::*;` at the top of the tests module re-exports it via the file's imports — verify it compiles and add `CreateForm` to the file's `use crate::app::{...}` list if needed.

- [ ] **Step 7: Run the modal test + full suite + clippy**

Run: `cargo test --lib prefilled_modal && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: the modal test passes; full suite (lib + integration) green; no clippy warnings.

- [ ] **Step 8: Manual verification**

Run: `cargo run`
- Select a session, press `N` → form opens showing the project's path and agent pre-filled, the step indicator reads `1 of 2`, focus on `name`.
- Type a name, press Enter → a new session is created in that project with the same agent.
- Press `N` again, type a name, press Space on the worktree step to enable it, configure base/branch, Enter on the branch step → worktree-backed session created.
- Confirm lowercase `n` still opens the blank `~/` form unchanged.

- [ ] **Step 9: Commit**

```bash
git add src/app.rs src/ui/footer.rs src/ui/modal_new.rs
git commit -m "feat(ui): N opens new-session form prefilled from selected project

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- `N` key, pre-fill dir+agent from `session_root`/`s.agent`, no-op when unselected → Task 3 (Steps 1-4).
- `prefilled` field + `for_project` (reusing `new` to pre-select the agent) → Task 1 (Steps 1-2, 6).
- Streamlined flow `Name → Worktree → [Base → Branch]`, skipping Dir/Agent → Task 1 `next_field` (Step 7); `step`/`total_steps` 2-or-4 → Task 1 (Step 8).
- Submit points (Worktree-off Enter, Branch Enter) + shared `build_create_action`, non-prefilled routed through it → Task 2 (Steps 3-5).
- Footer hint `⇧N new in proj` → Task 3 (Step 5).
- Modal renders unchanged, reads step counts → Task 3 (Step 6) verifies via render test.
- Tests enumerated in the spec → Task 1 (Step 4), Task 2 (Step 1), Task 3 (Steps 1, 6). One spec test detail adjusted: the modal test asserts visible text/counts instead of "name row focused", because the focus band is a background style invisible to `buf_to_string` (noted in Task 3 Step 6).

**Placeholder scan:** none — every code step shows exact code; every command shows expected output.

**Type consistency:** `CreateForm::for_project(project_dir, project_agent, presets)`, `prefilled: bool`, `build_create_action(form: &CreateForm, existing: &[String]) -> Result<Action, String>`, `CreateField::{Name,Worktree,Base,Branch,Dir,Agent}`, `Action::Create { name, dir, agent, worktree }`, `WorktreeSpec { base, new_branch }`, `session_root(&Session) -> &str`, `collapse_home`/`expand_tilde` — all match the existing `src/app.rs` definitions and are used consistently across tasks.
