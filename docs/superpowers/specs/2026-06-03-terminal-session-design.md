# Plain terminal session (create-form toggle)

Date: 2026-06-03
Status: approved (design)

Item #1 of the UI task list: a plain, non-agent terminal session in which the
user can run nvim, lazygit, or any command. Realized as a `terminal` toggle in
the new-session form: when on, the agent step is skipped and the session runs
the user's shell.

## Behavior

- A `terminal` toggle sits between the Dir and Worktree steps in the create
  form (`… → Terminal → Worktree → …`), in BOTH the blank `n` form and the
  prefilled `N` form.
- When the toggle is on:
  - The Agent step is skipped (no agent picker).
  - The session runs `$SHELL` (fallback `/bin/sh`).
  - The session is labeled in the list by the shell's basename (e.g. `zsh`),
    tagged via `@cm_agent` so it survives an app restart and never looks like a
    runnable agent command.
- Worktree + terminal combine freely (a shell in a fresh worktree).
- nvim/lazygit/plain commands run normally inside the attached session.

## Form model — `src/app.rs` (with a step-machine refactor)

Add to `CreateForm`:

```rust
    /// True when the session should run a plain shell instead of an agent.
    /// When set, the Agent step is skipped and `$SHELL` is launched.
    pub terminal: bool,
```

Add a `CreateField::Terminal` variant.

The step machine now has three independent dimensions (`prefilled`, `terminal`,
`worktree`). Replace the nested `match`-based `next_field`/`step`/`total_steps`
(including #4's prefilled branches) with one ordered field list:

```rust
/// The ordered steps for the current form configuration. Single source of truth
/// for next_field / step / total_steps / is_last_step.
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
```

Derived methods:

- `next_field(&self) -> CreateField`: position of `self.field` in the sequence,
  return the next element, wrapping to the first. (Fallback to the first element
  if `self.field` is somehow absent — e.g. Agent after toggling terminal on;
  callers re-derive each keypress so this stays consistent.)
- `step(&self) -> usize`: 1-based index of `self.field` in the sequence
  (fallback 1 if absent).
- `total_steps(&self) -> usize`: `field_sequence().len()`.
- `is_last_step(&self) -> bool`: `self.field` equals the last element of the
  sequence.

`advance()` is unchanged in spirit: set `self.field = self.next_field()`, and
refresh dir entries when the new field is `Dir`.

**Toggling terminal:** add a method mirroring the worktree toggle:

```rust
/// Flip the plain-shell toggle. When turning it on, focus may have been on a
/// step that no longer exists (Agent) — callers stay on Terminal, so no fixup
/// is needed here.
pub fn toggle_terminal(&mut self) {
    self.terminal = !self.terminal;
}
```

(Unlike `toggle_worktree`, no branch/disk work is needed.)

**Consequence — step counts change.** Because the toggle rows now count as real
steps, the `N of M` indicator grows: the blank form reads `of 5`
(Name, Dir, Terminal, Worktree, Agent) instead of `of 3`; with worktree, `of 7`.
Prefilled (`N`) reads `of 3` (Name, Terminal, Worktree) and `of 5` with
worktree. Existing modal step-count assertions are updated to match. This is the
accepted tradeoff for the cleaner, dimension-proof sequence model.

## Submit — `src/app.rs`

`Action::Create` gains a field:

```rust
    Create {
        name: String,
        dir: String,
        agent: String,
        worktree: Option<WorktreeSpec>,
        terminal: bool,
    },
```

`build_create_action` sets `terminal: form.terminal`. The `agent` field is still
populated from the form but is ignored by the IO layer when `terminal` is true.

Submit fires whenever `form.is_last_step()` is true. This generalizes the
existing rules (Agent-step submit for the full flow; prefilled Worktree/Branch
submit from #4) into one condition, since the last step varies by configuration:
Agent (full), Worktree (terminal, no worktree), or Branch (worktree on).

## Key handling — `src/app.rs` `handle_create_key`

- Add a `CreateField::Terminal` block (mirroring the Worktree block): `Esc`
  cancels; `Space` calls `toggle_terminal`; `Tab` advances; `Enter` submits when
  `is_last_step()` else advances; other keys ignored.
- Update the Worktree block's `Enter` and the text-field (`Agent`/`Branch`)
  `Enter` to submit via `is_last_step()` (through the existing `submit_create`
  helper) rather than the prefilled-specific checks from #4.

## IO — `src/tmux.rs` and `src/main.rs`

Decouple the command-to-run from the label-to-tag:

```rust
/// Creates a detached session running `command` in `dir`, tagged managed with
/// `@cm_agent = label`. For agents, command and label are the same string; for
/// a plain terminal, command is the shell and label its basename.
pub fn new_session(name: &str, dir: &str, command: &str, label: &str) -> io::Result<()>
```

- `new_session` runs `command` as the pane command and tags `@cm_agent = label`.
- `new_worktree_session(name, dir, command, label, repo_root)` updated likewise
  (delegates to `new_session`).
- Add a small testable helper in `src/tmux.rs` for the label:

  ```rust
  /// The basename of a shell path, used as the `@cm_agent` label for a terminal
  /// session: `/bin/zsh` → `zsh`, `bash` → `bash`.
  pub fn shell_basename(shell: &str) -> &str {
      shell.rsplit('/').next().unwrap_or(shell)
  }
  ```

- `src/main.rs` `handle_action` `Action::Create`: resolve the pair once —

  ```rust
  let (command, label) = if terminal {
      let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
      let label = tmux::shell_basename(&shell).to_string();
      (shell, label)
  } else {
      (agent.clone(), agent.clone())
  };
  ```

  then call `tmux::new_session(&name, &dir, &command, &label)` or
  `create_worktree_session(&name, &dir, &command, &label, &spec)`.
  `create_worktree_session` threads `command`/`label` through to
  `new_worktree_session`.
- The two `tests/tmux_integration.rs` call sites become
  `new_session(&name, dir, "bash", "bash")` and
  `new_worktree_session(&name, &wt_str, "bash", "bash", &repo_s)`.

The basename resolution lives in the IO layer (it reads `$SHELL`), keeping
`App`/`build_create_action` free of environment access.

## Rendering — `src/ui/modal_new.rs`

- Add a `terminal` toggle row immediately before the worktree row, styled like
  the worktree toggle: `terminal  [x]/[ ] plain shell   space`, with the band
  when focused.
- When `form.terminal` is true, render the agent row dimmed/disabled (it is
  skipped) — e.g. `agent   (terminal session)` in DIM — instead of the picker.
- The command-preview line shows the shell when terminal is on: render
  `tmux new -s <name> -c <dir> "$SHELL"`.
- Increase the panel height accounting by one row for the new toggle (extend the
  `BASE_ROWS` constant or the height computation accordingly).

## Display

The session list already renders `s.agent` (from `@cm_agent`) directly, so a
terminal session shows its shell basename (e.g. `zsh`) with no list-code change.

## Out of scope (works unchanged, not special-cased)

- Preview, status (running/idle via pane-diff), attach, kill, rename — already
  session-type agnostic.
- Numbered-prompt detection (`parse_prompt`) and the reply (`i`) / answer
  (`1-9`) keys remain active on terminal sessions. A shell rarely shows a
  consecutive numbered menu, and sending text merely types a command — harmless.
  No special-casing.

## Testing

`src/app.rs` unit tests:
- `field_sequence`/`next_field`: terminal on (no worktree) →
  Name → Dir → Terminal → Worktree → (wrap to Name); Agent absent.
- terminal + worktree → Name → Dir → Terminal → Worktree → Base → Branch →
  (wrap); Agent absent.
- `total_steps`/`step`/`is_last_step`: blank form `of 5`, Worktree is last when
  terminal-on-no-worktree; full (non-terminal) flow still ends on Agent.
- `toggle_terminal` flips the flag.
- Submit: terminal on, no worktree, Enter on Worktree step →
  `Action::Create { terminal: true, .. }`; terminal field carried.
- Non-terminal submit still produces `terminal: false`.

`src/tmux.rs` test:
- `shell_basename` maps `/bin/zsh → zsh`, `/bin/sh → sh`, and a bare `bash →
  bash`.

`src/ui/modal_new.rs` test:
- Render a form with `terminal = true`: the terminal toggle row shows `[x]`, the
  agent row shows the disabled hint, and the step indicator reads `of 5`.
