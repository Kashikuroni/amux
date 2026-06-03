# Plain Terminal Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `terminal` toggle to the new-session form that, when on, skips the agent step and launches the user's `$SHELL` (fallback `/bin/sh`) so the session is a plain terminal for nvim/lazygit/etc.

**Architecture:** Three tasks. (1) App-layer form model: a `terminal: bool` + `CreateField::Terminal`, and a refactor of the step machine to a single `field_sequence()` driving `next_field`/`step`/`total_steps`/`is_last_step` (folds in #4's prefilled branches; submit becomes "Enter on the last step"). (2) Action + IO: `Action::Create.terminal`, decouple the tmux command-run from the `@cm_agent` label, resolve `$SHELL` in `main.rs`. (3) Modal rendering: a terminal toggle row, a disabled agent row when on, shell in the command preview.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tmux.

---

## File Structure

- `src/app.rs` — `terminal` field, `CreateField::Terminal`, `current_mut` arm, `toggle_terminal`, `field_sequence` + derived step methods, the Terminal key block, `is_last_step` submit, and (Task 2) `Action::Create.terminal` + `build_create_action`.
- `src/tmux.rs` — `new_session`/`new_worktree_session` take separate `command` + `label`; add `shell_basename`.
- `src/main.rs` — resolve `$SHELL` and thread `command`/`label`; `create_worktree_session` signature.
- `tests/tmux_integration.rs` — update two call sites.
- `src/ui/modal_new.rs` — terminal toggle row, disabled agent row, preview, height; tests.

---

## Task 1: Form model + step-machine refactor + Terminal key handling

**Files:**
- Modify: `src/app.rs` — `CreateField` enum (~lines 11-18); `current_mut` (~78-85); `next_field` (~88-108); `total_steps` (~151-160); `step` (~219-236); add `field_sequence`/`is_last_step`/`toggle_terminal`; `CreateForm` struct field; `CreateForm::new`; `handle_create_key` (Terminal block + Worktree/text submit). Update existing tests.
- Modify: `src/ui/modal_new.rs` — step-count assertions in existing tests.

- [ ] **Step 1: Add the `Terminal` enum variant**

In `src/app.rs`, the `CreateField` enum becomes:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateField {
    Name,
    Dir,
    Terminal,
    Worktree,
    Base,
    Branch,
    Agent,
}
```

- [ ] **Step 2: Add the `terminal` field to `CreateForm` + initialize it**

In `pub struct CreateForm { ... }`, after `pub prefilled: bool,` add:

```rust
    /// True when the session should run a plain shell instead of an agent.
    /// When set, the Agent step is skipped and `$SHELL` is launched.
    pub terminal: bool,
```

In `CreateForm::new`, the returned `Self { ... }` ends with `prefilled: false,`. Make it:

```rust
            prefilled: false,
            terminal: false,
        }
```

(`for_project` builds on `new`, so it inherits `terminal: false` — correct; the toggle is then user-driven in both flows.)

- [ ] **Step 3: Add the `Terminal` arm to `current_mut`**

`current_mut`'s last arm currently is:

```rust
            CreateField::Worktree | CreateField::Base => &mut self.agent,
```

Replace with (Terminal is a toggle, never a text field — map it harmlessly):

```rust
            CreateField::Worktree | CreateField::Base | CreateField::Terminal => &mut self.agent,
```

- [ ] **Step 4: Build to catch non-exhaustive matches**

Run: `cargo build`
Expected: compiles, OR a non-exhaustive-match error pointing at any other `match` on `CreateField` (there should be none beyond `current_mut`; `next_field`/`step`/`total_steps` are rewritten next). Fix any by adding a `Terminal` arm before proceeding.

- [ ] **Step 5: Write failing model tests**

Add to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn default_flow_counts_after_refactor() {
        let mut form = CreateForm::new("claude", &[]);
        // Name, Dir, Terminal, Worktree, Agent.
        assert_eq!(form.total_steps(), 5);
        assert_eq!(form.step(), 1); // Name
        form.field = CreateField::Agent;
        assert_eq!(form.step(), 5);
        assert!(form.is_last_step());
    }

    #[test]
    fn terminal_flow_skips_agent_step() {
        let mut form = CreateForm::new("claude", &[]);
        form.terminal = true;
        // Name → Dir → Terminal → Worktree → wrap to Name (Agent skipped).
        assert_eq!(form.field, CreateField::Name);
        form.advance();
        assert_eq!(form.field, CreateField::Dir);
        form.advance();
        assert_eq!(form.field, CreateField::Terminal);
        form.advance();
        assert_eq!(form.field, CreateField::Worktree);
        form.advance();
        assert_eq!(form.field, CreateField::Name);
    }

    #[test]
    fn terminal_step_counts_and_last_step() {
        let mut form = CreateForm::new("claude", &[]);
        form.terminal = true;
        assert_eq!(form.total_steps(), 4); // Name, Dir, Terminal, Worktree (no Agent)
        form.field = CreateField::Worktree;
        assert!(form.is_last_step()); // Worktree is last when terminal & no worktree
        form.field = CreateField::Terminal;
        assert!(!form.is_last_step());
    }

    #[test]
    fn toggle_terminal_flips_flag() {
        let mut form = CreateForm::new("claude", &[]);
        assert!(!form.terminal);
        form.toggle_terminal();
        assert!(form.terminal);
        form.toggle_terminal();
        assert!(!form.terminal);
    }
```

- [ ] **Step 6: Run the new tests to verify they fail**

Run: `cargo test --lib default_flow_counts terminal_flow_skips terminal_step_counts toggle_terminal_flips`
Expected: FAIL — `field_sequence`/`is_last_step`/`toggle_terminal` don't exist and counts are wrong.

- [ ] **Step 7: Add `field_sequence`, replace `next_field`, add `is_last_step`**

Replace the entire `fn next_field(&self) -> CreateField { ... }` (the prefilled early-return block AND the trailing match) with:

```rust
    /// The ordered steps for the current configuration — the single source of
    /// truth for next_field / step / total_steps / is_last_step. Dir is dropped
    /// when prefilled (`N`); Agent is dropped when prefilled or terminal.
    fn field_sequence(&self) -> Vec<CreateField> {
        let mut v = vec![CreateField::Name];
        if !self.prefilled {
            v.push(CreateField::Dir);
        }
        v.push(CreateField::Terminal);
        v.push(CreateField::Worktree);
        if self.worktree {
            v.push(CreateField::Base);
            v.push(CreateField::Branch);
        }
        if !self.prefilled && !self.terminal {
            v.push(CreateField::Agent);
        }
        v
    }

    fn next_field(&self) -> CreateField {
        let seq = self.field_sequence();
        match seq.iter().position(|&f| f == self.field) {
            Some(i) => seq[(i + 1) % seq.len()],
            None => seq[0],
        }
    }

    /// True when the focused field is the final step (Enter here submits).
    pub fn is_last_step(&self) -> bool {
        self.field_sequence().last() == Some(&self.field)
    }
```

- [ ] **Step 8: Replace `total_steps` and `step` with sequence-derived versions**

Replace the entire `pub fn total_steps(&self) -> usize { ... }` (the prefilled early-return + if/else) with:

```rust
    /// Total number of steps shown in the `N of M` indicator.
    pub fn total_steps(&self) -> usize {
        self.field_sequence().len()
    }
```

Replace the entire `pub fn step(&self) -> usize { ... }` (the prefilled early-return + match) with:

```rust
    /// 1-based position of the focused field, for the `N of M` step indicator.
    pub fn step(&self) -> usize {
        let seq = self.field_sequence();
        seq.iter()
            .position(|&f| f == self.field)
            .map(|i| i + 1)
            .unwrap_or(1)
    }
```

- [ ] **Step 9: Add `toggle_terminal`**

In `impl CreateForm`, just after `toggle_worktree` (after its closing `}`), add:

```rust
    /// Flip the plain-shell toggle. No branch/disk work needed (unlike worktree);
    /// the Agent step simply disappears from `field_sequence` when on.
    pub fn toggle_terminal(&mut self) {
        self.terminal = !self.terminal;
    }
```

- [ ] **Step 10: Run the model tests to verify they pass**

Run: `cargo test --lib default_flow_counts terminal_flow_skips terminal_step_counts toggle_terminal_flips`
Expected: PASS (all four).

- [ ] **Step 11: Add the Terminal key block + generalize submit in `handle_create_key`**

In `handle_create_key`, immediately AFTER the Dir-step block (`if form.field == CreateField::Dir { ... }`) and BEFORE the Worktree-step block, insert:

```rust
        // Terminal toggle step.
        if form.field == CreateField::Terminal {
            match key.code {
                KeyCode::Esc => return None,
                KeyCode::Char(' ') => form.toggle_terminal(),
                KeyCode::Tab => form.advance(),
                KeyCode::Enter => {
                    if form.is_last_step() {
                        return self.submit_create(form);
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

In the Worktree-step block, replace its `KeyCode::Enter => { ... }` arm with:

```rust
                KeyCode::Enter => {
                    if form.is_last_step() {
                        return self.submit_create(form);
                    } else {
                        form.advance();
                    }
                }
```

In the final text-field `match key.code`, replace the `submit` binding line:

```rust
                let submit = form.field == CreateField::Agent
                    || (form.prefilled && form.field == CreateField::Branch);
```

with:

```rust
                let submit = form.is_last_step();
```

- [ ] **Step 12: Write + run a key-handling test for the toggle**

Add to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn terminal_toggle_via_space_in_create() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sh".into();
        form.dir = dir;
        form.field = CreateField::Terminal;
        app.mode = Mode::Create(form);
        app.handle_key(key(' ')); // Space toggles terminal on
        match &app.mode {
            Mode::Create(f) => assert!(f.terminal),
            other => panic!("expected Create, got {other:?}"),
        }
    }
```

Run: `cargo test --lib terminal_toggle_via_space`
Expected: PASS.

- [ ] **Step 13: Update existing tests whose step counts changed**

In `src/app.rs`, replace `prefilled_step_indicator_counts` with:

```rust
    #[test]
    fn prefilled_step_indicator_counts() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        assert_eq!(form.total_steps(), 3); // name, terminal, worktree
        assert_eq!(form.step(), 1); // on Name
        form.field = CreateField::Worktree;
        assert_eq!(form.step(), 3);
        form.worktree = true;
        assert_eq!(form.total_steps(), 5); // name, terminal, worktree, base, branch
        form.field = CreateField::Branch;
        assert_eq!(form.step(), 5);
    }
```

And replace `prefilled_flow_skips_dir_and_agent` with:

```rust
    #[test]
    fn prefilled_flow_skips_dir_and_agent() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        // Name → Terminal → Worktree → wrap to Name (Dir and Agent skipped).
        form.advance();
        assert_eq!(form.field, CreateField::Terminal);
        form.advance();
        assert_eq!(form.field, CreateField::Worktree);
        form.advance();
        assert_eq!(form.field, CreateField::Name);
    }
```

(`prefilled_flow_with_worktree_walks_base_then_branch` still holds — from Worktree it advances Base → Branch → Name — leave it unchanged.)

In `src/ui/modal_new.rs` tests, update the step-indicator assertions:
- `new_modal_shows_inline_labels_and_agent_segments`: change `assert!(s.contains("of 3"), "step indicator");` to `assert!(s.contains("of 5"), "step indicator");`
- `new_modal_shows_worktree_rows_when_enabled`: change `assert!(s.contains("of 5"), "dynamic step total");` to `assert!(s.contains("of 7"), "dynamic step total");`
- `new_modal_hides_worktree_rows_by_default`: change `assert!(s.contains("of 3"));` to `assert!(s.contains("of 5"));`
- `prefilled_modal_shows_two_steps_and_project_values`: rename to `prefilled_modal_shows_streamlined_steps_and_project_values` and change `assert!(s.contains("of 2"), ...)` to `assert!(s.contains("of 3"), "streamlined step total:\n{s}");`

- [ ] **Step 14: Full suite + clippy**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all lib + integration tests pass; no clippy warnings.

- [ ] **Step 15: Commit**

```bash
git add src/app.rs src/ui/modal_new.rs
git commit -m "feat(create): terminal toggle + field_sequence step machine

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Action.terminal + tmux command/label decoupling + $SHELL

**Files:**
- Modify: `src/app.rs` — `Action::Create` variant (~lines 425-430); `build_create_action` (~1543-1563); one test destructure.
- Modify: `src/tmux.rs` — `new_session`, `new_worktree_session`; add `shell_basename` + test.
- Modify: `src/main.rs` — `handle_action` `Action::Create` arm; `create_worktree_session`.
- Modify: `tests/tmux_integration.rs` — two call sites.

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn terminal_submit_carries_terminal_flag() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sh".into();
        form.dir = dir;
        form.terminal = true;
        form.field = CreateField::Worktree; // last step when terminal & no worktree
        app.mode = Mode::Create(form);
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Some(Action::Create { terminal, .. }) => assert!(terminal),
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }

    #[test]
    fn non_terminal_submit_sets_terminal_false() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "x".into();
        form.dir = dir;
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Some(Action::Create { terminal, .. }) => assert!(!terminal),
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }
```

Add to the `tests` module in `src/tmux.rs`:

```rust
    #[test]
    fn shell_basename_takes_last_component() {
        assert_eq!(shell_basename("/bin/zsh"), "zsh");
        assert_eq!(shell_basename("/bin/sh"), "sh");
        assert_eq!(shell_basename("bash"), "bash");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib terminal_submit_carries non_terminal_submit_sets shell_basename_takes`
Expected: FAIL — `Action::Create` has no `terminal` field (compile error) and `shell_basename` doesn't exist.

- [ ] **Step 3: Add `terminal` to the `Action::Create` variant**

In `src/app.rs`, the `Create { ... }` variant of `enum Action` becomes:

```rust
    Create {
        name: String,
        dir: String,
        agent: String,
        worktree: Option<WorktreeSpec>,
        terminal: bool,
    },
```

- [ ] **Step 4: Set `terminal` in `build_create_action`**

In `build_create_action`, the returned `Ok(Action::Create { ... })` becomes:

```rust
    Ok(Action::Create {
        name: form.name.trim().to_string(),
        dir: expand_tilde(&form.dir),
        agent: form.agent.clone(),
        worktree,
        terminal: form.terminal,
    })
```

- [ ] **Step 5: Fix the one test destructure that lacks `..`**

In `src/app.rs`, `prefilled_submit_without_worktree_creates_in_project` matches `Some(Action::Create { name, dir: d, agent, worktree })`. Add the field so it stays exhaustive:

```rust
            Some(Action::Create { name, dir: d, agent, worktree, terminal: _ }) => {
```

(Other `Action::Create` matches in tests already use `..`.)

- [ ] **Step 6: Add `shell_basename` to tmux.rs**

In `src/tmux.rs`, near the top of the impl-free functions (e.g. just below the `SOCKET`/`tmux()` helpers), add:

```rust
/// The basename of a shell path, used as the `@cm_agent` label for a terminal
/// session: `/bin/zsh` → `zsh`, `bash` → `bash`.
pub fn shell_basename(shell: &str) -> &str {
    shell.rsplit('/').next().unwrap_or(shell)
}
```

- [ ] **Step 7: Decouple command vs label in `new_session`**

Replace `pub fn new_session(name: &str, dir: &str, agent: &str) -> io::Result<()> { ... }` with:

```rust
/// Creates a detached session running `command` in `dir`, tagged managed with
/// `@cm_agent = label`. For agents, command and label are the same string; for a
/// plain terminal, command is the shell and label its basename.
pub fn new_session(name: &str, dir: &str, command: &str, label: &str) -> io::Result<()> {
    // Create at the current terminal size so the first attach needs no resize.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (cols, rows) = (cols.max(1).to_string(), rows.max(1).to_string());
    run(&[
        "new-session", "-d", "-s", name, "-x", &cols, "-y", &rows, "-c", dir, command,
    ])?;
    apply_resize_options();
    // If tagging fails, the session would exist untagged (invisible to list_sessions);
    // kill it so creation is all-or-nothing.
    if let Err(e) = run(&["set-option", "-t", name, "@cm_managed", "1"])
        .and_then(|_| run(&["set-option", "-t", name, "@cm_agent", label]))
    {
        let _ = run(&["kill-session", "-t", name]);
        return Err(e);
    }
    apply_key_bindings();
    let _ = run(&["set-option", "-g", "status", "off"]);
    Ok(())
}
```

- [ ] **Step 8: Thread command/label through `new_worktree_session`**

Replace `pub fn new_worktree_session(name: &str, dir: &str, agent: &str, repo_root: &str) -> io::Result<()> { ... }` with:

```rust
/// Like `new_session`, but also tags the session with `@cm_repo=<repo_root>` so
/// the UI knows it runs in a worktree (enables worktree-aware kill).
pub fn new_worktree_session(
    name: &str,
    dir: &str,
    command: &str,
    label: &str,
    repo_root: &str,
) -> io::Result<()> {
    new_session(name, dir, command, label)?;
    if let Err(e) = run(&["set-option", "-t", name, "@cm_repo", repo_root]) {
        let _ = run(&["kill-session", "-t", name]);
        return Err(e);
    }
    Ok(())
}
```

- [ ] **Step 9: Resolve `$SHELL` and thread command/label in main.rs**

In `src/main.rs` `handle_action`, replace the `Action::Create { ... } => { ... }` arm's head and dispatch with:

```rust
        Action::Create {
            name,
            dir,
            agent,
            worktree,
            terminal,
        } => {
            let (command, label) = if terminal {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let label = tmux::shell_basename(&shell).to_string();
                (shell, label)
            } else {
                (agent.clone(), agent.clone())
            };
            let result = match worktree {
                None => tmux::new_session(&name, &dir, &command, &label),
                Some(spec) => create_worktree_session(&name, &dir, &command, &label, &spec),
            };
            if let Err(e) = result {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
```

(`agent` is still bound; the `else` branch uses it. Keep the rest of the arm — the `if let Err` + `app.refresh()` — as shown.)

- [ ] **Step 10: Update `create_worktree_session` signature**

In `src/main.rs`, change `create_worktree_session` to take `command`/`label` instead of `agent` and pass them on:

```rust
fn create_worktree_session(
    name: &str,
    dir: &str,
    command: &str,
    label: &str,
    spec: &am::app::WorktreeSpec,
) -> io::Result<()> {
    let repo = am::git::repo_root(dir)
        .ok_or_else(|| io::Error::other(format!("not a git repo: {dir}")))?;
    am::git::ensure_gitignore(&repo, ".worktrees/")?;
    let wt_path = std::path::Path::new(&repo)
        .join(".worktrees")
        .join(&spec.new_branch);
    let wt_str = wt_path.to_string_lossy().to_string();
    am::git::add_worktree(&repo, &wt_str, &spec.new_branch, &spec.base)?;
    tmux::new_worktree_session(name, &wt_str, command, label, &repo)
}
```

- [ ] **Step 11: Update the integration-test call sites**

In `tests/tmux_integration.rs`:
- Change `tmux::new_session(&name, dir, "bash").expect("new_session");` to `tmux::new_session(&name, dir, "bash", "bash").expect("new_session");`
- Change `tmux::new_worktree_session(&name, &wt_str, "bash", &repo_s).expect("new_worktree_session");` to `tmux::new_worktree_session(&name, &wt_str, "bash", "bash", &repo_s).expect("new_worktree_session");`

- [ ] **Step 12: Run the failing tests, then the full suite + clippy**

Run: `cargo test --lib terminal_submit_carries non_terminal_submit_sets shell_basename_takes`
Expected: PASS.

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all lib + integration tests pass; no clippy warnings.

- [ ] **Step 13: Commit**

```bash
git add src/app.rs src/tmux.rs src/main.rs tests/tmux_integration.rs
git commit -m "feat: run \$SHELL for terminal sessions; decouple tmux command/label

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Modal rendering — terminal toggle row + disabled agent row

**Files:**
- Modify: `src/ui/modal_new.rs` — `BASE_ROWS` (~line 121); `agent_warn` (~134); terminal toggle row (before the worktree block ~270); agent block (~320-344); command preview (~358-382); add a render test.

- [ ] **Step 1: Write the failing render test**

Add to the `tests` module in `src/ui/modal_new.rs`:

```rust
    #[test]
    fn terminal_modal_shows_toggle_and_disables_agent() {
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.terminal = true;
        let mut t = Terminal::new(TestBackend::new(80, 32)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("terminal"), "terminal toggle row:\n{s}");
        assert!(s.contains("[x] plain shell"), "toggle checked:\n{s}");
        assert!(s.contains("(terminal session)"), "agent row disabled:\n{s}");
        assert!(s.contains("$SHELL"), "command preview shows shell:\n{s}");
        assert!(s.contains("of 4"), "terminal flow step total:\n{s}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib terminal_modal_shows_toggle`
Expected: FAIL — no terminal row, no disabled hint, preview shows the agent not `$SHELL`.

- [ ] **Step 3: Reserve a row in the height accounting**

In `src/ui/modal_new.rs`, the `BASE_ROWS` constant doc + value (currently `13`) become:

```rust
/// Fixed content rows (header, rule, name, dir + validation, terminal, worktree,
/// agent, rule, command, and the blanks between groups) when the worktree is off.
const BASE_ROWS: u16 = 14;
```

- [ ] **Step 4: Suppress the agent-not-found warning when terminal**

Change the `agent_warn` binding (currently `let agent_warn = !form.agent.is_empty() && resolve_agent_path(&form.agent).is_none();`) to:

```rust
    let agent_warn =
        !form.terminal && !form.agent.is_empty() && resolve_agent_path(&form.agent).is_none();
```

- [ ] **Step 5: Render the terminal toggle row before the worktree row**

Immediately before the `// worktree toggle.` block (the `if let Some(r) = row(x, y, w, bottom) { let focused = form.field == CreateField::Worktree; ... }`), insert:

```rust
    // terminal toggle.
    if let Some(r) = row(x, y, w, bottom) {
        let focused = form.field == CreateField::Terminal;
        let mark = if form.terminal { "[x]" } else { "[ ]" };
        let line = Line::from(vec![
            lbl("terminal"),
            Span::styled(format!("{mark} plain shell"), Style::default()),
            Span::styled("   space", Style::default().add_modifier(Modifier::DIM)),
        ]);
        f.render_widget(band(Paragraph::new(line), focused), r);
    }
    y += 1;
```

- [ ] **Step 6: Show a disabled agent row when terminal is on**

Replace the agent picker block (`// agent picker (+ a quiet warning sub-line ...)` → the `segment_row(...)` call) with:

```rust
    // agent picker — or a disabled hint when this is a terminal session.
    if let Some(r) = row(x, y, w, bottom) {
        if form.terminal {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    lbl("agent"),
                    Span::styled(
                        "(terminal session)",
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ])),
                r,
            );
        } else {
            segment_row(
                f,
                r,
                "agent",
                &form.agent_choices,
                form.agent_index,
                form.field == CreateField::Agent,
                "",
            );
        }
    }
    y += 1;
```

- [ ] **Step 7: Show the shell in the command preview**

In the command-preview block, the `agent` binding currently is:

```rust
        let agent = if form.agent.is_empty() {
            CUSTOM_AGENT_SLOT
        } else {
            form.agent.as_str()
        };
```

Replace with:

```rust
        let agent = if form.terminal {
            "$SHELL"
        } else if form.agent.is_empty() {
            CUSTOM_AGENT_SLOT
        } else {
            form.agent.as_str()
        };
```

- [ ] **Step 8: Run the render test + full suite + clippy**

Run: `cargo test --lib terminal_modal_shows_toggle`
Expected: PASS.

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass (including the Task 1 step-count updates); no clippy warnings.

- [ ] **Step 9: Manual verification**

Run: `cargo run`
- Press `n`, type a name, Tab to the directory and pick one, Tab to the `terminal` row, press Space → it shows `[x] plain shell` and the agent row reads `(terminal session)`; the step counter no longer includes an agent step.
- Press Enter on the worktree step (leave worktree off) → a session is created running your shell; attach it and confirm you get a normal shell prompt where `nvim`/`lazygit` work. The list shows the shell name (e.g. `zsh`).
- Repeat with the worktree toggle on → a shell session in a fresh worktree.
- Confirm a non-terminal session still creates with its agent as before, and `N` (prefilled) also offers the terminal toggle.

- [ ] **Step 10: Commit**

```bash
git add src/ui/modal_new.rs
git commit -m "feat(ui): terminal toggle row + disabled agent row in new-session modal

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- `terminal` field + `CreateField::Terminal` → Task 1 (Steps 1-2).
- `field_sequence` driving next_field/step/total_steps/is_last_step (folds in #4 prefilled branches) → Task 1 (Steps 7-8).
- Toggle between Dir and Worktree; agent skipped when on → Task 1 `field_sequence` order (Step 7) + `toggle_terminal` (Step 9) + Terminal key block (Step 11).
- Submit via `is_last_step` → Task 1 (Step 11).
- Step-count change + existing-test updates → Task 1 (Step 13).
- `Action::Create.terminal` + `build_create_action` → Task 2 (Steps 3-4).
- `$SHELL` (fallback `/bin/sh`), basename label, command/label decoupling → Task 2 (Steps 6-10) + `shell_basename` helper (Step 6).
- Integration-test call-site updates → Task 2 (Step 11).
- List shows shell basename → automatic via `@cm_agent = label` (no list-code change; covered by Task 2 IO).
- Terminal toggle row, disabled agent row, shell in preview, height +1 → Task 3 (Steps 3-7).
- Worktree + terminal combine → Task 2 worktree dispatch threads command/label (Step 9-10); exercised in Task 3 manual verification.
- Out-of-scope (prompt/reply unchanged) → no code touches them; nothing to do.

**Placeholder scan:** none — every code step shows exact code; every command states expected output.

**Type consistency:** `CreateField::Terminal`, `terminal: bool` (form + `Action::Create`), `field_sequence() -> Vec<CreateField>`, `is_last_step() -> bool`, `toggle_terminal()`, `shell_basename(&str) -> &str`, `new_session(name, dir, command, label)`, `new_worktree_session(name, dir, command, label, repo_root)`, `create_worktree_session(name, dir, command, label, spec)`, `submit_create` — consistent across tasks and matched to the current `src/app.rs`/`src/tmux.rs`/`src/main.rs` definitions.
