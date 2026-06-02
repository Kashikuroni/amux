# `N` — new session pre-filled from the selected project

Date: 2026-06-03
Status: approved (design)

Item #4 of the UI task list: pressing `N` (Shift+N) on a selected session opens
the new-session form pre-filled with that session's **project** path and agent,
streamlined so the user only enters a name and (optionally) configures a
worktree.

## Behavior

- **`n` (unchanged):** blank new-session form — `dir = "~/"`, Name focused, full
  step flow (`Name → Dir → Worktree → [Base → Branch] → Agent`).
- **`N` (Shift+N, new):** pre-filled, streamlined form for the selected
  session's project:
  - `dir` = the project root via `session_root(s)` (the repo root for worktree
    sessions; the `.worktrees/…`-stripped dir otherwise), shown collapsed to
    `~` for display.
  - `agent` = the selected session's `agent`, pre-selected in the picker.
  - Flow walks `Name → Worktree (→ Base → Branch if worktree enabled)` only. The
    Dir and Agent steps are removed from the flow; their rows still render (so
    the pre-filled values are visible) but never receive focus.
  - No session selected (empty list) → `N` is a no-op.

## Form changes — `src/app.rs`

### `CreateForm` struct
Add a field:

```rust
pub prefilled: bool,
```

`CreateForm::new` sets `prefilled: false`. All existing fields keep their
current defaults.

### New constructor

```rust
/// New-session form pre-filled for an existing project: `dir` and `agent` are
/// fixed, so the streamlined flow only walks Name → Worktree → [Base → Branch].
pub fn for_project(project_dir: &str, project_agent: &str, presets: &[String]) -> Self {
    // `new` already puts `project_agent` first in `agent_choices` and selects
    // it (index 0), so the project's agent is pre-chosen for free.
    let mut f = CreateForm::new(project_agent, presets);
    f.dir = collapse_home(project_dir);
    f.prefilled = true;
    f.field = CreateField::Name;
    f
}
```

(`collapse_home` is already defined in `app.rs` and is the inverse of the
`expand_tilde` applied on submit.)

### Step machine — `prefilled` branches

`next_field`:

```rust
fn next_field(&self) -> CreateField {
    if self.prefilled {
        return match self.field {
            CreateField::Name => CreateField::Worktree,
            CreateField::Worktree if self.worktree => CreateField::Base,
            CreateField::Worktree => CreateField::Name, // wrap; submit handled in key handler
            CreateField::Base => CreateField::Branch,
            CreateField::Branch => CreateField::Name,   // wrap
            CreateField::Dir | CreateField::Agent => CreateField::Name, // unreachable when prefilled
        };
    }
    // ... existing non-prefilled flow unchanged ...
}
```

`step` (1-based position for the `N of M` indicator):

```rust
pub fn step(&self) -> usize {
    if self.prefilled {
        return match self.field {
            CreateField::Name => 1,
            CreateField::Worktree => 2,
            CreateField::Base => 3,
            CreateField::Branch => 4,
            _ => 1,
        };
    }
    // ... existing ...
}
```

`total_steps`:

```rust
pub fn total_steps(&self) -> usize {
    if self.prefilled {
        if self.worktree { 4 } else { 2 }
    } else if self.worktree {
        5
    } else {
        3
    }
}
```

`advance` is unchanged — it already calls `next_field` and only refreshes dir
entries when landing on `CreateField::Dir`, which never happens when prefilled.

### Submit points — `handle_create_key`

Today the `Action::Create` is built and returned only from the Agent step's
`Enter` branch. Extract that assembly into one shared place to avoid
duplicating the `WorktreeSpec` logic:

```rust
/// Builds the create action from a completed form, or an error string if the
/// name/dir fail validation. Used by both the Agent-step submit (non-prefilled)
/// and the Worktree/Branch submit (prefilled).
fn build_create_action(form: &CreateForm, existing: &[String]) -> Result<Action, String> {
    validate_create(&form.name, &form.dir, existing)?;
    let worktree = if form.worktree {
        Some(WorktreeSpec {
            base: form.base_branches.get(form.base_index).cloned().unwrap_or_default(),
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

Submit wiring (prefilled mode):
- **Worktree step:** `Tab` → advance (unchanged). `Enter` → if `form.worktree`
  advance to Base; else **submit** via `build_create_action`.
- **Branch step:** `Enter` → **submit** via `build_create_action`.
- **Name step:** `Enter` → advance to Worktree (unchanged behavior — advance).

Non-prefilled mode keeps its current behavior (Enter submits only on the Agent
step), now routed through `build_create_action` instead of inline assembly.

On a validation error the handler keeps the existing pattern: set `self.error`,
re-store `Mode::Create(form)`, return `None`.

### Key handling — `handle_list_key`

Add an arm (the existing `'n'` arm stays as-is):

```rust
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

`session_root(s)` borrows `s`, so `dir`/`agent` are cloned out before assigning
`self.mode`. `latin_code` already maps a Cyrillic uppercase key to its Latin
QWERTY-position letter, so `N` works on a Russian layout like the existing `K`/
`J`/`R` Shift arms.

## Footer — `src/ui/footer.rs`

In the `Mode::List` hint vector, insert after `("n", "new", true)`:

```rust
("⇧N", "new in proj", false),
```

The footer is a single clipping `Line`; one more item is fine.

## Rendering — `src/ui/modal_new.rs`

No structural change. The modal already renders every row and reads
`form.step()` / `form.total_steps()` for the `N of M` indicator, so the
streamlined counts flow through automatically. Pre-filled dir/agent rows render
unfocused (no selection band); the directory validation sub-line ("exists  ·
⎇ branch") confirms the project. The command-preview line shows the real
`tmux new -s <name> -c <dir> "<agent>"`.

## Testing

`src/app.rs` unit tests:
- `for_project` sets `dir` (collapsed), `agent`, `prefilled = true`,
  `field == Name`, and pre-selects the project agent (`agent_index == 0`,
  `agent == project_agent`).
- Prefilled `next_field`: `Name → Worktree`; with `worktree == false`,
  `Worktree → Name` (wrap); with `worktree == true`,
  `Worktree → Base → Branch → Name`.
- `total_steps`/`step`: prefilled no-worktree → total 2 (Name=1, Worktree=2);
  prefilled worktree → total 4 (Base=3, Branch=4).
- Pressing `N` (`KeyCode::Char('N')`) with a selected session enters
  `Mode::Create` with `prefilled == true`, `dir` = project root, `agent` =
  selected session's agent.
- `N` with no sessions is a no-op (mode stays `List`).
- Submit, prefilled, worktree off: set a name, send `Enter` on the Worktree step
  → `Action::Create { dir = project, agent = project agent, worktree: None }`.
- Submit, prefilled, worktree on: toggle worktree, set a branch, `Enter` on the
  Branch step → `Action::Create` with the expected `WorktreeSpec { base,
  new_branch }`.

`src/ui/modal_new.rs` test:
- Render a prefilled form (no worktree) into a `TestBackend`; assert `"of 2"`,
  the project path appears on the directory row, the project agent is shown
  selected, and the name row is the focused one.

## Out of scope
- The plain non-agent terminal (item #1). When that lands, a session's `agent`
  may be a shell command; `N` will pre-fill it unchanged — no special handling
  required here.
