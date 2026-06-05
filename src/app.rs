use crate::config::Config;
use crate::tmux::{Session, Status};
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Sentinel label for the free-text agent slot in `CreateForm::agent_choices`.
pub const CUSTOM_AGENT_SLOT: &str = "custom\u{2026}"; // "custom…"

/// Claude Code model aliases offered under the agent row. Aliases (not full
/// names) so claude itself resolves them to the current model versions.
pub const CLAUDE_MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];
/// Effort slider positions per model; index 0 is "auto" (no --effort flag).
/// Sonnet 4.6 has no xhigh; haiku does not support effort at all.
const EFFORTS_OPUS: &[&str] = &["auto", "low", "medium", "high", "xhigh", "max"];
const EFFORTS_SONNET: &[&str] = &["auto", "low", "medium", "high", "max"];
const EFFORTS_HAIKU: &[&str] = &["auto"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateField {
    Name,
    Dir,
    Terminal,
    Worktree,
    Branch,
    Base,
    Agent,
}

/// One row of the branch typeahead picker.
#[derive(Debug, Clone, PartialEq)]
pub enum BranchEntry {
    Existing(String),
    /// The `+ create "<name>"` row; carries the typed name.
    Create(String),
}

#[derive(Debug, Clone)]
pub struct CreateForm {
    pub name: String,
    pub dir: String,
    pub agent: String,
    pub field: CreateField,
    pub dir_entries: Vec<String>,
    pub dir_selected: usize,
    pub agent_choices: Vec<String>,
    pub agent_index: usize,
    /// Cursor/selection in the claude model list; `None` = the agent row is
    /// focused and no --model flag will be passed (claude picks its default).
    pub model_index: Option<usize>,
    /// Effort slider position for the highlighted model; 0 = auto (no flag).
    pub effort_index: usize,
    pub branches: Vec<String>,
    pub current_branch: Option<String>,
    pub branch_input: String,
    pub branch_entries: Vec<BranchEntry>,
    pub branch_selected: usize,
    pub base_branches: Vec<String>,
    pub base_index: usize,
    /// Typed filter for the base-branch search; matches are a substring filter
    /// over `base_branches`, `base_index` highlights within the matches.
    pub base_filter: String,
    /// True when opened pre-filled for an existing project (`N`): `dir` is
    /// fixed, so the flow skips the Dir step (agent is still selectable).
    pub prefilled: bool,
    /// True when the session should run a plain shell instead of an agent.
    /// When set, the Agent step is skipped and `$SHELL` is launched.
    pub terminal: bool,
    /// True when the user wants to create/use a git worktree for this session.
    /// When set, the Branch (and optionally Base) steps appear.
    pub worktree: bool,
}

impl CreateForm {
    pub fn new(default_agent: &str, presets: &[String]) -> Self {
        // choices = default first, then any presets not equal to default, then a custom slot
        let mut choices: Vec<String> = vec![default_agent.to_string()];
        for p in presets {
            if !choices.contains(p) {
                choices.push(p.clone());
            }
        }
        choices.push(CUSTOM_AGENT_SLOT.to_string());
        Self {
            name: String::new(),
            dir: "~/".to_string(),
            agent: default_agent.to_string(),
            field: CreateField::Name,
            dir_entries: Vec::new(),
            dir_selected: 0,
            agent_choices: choices,
            agent_index: 0,
            model_index: None,
            effort_index: 0,
            branches: Vec::new(),
            current_branch: None,
            branch_input: String::new(),
            branch_entries: Vec::new(),
            branch_selected: 0,
            base_branches: Vec::new(),
            base_index: 0,
            base_filter: String::new(),
            prefilled: false,
            terminal: false,
            worktree: false,
        }
    }

    /// New-session form pre-filled for an existing project: `dir` is fixed, so
    /// the flow walks Name → Terminal → [Branch → [Base]] → Agent.
    /// `new(project_agent, ...)` already puts `project_agent` first in
    /// `agent_choices` and selects it (index 0), so the agent is pre-chosen.
    pub fn for_project(project_dir: &str, project_agent: &str, presets: &[String]) -> Self {
        let mut f = CreateForm::new(project_agent, presets);
        f.dir = collapse_home(project_dir);
        f.prefilled = true;
        f.field = CreateField::Name;
        f.load_branches();
        f
    }

    fn current_mut(&mut self) -> &mut String {
        match self.field {
            CreateField::Name => &mut self.name,
            CreateField::Dir => &mut self.dir,
            CreateField::Branch => &mut self.branch_input,
            CreateField::Agent => &mut self.agent,
            CreateField::Base => &mut self.base_filter,
            CreateField::Terminal | CreateField::Worktree => &mut self.agent,
        }
    }

    /// The ordered steps for the current configuration — the single source of
    /// truth for next_field / step / total_steps / is_last_step. Dir is dropped
    /// when prefilled (`N`); Agent is dropped when terminal.
    fn field_sequence(&self) -> Vec<CreateField> {
        let mut v = vec![CreateField::Name];
        if !self.prefilled {
            v.push(CreateField::Dir);
        }
        v.push(CreateField::Terminal);
        if !self.branches.is_empty() {
            v.push(CreateField::Worktree);
            if self.worktree {
                v.push(CreateField::Branch);
                if self.branch_is_new() {
                    v.push(CreateField::Base);
                }
            }
        }
        if !self.terminal {
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

    /// True when the focused field is the final step.
    pub fn is_last_step(&self) -> bool {
        self.field_sequence().last() == Some(&self.field)
    }

    /// Advance focus to the next field (used by ↓/j and tests).
    pub fn advance(&mut self) {
        let leaving_dir = self.field == CreateField::Dir;
        self.field = self.next_field();
        if leaving_dir {
            self.load_branches();
        }
        if self.field == CreateField::Dir {
            self.refresh_dir_entries();
        }
    }

    fn prev_field(&self) -> CreateField {
        let seq = self.field_sequence();
        match seq.iter().position(|&f| f == self.field) {
            // Wrap to the last step from the first, mirroring `next_field`.
            Some(i) => seq[(i + seq.len() - 1) % seq.len()],
            None => seq[0],
        }
    }

    /// Move focus to the previous field (↑/k). Mirror of `advance`.
    pub fn retreat(&mut self) {
        self.field = self.prev_field();
        if self.field == CreateField::Dir {
            self.refresh_dir_entries();
        }
    }

    /// (Re)reads the dir's branches — called only at the moments the dir
    /// becomes fixed (form creation for `N`; leaving the Dir step for `n`),
    /// never per-keypress/render. Empty result ⇒ not a repo ⇒ no Branch step.
    fn load_branches(&mut self) {
        let dir = expand_tilde(&self.dir);
        self.branches = crate::git::list_branches(&dir);
        self.current_branch = crate::git::current_branch(&dir);
        self.branch_input.clear();
        self.base_index = 0;
        self.refresh_branch_entries();
    }

    /// Recomputes the picker rows: case-insensitive substring matches of
    /// `branch_input`, plus a `+ create` row when the input matches no branch
    /// exactly. Resets the highlight to the first row.
    pub fn refresh_branch_entries(&mut self) {
        let q = self.branch_input.to_lowercase();
        let mut entries: Vec<BranchEntry> = self
            .branches
            .iter()
            .filter(|b| b.to_lowercase().contains(&q))
            .cloned()
            .map(BranchEntry::Existing)
            .collect();
        let input = self.branch_input.trim();
        if !input.is_empty() && !self.branches.iter().any(|b| b == input) {
            entries.push(BranchEntry::Create(input.to_string()));
        }
        self.branch_entries = entries;
        self.branch_selected = 0;
        self.base_branches = self.branches.clone();
        self.base_index = 0;
    }

    /// True when the picker highlight is on the `+ create` row.
    pub fn branch_is_new(&self) -> bool {
        matches!(
            self.branch_entries.get(self.branch_selected),
            Some(BranchEntry::Create(_))
        )
    }

    fn branch_select_next(&mut self) {
        if self.branch_entries.is_empty() {
            return;
        }
        self.branch_selected = (self.branch_selected + 1) % self.branch_entries.len();
    }

    fn branch_select_prev(&mut self) {
        if self.branch_entries.is_empty() {
            return;
        }
        self.branch_selected = if self.branch_selected == 0 {
            self.branch_entries.len() - 1
        } else {
            self.branch_selected - 1
        };
    }

    /// True when the loaded branch list says the dir is a git repo (the Branch
    /// picker and its Base sub-step only exist then).
    pub fn dir_is_repo(&self) -> bool {
        !self.branches.is_empty()
    }

    /// Test accessor for the private sequence.
    #[cfg(test)]
    pub fn field_sequence_for_test(&self) -> Vec<CreateField> {
        self.field_sequence()
    }

    /// Flip the plain-shell toggle. No branch/disk work needed; the Agent step
    /// simply disappears from `field_sequence` when on.
    pub fn toggle_terminal(&mut self) {
        self.terminal = !self.terminal;
    }

    /// Flip the worktree toggle. When turning off, clears the branch input so
    /// no stale branch lingers in the hidden Branch/Base steps.
    pub fn toggle_worktree(&mut self) {
        self.worktree = !self.worktree;
        if !self.worktree {
            self.branch_input.clear();
            self.refresh_branch_entries();
        }
    }

    /// Branches matching the typed filter (case-insensitive substring); the
    /// full list when the filter is empty. Order follows `base_branches`
    /// (current branch first — see git::list_branches).
    pub fn base_matches(&self) -> Vec<String> {
        let f = self.base_filter.to_lowercase();
        let branches = if self.base_branches.is_empty() {
            &self.branches
        } else {
            &self.base_branches
        };
        branches
            .iter()
            .filter(|b| b.to_lowercase().contains(&f))
            .cloned()
            .collect()
    }

    /// The highlighted match — what submit uses as the worktree base.
    pub fn selected_base(&self) -> Option<String> {
        self.base_matches().get(self.base_index).cloned()
    }

    /// Move the match highlight by `delta` (wraps). No-op without matches.
    pub fn base_select(&mut self, delta: isize) {
        let n = self.base_matches().len() as isize;
        if n == 0 {
            return;
        }
        self.base_index = (((self.base_index as isize + delta) % n + n) % n) as usize;
    }

    /// Edit the filter; the highlight resets to the first match.
    pub fn base_filter_push(&mut self, c: char) {
        self.base_filter.push(c);
        self.base_index = 0;
    }

    /// Mirror of `base_filter_push`: shrink the filter; highlight resets too.
    pub fn base_filter_pop(&mut self) {
        self.base_filter.pop();
        self.base_index = 0;
    }

    /// Total number of steps shown in the `N of M` indicator.
    pub fn total_steps(&self) -> usize {
        self.field_sequence().len()
    }

    /// Recompute the subdir listing for the current `dir` text and reset highlight.
    pub fn refresh_dir_entries(&mut self) {
        let (base, filter) = crate::browse::split_path(&self.dir);
        self.dir_entries = crate::browse::list(&expand_tilde(&base), &filter);
        self.dir_selected = 0;
    }

    fn dir_select_next(&mut self) {
        if self.dir_entries.is_empty() {
            return;
        }
        self.dir_selected = (self.dir_selected + 1) % self.dir_entries.len();
    }

    fn dir_select_prev(&mut self) {
        if self.dir_entries.is_empty() {
            return;
        }
        self.dir_selected = if self.dir_selected == 0 {
            self.dir_entries.len() - 1
        } else {
            self.dir_selected - 1
        };
    }

    /// True when the current choice is the free-text "custom…" slot.
    pub fn agent_is_custom(&self) -> bool {
        self.agent_choices
            .get(self.agent_index)
            .map(|c| c == CUSTOM_AGENT_SLOT)
            .unwrap_or(false)
    }

    /// Move agent selection by `delta` (wraps); sets `agent` to the chosen command,
    /// or clears it for the custom slot so the user can type a command.
    pub fn cycle_agent(&mut self, delta: isize) {
        let n = self.agent_choices.len() as isize;
        if n == 0 {
            return;
        }
        self.agent_index = (((self.agent_index as isize + delta) % n + n) % n) as usize;
        if self.agent_is_custom() {
            self.agent.clear();
        } else {
            self.agent = self.agent_choices[self.agent_index].clone();
        }
        // A different agent invalidates any claude model/effort choice.
        self.model_index = None;
        self.effort_index = 0;
    }

    /// True when the agent command's binary is claude (covers presets and
    /// custom commands like `claude --some-flag`).
    pub fn agent_is_claude(&self) -> bool {
        self.agent.split_whitespace().next() == Some("claude")
    }

    /// True when the model list renders under the agent row.
    pub fn model_list_visible(&self) -> bool {
        !self.terminal && self.agent_is_claude()
    }

    /// Effort slider positions for the highlighted model (auto-only for haiku
    /// and while on the agent row).
    /// Indices follow CLAUDE_MODELS order: 0 = opus, 1 = sonnet, 2+ = haiku.
    pub fn effort_levels(&self) -> &'static [&'static str] {
        match self.model_index {
            Some(0) => EFFORTS_OPUS,
            Some(1) => EFFORTS_SONNET,
            _ => EFFORTS_HAIKU,
        }
    }

    /// Move the model cursor down: the agent row enters the list, models walk
    /// toward haiku. Returns false when the cursor should leave the list (the
    /// caller advances to the next field; the selection survives).
    pub fn model_down(&mut self) -> bool {
        if !self.model_list_visible() {
            return false;
        }
        match self.model_index {
            None => {
                self.model_index = Some(0);
                self.effort_index = 0;
                true
            }
            Some(i) if i + 1 < CLAUDE_MODELS.len() => {
                self.model_index = Some(i + 1);
                self.effort_index = 0;
                true
            }
            Some(_) => false,
        }
    }

    /// Move the model cursor up; every move resets effort to auto. From the
    /// first model returns to the agent row. False when already on the agent row.
    pub fn model_up(&mut self) -> bool {
        // No visibility guard needed: model_index is always None off-claude.
        match self.model_index {
            Some(0) => {
                self.model_index = None;
                self.effort_index = 0;
                true
            }
            Some(i) => {
                self.model_index = Some(i - 1);
                self.effort_index = 0;
                true
            }
            None => false,
        }
    }

    /// Move the effort slider by `delta`, clamped to the model's levels (a
    /// slider, not a carousel — no wrap). No-op for haiku (auto only).
    pub fn cycle_effort(&mut self, delta: isize) {
        let n = self.effort_levels().len() as isize;
        let i = self.effort_index as isize + delta;
        self.effort_index = i.clamp(0, n - 1) as usize;
    }

    /// The highlighted model's alias, if the cursor is in the list.
    pub fn selected_model(&self) -> Option<&'static str> {
        self.model_index.map(|i| CLAUDE_MODELS[i])
    }

    /// The slider's effort level; None at the auto position.
    pub fn selected_effort(&self) -> Option<&'static str> {
        match self.effort_index {
            0 => None,
            i => self.effort_levels().get(i).copied(),
        }
    }

    /// (--model, --effort) values exactly as they will be submitted: empty
    /// unless a claude model is selected; effort never without a model.
    pub fn model_flags(&self) -> (Option<&'static str>, Option<&'static str>) {
        if self.terminal || !self.agent_is_claude() {
            return (None, None);
        }
        let model = self.selected_model();
        (model, model.and(self.selected_effort()))
    }

    /// Jump to the custom… slot for free typing (typing/Backspace on a preset).
    pub fn switch_to_custom(&mut self) {
        self.agent_index = self.agent_choices.len().saturating_sub(1);
        self.agent.clear();
        self.model_index = None;
        self.effort_index = 0;
    }

    /// Appends pasted text to the focused field (reloads dir listing if on Dir,
    /// or the branch picker rows if on Branch).
    pub fn paste(&mut self, text: &str) {
        self.current_mut().push_str(text);
        if self.field == CreateField::Dir {
            self.refresh_dir_entries();
        }
        if self.field == CreateField::Branch {
            self.refresh_branch_entries();
        }
        if self.field == CreateField::Base {
            self.base_index = 0;
        }
    }

    /// 1-based position of the focused field, for the `N of M` step indicator.
    pub fn step(&self) -> usize {
        let seq = self.field_sequence();
        seq.iter()
            .position(|&f| f == self.field)
            .map(|i| i + 1)
            .unwrap_or(1)
    }

    /// Append the highlighted subdir to the path (preserving `~`) and reload entries.
    fn enter_selected_dir(&mut self) {
        let Some(name) = self.dir_entries.get(self.dir_selected).cloned() else {
            return;
        };
        let (base, _filter) = crate::browse::split_path(&self.dir);
        self.dir = format!("{base}{name}/");
        self.refresh_dir_entries();
    }
}

#[derive(Debug, Clone)]
pub struct RenameForm {
    pub old: String,
    pub buffer: String,
}

impl RenameForm {
    pub fn new(old: String) -> Self {
        Self {
            buffer: old.clone(),
            old,
        }
    }
}

/// State of the self-update modal: the offered release and, after `y`,
/// the install progress.
#[derive(Debug, Clone)]
pub struct UpdateModal {
    pub info: crate::update::UpdateInfo,
    /// None = still asking y/n; Some = install in flight (or finished).
    pub stage: Option<crate::update::UpdateStage>,
}

/// Form carried by `Mode::ConfirmDelete`: the session to kill + worktree toggle.
#[derive(Debug, Clone)]
pub struct KillForm {
    pub name: String,
    /// (repo_root, worktree_path) when the session is worktree-backed; enables the toggle.
    pub worktree: Option<(String, String)>,
    pub remove_worktree: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GitAction {
    Promote,
    DeleteBranch,
    BranchCleanup,
}

#[derive(Debug, Clone)]
pub struct BranchItem {
    pub name: String,
    pub protected: bool,
}

/// Form carried by `Mode::Git`.
#[derive(Debug, Clone)]
pub struct GitForm {
    pub session_name: String,
    pub branch: String,
    /// Absolute path to the project's git root.
    pub repo_root: String,
    /// Some(path) for Promote; None for DeleteBranch and BranchCleanup.
    pub worktree_path: Option<String>,
    /// Promote only: working tree was dirty when modal opened → stash needed.
    pub has_stash: bool,
    pub action: GitAction,
    /// BranchCleanup only: list of candidate branches.
    pub branches: Vec<BranchItem>,
    /// BranchCleanup only: selected indices (pre-selected = all non-protected).
    pub selected: std::collections::HashSet<usize>,
    /// BranchCleanup only: cursor row.
    pub cursor: usize,
}

/// Free-text reply being composed for a specific session.
///
/// Editing is delegated to [`crate::editor::TextArea`] which holds the buffer
/// and character-indexed cursor.
#[derive(Debug, Clone)]
pub struct ReplyForm {
    pub name: String,
    pub area: crate::editor::TextArea,
}

impl ReplyForm {
    /// Fresh empty composer — used by tests within this module.
    #[cfg(test)]
    fn new(name: String) -> Self {
        Self::with_draft(name, String::new())
    }

    /// Composer pre-filled with a saved draft, cursor at the end.
    fn with_draft(name: String, draft: String) -> Self {
        Self {
            name,
            area: crate::editor::TextArea::new(draft),
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.area.insert_char(c)
    }

    pub fn insert_str(&mut self, s: &str) {
        self.area.insert_str(s)
    }

    pub fn backspace(&mut self) {
        self.area.backspace()
    }

    pub fn delete(&mut self) {
        self.area.delete()
    }

    pub fn left(&mut self) {
        self.area.left()
    }

    pub fn right(&mut self) {
        self.area.right()
    }

    pub fn home(&mut self) {
        self.area.home()
    }

    pub fn end(&mut self) {
        self.area.end()
    }

    pub fn up(&mut self) {
        self.area.up()
    }

    pub fn down(&mut self) {
        self.area.down()
    }

    pub fn delete_word(&mut self) {
        self.area.delete_word()
    }

    pub fn delete_to_line_start(&mut self) {
        self.area.delete_to_line_start()
    }

    /// Delegates to `TextArea::line_bounds` for tests within this module.
    #[cfg(test)]
    fn line_bounds(&self) -> (usize, usize) {
        self.area.line_bounds()
    }
}

#[derive(Debug, Clone)]
// One Mode lives in App at a time (never in collections), so the size gap
// between Create(CreateForm) and the rest costs nothing in practice.
#[allow(clippy::large_enum_variant)]
pub enum Mode {
    List,
    Create(CreateForm),
    Rename(RenameForm),
    ConfirmDelete(KillForm),
    /// Typed confirmation before restarting all Claude sessions (entered with
    /// `u`, which is easy to hit by accident). Holds the text typed so far;
    /// Enter fires only when it spells a confirmation word (`confirms_restart`).
    ConfirmRestart(String),
    Help,
    Filter,
    Reply(ReplyForm),
    /// Awaiting a 1–9 digit to jump to that session (entered with `s`).
    SelectSession,
    /// Editing a project's display name (entered with Shift+R). Display-only —
    /// never renames the directory.
    RenameProject(ProjectRenameForm),
    /// Focused-note mode: the user is reading/editing the right-pane note.
    Note(NoteState),
    /// Full-screen log of recent OAuth calls (usage + profile endpoints).
    UsageLog,
    /// Self-update offer / install progress (opened automatically when a
    /// newer release is found and the app is idle in the list).
    ConfirmUpdate(UpdateModal),
    /// Git operation panel (promote worktree, delete branch, or batch cleanup).
    Git(GitForm),
}

/// Which note `Mode::Note` is editing.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteTarget {
    /// A project's note, keyed by its root path.
    Project(String),
    Session(String),
}

/// Render vs raw-edit sub-mode inside a focused note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSub {
    Render,
    Edit,
}

/// Focused-note state (the user pressed Tab into the note pane).
#[derive(Debug, Clone)]
pub struct NoteState {
    pub target: NoteTarget,
    pub sub: NoteSub,
    /// Task ordinal the render cursor is on.
    pub cursor: usize,
    /// Visual-selection anchor (task ordinal), or None when not selecting.
    pub anchor: Option<usize>,
    /// Edit buffer; only meaningful in `Edit` sub-mode.
    pub editor: crate::editor::TextArea,
    /// True while a "clear note?" confirmation is pending (render sub-mode): the
    /// next key either confirms (`y`) or cancels the wipe.
    pub confirm_clear: bool,
}

/// Display-name editor for a project, keyed by its root path.
#[derive(Debug, Clone)]
pub struct ProjectRenameForm {
    pub root: String,
    pub buffer: String,
}

/// Worktree parameters carried by `Action::Create` when the chosen branch
/// requires one (any branch other than the directory's current branch).
#[derive(Debug, Clone, PartialEq)]
pub enum WorktreeSpec {
    /// Fork a new branch from `base` (the `+ create` picker entry).
    New { base: String, branch: String },
    /// Check out an existing branch — reuse its registered worktree or add one.
    Existing { branch: String },
}

/// Side effects the event loop must perform (kept out of `App` so it stays IO-free).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Attach(String),
    Create {
        name: String,
        dir: String,
        agent: String,
        worktree: Option<WorktreeSpec>,
        terminal: bool,
        /// Claude model alias for --model; None = claude picks its default.
        model: Option<String>,
        /// Effort level for --effort; never Some without `model`.
        effort: Option<String>,
    },
    Kill {
        name: String,
        remove_worktree: bool,
    },
    Rename {
        old: String,
        new: String,
    },
    /// Send a menu digit (then Enter) to a session's agent.
    SendChoice {
        name: String,
        digit: char,
    },
    /// Send free-text (then Enter) to a session's agent.
    SendText {
        name: String,
        text: String,
    },
    /// Send Shift+Tab to a session's agent (e.g. cycle Claude Code's mode).
    SendShiftTab {
        name: String,
    },
    /// Send double Ctrl+C to all Claude sessions and begin watching for
    /// their `claude --resume <uuid>` output so they can be restarted.
    RestartAllClaude,
    /// Start downloading/installing the offered release (main spawns the
    /// installer thread; progress comes back via `set_update_stage`).
    StartUpdate(crate::update::UpdateInfo),
    /// Restore the terminal and exec() the freshly installed binary.
    RestartSelf,
    /// Promote a worktree session to the project root: stash (if dirty),
    /// remove worktree, then send `cd <root> && git checkout <branch>` to the session.
    PromoteWorktree { name: String, branch: String, has_stash: bool },
    /// Delete a local git branch (safe, refuses unmerged).
    DeleteBranch { name: String, branch: String },
    /// Delete a set of merged branches in a repo.
    CleanupBranches { repo_root: String, branches: Vec<String> },
}

#[derive(Copy, Clone)]
enum ModeKind {
    List,
    Create,
    Rename,
    ConfirmDelete,
    ConfirmRestart,
    Help,
    Filter,
    Reply,
    SelectSession,
    RenameProject,
    Note,
    UsageLog,
    ConfirmUpdate,
    Git,
}

/// What the right pane renders: the live session preview, the selected session's
/// note, or the selected session's project note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    Preview,
    SessionNote,
    ProjectNote,
}

pub struct App {
    pub config: Config,
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub mode: Mode,
    pub preview: String,
    pub snapshots: HashMap<String, u64>,
    /// Latest `dir → GitInfo` from the background git reader (see `git_worker`).
    /// Empty when no worker is attached (the refresh then reads git inline).
    pub git_cache: HashMap<String, crate::git::GitInfo>,
    /// Background git reader; `None` in tests (git is read synchronously then).
    pub git_worker: Option<crate::git::GitReader>,
    pub error: Option<String>,
    pub should_quit: bool,
    pub filter: Option<String>,
    pub spinner_frame: usize,
    pub now_unix: i64,
    pub tmux_missing: bool,
    pub clock: String,
    /// UTC minute (`now_unix / 60`) the cached `clock` was computed for, so the
    /// `date` fork only happens once per minute instead of every refresh.
    last_clock_minute: i64,
    /// Detected numbered prompt per session: option labels for digits 1..N.
    pub prompts: HashMap<String, Vec<String>>,
    /// Lines the preview is scrolled up from the bottom (0 = latest/bottom).
    pub preview_scroll: u16,
    /// Preview content area (cols, rows), written by the renderer each frame so
    /// the capture logic can size the tmux window to match. `Cell` because render
    /// only holds `&App`. (0, 0) until the first frame.
    pub preview_dims: std::cell::Cell<(u16, u16)>,
    /// Last (session, cols, rows) we resized a window to, to skip redundant
    /// `resize-window` calls (which would needlessly reflow the agent).
    pub preview_sized: Option<(String, u16, u16)>,
    /// Left (sessions) pane width as a percentage of the body.
    pub split_pct: u16,
    /// Latest Claude Code subscription usage (5h / 7d), shown in the header.
    /// `None` until the first successful fetch (or if unauthenticated).
    pub usage: Option<crate::usage::Usage>,
    /// Subscription plan badge (e.g. "Max 5×"), shown in the header.
    pub plan: Option<String>,
    /// Reason the latest usage fetch failed ("429", "no auth", …), or `None` if
    /// it succeeded. Shown in the header so an empty limits area is explainable.
    pub usage_error: Option<String>,
    /// Rolling log of OAuth calls (up to 50 entries), shared with the background
    /// poller. Read by the `Mode::UsageLog` modal for debugging.
    pub usage_log: crate::usage::UsageLog,
    /// How many display-rows the usage-log modal is scrolled up from the top.
    pub usage_log_scroll: u16,
    /// A newer release found by the startup check; renders the header badge.
    pub update: Option<crate::update::UpdateInfo>,
    /// The offer modal is shown at most once per run.
    pub update_prompted: bool,
    /// Sessions that received double Ctrl+C and are waiting for a
    /// `claude --resume <uuid>` command to appear in their pane output.
    /// Maps session name → `now_unix` when the restart was initiated (for
    /// the 30-second timeout).
    pub restarting: HashMap<String, i64>,
    /// User's custom session order *within projects* (by name). Empty = tmux order.
    pub order: Vec<String>,
    /// User's custom project (group) order, by project root path.
    pub project_order: Vec<String>,
    /// Display-name overrides for projects, keyed by project root path.
    pub project_names: std::collections::BTreeMap<String, String>,
    /// Set when persisted state (split width / order / names) changed and needs
    /// saving. The event loop saves and clears it; keeps `App` itself IO-free.
    pub dirty: bool,
    /// Per-project notes (markdown), keyed by project root path.
    pub project_notes: std::collections::BTreeMap<String, String>,
    /// Per-session notes (markdown), keyed by tmux session name.
    pub notes: std::collections::BTreeMap<String, String>,
    /// In-progress reply drafts, keyed by tmux session name.
    pub drafts: std::collections::BTreeMap<String, String>,
    /// Which content the right pane shows.
    pub right_pane: RightPane,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            sessions: Vec::new(),
            selected: 0,
            mode: Mode::List,
            preview: String::new(),
            snapshots: HashMap::new(),
            git_cache: HashMap::new(),
            git_worker: None,
            error: None,
            should_quit: false,
            filter: None,
            spinner_frame: 0,
            now_unix: crate::timeutil::now_unix(),
            tmux_missing: false,
            clock: crate::timeutil::clock_hhmm(),
            last_clock_minute: crate::timeutil::now_unix() / 60,
            prompts: HashMap::new(),
            preview_scroll: 0,
            preview_dims: std::cell::Cell::new((0, 0)),
            preview_sized: None,
            split_pct: 40,
            usage: None,
            plan: None,
            usage_error: None,
            usage_log: crate::usage::new_log(),
            usage_log_scroll: 0,
            update: None,
            update_prompted: false,
            restarting: HashMap::new(),
            order: Vec::new(),
            project_order: Vec::new(),
            project_names: std::collections::BTreeMap::new(),
            dirty: false,
            project_notes: std::collections::BTreeMap::new(),
            notes: std::collections::BTreeMap::new(),
            drafts: std::collections::BTreeMap::new(),
            right_pane: RightPane::Preview,
        }
    }

    /// Applies persisted UI state (split width + ordering + names) at startup.
    pub fn apply_state(&mut self, state: crate::state::State) {
        if let Some(pct) = state.split_pct {
            self.split_pct = pct.clamp(20, 75);
        }
        self.order = state.order;
        self.project_order = state.project_order;
        self.project_names = state.project_names;
        self.project_notes = state.project_notes;
        self.notes = state.notes;
        self.drafts = state.drafts;
    }

    /// Snapshots the persistable UI state for saving to disk.
    pub fn snapshot_state(&self) -> crate::state::State {
        crate::state::State {
            split_pct: Some(self.split_pct),
            order: self.order.clone(),
            project_order: self.project_order.clone(),
            project_names: self.project_names.clone(),
            project_notes: self.project_notes.clone(),
            notes: self.notes.clone(),
            drafts: self.drafts.clone(),
        }
    }

    /// Display name for a project given its root path: the user's override if set,
    /// otherwise the default (the root's last path component).
    pub fn project_display_name(&self, root: &str) -> String {
        self.project_names
            .get(root)
            .cloned()
            .unwrap_or_else(|| project_default_name(root).to_string())
    }

    /// Scroll the preview up (into history) / down (toward latest) by `n` lines.
    pub fn preview_scroll_up(&mut self, n: u16) {
        self.preview_scroll = self.preview_scroll.saturating_add(n).min(5000);
    }
    pub fn preview_scroll_down(&mut self, n: u16) {
        self.preview_scroll = self.preview_scroll.saturating_sub(n);
    }
    /// Jump the preview to the latest output (bottom).
    fn preview_to_end(&mut self) {
        self.preview_scroll = 0;
    }

    /// Adjust the left pane width by `delta` percent, clamped to a sane range.
    fn resize_split(&mut self, delta: i16) {
        let next = (self.split_pct as i16 + delta).clamp(20, 75);
        if next as u16 != self.split_pct {
            self.split_pct = next as u16;
            self.dirty = true; // persist the new width
        }
    }

    /// Move the selected session up (`delta = -1`) or down (`delta = +1`).
    /// Within its project the session swaps with a sibling; at the project's edge
    /// the whole project swaps with the neighbouring project (so a session never
    /// leaves its group). Persisted; the selection follows the session. No-op
    /// while a filter is active (hidden neighbours make a move ambiguous).
    fn move_selected(&mut self, delta: isize) {
        if self.filter.is_some() || self.sessions.len() < 2 {
            return;
        }
        let sel_name = self.sessions[self.selected.min(self.sessions.len() - 1)]
            .name
            .clone();
        let mut groups = group_in_order(std::mem::take(&mut self.sessions));
        // Locate the selected session as (group index, index within group).
        let Some((gi, si)) = groups.iter().enumerate().find_map(|(gi, (_, gs))| {
            gs.iter()
                .position(|s| s.name == sel_name)
                .map(|si| (gi, si))
        }) else {
            self.sessions = groups.into_iter().flat_map(|(_, gs)| gs).collect();
            return;
        };
        let moved = if delta > 0 {
            if si + 1 < groups[gi].1.len() {
                groups[gi].1.swap(si, si + 1);
                true
            } else if gi + 1 < groups.len() {
                groups.swap(gi, gi + 1);
                true
            } else {
                false
            }
        } else if si > 0 {
            groups[gi].1.swap(si, si - 1);
            true
        } else if gi > 0 {
            groups.swap(gi, gi - 1);
            true
        } else {
            false
        };
        self.sessions = groups.into_iter().flat_map(|(_, gs)| gs).collect();
        self.selected = self
            .sessions
            .iter()
            .position(|s| s.name == sel_name)
            .unwrap_or(self.selected);
        if moved {
            // Re-derive both orders from the new arrangement (drops dead entries).
            self.order = self.sessions.iter().map(|s| s.name.clone()).collect();
            self.project_order = unique_roots(&self.sessions);
            self.dirty = true;
            self.update_preview();
        }
    }

    /// Inserts pasted text into whatever text field is currently focused.
    pub fn handle_paste(&mut self, text: &str) {
        // Most terminals deliver a CR for newlines in a paste; normalize away.
        let text = text.replace('\r', "");
        match &mut self.mode {
            Mode::Reply(f) => f.insert_str(&text),
            Mode::Rename(f) => f.buffer.push_str(&text),
            Mode::RenameProject(f) => f.buffer.push_str(&text),
            Mode::Create(f) => f.paste(&text),
            Mode::Filter => {
                if let Some(s) = self.filter.as_mut() {
                    s.push_str(&text);
                }
            }
            _ => {}
        }
    }

    /// Detected prompt options for the currently selected session, if any.
    pub fn selected_prompt(&self) -> Option<&Vec<String>> {
        let name = self.selected_session()?.name.clone();
        self.prompts.get(&name)
    }

    /// Indices into `self.sessions` that match the active filter (all if none).
    pub fn visible_indices(&self) -> Vec<usize> {
        match &self.filter {
            None => (0..self.sessions.len()).collect(),
            Some(q) => {
                let q = q.to_lowercase();
                self.sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.name.to_lowercase().contains(&q))
                    .map(|(i, _)| i)
                    .collect()
            }
        }
    }

    /// The session currently highlighted (mapping `selected` through the filter).
    pub fn selected_session(&self) -> Option<&Session> {
        let vis = self.visible_indices();
        vis.get(self.selected).and_then(|&i| self.sessions.get(i))
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected_session().map(|s| s.name.clone())
    }

    pub fn select_next(&mut self) {
        let n = self.visible_indices().len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected + 1) % n;
    }

    pub fn select_prev(&mut self) {
        let n = self.visible_indices().len();
        if n == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            n - 1
        } else {
            self.selected - 1
        };
    }

    fn clamp_selection(&mut self) {
        let n = self.visible_indices().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    fn select_first(&mut self) {
        self.selected = 0;
    }

    fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::List => ModeKind::List,
            Mode::Create(_) => ModeKind::Create,
            Mode::Rename(_) => ModeKind::Rename,
            Mode::ConfirmDelete(_) => ModeKind::ConfirmDelete,
            Mode::ConfirmRestart(_) => ModeKind::ConfirmRestart,
            Mode::Help => ModeKind::Help,
            Mode::Filter => ModeKind::Filter,
            Mode::Reply(_) => ModeKind::Reply,
            Mode::SelectSession => ModeKind::SelectSession,
            Mode::RenameProject(_) => ModeKind::RenameProject,
            Mode::Note(_) => ModeKind::Note,
            Mode::UsageLog => ModeKind::UsageLog,
            Mode::ConfirmUpdate(_) => ModeKind::ConfirmUpdate,
            Mode::Git(_) => ModeKind::Git,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.mode_kind() {
            ModeKind::List => self.handle_list_key(key),
            ModeKind::Help => {
                if latin_code(key.code) == KeyCode::Char('q') {
                    self.should_quit = true;
                }
                self.mode = Mode::List;
                None
            }
            ModeKind::ConfirmDelete => self.handle_confirm_key(key),
            ModeKind::ConfirmRestart => self.handle_confirm_restart_key(key),
            ModeKind::Create => self.handle_create_key(key),
            ModeKind::Rename => self.handle_rename_key(key),
            ModeKind::Filter => self.handle_filter_key(key),
            ModeKind::Reply => self.handle_reply_key(key),
            ModeKind::SelectSession => self.handle_select_session_key(key),
            ModeKind::RenameProject => self.handle_rename_project_key(key),
            ModeKind::Note => self.handle_note_key(key),
            ModeKind::ConfirmUpdate => self.handle_confirm_update_key(key),
            ModeKind::UsageLog => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match latin_code(key.code) {
                    KeyCode::Char('k') if ctrl => {
                        self.usage_log_scroll = self.usage_log_scroll.saturating_add(10);
                    }
                    KeyCode::Char('j') if ctrl => {
                        self.usage_log_scroll = self.usage_log_scroll.saturating_sub(10);
                    }
                    KeyCode::PageUp => {
                        self.usage_log_scroll = self.usage_log_scroll.saturating_add(10);
                    }
                    KeyCode::PageDown => {
                        self.usage_log_scroll = self.usage_log_scroll.saturating_sub(10);
                    }
                    KeyCode::Char('y') if ctrl => {
                        if let Ok(g) = self.usage_log.lock() {
                            crate::clip::copy(&crate::usage::format_log_plain(&g));
                        }
                    }
                    _ => {
                        self.mode = Mode::List;
                    }
                }
                None
            }
            ModeKind::Git => self.handle_git_key(key),
        }
    }

    /// Opens the update offer when one is pending, the user is idle in the
    /// list, and we haven't asked yet this run.
    pub fn offer_update_if_idle(&mut self) {
        if self.update_prompted || !matches!(self.mode, Mode::List) {
            return;
        }
        let Some(info) = self.update.clone() else {
            return;
        };
        self.update_prompted = true;
        self.mode = Mode::ConfirmUpdate(UpdateModal { info, stage: None });
    }

    /// Feeds installer progress in. A hidden modal stays hidden; Done clears
    /// the header badge either way (the binary on disk is current now).
    pub fn set_update_stage(&mut self, stage: crate::update::UpdateStage) {
        if matches!(stage, crate::update::UpdateStage::Done(_)) {
            self.update = None;
        }
        if let Mode::ConfirmUpdate(m) = &mut self.mode {
            m.stage = Some(stage);
        }
    }

    /// Immediately refreshes the preview for the selected session (one capture),
    /// so switching sessions feels instant instead of waiting for the next tick.
    pub fn update_preview(&mut self) {
        // Reset scroll to the latest output whenever the selection changes, and
        // capture extra scrollback so the preview can be paged back.
        self.preview_scroll = 0;
        match self.selected_name() {
            Some(name) => {
                self.fit_preview_window(&name);
                self.preview = crate::tmux::capture_scrollback(&name, 500)
                    .or_else(|_| crate::tmux::capture_pane(&name))
                    .unwrap_or_default();
            }
            None => self.preview.clear(),
        }
    }

    /// Size `name`'s tmux window to the preview content area so its capture
    /// reflows to the preview width (no wrapped, doubled input box). Skips the
    /// call when the window is already at that size for this session.
    fn fit_preview_window(&mut self, name: &str) {
        let (cols, rows) = self.preview_dims.get();
        if cols == 0 || rows == 0 {
            return; // No frame rendered yet — nothing to match.
        }
        let target = (name.to_string(), cols, rows);
        if self.preview_sized.as_ref() == Some(&target) {
            return;
        }
        crate::tmux::resize_window(name, cols, rows);
        self.preview_sized = Some(target);
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Hotkeys are layout-independent: a Cyrillic char is mapped to the Latin
        // letter on the same physical key.
        match latin_code(key.code) {
            KeyCode::Char('q') => self.should_quit = true,
            // Plain j/k move the selection; Ctrl+j/Ctrl+k scroll the preview (below).
            KeyCode::Char('j') if !ctrl => {
                self.select_next();
                self.update_preview();
            }
            KeyCode::Down => {
                self.select_next();
                self.update_preview();
            }
            KeyCode::Char('k') if !ctrl => {
                self.select_prev();
                self.update_preview();
            }
            KeyCode::Up => {
                self.select_prev();
                self.update_preview();
            }
            KeyCode::Char('n') => {
                self.error = None;
                self.mode = Mode::Create(CreateForm::new(
                    &self.config.default_agent,
                    &self.config.agent_presets,
                ));
            }
            // Shift+N: new session pre-filled from the selected session's project
            // (path pre-filled; agent still selectable in the form). No-op if
            // nothing is selected.
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
            KeyCode::Char('d') => {
                if let Some(s) = self.selected_session() {
                    let worktree = s.worktree_repo.clone().map(|repo| (repo, s.dir.clone()));
                    self.mode = Mode::ConfirmDelete(KillForm {
                        name: s.name.clone(),
                        worktree,
                        remove_worktree: false,
                    });
                }
            }
            // u: restart all Claude sessions (double Ctrl+C, then auto-resume).
            // Destructive and adjacent to plain typing (e.g. a reply started
            // without `i`), so it asks for a typed confirmation first.
            KeyCode::Char('u') => self.mode = Mode::ConfirmRestart(String::new()),
            KeyCode::Char('r') => {
                if let Some(name) = self.selected_name() {
                    self.mode = Mode::Rename(RenameForm::new(name));
                }
            }
            // Shift+R: rename the selected session's project (display-only).
            KeyCode::Char('R') => {
                if let Some(s) = self.selected_session() {
                    let root = session_root(s).to_string();
                    let buffer = self.project_display_name(&root);
                    self.mode = Mode::RenameProject(ProjectRenameForm { root, buffer });
                }
            }
            // Shift+Tab: forward to the agent so it cycles its own mode.
            KeyCode::BackTab => {
                if let Some(name) = self.selected_name() {
                    return Some(Action::SendShiftTab { name });
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('L') => {
                self.usage_log_scroll = 0;
                self.mode = Mode::UsageLog;
            }
            KeyCode::Enter | KeyCode::Char('o') => {
                if let Some(name) = self.selected_name() {
                    return Some(Action::Attach(name));
                }
            }
            KeyCode::Char('/') => {
                self.filter = Some(String::new());
                self.selected = 0;
                self.mode = Mode::Filter;
            }
            KeyCode::Char('g') if !ctrl => {
                self.select_first();
                self.update_preview();
            }
            // Preview scroll (without attaching). G jumps to the latest output.
            // Note: Ctrl+J only arrives distinctly under the kitty keyboard
            // protocol; on terminals without it, Ctrl+J == Enter (= attach).
            KeyCode::Char('G') => self.preview_to_end(),
            KeyCode::PageUp => self.preview_scroll_up(10),
            KeyCode::PageDown => self.preview_scroll_down(10),
            KeyCode::Char('k') if ctrl => self.preview_scroll_up(10),
            KeyCode::Char('j') if ctrl => self.preview_scroll_down(10),
            KeyCode::Char('g') if ctrl => {
                if let Some(s) = self.selected_session() {
                    let root = session_root(s).to_string();
                    if is_worktree(s) {
                        let branch = s.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default();
                        let has_stash = crate::git::is_dirty(&s.dir);
                        self.mode = Mode::Git(GitForm {
                            session_name: s.name.clone(),
                            branch,
                            repo_root: root,
                            worktree_path: Some(s.dir.clone()),
                            has_stash,
                            action: GitAction::Promote,
                            branches: vec![],
                            selected: std::collections::HashSet::new(),
                            cursor: 0,
                        });
                    } else if let Some(g) = &s.git {
                        let branch = g.branch.clone();
                        if !crate::git::PROTECTED_BRANCHES.contains(&branch.as_str()) {
                            self.mode = Mode::Git(GitForm {
                                session_name: s.name.clone(),
                                branch,
                                repo_root: root,
                                worktree_path: None,
                                has_stash: false,
                                action: GitAction::DeleteBranch,
                                branches: vec![],
                                selected: std::collections::HashSet::new(),
                                cursor: 0,
                            });
                        }
                    }
                }
            }
            KeyCode::Char('l') if ctrl => {
                if let Some(s) = self.selected_session() {
                    let root = session_root(s).to_string();
                    let raw = crate::git::list_merged_branches(&root);
                    if raw.is_empty() {
                        self.error = Some("no merged branches found".into());
                    } else {
                        let branches: Vec<BranchItem> = raw
                            .into_iter()
                            .map(|name| {
                                let protected = crate::git::PROTECTED_BRANCHES.contains(&name.as_str());
                                BranchItem { name, protected }
                            })
                            .collect();
                        let selected: std::collections::HashSet<usize> = branches
                            .iter()
                            .enumerate()
                            .filter(|(_, b)| !b.protected)
                            .map(|(i, _)| i)
                            .collect();
                        self.mode = Mode::Git(GitForm {
                            session_name: s.name.clone(),
                            branch: String::new(),
                            repo_root: root,
                            worktree_path: None,
                            has_stash: false,
                            action: GitAction::BranchCleanup,
                            branches,
                            selected,
                            cursor: 0,
                        });
                    }
                }
            }
            KeyCode::End => self.preview_to_end(),
            // Resize the split: [ / ] small step; { / } or Ctrl+←/→ bigger step.
            KeyCode::Char('[') => self.resize_split(-3),
            KeyCode::Char(']') => self.resize_split(3),
            KeyCode::Char('{') => self.resize_split(-8),
            KeyCode::Char('}') => self.resize_split(8),
            KeyCode::Left if ctrl => self.resize_split(-8),
            KeyCode::Right if ctrl => self.resize_split(8),
            // Reorder the selected session within the list (Shift+K up / Shift+J
            // down). Persisted; mirrors the j/k navigation keys.
            KeyCode::Char('K') => self.move_selected(-1),
            KeyCode::Char('J') => self.move_selected(1),
            // Enter session-select mode: a following 1–9 jumps to that session.
            KeyCode::Char('s') => self.mode = Mode::SelectSession,
            // Quick reply: answer a detected numbered prompt with 1..9.
            KeyCode::Char(c @ '1'..='9') => {
                let idx = c as usize - '1' as usize;
                if let Some(opts) = self.selected_prompt() {
                    if idx < opts.len() {
                        if let Some(name) = self.selected_name() {
                            return Some(Action::SendChoice { name, digit: c });
                        }
                    }
                }
            }
            // Free-text reply to the selected session, restoring its draft.
            KeyCode::Char('i') => {
                if let Some(name) = self.selected_name() {
                    let draft = self.drafts.get(&name).cloned().unwrap_or_default();
                    self.mode = Mode::Reply(ReplyForm::with_draft(name, draft));
                }
            }
            KeyCode::Char('t') => {
                self.right_pane = match self.right_pane {
                    RightPane::SessionNote => RightPane::Preview,
                    _ => RightPane::SessionNote,
                };
            }
            KeyCode::Char('T') => {
                // Project note of the selected session; no-op with nothing selected.
                if self.selected_session().is_some() {
                    self.right_pane = match self.right_pane {
                        RightPane::ProjectNote => RightPane::Preview,
                        _ => RightPane::ProjectNote,
                    };
                }
            }
            KeyCode::Tab if self.right_pane != RightPane::Preview => {
                let target = match self.right_pane {
                    RightPane::ProjectNote => match self.selected_session() {
                        Some(s) => NoteTarget::Project(session_root(s).to_string()),
                        None => return None,
                    },
                    _ => match self.selected_name() {
                        Some(name) => NoteTarget::Session(name),
                        None => return None,
                    },
                };
                self.mode = Mode::Note(NoteState {
                    target,
                    sub: NoteSub::Render,
                    cursor: 0,
                    anchor: None,
                    editor: crate::editor::TextArea::default(),
                    confirm_clear: false,
                });
            }
            // esc means "exit notes" everywhere: when the pane shows a note but
            // isn't focused (after a Tab defocus, or t/T browse), close to preview.
            KeyCode::Esc if self.right_pane != RightPane::Preview => {
                self.right_pane = RightPane::Preview;
            }
            _ => {}
        }
        None
    }

    /// Session-select mode (entered with `s`): a 1–9 digit jumps to that visible
    /// session; Esc cancels. An out-of-range digit is ignored (stays in mode).
    fn handle_select_session_key(&mut self, key: KeyEvent) -> Option<Action> {
        match latin_code(key.code) {
            KeyCode::Esc => self.mode = Mode::List,
            KeyCode::Char(c @ '1'..='9') => {
                let pos = c as usize - '1' as usize;
                if pos < self.visible_indices().len() {
                    self.selected = pos;
                    self.update_preview();
                    self.mode = Mode::List;
                }
            }
            _ => {} // ignore other keys; stay in select mode
        }
        None
    }

    /// Persists the composer buffer as the session's draft — an empty buffer
    /// removes the entry. Marks state dirty so the autosave loop writes it.
    fn save_draft(&mut self, form: &ReplyForm) {
        if form.area.buffer.is_empty() {
            if self.drafts.remove(&form.name).is_some() {
                self.dirty = true;
            }
        } else {
            self.drafts
                .insert(form.name.clone(), form.area.buffer.clone());
            self.dirty = true;
        }
    }

    fn handle_reply_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Reply(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // Esc closes the composer but keeps the message as the session's draft.
            KeyCode::Esc => {
                self.save_draft(&form);
                return None; // mode already reset to List
            }
            // Newline on Shift+Enter; plain Enter sends (see below). Shift+Enter
            // requires the kitty keyboard protocol to be reported distinctly
            // (enabled in main); Alt+Enter is a fallback on terminals without it.
            KeyCode::Enter if shift || alt => form.insert_char('\n'),
            // Editing chords (readline-ish), layout-independent via latin_code.
            KeyCode::Char(_) if ctrl => match latin_code(key.code) {
                KeyCode::Char('w') => form.delete_word(),
                KeyCode::Char('u') => form.delete_to_line_start(),
                KeyCode::Char('a') => form.home(),
                KeyCode::Char('e') => form.end(),
                // "copy all": the whole buffer to the system clipboard.
                KeyCode::Char('y') => crate::clip::copy(&form.area.buffer),
                // "clear all": wipe the buffer (Esc then drops the draft too).
                KeyCode::Char('x') => form.area = crate::editor::TextArea::default(),
                _ => {}
            },
            // Plain text entry — guard against control chords leaking through.
            KeyCode::Char(c) if !ctrl => form.insert_char(c),
            // Plain Enter sends the composed message and drops the draft.
            KeyCode::Enter => {
                let text = form.area.buffer.trim().to_string();
                if text.is_empty() {
                    self.save_draft(&form); // nothing to send — close like Esc
                    return None;
                }
                if self.drafts.remove(&form.name).is_some() {
                    self.dirty = true;
                }
                return Some(Action::SendText {
                    name: form.name,
                    text,
                });
            }
            KeyCode::Backspace => form.backspace(),
            KeyCode::Delete => form.delete(),
            KeyCode::Left => form.left(),
            KeyCode::Right => form.right(),
            KeyCode::Up => form.up(),
            KeyCode::Down => form.down(),
            KeyCode::Home => form.home(),
            KeyCode::End => form.end(),
            _ => {}
        }
        self.mode = Mode::Reply(form);
        None
    }

    fn handle_confirm_update_key(&mut self, key: KeyEvent) -> Option<Action> {
        use crate::update::UpdateStage as S;
        let Mode::ConfirmUpdate(m) = &self.mode else {
            return None;
        };
        let (info, stage) = (m.info.clone(), m.stage.clone());
        match (stage, latin_code(key.code)) {
            (None, KeyCode::Char('y')) => {
                if let Mode::ConfirmUpdate(m) = &mut self.mode {
                    m.stage = Some(S::Downloading);
                }
                Some(Action::StartUpdate(info))
            }
            (None, KeyCode::Char('n') | KeyCode::Esc) => {
                self.mode = Mode::List;
                None
            }
            (Some(S::Done(_)), KeyCode::Char('r')) => Some(Action::RestartSelf),
            // Failed: esc/n closes. In-flight: esc hides, install runs on.
            (Some(_), KeyCode::Esc | KeyCode::Char('n')) => {
                self.mode = Mode::List;
                None
            }
            _ => None,
        }
    }

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

    fn handle_git_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Git(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match form.action {
            GitAction::Promote | GitAction::DeleteBranch => match latin_code(key.code) {
                KeyCode::Char('y') => {
                    return Some(match form.action {
                        GitAction::Promote => Action::PromoteWorktree {
                            name: form.session_name,
                            branch: form.branch,
                            has_stash: form.has_stash,
                        },
                        GitAction::DeleteBranch => Action::DeleteBranch {
                            name: form.session_name,
                            branch: form.branch,
                        },
                        _ => unreachable!(),
                    });
                }
                KeyCode::Char('n') | KeyCode::Esc => {}
                _ => self.mode = Mode::Git(form),
            },
            GitAction::BranchCleanup => match latin_code(key.code) {
                KeyCode::Char('y') => {
                    let mut branches: Vec<String> = form
                        .selected
                        .iter()
                        .map(|&i| form.branches[i].name.clone())
                        .collect();
                    branches.sort();
                    if !branches.is_empty() {
                        return Some(Action::CleanupBranches {
                            repo_root: form.repo_root,
                            branches,
                        });
                    }
                }
                KeyCode::Char(' ') => {
                    let i = form.cursor;
                    if i < form.branches.len() && !form.branches[i].protected {
                        if form.selected.contains(&i) {
                            form.selected.remove(&i);
                        } else {
                            form.selected.insert(i);
                        }
                    }
                    self.mode = Mode::Git(form);
                }
                KeyCode::Char('a') => {
                    form.selected = form
                        .branches
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| !b.protected)
                        .map(|(i, _)| i)
                        .collect();
                    self.mode = Mode::Git(form);
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    form.cursor =
                        (form.cursor + 1).min(form.branches.len().saturating_sub(1));
                    self.mode = Mode::Git(form);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    form.cursor = form.cursor.saturating_sub(1);
                    self.mode = Mode::Git(form);
                }
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.mode = Mode::Git(form),
            },
        }
        None
    }

    /// Typed confirmation for `u` (restart all Claude sessions). Characters are
    /// taken raw — not layout-mapped via `latin_code` — so the word can be typed
    /// on any layout. Enter fires only on a full match; otherwise the dialog
    /// stays open for editing. Esc cancels.
    fn handle_confirm_restart_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::ConfirmRestart(mut buffer) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter if confirms_restart(&buffer) => {
                return Some(Action::RestartAllClaude);
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        }
        self.mode = Mode::ConfirmRestart(buffer);
        None
    }

    /// Validate + submit a completed create form. On success returns the
    /// `Action::Create` (caller returns it up); on failure records the error and
    /// re-stores the form so the user can correct it. Either way the caller
    /// should `return` the resulting `Option<Action>`.
    fn submit_create(&mut self, form: CreateForm) -> Option<Action> {
        let existing: Vec<String> = self.sessions.iter().map(|s| s.name.clone()).collect();
        match build_create_action(&form, &existing) {
            Ok(action) => {
                self.error = None;
                Some(action)
            }
            Err(e) => {
                self.error = Some(e);
                self.mode = Mode::Create(form);
                None
            }
        }
    }

    fn handle_create_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Take the form out so we can borrow `self.sessions` for validation.
        let Mode::Create(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        // Clear any prior validation error; the handlers below re-set it if this
        // key's submit still fails, so the banner always reflects the last action.
        self.error = None;

        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        // Keys with one meaning on every step: close, submit, walk fields.
        // (Tab never moves focus — it only cycles completion candidates below.)
        match key.code {
            KeyCode::Esc => return None, // mode already reset to List
            KeyCode::Enter if shift => return self.submit_create(form),
            KeyCode::Enter => {
                form.advance();
                self.mode = Mode::Create(form);
                return None;
            }
            KeyCode::Up => {
                // On the Agent step the model list is walked first; the agent
                // row is the exit at the top.
                if form.field == CreateField::Branch {
                    form.branch_select_prev();
                } else if form.field == CreateField::Dir {
                    form.dir_select_prev();
                } else if !(form.field == CreateField::Agent && form.model_up()) {
                    form.retreat();
                }
                self.mode = Mode::Create(form);
                return None;
            }
            KeyCode::Down => {
                if form.field == CreateField::Branch {
                    form.branch_select_next();
                } else if form.field == CreateField::Dir {
                    form.dir_select_next();
                } else if !(form.field == CreateField::Agent && form.model_down()) {
                    form.advance();
                }
                self.mode = Mode::Create(form);
                return None;
            }
            _ => {}
        }

        match form.field {
            // Dir: free text + completion. Tab cycles the candidates, → descends
            // into the highlighted one.
            CreateField::Dir => match key.code {
                KeyCode::Backspace => {
                    form.dir.pop();
                    form.refresh_dir_entries();
                }
                KeyCode::Char(c) => {
                    form.dir.push(c);
                    form.refresh_dir_entries();
                }
                KeyCode::Tab | KeyCode::Right => form.enter_selected_dir(),
                _ => {}
            },
            // Toggles: space/←/→ flip. h/j/k/l are reserved for agent/model
            // controls and otherwise stay available to text fields.
            CreateField::Terminal => match key.code {
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => form.toggle_terminal(),
                _ => {}
            },
            CreateField::Worktree => match key.code {
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => form.toggle_worktree(),
                _ => {}
            },
            // Base: branch search. Typing filters, Tab cycles the matches.
            CreateField::Base => match key.code {
                KeyCode::Backspace => form.base_filter_pop(),
                KeyCode::Char(c) => form.base_filter_push(c),
                KeyCode::Tab => form.base_select(1),
                KeyCode::BackTab => form.base_select(-1),
                _ => {}
            },
            // Model list (the cursor is on a model row): j/k walk it, h/l slide
            // the effort. Plain letters do nothing — it's not a text field.
            CreateField::Agent if form.model_index.is_some() => match key.code {
                KeyCode::Char('j') => {
                    form.model_down();
                }
                KeyCode::Char('k') => {
                    form.model_up();
                }
                KeyCode::Left | KeyCode::Char('h') => form.cycle_effort(-1),
                KeyCode::Right | KeyCode::Char('l') => form.cycle_effort(1),
                _ => {}
            },
            // Agent row. h/l/j/k act as keys only while a preset is selected —
            // on the custom slot they're typed so a command can contain them.
            CreateField::Agent => match key.code {
                KeyCode::Left => form.cycle_agent(-1),
                KeyCode::Right => form.cycle_agent(1),
                KeyCode::Char('h') if !form.agent_is_custom() => form.cycle_agent(-1),
                KeyCode::Char('l') if !form.agent_is_custom() => form.cycle_agent(1),
                KeyCode::Char('j') if !form.agent_is_custom() => {
                    form.model_down();
                }
                KeyCode::Backspace => {
                    if !form.agent_is_custom() {
                        // Backspace off a preset: jump to custom and start fresh.
                        form.switch_to_custom();
                    }
                    form.agent.pop();
                }
                KeyCode::Char(c) => {
                    if !form.agent_is_custom() {
                        form.switch_to_custom();
                    }
                    form.agent.push(c);
                }
                _ => {}
            },
            // Name / Branch: plain text fields.
            CreateField::Name => match key.code {
                KeyCode::Backspace => {
                    form.current_mut().pop();
                }
                KeyCode::Char(c) => {
                    form.current_mut().push(c);
                }
                _ => {}
            },
            CreateField::Branch => match key.code {
                KeyCode::Backspace => {
                    form.branch_input.pop();
                    form.refresh_branch_entries();
                }
                KeyCode::Char(c) => {
                    form.branch_input.push(c);
                    form.refresh_branch_entries();
                }
                KeyCode::Tab => form.branch_select_next(),
                KeyCode::BackTab => form.branch_select_prev(),
                _ => {}
            },
        }
        self.mode = Mode::Create(form);
        None
    }

    fn handle_rename_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Rename(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Backspace => {
                form.buffer.pop();
            }
            KeyCode::Char(c) => form.buffer.push(c),
            KeyCode::Enter => {
                let new = form.buffer.trim().to_string();
                if new.is_empty() || new == form.old {
                    return None;
                }
                return Some(Action::Rename {
                    old: form.old.clone(),
                    new,
                });
            }
            _ => {}
        }
        self.mode = Mode::Rename(form);
        None
    }

    /// Project-rename mode: edits the display-name override (never the directory).
    /// Enter commits (empty or equal-to-default clears the override); Esc cancels.
    fn handle_rename_project_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::RenameProject(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Backspace => {
                form.buffer.pop();
            }
            KeyCode::Char(c) => form.buffer.push(c),
            KeyCode::Enter => {
                let name = form.buffer.trim();
                if name.is_empty() || name == project_default_name(&form.root) {
                    // Back to the default → drop any override.
                    self.project_names.remove(&form.root);
                } else {
                    self.project_names
                        .insert(form.root.clone(), name.to_string());
                }
                self.dirty = true;
                return None;
            }
            _ => {}
        }
        self.mode = Mode::RenameProject(form);
        None
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.filter = None;
                self.selected = 0;
                self.mode = Mode::List;
            }
            KeyCode::Enter | KeyCode::Down | KeyCode::Up => {
                // Accept the filter and return to list navigation (keep filter active).
                self.mode = Mode::List;
                if key.code == KeyCode::Down {
                    self.select_next();
                } else if key.code == KeyCode::Up {
                    self.select_prev();
                }
            }
            KeyCode::Backspace => {
                if let Some(f) = self.filter.as_mut() {
                    f.pop();
                }
                self.selected = 0;
            }
            KeyCode::Char(c) => {
                if let Some(f) = self.filter.as_mut() {
                    f.push(c);
                }
                self.selected = 0;
            }
            _ => {}
        }
        None
    }

    /// The markdown text for a note target (read-only). Missing entry = "".
    pub fn note_text(&self, target: &NoteTarget) -> &str {
        match target {
            NoteTarget::Project(root) => self
                .project_notes
                .get(root)
                .map(String::as_str)
                .unwrap_or(""),
            NoteTarget::Session(name) => self.notes.get(name).map(String::as_str).unwrap_or(""),
        }
    }

    /// Mutable handle to a note target, creating an empty entry if needed.
    pub fn note_text_mut(&mut self, target: &NoteTarget) -> &mut String {
        match target {
            NoteTarget::Project(root) => self.project_notes.entry(root.clone()).or_default(),
            NoteTarget::Session(name) => self.notes.entry(name.clone()).or_default(),
        }
    }

    fn handle_note_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Note(mut ns) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match ns.sub {
            NoteSub::Render => {
                // A pending "clear note?" confirmation captures the next key:
                // `y` wipes the note, anything else cancels.
                if ns.confirm_clear {
                    ns.confirm_clear = false;
                    if latin_code(key.code) == KeyCode::Char('y') {
                        *self.note_text_mut(&ns.target) = String::new();
                        ns.cursor = 0;
                        ns.anchor = None;
                        self.dirty = true;
                    }
                    self.mode = Mode::Note(ns);
                    return None;
                }
                let task_count = crate::note::task_line_indices(self.note_text(&ns.target)).len();
                let last = task_count.saturating_sub(1);
                // Normalize the key to its QWERTY position so the render-mode
                // chords (j/k/V/y/e/c/space) work on any keyboard layout.
                match latin_code(key.code) {
                    KeyCode::Esc => {
                        if ns.anchor.is_some() {
                            ns.anchor = None; // first esc clears the selection
                            self.mode = Mode::Note(ns);
                        } else {
                            // Fully exit: drop focus AND the note pane, back to the
                            // live preview (mode is already List from the replace).
                            self.right_pane = RightPane::Preview;
                        }
                        return None;
                    }
                    KeyCode::Tab => {
                        // Defocus back to the list but keep the note shown, so the
                        // user can move the selection (or t/T) and Tab back in to a
                        // different note. Mode is already List from the replace.
                        return None;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        ns.cursor = (ns.cursor + 1).min(last);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        ns.cursor = ns.cursor.saturating_sub(1);
                    }
                    KeyCode::Char('V') => {
                        ns.anchor = Some(ns.cursor);
                    }
                    KeyCode::Char(' ') => {
                        let range = selection_range(&ns);
                        let buf = self.note_text_mut(&ns.target);
                        for ord in range {
                            crate::note::toggle(buf, ord);
                        }
                        ns.anchor = None;
                        self.dirty = true;
                    }
                    KeyCode::Char('y') => {
                        let ords: Vec<usize> = selection_range(&ns).collect();
                        let text =
                            crate::note::selected_as_numbered(self.note_text(&ns.target), &ords);
                        crate::clip::copy(&text);
                        ns.anchor = None;
                    }
                    KeyCode::Char('e') => {
                        ns.editor =
                            crate::editor::TextArea::new(self.note_text(&ns.target).to_string());
                        ns.sub = NoteSub::Edit;
                    }
                    // Arm the clear confirmation only if there's something to wipe.
                    KeyCode::Char('c') if !self.note_text(&ns.target).is_empty() => {
                        ns.confirm_clear = true;
                    }
                    _ => {}
                }
                self.mode = Mode::Note(ns);
                None
            }
            NoteSub::Edit => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Esc => {
                        // Commit the edited buffer back to the note, re-parse on render.
                        *self.note_text_mut(&ns.target) = ns.editor.buffer.clone();
                        self.dirty = true;
                        ns.cursor = 0;
                        ns.anchor = None;
                        ns.sub = NoteSub::Render;
                    }
                    KeyCode::Enter => ns.editor.insert_char('\n'),
                    // Ctrl shortcuts are layout-independent; typed text below stays
                    // raw so the user can write Cyrillic (and any other) characters.
                    KeyCode::Char(_) if ctrl => match latin_code(key.code) {
                        KeyCode::Char('w') => ns.editor.delete_word(),
                        KeyCode::Char('u') => ns.editor.delete_to_line_start(),
                        _ => {}
                    },
                    KeyCode::Char(c) => ns.editor.insert_char(c),
                    KeyCode::Backspace => ns.editor.backspace(),
                    KeyCode::Delete => ns.editor.delete(),
                    KeyCode::Left => ns.editor.left(),
                    KeyCode::Right => ns.editor.right(),
                    KeyCode::Up => ns.editor.up(),
                    KeyCode::Down => ns.editor.down(),
                    KeyCode::Home => ns.editor.home(),
                    KeyCode::End => ns.editor.end(),
                    _ => {}
                }
                self.mode = Mode::Note(ns);
                None
            }
        }
    }

    /// Spawn the background git reader so `refresh` reads git off the UI thread.
    /// Call once at startup. Without it, `refresh` reads git synchronously.
    pub fn attach_git_worker(&mut self) {
        self.git_worker = Some(crate::git::spawn_reader());
    }

    /// Re-derives sessions from tmux and recomputes statuses + preview.
    pub fn refresh(&mut self) {
        match crate::tmux::list_sessions() {
            Ok(mut sessions) => {
                let selected_name = self.selected_name();
                // Size the previewed window to the preview area before capturing,
                // so its scrollback reflows to the preview width (no doubled box).
                if let Some(name) = &selected_name {
                    self.fit_preview_window(name);
                }
                self.now_unix = crate::timeutil::now_unix();
                // Re-fork `date` only when the minute actually changes.
                let minute = self.now_unix / 60;
                if minute != self.last_clock_minute {
                    self.clock = crate::timeutil::clock_hhmm();
                    self.last_clock_minute = minute;
                }
                let mut new_snaps = HashMap::new();
                let mut new_prompts = HashMap::new();
                let mut new_preview = None;
                for s in &mut sessions {
                    if let Ok(content) = crate::tmux::capture_pane(&s.name) {
                        let h = content_hash(&content);
                        s.status = compute_status(self.snapshots.get(&s.name).copied(), h);
                        new_snaps.insert(s.name.clone(), h);
                        let opts = parse_prompt(&content);
                        if !opts.is_empty() {
                            // A pending numbered prompt means the agent is blocked
                            // on the user; this overrides the pane-diff status.
                            s.status = Status::Waiting;
                            new_prompts.insert(s.name.clone(), opts);
                        }
                        if selected_name.as_deref() == Some(s.name.as_str()) {
                            // Preview keeps scrollback so it can be paged back.
                            new_preview = Some(
                                crate::tmux::capture_scrollback(&s.name, 500).unwrap_or(content),
                            );
                        }
                    }
                }
                // Git info: served from the background reader's cache when a
                // worker is attached (never blocks the UI), else read inline.
                // Read from `cwd` (the active pane's live path), not `dir`: an
                // agent that `cd`s into another repo/worktree and switches
                // branches there must show that branch, not the start dir's.
                if self.git_worker.is_some() {
                    // Apply the most recently received `dir → GitInfo` results.
                    let mut latest = None;
                    if let Some(w) = &self.git_worker {
                        while let Ok(map) = w.rx.try_recv() {
                            latest = Some(map);
                        }
                    }
                    if let Some(map) = latest {
                        self.git_cache = map;
                    }
                    for s in &mut sessions {
                        s.git = self.git_cache.get(&s.cwd).cloned();
                    }
                    // Ask the worker to refresh git for the current directories.
                    if let Some(w) = &self.git_worker {
                        let dirs: Vec<String> = sessions.iter().map(|s| s.cwd.clone()).collect();
                        let _ = w.tx.send(dirs);
                    }
                } else {
                    for s in &mut sessions {
                        s.git = crate::git::read(&s.cwd);
                    }
                }
                self.snapshots = new_snaps;
                self.prompts = new_prompts;
                self.sessions = apply_grouped_order(&self.project_order, &self.order, sessions);
                // A draft lives exactly as long as its session: drop entries for
                // sessions that no longer exist (covers ones that died while
                // amux wasn't running). Session notes are deliberately NOT pruned
                // here — they're user knowledge, dropped only on explicit kill.
                self.prune_dead_drafts();
                self.clamp_selection();
                if let Some(p) = new_preview {
                    self.preview = p;
                } else {
                    // Selection may have moved; show the now-selected session next tick.
                    self.preview.clear();
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Removes drafts whose session is gone. Only called from `refresh` with a
    /// freshly fetched session list — never after a failed tmux read (which
    /// must not wipe drafts).
    fn prune_dead_drafts(&mut self) {
        let sessions = &self.sessions;
        let before = self.drafts.len();
        self.drafts
            .retain(|name, _| sessions.iter().any(|s| s.name == *name));
        if self.drafts.len() != before {
            self.dirty = true;
        }
    }
}

/// Maps a Cyrillic (ЙЦУКЕН) character to the Latin letter on the same physical
/// key, so command hotkeys (j/k/o/i/n/d/r/g/q…) work on a Russian layout too.
/// Non-Cyrillic input passes through unchanged.
pub fn latinize(c: char) -> char {
    const CYR: &str = "йцукенгшщзхъфывапролджэячсмитьбю";
    const LAT: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.";
    let lower = c.to_lowercase().next().unwrap_or(c);
    if let Some(pos) = CYR.chars().position(|x| x == lower) {
        let l = LAT.chars().nth(pos).unwrap();
        return if c.is_uppercase() {
            l.to_ascii_uppercase()
        } else {
            l
        };
    }
    c
}

/// `key.code` with any Cyrillic `Char` remapped to its Latin QWERTY-position key.
pub fn latin_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(c) => KeyCode::Char(latinize(c)),
        other => other,
    }
}

/// True when the confirm-restart buffer spells out an accepted confirmation
/// word: "yes" (the documented one) or "да" (undocumented alias so the confirm
/// works without leaving a Russian layout). Case-insensitive, surrounding
/// whitespace ignored — but the word must be complete.
fn confirms_restart(buffer: &str) -> bool {
    let t = buffer.trim().to_lowercase();
    t == "yes" || t == "да"
}

/// Removes ANSI/CSI escape sequences so captured pane text can be matched.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip the escape and everything up to its final byte (a letter).
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Detects a bottom-anchored numbered menu (Claude Code permission/choice prompt)
/// in captured pane content. Returns the option labels for digits 1..N, or empty
/// if no consecutive 1., 2., … run is found in the last lines.
pub fn parse_prompt(content: &str) -> Vec<String> {
    let plain = strip_ansi(content);
    let lines: Vec<&str> = plain.lines().collect();
    let start = lines.len().saturating_sub(20);
    let mut opts: Vec<String> = Vec::new();
    let mut expect = 1u32;
    for line in &lines[start..] {
        // Drop leading selection markers/indentation (❯, >, ●, ·, spaces).
        let t = line
            .trim_start()
            .trim_start_matches(['❯', '>', '●', '·', ' '])
            .trim_start();
        if let Some(rest) = t.strip_prefix(&format!("{expect}.")) {
            let label = rest.trim();
            if !label.is_empty() {
                opts.push(label.chars().take(40).collect());
                expect += 1;
            }
        }
    }
    if opts.len() >= 2 {
        opts
    } else {
        Vec::new()
    }
}

/// Hash of pane content for in-process change detection between ticks.
/// Uses `DefaultHasher`; values are not stable across process restarts.
pub fn content_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// First observation (no previous snapshot) is `Idle`; changed → `Running`.
pub fn compute_status(prev: Option<u64>, current: u64) -> Status {
    match prev {
        Some(p) if p != current => Status::Running,
        _ => Status::Idle, // first observation OR content unchanged
    }
}

/// The project root for a session directory: the path with any trailing
/// `/.worktrees/<branch>...` segment stripped. Sessions sharing a root belong to
/// the same project. Returns a trimmed slice of `dir`.
pub fn project_root(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches('/');
    // Find a path component exactly equal to ".worktrees" and cut before it.
    if let Some(pos) = trimmed.find("/.worktrees/") {
        return &trimmed[..pos];
    }
    if let Some(stripped) = trimmed.strip_suffix("/.worktrees") {
        return stripped;
    }
    trimmed
}

/// The project root for a session: the worktree's repo root (from `@cm_repo`) if
/// this is a worktree session, otherwise its directory with any `.worktrees/…`
/// suffix stripped. Sessions sharing a root are one project.
pub fn session_root(s: &Session) -> &str {
    match s.worktree_repo.as_deref() {
        Some(r) => r.trim_end_matches('/'),
        None => project_root(&s.dir),
    }
}

/// True if the session runs in a worktree rather than the project root — i.e.
/// its directory sits below the project root (e.g. under `.worktrees/`).
pub fn is_worktree(s: &Session) -> bool {
    session_root(s) != s.dir.trim_end_matches('/')
}

/// Default display name for a project: the last path component of its root.
pub fn project_default_name(root: &str) -> &str {
    root.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(root)
}

/// Groups `sessions` into `(root, sessions)` buckets, contiguous and in the
/// order each root is first seen. Order within a bucket is the input order.
pub fn group_in_order(sessions: Vec<Session>) -> Vec<(String, Vec<Session>)> {
    let mut groups: Vec<(String, Vec<Session>)> = Vec::new();
    for s in sessions {
        let root = session_root(&s).to_string();
        if let Some(g) = groups.iter_mut().find(|(r, _)| *r == root) {
            g.1.push(s);
        } else {
            groups.push((root, vec![s]));
        }
    }
    groups
}

/// Project roots in first-seen order (the persisted project order after a move).
pub fn unique_roots(sessions: &[Session]) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for s in sessions {
        let root = session_root(s).to_string();
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// Orders sessions into grouped display order: projects ordered by
/// `project_order` (unknown roots appended in first-seen order), and within each
/// project sessions ordered by `order` (by name; unknown appended). Groups stay
/// contiguous. Stale entries in either list are ignored.
pub fn apply_grouped_order(
    project_order: &[String],
    order: &[String],
    sessions: Vec<Session>,
) -> Vec<Session> {
    let mut groups = group_in_order(sessions);
    let proj_rank = |root: &str| {
        project_order
            .iter()
            .position(|r| r == root)
            .unwrap_or(usize::MAX)
    };
    groups.sort_by_key(|(root, _)| proj_rank(root));
    let sess_rank = |name: &str| order.iter().position(|n| n == name).unwrap_or(usize::MAX);
    for (_, gs) in &mut groups {
        gs.sort_by_key(|s| sess_rank(&s.name));
    }
    groups.into_iter().flat_map(|(_, gs)| gs).collect()
}

/// Expands a leading `~` using `$HOME`. Leaves other paths untouched.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{home}{rest}");
            }
        }
    }
    path.to_string()
}

/// Collapses a leading `$HOME` to `~` for display: `/Users/me/work` → `~/work`.
/// Inverse of [`expand_tilde`]. Leaves paths outside `$HOME` untouched.
pub fn collapse_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if path == home {
                return "~".to_string();
            }
            if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
                return format!("~/{rest}");
            }
        }
    }
    path.to_string()
}

/// Resolves the first word of `cmd` on PATH via `command -v`. Returns the path,
/// or None if not found / empty. Display-only; never executes the command.
pub fn resolve_agent_path(cmd: &str) -> Option<String> {
    let bin = cmd.split_whitespace().next()?;
    // Pass `bin` as a positional arg ($0) so `command -v` receives it as data,
    // not as shell code — prevents injection via ';', '$(...)', backticks, etc.
    let out = std::process::Command::new("sh")
        .args(["-c", "command -v -- \"$0\"", bin])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(p)
    }
}

/// Human-friendly label for a known agent command (matched on its first word),
/// falling back to the command itself for anything unrecognized.
pub fn agent_display_name(cmd: &str) -> String {
    match cmd.split_whitespace().next().unwrap_or("") {
        "claude" => "Claude Code".to_string(),
        "codex" => "Codex".to_string(),
        "aider" => "Aider".to_string(),
        "gemini" => "Gemini".to_string(),
        other => other.to_string(),
    }
}

/// Collapses the middle of a path for compact display: `~/work/proj-c/auth` →
/// `~/…/auth`. Paths with two or fewer segments are returned unchanged.
pub fn abbreviate_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let segs: Vec<&str> = trimmed.split('/').collect();
    if segs.len() <= 2 {
        return trimmed.to_string();
    }
    format!("{}/\u{2026}/{}", segs[0], segs[segs.len() - 1])
}

/// Builds the create action from a completed form, or an error string if the
/// name/dir fail validation. The highlighted branch picker row decides the
/// worktree: the dir's own branch → plain session; another existing branch →
/// `Existing`; a `+ create` row → `New { base, branch }` (the branch name is
/// validated). Shared by every submit path so the assembly lives in one place.
pub fn build_create_action(form: &CreateForm, existing: &[String]) -> Result<Action, String> {
    validate_create(&form.name, &form.dir, existing)?;
    let worktree = if form.worktree {
        if let Some(entry) = form.branch_entries.get(form.branch_selected) {
            match entry {
                BranchEntry::Existing(branch)
                    if form.current_branch.as_deref() == Some(branch.as_str()) =>
                {
                    None
                }
                BranchEntry::Existing(branch) => Some(WorktreeSpec::Existing {
                    branch: branch.clone(),
                }),
                BranchEntry::Create(branch) => {
                    validate_branch(branch)?;
                    let base = match form.selected_base() {
                        Some(b) => b,
                        // Parity with the old picker: an empty repo submits an empty base.
                        None if form.base_branches.is_empty() => String::new(),
                        None => return Err(format!("no branch matches '{}'", form.base_filter)),
                    };
                    Some(WorktreeSpec::New {
                        base,
                        branch: branch.clone(),
                    })
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let (model, effort) = form.model_flags();
    Ok(Action::Create {
        name: form.name.trim().to_string(),
        dir: expand_tilde(&form.dir),
        agent: form.agent.clone(),
        worktree,
        terminal: form.terminal,
        model: model.map(str::to_string),
        effort: effort.map(str::to_string),
    })
}

/// The final command a create runs: the agent plus claude model/effort flags.
/// Shared by the submit path (main.rs) and the modal's command preview.
pub fn compose_agent_command(agent: &str, model: Option<&str>, effort: Option<&str>) -> String {
    let mut cmd = agent.to_string();
    if let Some(m) = model {
        cmd.push_str(&format!(" --model {m}"));
    }
    if let Some(e) = effort {
        cmd.push_str(&format!(" --effort {e}"));
    }
    cmd
}

/// Validates create-form input. `dir` is checked after tilde expansion.
pub fn validate_create(name: &str, dir: &str, existing: &[String]) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    if name.contains(':') || name.contains('.') {
        return Err("name cannot contain ':' or '.'".into());
    }
    if existing.iter().any(|n| n == name) {
        return Err(format!("session '{name}' already exists"));
    }
    let expanded = expand_tilde(dir);
    if !std::path::Path::new(&expanded).is_dir() {
        return Err(format!("directory not found: {expanded}"));
    }
    Ok(())
}

/// Validates a new worktree branch name before it is used to build a filesystem
/// path (`<repo>/.worktrees/<branch>`). Normal git names — including nested ones
/// like `feature/x` — are allowed, but values that could escape the worktrees
/// directory or be misread as a CLI flag are rejected.
pub fn validate_branch(branch: &str) -> Result<(), String> {
    let b = branch.trim();
    if b.is_empty() {
        return Err("branch name is empty".into());
    }
    if b.starts_with('-') {
        return Err("branch name cannot start with '-'".into());
    }
    if b.starts_with('/') {
        return Err("branch name cannot be an absolute path".into());
    }
    if b.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err("branch name cannot contain '.' or '..' path segments".into());
    }
    Ok(())
}

/// The task ordinals covered by the current selection (anchor..=cursor), or just
/// the cursor when nothing is selected.
fn selection_range(ns: &NoteState) -> std::ops::RangeInclusive<usize> {
    match ns.anchor {
        Some(a) => a.min(ns.cursor)..=a.max(ns.cursor),
        None => ns.cursor..=ns.cursor,
    }
}

/// Task ordinals currently selected (for render highlight). Empty when no
/// selection is active.
pub fn selection_set(ns: &NoteState) -> std::collections::HashSet<usize> {
    match ns.anchor {
        Some(_) => selection_range(ns).collect(),
        None => std::collections::HashSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn temp_git_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "amux_app_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("README.md"), "test\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-c",
                "user.email=a@b.c",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ])
            .current_dir(&dir)
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn reply_enter_sends_shift_enter_newlines() {
        let mut app = app_with_two_sessions();
        app.mode = Mode::Reply(ReplyForm::new("a".into()));
        // Type a couple chars.
        app.handle_key(key('h'));
        app.handle_key(key('i'));
        // Shift+Enter inserts a newline (stays in Reply mode, no action).
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(act.is_none());
        assert!(matches!(app.mode, Mode::Reply(_)));
        app.handle_key(key('x'));
        // Plain Enter sends the whole buffer.
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match act {
            Some(Action::SendText { name, text }) => {
                assert_eq!(name, "a");
                assert_eq!(text, "hi\nx");
            }
            other => panic!("expected SendText, got {other:?}"),
        }
    }

    #[test]
    fn esc_saves_reply_draft_and_i_restores_it() {
        let mut app = app_with_two_sessions();
        app.selected = 0; // session "a"
        app.handle_key(key('i'));
        app.handle_key(key('h'));
        app.handle_key(key('i'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List));
        assert_eq!(app.drafts.get("a").map(String::as_str), Some("hi"));
        assert!(app.dirty, "draft must persist via autosave");
        // Reopen: buffer restored with the cursor at the end (typing appends).
        app.handle_key(key('i'));
        app.handle_key(key('!'));
        match &app.mode {
            Mode::Reply(f) => assert_eq!(f.area.buffer, "hi!"),
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn drafts_are_per_session() {
        let mut app = app_with_two_sessions();
        app.drafts.insert("b".into(), "for b".into());
        app.selected = 0; // session "a"
        app.handle_key(key('i'));
        match &app.mode {
            Mode::Reply(f) => assert_eq!(f.area.buffer, "", "a has no draft"),
            other => panic!("expected Reply, got {other:?}"),
        }
    }

    #[test]
    fn sending_clears_the_draft() {
        let mut app = app_with_two_sessions();
        app.drafts.insert("a".into(), "hello".into());
        app.dirty = false;
        app.selected = 0;
        app.handle_key(key('i'));
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(act, Some(Action::SendText { .. })));
        assert!(!app.drafts.contains_key("a"));
        assert!(app.dirty);
    }

    #[test]
    fn esc_with_emptied_buffer_removes_the_draft() {
        let mut app = app_with_two_sessions();
        app.drafts.insert("a".into(), "x".into());
        app.selected = 0;
        app.handle_key(key('i')); // restores "x", cursor at end
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.drafts.contains_key("a"));
    }

    fn app_with_two_sessions() -> App {
        let mut app = App::new(Config::default());
        app.sessions = vec![
            Session {
                name: "a".into(),
                dir: "/a".into(),
                cwd: "/a".into(),
                created: 1,
                agent: "claude".into(),
                status: Status::Idle,
                attached: false,
                git: None,
                worktree_repo: None,
            },
            Session {
                name: "b".into(),
                dir: "/b".into(),
                cwd: "/b".into(),
                created: 2,
                agent: "claude".into(),
                status: Status::Idle,
                attached: false,
                git: None,
                worktree_repo: None,
            },
        ];
        app
    }

    fn named(name: &str) -> Session {
        Session {
            name: name.into(),
            dir: "/x".into(),
            cwd: "/x".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }
    }

    fn at(name: &str, dir: &str) -> Session {
        Session {
            dir: dir.into(),
            cwd: dir.into(),
            ..named(name)
        }
    }

    #[test]
    fn project_root_strips_worktrees_segment() {
        assert_eq!(project_root("/home/u/proj"), "/home/u/proj");
        assert_eq!(project_root("/home/u/proj/"), "/home/u/proj");
        assert_eq!(
            project_root("/home/u/proj/.worktrees/feat-x"),
            "/home/u/proj"
        );
        assert_eq!(project_default_name("/home/u/proj"), "proj");
    }

    #[test]
    fn session_root_prefers_worktree_repo() {
        let mut s = at("w", "/home/u/proj/.worktrees/feat");
        s.worktree_repo = Some("/home/u/proj".into());
        assert_eq!(session_root(&s), "/home/u/proj");
    }

    #[test]
    fn is_worktree_detects_subdir_sessions() {
        // Root session: dir == project root → not a worktree.
        assert!(!is_worktree(&at("main", "/home/u/proj")));
        // Path-based worktree (no @cm_repo set).
        assert!(is_worktree(&at("feat", "/home/u/proj/.worktrees/feat")));
        // Worktree flagged via @cm_repo.
        let mut s = at("feat", "/home/u/proj/.worktrees/feat");
        s.worktree_repo = Some("/home/u/proj".into());
        assert!(is_worktree(&s));
    }

    #[test]
    fn apply_grouped_order_keeps_groups_contiguous_and_ordered() {
        // Two projects (/p1, /p2) interleaved on input.
        let sessions = vec![
            at("a", "/p1"),
            at("x", "/p2"),
            at("b", "/p1"),
            at("y", "/p2"),
        ];
        // Want project /p2 first; within /p1 want "b" before "a".
        let project_order = vec!["/p2".to_string(), "/p1".to_string()];
        let order = vec!["b".to_string(), "a".to_string()];
        let out = apply_grouped_order(&project_order, &order, sessions);
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        // /p2 group (x,y in input order) then /p1 group (b,a per order).
        assert_eq!(names, vec!["x", "y", "b", "a"]);
    }

    #[test]
    fn empty_orders_preserve_grouped_input_order() {
        let sessions = vec![at("a", "/p1"), at("x", "/p2"), at("b", "/p1")];
        let out = apply_grouped_order(&[], &[], sessions);
        // Groups stay contiguous in first-seen order: /p1 (a,b) then /p2 (x).
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "x"]);
    }

    fn app_with(sessions: Vec<Session>) -> App {
        let mut app = App::new(Config::default());
        app.sessions = sessions;
        app
    }

    fn names_of(app: &App) -> Vec<String> {
        app.sessions.iter().map(|s| s.name.clone()).collect()
    }

    #[test]
    fn shift_j_moves_session_within_project() {
        let mut app = app_with(vec![at("a", "/p"), at("b", "/p"), at("c", "/p")]);
        app.selected = 0;
        app.handle_key(key('J'));
        assert_eq!(names_of(&app), vec!["b", "a", "c"]);
        assert_eq!(app.selected, 1); // follows "a"
    }

    #[test]
    fn shift_j_at_project_edge_swaps_whole_project() {
        // /p1 = [a, b], /p2 = [x]; move "b" (bottom of /p1) down → projects swap.
        let mut app = app_with(vec![at("a", "/p1"), at("b", "/p1"), at("x", "/p2")]);
        app.selected = 1; // "b"
        app.handle_key(key('J'));
        assert_eq!(names_of(&app), vec!["x", "a", "b"]);
        assert_eq!(
            app.project_order,
            vec!["/p2".to_string(), "/p1".to_string()]
        );
        assert!(app.dirty);
    }

    #[test]
    fn shift_r_renames_project_display_only() {
        let mut app = app_with(vec![at("s", "/home/u/p")]);
        app.selected = 0;
        app.handle_key(key('R'));
        assert!(matches!(app.mode, Mode::RenameProject(_)));
        // Buffer starts at the default name "p"; append to make "px".
        app.handle_key(key('x'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.project_names.get("/home/u/p").map(String::as_str),
            Some("px")
        );
        assert!(app.dirty);
        // Renaming back to the default clears the override.
        app.handle_key(key('R'));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.project_names.is_empty());
    }

    #[test]
    fn shift_tab_sends_to_selected_session() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        let act = app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
        assert!(matches!(act, Some(Action::SendShiftTab { name }) if name == "s"));
    }

    /// Types `s` into the app one char at a time (raw codes, layout preserved).
    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[test]
    fn u_key_opens_restart_confirmation_instead_of_acting() {
        let mut app = App::new(Config::default());
        let action = app.handle_key(key('u'));
        assert!(action.is_none(), "u must not restart directly: {action:?}");
        assert!(matches!(app.mode, Mode::ConfirmRestart(_)));
    }

    #[test]
    fn restart_confirmation_accepts_full_yes() {
        let mut app = App::new(Config::default());
        app.handle_key(key('u'));
        type_str(&mut app, "yes");
        let action = app.handle_key(enter());
        assert!(
            matches!(action, Some(Action::RestartAllClaude)),
            "expected RestartAllClaude, got {action:?}"
        );
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn restart_confirmation_accepts_russian_da() {
        let mut app = App::new(Config::default());
        app.handle_key(key('u'));
        type_str(&mut app, "да");
        let action = app.handle_key(enter());
        assert!(
            matches!(action, Some(Action::RestartAllClaude)),
            "expected RestartAllClaude, got {action:?}"
        );
    }

    #[test]
    fn restart_confirmation_is_case_insensitive() {
        for word in ["YES", "Yes", "ДА", "Да"] {
            let mut app = App::new(Config::default());
            app.handle_key(key('u'));
            type_str(&mut app, word);
            let action = app.handle_key(enter());
            assert!(
                matches!(action, Some(Action::RestartAllClaude)),
                "{word:?} must confirm, got {action:?}"
            );
        }
    }

    #[test]
    fn restart_confirmation_rejects_partial_or_wrong_text() {
        for text in ["", "y", "ye", "yess", "no", "д"] {
            let mut app = App::new(Config::default());
            app.handle_key(key('u'));
            type_str(&mut app, text);
            let action = app.handle_key(enter());
            assert!(action.is_none(), "{text:?} must not confirm: {action:?}");
            assert!(
                matches!(app.mode, Mode::ConfirmRestart(_)),
                "{text:?}: dialog must stay open"
            );
        }
    }

    #[test]
    fn restart_confirmation_supports_backspace_editing() {
        let mut app = App::new(Config::default());
        app.handle_key(key('u'));
        type_str(&mut app, "yex");
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        type_str(&mut app, "s");
        let action = app.handle_key(enter());
        assert!(matches!(action, Some(Action::RestartAllClaude)));
    }

    #[test]
    fn restart_confirmation_esc_cancels() {
        let mut app = App::new(Config::default());
        app.handle_key(key('u'));
        type_str(&mut app, "yes");
        let action = app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(action.is_none());
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn shift_j_k_reorder_selected_and_mark_dirty() {
        let mut app = app_with_two_sessions(); // [a, b], selected 0
                                               // Shift+J moves "a" down; selection follows.
        app.handle_key(key('J'));
        let names: Vec<&str> = app.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["b", "a"]);
        assert_eq!(app.selected, 1);
        assert_eq!(app.order, vec!["b".to_string(), "a".to_string()]);
        assert!(app.dirty);
        // Shift+K moves it back up.
        app.dirty = false;
        app.handle_key(key('K'));
        let names: Vec<&str> = app.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(app.selected, 0);
        assert!(app.dirty);
    }

    #[test]
    fn reorder_is_noop_at_edges_and_when_filtered() {
        let mut app = app_with_two_sessions();
        // At the top, Shift+K does nothing.
        app.handle_key(key('K'));
        assert_eq!(app.selected, 0);
        assert!(!app.dirty);
        // With a filter active, reordering is disabled.
        app.filter = Some(String::new());
        app.handle_key(key('J'));
        let names: Vec<&str> = app.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(!app.dirty);
    }

    #[test]
    fn s_then_digit_jumps_to_session_by_number() {
        let mut app = app_with_two_sessions();
        assert_eq!(app.selected, 0);
        // `s` enters select mode; a following digit jumps and exits to List.
        app.handle_key(key('s'));
        assert!(matches!(app.mode, Mode::SelectSession));
        app.handle_key(key('2'));
        assert_eq!(app.selected, 1);
        assert!(matches!(app.mode, Mode::List));
        // `s` then 1 → first session.
        app.handle_key(key('s'));
        app.handle_key(key('1'));
        assert_eq!(app.selected, 0);
        // Out-of-range digit is ignored and stays in select mode.
        app.handle_key(key('s'));
        app.handle_key(key('9'));
        assert_eq!(app.selected, 0);
        assert!(matches!(app.mode, Mode::SelectSession));
        // Esc cancels back to List.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn j_and_k_navigate_with_wrap() {
        let mut app = app_with_two_sessions();
        assert_eq!(app.selected, 0);
        app.handle_key(key('j'));
        assert_eq!(app.selected, 1);
        app.handle_key(key('j'));
        assert_eq!(app.selected, 0); // wraps
        app.handle_key(key('k'));
        assert_eq!(app.selected, 1); // wraps backward
    }

    #[test]
    fn q_sets_quit() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn enter_returns_attach_for_selected() {
        let mut app = app_with_two_sessions();
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, Some(Action::Attach("a".into())));
    }

    #[test]
    fn d_then_y_returns_kill() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('d'));
        assert!(matches!(app.mode, Mode::ConfirmDelete(_)));
        let action = app.handle_key(key('y'));
        assert_eq!(
            action,
            Some(Action::Kill {
                name: "a".into(),
                remove_worktree: false
            })
        );
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn kill_without_worktree_yields_plain_kill() {
        let mut app = app_with_two_sessions(); // sessions have worktree_repo: None
        app.handle_key(key('d'));
        let action = app.handle_key(key('y'));
        match action {
            Some(Action::Kill {
                name,
                remove_worktree,
            }) => {
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
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        let action = app.handle_key(key('y'));
        match action {
            Some(Action::Kill {
                name,
                remove_worktree,
            }) => {
                assert_eq!(name, "a");
                assert!(remove_worktree);
            }
            other => panic!("expected Kill, got {other:?}"),
        }
    }

    #[test]
    fn n_opens_create_form_prefilled_with_default_agent() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('n'));
        match &app.mode {
            Mode::Create(form) => assert_eq!(form.agent, "claude"),
            _ => panic!("expected create mode"),
        }
    }

    #[test]
    fn shift_n_opens_prefilled_form_for_project() {
        let mut app = app_with(vec![at("s", "/home/u/proj")]);
        app.selected = 0;
        app.handle_key(key('N'));
        match &app.mode {
            Mode::Create(form) => {
                assert!(form.prefilled);
                assert_eq!(form.dir, "/home/u/proj");
                assert_eq!(form.agent, "claude");
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

    #[test]
    fn status_is_idle_on_first_observation() {
        assert_eq!(compute_status(None, 42), Status::Idle);
    }

    #[test]
    fn status_is_running_when_content_changed() {
        assert_eq!(compute_status(Some(1), 2), Status::Running);
    }

    #[test]
    fn status_is_idle_when_content_unchanged() {
        assert_eq!(compute_status(Some(7), 7), Status::Idle);
    }

    #[test]
    fn expand_tilde_replaces_leading_home() {
        let Ok(home) = std::env::var("HOME") else {
            return; // no HOME in this environment; nothing to assert
        };
        assert_eq!(expand_tilde("~/proj"), format!("{home}/proj"));
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    }

    #[test]
    fn collapse_home_replaces_leading_home() {
        let Ok(home) = std::env::var("HOME") else {
            return; // no HOME in this environment; nothing to assert
        };
        assert_eq!(collapse_home(&format!("{home}/proj")), "~/proj");
        assert_eq!(collapse_home(&home), "~");
        assert_eq!(collapse_home("/abs/path"), "/abs/path");
        // a path that merely shares a prefix substring is left untouched
        assert_eq!(
            collapse_home(&format!("{home}x/proj")),
            format!("{home}x/proj")
        );
    }

    #[test]
    fn validate_rejects_empty_and_duplicate_and_bad_name() {
        let existing = vec!["taken".to_string()];
        assert!(validate_create("", "/tmp", &existing).is_err());
        assert!(validate_create("taken", "/tmp", &existing).is_err());
        assert!(validate_create("a.b", "/tmp", &existing).is_err());
    }

    #[test]
    fn validate_rejects_missing_dir_and_accepts_existing() {
        let existing: Vec<String> = vec![];
        assert!(validate_create("ok", "/no/such/dir/xyz", &existing).is_err());
        assert!(validate_create("ok", "/tmp", &existing).is_ok());
    }

    #[test]
    fn validate_branch_blocks_traversal_and_flags() {
        // Allowed: normal and nested git branch names.
        assert!(validate_branch("feature-x").is_ok());
        assert!(validate_branch("feature/x").is_ok());
        // Rejected: escapes, absolute paths, flag-like, empty.
        assert!(validate_branch("../escape").is_err());
        assert!(validate_branch("a/../../etc").is_err());
        assert!(validate_branch("/abs/path").is_err());
        assert!(validate_branch("-rf").is_err());
        assert!(validate_branch("   ").is_err());
    }

    #[test]
    fn dir_list_navigation_wraps() {
        let mut form = CreateForm::new("claude", &[]);
        form.dir_entries = vec!["a".into(), "b".into(), "c".into()];
        form.dir_selected = 0;
        form.dir_select_next();
        assert_eq!(form.dir_selected, 1);
        form.dir_select_next();
        form.dir_select_next();
        assert_eq!(form.dir_selected, 0); // wraps forward
        form.dir_select_prev();
        assert_eq!(form.dir_selected, 2); // wraps backward
    }

    #[test]
    fn entering_selected_dir_descends_and_reloads() {
        let base = std::env::temp_dir().join(format!("cm_pick_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub_a")).unwrap();
        std::fs::create_dir_all(base.join("sub_b")).unwrap();

        let mut form = CreateForm::new("claude", &[]);
        form.dir = format!("{}/", base.display());
        form.refresh_dir_entries();
        assert_eq!(
            form.dir_entries,
            vec!["sub_a".to_string(), "sub_b".to_string()]
        );

        form.dir_selected = 1; // highlight sub_b
        form.enter_selected_dir();
        assert_eq!(form.dir, format!("{}/sub_b/", base.display()));
        assert!(form.dir_entries.is_empty()); // sub_b has no children

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_form_starts_in_home_dir() {
        let form = CreateForm::new("claude", &[]);
        assert_eq!(form.dir, "~/");
    }

    fn cform(app: &App) -> &CreateForm {
        match &app.mode {
            Mode::Create(f) => f,
            other => panic!("expected Create mode, got {other:?}"),
        }
    }

    /// Form with `branches` seeded as if the dir were a git repo (no git calls).
    fn form_with_branches(branches: &[&str], current: Option<&str>) -> CreateForm {
        let mut f = CreateForm::new("claude", &[]);
        f.branches = branches.iter().map(|s| s.to_string()).collect();
        f.current_branch = current.map(str::to_string);
        f.refresh_branch_entries();
        f
    }

    #[test]
    fn branch_entries_filter_is_case_insensitive_substring() {
        let mut f = form_with_branches(&["main", "feature-x", "Feature-y", "fix"], Some("main"));
        f.branch_input = "FEAT".into();
        f.refresh_branch_entries();
        assert_eq!(
            f.branch_entries,
            vec![
                BranchEntry::Existing("feature-x".into()),
                BranchEntry::Existing("Feature-y".into()),
                BranchEntry::Create("FEAT".into()),
            ]
        );
    }

    #[test]
    fn branch_entries_empty_input_lists_all_without_create() {
        let f = form_with_branches(&["main", "dev"], Some("main"));
        assert_eq!(
            f.branch_entries,
            vec![
                BranchEntry::Existing("main".into()),
                BranchEntry::Existing("dev".into()),
            ]
        );
    }

    #[test]
    fn branch_entries_exact_match_suppresses_create() {
        let mut f = form_with_branches(&["main", "dev"], Some("main"));
        f.branch_input = "dev".into();
        f.refresh_branch_entries();
        assert_eq!(f.branch_entries, vec![BranchEntry::Existing("dev".into())]);
    }

    #[test]
    fn branch_entries_no_match_leaves_only_create() {
        let mut f = form_with_branches(&["main"], Some("main"));
        f.branch_input = "brand-new".into();
        f.refresh_branch_entries();
        assert_eq!(
            f.branch_entries,
            vec![BranchEntry::Create("brand-new".into())]
        );
    }

    #[test]
    fn sequence_skips_branch_without_repo() {
        let f = CreateForm::new("claude", &[]); // branches empty
        assert_eq!(
            f.field_sequence_for_test(),
            vec![
                CreateField::Name,
                CreateField::Dir,
                CreateField::Terminal,
                CreateField::Agent
            ]
        );
    }

    #[test]
    fn sequence_includes_branch_with_repo_and_base_only_for_create() {
        let mut f = form_with_branches(&["main", "dev"], Some("main"));
        // worktree=false (default): Worktree step present, Branch hidden.
        assert_eq!(
            f.field_sequence_for_test(),
            vec![
                CreateField::Name,
                CreateField::Dir,
                CreateField::Terminal,
                CreateField::Worktree,
                CreateField::Agent
            ]
        );
        // Enable worktree: Branch now appears.
        f.worktree = true;
        assert_eq!(
            f.field_sequence_for_test(),
            vec![
                CreateField::Name,
                CreateField::Dir,
                CreateField::Terminal,
                CreateField::Worktree,
                CreateField::Branch,
                CreateField::Agent
            ]
        );
        // Select the Create entry → Base appears after Branch.
        f.branch_input = "new-one".into();
        f.refresh_branch_entries();
        assert!(f.branch_is_new());
        assert_eq!(
            f.field_sequence_for_test(),
            vec![
                CreateField::Name,
                CreateField::Dir,
                CreateField::Terminal,
                CreateField::Worktree,
                CreateField::Branch,
                CreateField::Base,
                CreateField::Agent
            ]
        );
    }

    #[test]
    fn build_create_maps_current_branch_to_plain_session() {
        let mut f = form_with_branches(&["main", "dev"], Some("main"));
        f.name = "s1".into();
        f.dir = "/tmp".into();
        // entry 0 is "main" (current).
        assert_eq!(f.branch_selected, 0);
        match build_create_action(&f, &[]) {
            Ok(Action::Create { worktree, .. }) => assert_eq!(worktree, None),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn build_create_maps_other_existing_branch_to_existing_spec() {
        let mut f = form_with_branches(&["main", "dev"], Some("main"));
        f.name = "s1".into();
        f.dir = "/tmp".into();
        f.branch_selected = 1; // "dev"
        f.worktree = true;
        match build_create_action(&f, &[]) {
            Ok(Action::Create {
                worktree: Some(WorktreeSpec::Existing { branch }),
                ..
            }) => assert_eq!(branch, "dev"),
            other => panic!("expected Existing spec, got {other:?}"),
        }
    }

    #[test]
    fn build_create_maps_create_entry_to_new_spec_with_base() {
        let mut f = form_with_branches(&["main", "dev"], Some("main"));
        f.name = "s1".into();
        f.dir = "/tmp".into();
        f.branch_input = "feat-z".into();
        f.refresh_branch_entries();
        f.branch_selected = f.branch_entries.len() - 1; // the Create entry
        f.base_index = 1; // base = "dev"
        f.worktree = true;
        match build_create_action(&f, &[]) {
            Ok(Action::Create {
                worktree: Some(WorktreeSpec::New { base, branch }),
                ..
            }) => {
                assert_eq!(branch, "feat-z");
                assert_eq!(base, "dev");
            }
            other => panic!("expected New spec, got {other:?}"),
        }
    }

    #[test]
    fn build_create_rejects_unsafe_new_branch_name() {
        let mut f = form_with_branches(&["main"], Some("main"));
        f.name = "ok".into();
        f.dir = "/tmp".into();
        f.branch_input = "../escape".into();
        f.refresh_branch_entries();
        f.worktree = true;
        assert!(f.branch_is_new());
        assert!(build_create_action(&f, &[]).is_err());
    }

    #[test]
    fn branch_step_typing_filters_and_arrows_move() {
        let mut app = App::new(Config::default());
        app.sessions = Vec::new();
        let mut form = form_with_branches(&["main", "dev", "devops"], Some("main"));
        form.name = "s".into();
        form.dir = "/tmp".into();
        form.field = CreateField::Branch;
        app.mode = Mode::Create(form);
        // Type "dev" → entries narrow to dev, devops (no create: "dev" is exact).
        for c in ['d', 'e', 'v'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let f = cform(&app);
        assert_eq!(
            f.branch_entries,
            vec![
                BranchEntry::Existing("dev".into()),
                BranchEntry::Existing("devops".into()),
            ]
        );
        // Down moves the selection.
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(cform(&app).branch_selected, 1);
    }

    #[test]
    fn prefilled_form_with_repo_includes_branch_step() {
        // for_project on a real repo dir picks up its branches.
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("cm_prefill_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(d)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);

        let f = CreateForm::for_project(d, "claude", &[]);
        assert_eq!(f.branches.first().map(String::as_str), Some("main"));
        assert!(f.field_sequence_for_test().contains(&CreateField::Worktree));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retreat_mirrors_advance_and_wraps_to_last() {
        let mut form = CreateForm::new("claude", &["claude".into()]);
        let last = *form.field_sequence().last().unwrap();
        // advance then retreat returns to the start.
        form.advance();
        assert_ne!(form.field, CreateField::Name);
        form.retreat();
        assert_eq!(form.field, CreateField::Name);
        // retreat from the first field wraps to the final step.
        form.retreat();
        assert_eq!(form.field, last);
    }

    #[test]
    fn arrows_walk_fields_both_ways() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.field = CreateField::Terminal;
        app.mode = Mode::Create(form);
        // Up from Terminal retreats to Dir.
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(cform(&app).field, CreateField::Dir);
        // Down on Dir walks the picker (dir_entries is empty → no-op on selection),
        // so the field stays at Dir. Advance to Terminal via Enter instead.
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(cform(&app).field, CreateField::Terminal);
    }

    #[test]
    fn base_typing_filters_and_tab_cycles() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.field = CreateField::Base;
        form.base_branches = vec!["main".into(), "dev".into(), "feature".into()];
        app.mode = Mode::Create(form);
        app.handle_key(key('e')); // filter: dev, feature
        assert_eq!(cform(&app).base_filter, "e");
        assert_eq!(cform(&app).selected_base().as_deref(), Some("dev"));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(cform(&app).selected_base().as_deref(), Some("feature"));
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(cform(&app).selected_base().as_deref(), Some("dev"));
    }

    #[test]
    fn agent_h_l_cycles_preset_but_types_on_custom_slot() {
        let mut app = App::new(Config::default());
        // On a preset, l moves to the next choice.
        let mut form = CreateForm::new("claude", &["codex".into()]);
        form.field = CreateField::Agent;
        form.agent_index = 0;
        app.mode = Mode::Create(form);
        app.handle_key(key('l'));
        assert_eq!(cform(&app).agent_index, 1);

        // On the custom slot, h/l are literal text so a command can contain them.
        let mut custom = CreateForm::new("claude", &["codex".into()]);
        custom.field = CreateField::Agent;
        custom.agent_index = custom.agent_choices.len() - 1; // the custom slot
        custom.agent.clear();
        assert!(custom.agent_is_custom());
        app.mode = Mode::Create(custom);
        app.handle_key(key('h'));
        app.handle_key(key('l'));
        assert_eq!(cform(&app).agent, "hl");
    }

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
    fn prefilled_flow_skips_dir_but_keeps_agent() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        // Name → Terminal → Agent → wrap to Name (Dir skipped; no repo → no Branch).
        form.advance();
        assert_eq!(form.field, CreateField::Terminal);
        form.advance();
        assert_eq!(form.field, CreateField::Agent);
        form.advance();
        assert_eq!(form.field, CreateField::Name);
    }

    #[test]
    fn prefilled_flow_with_new_branch_walks_branch_then_base() {
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        // Seed branches as if the project dir were a repo; a typed new name
        // selects the + create row, which adds the Base step.
        form.branches = vec!["main".into()];
        form.current_branch = Some("main".into());
        form.branch_input = "feat".into();
        form.refresh_branch_entries();
        assert!(form.branch_is_new());
        // Enable worktree so Branch/Base steps appear.
        form.worktree = true;
        form.field = CreateField::Terminal;
        form.advance();
        assert_eq!(form.field, CreateField::Worktree);
        form.advance();
        assert_eq!(form.field, CreateField::Branch);
        form.advance();
        assert_eq!(form.field, CreateField::Base);
        form.advance();
        assert_eq!(form.field, CreateField::Agent);
    }

    #[test]
    fn default_flow_counts_after_refactor() {
        // Non-repo dir (no branches loaded) → Name, Dir, Terminal, Agent.
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.total_steps(), 4);
        assert_eq!(form.step(), 1);
        form.field = CreateField::Agent;
        assert_eq!(form.step(), 4);
        assert!(form.is_last_step());
    }

    #[test]
    fn terminal_flow_skips_agent_step() {
        // Non-repo dir → no Branch step: Name → Dir → Terminal → wrap to Name.
        let mut form = CreateForm::new("claude", &[]);
        form.terminal = true;
        assert_eq!(form.field, CreateField::Name);
        form.advance();
        assert_eq!(form.field, CreateField::Dir);
        form.advance();
        assert_eq!(form.field, CreateField::Terminal);
        form.advance();
        assert_eq!(form.field, CreateField::Name);
    }

    #[test]
    fn terminal_step_counts_and_last_step() {
        // Non-repo dir → Name, Dir, Terminal (Terminal is the last step).
        let mut form = CreateForm::new("claude", &[]);
        form.terminal = true;
        assert_eq!(form.total_steps(), 3);
        form.field = CreateField::Terminal;
        assert!(form.is_last_step());
        form.field = CreateField::Dir;
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

    #[test]
    fn worktree_field_gates_branch_step() {
        let mut form = CreateForm::new("claude", &[]);
        form.branches = vec!["main".to_string(), "dev".to_string()];
        form.current_branch = Some("main".to_string());
        form.refresh_branch_entries();

        // With worktree=false (default), Worktree step is present but Branch is not.
        let seq = form.field_sequence_for_test();
        assert!(seq.contains(&CreateField::Worktree), "Worktree step must be present when branches non-empty");
        assert!(!seq.contains(&CreateField::Branch), "Branch step must be hidden when worktree=false");

        form.worktree = true;
        let seq = form.field_sequence_for_test();
        assert!(seq.contains(&CreateField::Branch), "Branch step must appear when worktree=true");
    }

    #[test]
    fn terminal_toggle_via_space_in_create() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sh".into();
        form.dir = dir;
        form.field = CreateField::Terminal;
        app.mode = Mode::Create(form);
        app.handle_key(key(' '));
        match &app.mode {
            Mode::Create(f) => assert!(f.terminal),
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn prefilled_submit_without_worktree_creates_in_project() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::for_project(&dir, "claude", &[]);
        form.name = "sess".into();
        form.field = CreateField::Terminal; // Shift+Enter submits from anywhere
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        match act {
            Some(Action::Create {
                name,
                dir: d,
                agent,
                worktree,
                ..
            }) => {
                assert_eq!(name, "sess");
                assert_eq!(d, dir);
                assert_eq!(agent, "claude");
                assert_eq!(worktree, None);
            }
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }

    #[test]
    fn enter_advances_from_name_field() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "quick".into();
        form.dir = dir;
        assert_eq!(form.field, CreateField::Name);
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(act.is_none());
        assert_eq!(cform(&app).field, CreateField::Dir);
    }

    #[test]
    fn shift_enter_with_invalid_form_shows_error_and_keeps_form() {
        let mut app = App::new(Config::default());
        let form = CreateForm::new("claude", &[]); // empty name → invalid
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(act.is_none());
        assert!(app.error.is_some());
        assert!(matches!(app.mode, Mode::Create(_)));
    }

    #[test]
    fn j_dives_into_model_list_and_h_l_set_effort() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        app.handle_key(key('j')); // agent row → opus
        assert_eq!(cform(&app).model_index, Some(0));
        app.handle_key(key('j')); // opus → sonnet
        assert_eq!(cform(&app).model_index, Some(1));
        app.handle_key(key('l')); // auto → low
        app.handle_key(key('l')); // low → medium
        assert_eq!(cform(&app).effort_index, 2);
        app.handle_key(key('k')); // back to opus — effort resets
        assert_eq!(cform(&app).model_index, Some(0));
        assert_eq!(cform(&app).effort_index, 0);
        app.handle_key(key('k')); // opus → agent row (auto)
        assert_eq!(cform(&app).model_index, None);
    }

    #[test]
    fn down_past_haiku_leaves_list_but_keeps_selection() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Agent;
        form.model_index = Some(2); // haiku
        app.mode = Mode::Create(form);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(cform(&app).field, CreateField::Name); // wrapped
        assert_eq!(cform(&app).model_index, Some(2)); // selection survives
    }

    #[test]
    fn enter_on_model_row_submits_with_flags() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sess".into();
        form.dir = dir;
        form.field = CreateField::Agent;
        form.model_index = Some(1); // sonnet
        form.effort_index = 3; // high
        app.mode = Mode::Create(form);
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)) {
            Some(Action::Create { model, effort, .. }) => {
                assert_eq!(model.as_deref(), Some("sonnet"));
                assert_eq!(effort.as_deref(), Some("high"));
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn arrows_toggle_terminal() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Terminal;
        app.mode = Mode::Create(form);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(cform(&app).terminal);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!cform(&app).terminal);
    }

    #[test]
    fn j_k_type_into_text_fields() {
        let mut app = App::new(Config::default());
        let form = CreateForm::new("claude", &[]); // field = Name
        app.mode = Mode::Create(form);
        app.handle_key(key('j'));
        app.handle_key(key('k'));
        assert_eq!(cform(&app).name, "jk");
    }

    #[test]
    fn prefilled_submit_with_new_branch_carries_spec() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::for_project(&dir, "claude", &[]);
        form.name = "sess".into();
        // Seed branches as if the dir were a git repo, then pick the + create row.
        form.branches = vec!["main".into()];
        form.current_branch = Some("main".into());
        form.branch_input = "feat".into();
        form.refresh_branch_entries();
        assert!(form.branch_is_new());
        form.worktree = true;
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        match act {
            Some(Action::Create {
                worktree: Some(WorktreeSpec::New { branch, base }),
                ..
            }) => {
                assert_eq!(branch, "feat");
                assert_eq!(base, "main");
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
        let act = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        match act {
            Some(Action::Create { name, dir: d, .. }) => {
                assert_eq!(name, "sess");
                assert_eq!(d, dir);
            }
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }

    #[test]
    fn prefilled_step_indicator_counts() {
        // Non-repo dir → no Branch step: name, terminal, agent.
        let mut form = CreateForm::for_project("/home/u/proj", "claude", &[]);
        assert_eq!(form.total_steps(), 3);
        assert_eq!(form.step(), 1);
        form.field = CreateField::Terminal;
        assert_eq!(form.step(), 2);
        // A repo adds the Worktree step; Branch/Base hidden until opted-in.
        form.branches = vec!["main".into()];
        form.current_branch = Some("main".into());
        form.refresh_branch_entries();
        assert_eq!(form.total_steps(), 4); // + worktree
        // Enable worktree and type a new branch name → Branch + Base appear.
        form.worktree = true;
        form.branch_input = "feat".into();
        form.refresh_branch_entries();
        assert_eq!(form.total_steps(), 6); // + branch + base for the new branch
        form.field = CreateField::Base;
        assert_eq!(form.step(), 5);
    }

    #[test]
    fn filter_limits_visible_sessions() {
        let mut app = app_with_two_sessions();
        app.filter = Some("b".into());
        let vis = app.visible_indices();
        assert_eq!(vis, vec![1]);
        assert_eq!(app.selected_name().as_deref(), Some("b"));
    }

    #[test]
    fn slash_enters_filter_mode_and_typing_filters() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('/'));
        assert!(matches!(app.mode, Mode::Filter));
        app.handle_key(key('b'));
        assert_eq!(app.filter.as_deref(), Some("b"));
        assert_eq!(app.visible_indices(), vec![1]);
    }

    #[test]
    fn esc_clears_filter() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('/'));
        app.handle_key(key('b'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.filter.is_none());
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn g_jumps_to_first_session() {
        let mut app = app_with_two_sessions();
        app.select_next();
        assert_eq!(app.selected, 1);
        app.handle_key(key('g'));
        assert_eq!(app.selected, 0);
        // `G` no longer moves the selection — it scrolls the preview to latest.
        app.select_next();
        app.handle_key(key('G'));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn agent_cycle_wraps_and_sets_command() {
        let mut form = CreateForm::new("claude", &["claude".into(), "codex".into()]);
        assert_eq!(form.agent, "claude");
        form.cycle_agent(1);
        assert_eq!(form.agent_choices[form.agent_index], "codex");
        assert_eq!(form.agent, "codex");
        // step to custom slot → agent cleared for free typing
        form.cycle_agent(1);
        assert!(form.agent_is_custom());
        assert_eq!(form.agent, "");
        // wrap back to first
        form.cycle_agent(1);
        assert_eq!(form.agent, "claude");
        // negative delta wraps the other direction
        form.cycle_agent(-1);
        assert!(form.agent_is_custom());
        assert_eq!(form.agent, "");
    }

    #[test]
    fn latinize_maps_russian_layout() {
        // Physical j/k/o/i/n keys produce these Cyrillic chars on ЙЦУКЕН.
        assert_eq!(latinize('о'), 'j');
        assert_eq!(latinize('л'), 'k');
        assert_eq!(latinize('щ'), 'o');
        assert_eq!(latinize('ш'), 'i');
        assert_eq!(latinize('т'), 'n');
        assert_eq!(latinize('П'), 'G'); // Shift preserved
        assert_eq!(latinize('a'), 'a'); // Latin passes through
        assert_eq!(latinize('1'), '1');
    }

    #[test]
    fn parse_prompt_detects_numbered_menu() {
        let pane = "Do you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again\n  3. No, tell Claude what to do\n";
        let opts = parse_prompt(pane);
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0], "Yes");
        assert!(opts[2].starts_with("No"));
    }

    #[test]
    fn parse_prompt_strips_ansi() {
        let pane = "\u{1b}[1m❯ 1.\u{1b}[0m Yes\n  2. No\n";
        let opts = parse_prompt(pane);
        assert_eq!(opts, vec!["Yes".to_string(), "No".to_string()]);
    }

    #[test]
    fn parse_prompt_empty_without_menu() {
        assert!(parse_prompt("just some normal output\nno menu here\n").is_empty());
        assert!(parse_prompt("1. only one option\n").is_empty());
    }

    #[test]
    fn step_tracks_focused_field() {
        // Non-repo dir → Name, Dir, Terminal, Agent (Agent is step 4).
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.step(), 1);
        form.field = CreateField::Dir;
        assert_eq!(form.step(), 2);
        form.field = CreateField::Agent;
        assert_eq!(form.step(), 4);
    }

    #[test]
    fn agent_display_name_maps_known_and_falls_back() {
        assert_eq!(agent_display_name("claude"), "Claude Code");
        assert_eq!(agent_display_name("codex --yolo"), "Codex");
        assert_eq!(agent_display_name("aider"), "Aider");
        assert_eq!(agent_display_name("gemini"), "Gemini");
        assert_eq!(agent_display_name("my-tool --flag"), "my-tool");
        assert_eq!(agent_display_name(""), "");
    }

    #[test]
    fn reply_insert_and_edit_is_char_aware() {
        let mut f = ReplyForm::new("s".into());
        // Cyrillic: each char is multi-byte; cursor must track chars, not bytes.
        for c in "привет".chars() {
            f.insert_char(c);
        }
        assert_eq!(f.area.buffer, "привет");
        assert_eq!(f.area.cursor, 6);
        // Move into the middle and insert (cursor lands before "е").
        f.left();
        f.left();
        f.insert_char('Х');
        assert_eq!(f.area.buffer, "привХет");
        // Backspace removes the char we just inserted.
        f.backspace();
        assert_eq!(f.area.buffer, "привет");
        // Delete (forward) removes "е".
        f.delete();
        assert_eq!(f.area.buffer, "привт");
    }

    #[test]
    fn reply_delete_word_and_line_start() {
        let mut f = ReplyForm::new("s".into());
        f.insert_str("hello world foo");
        f.delete_word();
        assert_eq!(f.area.buffer, "hello world ");
        f.delete_to_line_start();
        assert_eq!(f.area.buffer, "");
        assert_eq!(f.area.cursor, 0);
    }

    #[test]
    fn reply_up_down_preserve_column_within_lines() {
        let mut f = ReplyForm::new("s".into());
        f.insert_str("abcd\nef\nghij");
        // cursor at end (line "ghij", col 4)
        assert_eq!(f.area.cursor, 12);
        f.up(); // onto "ef" (len 2) → column clamps to 2
        let (start, _) = f.line_bounds();
        assert_eq!(f.area.cursor - start, 2);
        f.up(); // onto "abcd", same column 2
        let (start, _) = f.line_bounds();
        assert_eq!(f.area.cursor - start, 2);
        f.home();
        assert_eq!(f.area.cursor, 0);
        f.end();
        assert_eq!(f.area.cursor, 4); // end of first logical line, before '\n'
    }

    #[test]
    fn abbreviate_path_collapses_middle() {
        assert_eq!(
            abbreviate_path("~/work/proj-c/auth-rewrite"),
            "~/\u{2026}/auth-rewrite"
        );
        assert_eq!(
            abbreviate_path("~/work/proj-c/auth-rewrite/"),
            "~/\u{2026}/auth-rewrite"
        );
        assert_eq!(abbreviate_path("~/work"), "~/work");
        assert_eq!(abbreviate_path("~/"), "~");
        assert_eq!(abbreviate_path("/a/b/c"), "/\u{2026}/c");
    }

    #[test]
    fn tab_cycles_base_branch() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.branches = vec!["main".into(), "dev".into()];
        form.field = CreateField::Base;
        app.mode = Mode::Create(form);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        match &app.mode {
            Mode::Create(f) => assert_eq!(f.base_index, 1),
            _ => panic!(),
        }
    }

    #[test]
    fn non_repo_flow_skips_branch_steps() {
        let mut form = CreateForm::new("claude", &[]);
        assert!(!form.dir_is_repo());
        form.field = CreateField::Terminal;
        form.advance(); // no repo → no Branch/Base, straight to Agent
        assert_eq!(form.field, CreateField::Agent);
    }

    #[test]
    fn repo_flow_visits_branch_and_base() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let repo = temp_git_repo("visits");
        let mut form = CreateForm::new("claude", &[]);
        form.dir = repo.to_str().unwrap().to_string();
        form.load_branches();
        assert!(form.dir_is_repo());
        form.refresh_branch_entries();
        // Enable worktree so Branch/Base steps appear.
        form.worktree = true;
        form.field = CreateField::Terminal;
        form.advance();
        assert_eq!(form.field, CreateField::Worktree);
        form.advance();
        assert_eq!(form.field, CreateField::Branch);
        // An existing branch is highlighted → no Base step.
        form.advance();
        assert_eq!(form.field, CreateField::Agent);
        // Typing a new name selects the + create row → Base appears.
        form.branch_input = "brand-new".into();
        form.refresh_branch_entries();
        assert!(form.branch_is_new());
        form.field = CreateField::Branch;
        form.advance();
        assert_eq!(form.field, CreateField::Base);
        form.advance();
        assert_eq!(form.field, CreateField::Agent);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn step_count_grows_with_branch_picker() {
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.total_steps(), 4); // name, dir, terminal, agent
        form.branches = vec!["main".into()];
        form.refresh_branch_entries();
        assert_eq!(form.total_steps(), 5); // + worktree (branch hidden until opted-in)
        form.worktree = true;
        assert_eq!(form.total_steps(), 6); // + branch (worktree opted-in)
        form.branch_input = "feat".into();
        form.refresh_branch_entries();
        assert_eq!(form.total_steps(), 7); // + base for the + create row
    }

    #[test]
    fn dir_tab_cycles_candidates_and_right_descends() {
        let base = std::env::temp_dir().join(format!("cm_tab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub_a")).unwrap();
        std::fs::create_dir_all(base.join("sub_b")).unwrap();

        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Dir;
        form.dir = format!("{}/", base.display());
        form.refresh_dir_entries();
        app.mode = Mode::Create(form);

        // ↓ / ↑ now navigate the picker list.
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(cform(&app).dir_selected, 1);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)); // wraps
        assert_eq!(cform(&app).dir_selected, 0);
        // Tab and Right both descend into the selected entry.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(cform(&app).dir, format!("{}/sub_a/", base.display()));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_action_carries_worktree_spec() {
        // The + create row (selected on the Branch step) → New spec; Shift+Enter
        // submits the completed form.
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.name = "iso".into();
        form.dir = "/tmp".into(); // exists as a dir
        form.branches = vec!["main".into()];
        form.current_branch = Some("main".into());
        form.base_index = 0;
        form.branch_input = "iso-branch".into();
        form.refresh_branch_entries(); // sole Create row, selected
        form.worktree = true;
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        match action {
            Some(Action::Create {
                worktree: Some(WorktreeSpec::New { branch, base }),
                ..
            }) => {
                assert_eq!(branch, "iso-branch");
                assert_eq!(base, "main");
            }
            other => panic!("expected Create with worktree, got {other:?}"),
        }
    }

    #[test]
    fn terminal_submit_carries_terminal_flag() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let mut app = app_with(vec![at("s", "/p")]);
        let mut form = CreateForm::new("claude", &[]);
        form.name = "sh".into();
        form.dir = dir;
        form.terminal = true;
        // Non-repo dir → Terminal is the last step when terminal is on.
        form.field = CreateField::Terminal;
        app.mode = Mode::Create(form);
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)) {
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
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)) {
            Some(Action::Create { terminal, .. }) => assert!(!terminal),
            other => panic!("expected Action::Create, got {other:?}"),
        }
    }

    #[test]
    fn t_toggles_session_note_pane() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        app.handle_key(key('t'));
        assert_eq!(app.right_pane, RightPane::SessionNote);
        app.handle_key(key('t'));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn shift_t_toggles_project_note_pane() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        app.handle_key(key('T'));
        assert_eq!(app.right_pane, RightPane::ProjectNote);
        app.handle_key(key('T'));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn shift_t_is_noop_without_sessions() {
        let mut app = App::new(Config::default());
        app.handle_key(key('T'));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn tab_focuses_the_shown_project_note() {
        // Root session and a worktree session of the same project focus the
        // SAME note (keyed by the project root, not the session).
        let mut app = app_with(vec![at("a", "/p"), at("b", "/p/.worktrees/x")]);
        app.selected = 0;
        app.handle_key(key('T'));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.target, NoteTarget::Project("/p".into())),
            other => panic!("expected Mode::Note, got {other:?}"),
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)); // exit note
        app.selected = 1;
        app.handle_key(key('T'));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.target, NoteTarget::Project("/p".into())),
            other => panic!("expected Mode::Note, got {other:?}"),
        }
    }

    #[test]
    fn tab_focuses_the_shown_session_note() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.selected = 0;
        app.handle_key(key('t')); // show session note
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.target, NoteTarget::Session("s".into())),
            other => panic!("expected Mode::Note, got {other:?}"),
        }
    }

    #[test]
    fn killing_a_session_drops_its_note() {
        let mut app = App::new(Config::default());
        app.notes.insert("s".into(), "- [ ] x".into());
        app.notes.remove("s"); // mirrors the Kill handler
        assert!(!app.notes.contains_key("s"));
    }

    #[test]
    fn renaming_moves_the_note() {
        let mut app = App::new(Config::default());
        app.notes.insert("old".into(), "- [ ] x".into());
        if let Some(t) = app.notes.remove("old") {
            app.notes.insert("new".into(), t);
        }
        assert_eq!(app.notes.get("new").map(String::as_str), Some("- [ ] x"));
        assert!(!app.notes.contains_key("old"));
    }

    #[test]
    fn killing_a_session_drops_its_draft() {
        let mut app = App::new(Config::default());
        app.drafts.insert("s".into(), "draft".into());
        app.drafts.remove("s"); // mirrors the Kill handler
        assert!(!app.drafts.contains_key("s"));
    }

    #[test]
    fn renaming_moves_the_draft() {
        let mut app = App::new(Config::default());
        app.drafts.insert("old".into(), "draft".into());
        // Mirrors the Rename handler.
        if let Some(d) = app.drafts.remove("old") {
            app.drafts.insert("new".into(), d);
        }
        assert_eq!(app.drafts.get("new").map(String::as_str), Some("draft"));
        assert!(!app.drafts.contains_key("old"));
    }

    #[test]
    fn prune_dead_drafts_drops_only_dead_sessions() {
        let mut app = app_with_two_sessions(); // sessions "a" and "b"
        app.drafts.insert("a".into(), "keep".into());
        app.drafts.insert("dead".into(), "drop".into());
        app.prune_dead_drafts();
        assert_eq!(app.drafts.get("a").map(String::as_str), Some("keep"));
        assert!(!app.drafts.contains_key("dead"));
        assert!(app.dirty, "removal must be persisted");
    }

    #[test]
    fn prune_dead_drafts_without_removals_keeps_state_clean() {
        let mut app = app_with_two_sessions();
        app.drafts.insert("a".into(), "keep".into());
        app.prune_dead_drafts();
        assert!(!app.dirty, "no removal → nothing to save");
    }

    fn note_app_with(text: &str) -> App {
        let mut app = App::new(Config::default());
        app.project_notes.insert("/p".into(), text.into());
        app.mode = Mode::Note(NoteState {
            target: NoteTarget::Project("/p".into()),
            sub: NoteSub::Render,
            cursor: 0,
            anchor: None,
            editor: crate::editor::TextArea::default(),
            confirm_clear: false,
        });
        app
    }

    /// The "/p" project note's current text (the target `note_app_with` edits).
    fn proj_note(app: &App) -> &str {
        app.project_notes
            .get("/p")
            .map(String::as_str)
            .unwrap_or("")
    }

    fn note_state(app: &App) -> &NoteState {
        match &app.mode {
            Mode::Note(ns) => ns,
            _ => panic!("not in note mode"),
        }
    }

    #[test]
    fn j_k_move_task_cursor_within_bounds() {
        let mut app = note_app_with("- [ ] a\n- [ ] b\n- [ ] c");
        app.handle_key(key('j'));
        assert_eq!(note_state(&app).cursor, 1);
        app.handle_key(key('j'));
        app.handle_key(key('j')); // clamp at last task (index 2)
        assert_eq!(note_state(&app).cursor, 2);
        app.handle_key(key('k'));
        assert_eq!(note_state(&app).cursor, 1);
    }

    #[test]
    fn space_toggles_task_under_cursor() {
        let mut app = note_app_with("- [ ] a\n- [ ] b");
        app.handle_key(key('j')); // cursor on task 1
        app.handle_key(key(' '));
        assert_eq!(proj_note(&app), "- [ ] a\n- [x] b");
    }

    #[test]
    fn visual_select_then_space_toggles_range() {
        let mut app = note_app_with("- [ ] a\n- [ ] b\n- [ ] c");
        app.handle_key(key('V')); // anchor at 0
        app.handle_key(key('j')); // extend to 1
        app.handle_key(key(' ')); // toggle 0..=1
        assert_eq!(proj_note(&app), "- [x] a\n- [x] b\n- [ ] c");
        assert!(
            note_state(&app).anchor.is_none(),
            "selection cleared after toggle"
        );
    }

    #[test]
    fn e_enters_edit_seeded_from_note() {
        let mut app = note_app_with("- [ ] a");
        app.handle_key(key('e'));
        let ns = note_state(&app);
        assert_eq!(ns.sub, NoteSub::Edit);
        assert_eq!(ns.editor.buffer, "- [ ] a");
    }

    #[test]
    fn esc_from_render_exits_to_preview() {
        let mut app = note_app_with("- [ ] a");
        app.right_pane = RightPane::ProjectNote;
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List), "focus dropped");
        assert_eq!(app.right_pane, RightPane::Preview, "pane closed to preview");
    }

    #[test]
    fn tab_defocuses_but_keeps_note_pane() {
        let mut app = note_app_with("- [ ] a");
        app.right_pane = RightPane::ProjectNote;
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List), "focus dropped");
        assert_eq!(
            app.right_pane,
            RightPane::ProjectNote,
            "note still shown after defocus"
        );
    }

    #[test]
    fn esc_in_browse_closes_pane_to_preview() {
        let mut app = app_with(vec![at("s", "/p")]);
        app.right_pane = RightPane::SessionNote; // unfocused browse
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.right_pane, RightPane::Preview);
    }

    #[test]
    fn render_chords_work_on_cyrillic_layout() {
        // On a Russian layout the physical keys emit Cyrillic: e→у, j→о, V→М.
        let mut app = note_app_with("- [ ] a\n- [ ] b");
        app.handle_key(key('о')); // physical 'j' → move cursor down
        assert_eq!(note_state(&app).cursor, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('М'), KeyModifiers::SHIFT)); // 'V' select
        assert_eq!(note_state(&app).anchor, Some(1));
        app.handle_key(key('у')); // physical 'e' → enter edit
        assert_eq!(note_state(&app).sub, NoteSub::Edit);
    }

    #[test]
    fn edit_mode_types_cyrillic_literally() {
        let mut app = note_app_with("");
        app.handle_key(key('у')); // 'e' → edit
        assert_eq!(note_state(&app).sub, NoteSub::Edit);
        app.handle_key(key('п')); // a Cyrillic letter must be inserted as-is
        app.handle_key(key('р'));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.editor.buffer, "пр"),
            _ => panic!("expected edit mode"),
        }
    }

    #[test]
    fn project_note_persists_to_snapshot() {
        // Editing a project note marks state dirty and snapshot_state carries it,
        // so the autosave loop writes it to state.toml permanently.
        let mut app = App::new(Config::default());
        app.mode = Mode::Note(NoteState {
            target: NoteTarget::Project("/p".into()),
            sub: NoteSub::Edit,
            cursor: 0,
            anchor: None,
            editor: crate::editor::TextArea::new("- [ ] buy milk".to_string()),
            confirm_clear: false,
        });
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)); // commit
        assert_eq!(proj_note(&app), "- [ ] buy milk");
        assert!(app.dirty, "edit must mark state dirty for autosave");
        assert_eq!(
            app.snapshot_state()
                .project_notes
                .get("/p")
                .map(String::as_str),
            Some("- [ ] buy milk")
        );
    }

    #[test]
    fn c_then_y_clears_note_with_confirmation() {
        let mut app = note_app_with("- [ ] a\n- [ ] b");
        app.handle_key(key('c'));
        assert!(note_state(&app).confirm_clear, "c arms the confirmation");
        assert_eq!(
            proj_note(&app),
            "- [ ] a\n- [ ] b",
            "not cleared until confirmed"
        );
        app.handle_key(key('y'));
        assert_eq!(proj_note(&app), "");
        assert!(!note_state(&app).confirm_clear);
        assert!(app.dirty);
    }

    #[test]
    fn clear_confirmation_cancels_on_other_key() {
        let mut app = note_app_with("- [ ] keep");
        app.handle_key(key('c'));
        app.handle_key(key('n')); // anything but y cancels
        assert!(!note_state(&app).confirm_clear);
        assert_eq!(proj_note(&app), "- [ ] keep", "note untouched on cancel");
    }

    fn note_app_editing(text: &str) -> App {
        let mut app = App::new(Config::default());
        app.project_notes.insert("/p".into(), text.into());
        app.mode = Mode::Note(NoteState {
            target: NoteTarget::Project("/p".into()),
            sub: NoteSub::Edit,
            cursor: 0,
            anchor: None,
            editor: crate::editor::TextArea::new(text.to_string()),
            confirm_clear: false,
        });
        app
    }

    #[test]
    fn typing_in_edit_writes_back_to_note() {
        let mut app = note_app_editing("- [ ] a");
        app.handle_key(key('!')); // appended at end (cursor at end)
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)); // esc commits
        assert_eq!(proj_note(&app), "- [ ] a!");
        assert_eq!(note_state(&app).sub, NoteSub::Render);
    }

    #[test]
    fn enter_inserts_newline_not_submit() {
        let mut app = note_app_editing("a");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(key('b'));
        match &app.mode {
            Mode::Note(ns) => assert_eq!(ns.editor.buffer, "a\nb"),
            _ => panic!("still editing"),
        }
    }

    #[test]
    fn model_list_visible_only_for_claude_agent() {
        let mut form = CreateForm::new("claude", &["codex".into()]);
        assert!(form.model_list_visible());
        form.cycle_agent(1); // codex
        assert!(!form.model_list_visible());
        // A custom command whose binary is claude still gets the list.
        form.agent = "claude --dangerously-skip-permissions".into();
        assert!(form.model_list_visible());
        form.terminal = true;
        assert!(!form.model_list_visible());
    }

    #[test]
    fn model_down_enters_walks_and_exits_list() {
        let mut form = CreateForm::new("claude", &[]);
        assert!(form.model_down()); // agent row → opus
        assert_eq!(form.model_index, Some(0));
        assert!(form.model_down()); // opus → sonnet
        assert!(form.model_down()); // sonnet → haiku
        assert_eq!(form.model_index, Some(2));
        // Past the last model: false → caller advances; selection survives.
        assert!(!form.model_down());
        assert_eq!(form.model_index, Some(2));
    }

    #[test]
    fn model_down_is_noop_for_non_claude() {
        let mut form = CreateForm::new("codex", &[]);
        assert!(!form.model_down());
        assert_eq!(form.model_index, None);
    }

    #[test]
    fn model_up_returns_to_agent_row_and_resets() {
        let mut form = CreateForm::new("claude", &[]);
        form.model_index = Some(1);
        form.effort_index = 2;
        assert!(form.model_up()); // sonnet → opus, effort resets
        assert_eq!(form.model_index, Some(0));
        assert_eq!(form.effort_index, 0);
        assert!(form.model_up()); // opus → agent row (auto)
        assert_eq!(form.model_index, None);
        assert!(!form.model_up()); // already on the agent row
    }

    #[test]
    fn effort_slider_clamps_per_model() {
        let mut form = CreateForm::new("claude", &[]);
        form.model_index = Some(0); // opus: auto low medium high xhigh max
        form.cycle_effort(-1);
        assert_eq!(form.effort_index, 0); // clamped at auto
        for _ in 0..10 {
            form.cycle_effort(1);
        }
        assert_eq!(form.effort_index, 5); // clamped at max
        form.model_index = Some(1); // sonnet: no xhigh → top index 4
        form.effort_index = 0;
        for _ in 0..10 {
            form.cycle_effort(1);
        }
        assert_eq!(form.effort_index, 4);
        assert_eq!(form.selected_effort(), Some("max"));
        form.model_index = Some(2); // haiku: only auto, slider is a no-op
        form.effort_index = 0;
        form.cycle_effort(1);
        assert_eq!(form.effort_index, 0);
    }

    #[test]
    fn selected_model_and_effort_map_to_flags() {
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.model_flags(), (None, None)); // agent row = auto
        form.model_index = Some(0);
        assert_eq!(form.model_flags(), (Some("opus"), None)); // auto effort
        form.effort_index = 3;
        assert_eq!(form.model_flags(), (Some("opus"), Some("high")));
        form.terminal = true;
        assert_eq!(form.model_flags(), (None, None)); // terminal drops flags
    }

    #[test]
    fn cycle_agent_resets_model_selection() {
        let mut form = CreateForm::new("claude", &["codex".into()]);
        form.model_index = Some(1);
        form.effort_index = 2;
        form.cycle_agent(1);
        assert_eq!(form.model_index, None);
        assert_eq!(form.effort_index, 0);
    }

    #[test]
    fn switch_to_custom_clears_agent_and_model() {
        let mut form = CreateForm::new("claude", &[]);
        form.model_index = Some(0);
        form.switch_to_custom();
        assert!(form.agent_is_custom());
        assert_eq!(form.agent, "");
        assert_eq!(form.model_index, None);
    }

    #[test]
    fn base_matches_filters_case_insensitive_substring() {
        let mut form = CreateForm::new("claude", &[]);
        form.base_branches = vec!["main".into(), "dev".into(), "Feature/X".into()];
        assert_eq!(form.base_matches().len(), 3); // empty filter = all
        form.base_filter = "Eat".into();
        assert_eq!(form.base_matches(), vec!["Feature/X".to_string()]);
        form.base_filter = "zzz".into();
        assert!(form.base_matches().is_empty());
    }

    #[test]
    fn base_select_wraps_over_matches() {
        let mut form = CreateForm::new("claude", &[]);
        form.base_branches = vec!["main".into(), "dev".into(), "feature".into()];
        form.base_filter = "e".into(); // dev, feature
        form.base_select(1);
        assert_eq!(form.selected_base().as_deref(), Some("feature"));
        form.base_select(1); // wraps
        assert_eq!(form.selected_base().as_deref(), Some("dev"));
        form.base_select(-1);
        assert_eq!(form.selected_base().as_deref(), Some("feature"));
    }

    #[test]
    fn base_filter_edit_resets_highlight() {
        let mut form = CreateForm::new("claude", &[]);
        form.base_branches = vec!["main".into(), "dev".into()];
        form.base_select(1);
        assert_eq!(form.base_index, 1);
        form.base_filter_push('d');
        assert_eq!(form.base_index, 0);
        form.base_select(1);
        form.base_filter_pop();
        assert_eq!(form.base_index, 0); // pop resets the highlight too
    }

    #[test]
    fn build_create_uses_highlighted_base_match() {
        let mut form = form_with_branches(&["main", "dev"], Some("main"));
        form.name = "ok".into();
        form.dir = "/tmp".into();
        form.branch_input = "feat-y".into();
        form.refresh_branch_entries(); // + create row selected
        form.base_filter = "de".into();
        form.worktree = true;
        match build_create_action(&form, &[]) {
            Ok(Action::Create {
                worktree: Some(WorktreeSpec::New { base, branch }),
                ..
            }) => {
                assert_eq!(base, "dev");
                assert_eq!(branch, "feat-y");
            }
            other => panic!("expected worktree create, got {other:?}"),
        }
    }

    #[test]
    fn build_create_rejects_unmatched_base_filter() {
        let mut form = form_with_branches(&["main"], Some("main"));
        form.name = "ok".into();
        form.dir = "/tmp".into();
        form.branch_input = "feat-y".into();
        form.refresh_branch_entries(); // + create row selected
        form.base_filter = "nope".into();
        form.worktree = true;
        assert!(build_create_action(&form, &[]).is_err());
    }

    #[test]
    fn compose_agent_command_appends_flags() {
        assert_eq!(compose_agent_command("claude", None, None), "claude");
        assert_eq!(
            compose_agent_command("claude", Some("opus"), None),
            "claude --model opus"
        );
        assert_eq!(
            compose_agent_command("claude", Some("sonnet"), Some("high")),
            "claude --model sonnet --effort high"
        );
        assert_eq!(
            compose_agent_command("claude --dangerously-skip-permissions", Some("haiku"), None),
            "claude --dangerously-skip-permissions --model haiku"
        );
    }

    #[test]
    fn build_create_carries_model_and_effort() {
        let mut form = CreateForm::new("claude", &[]);
        form.name = "ok".into();
        form.dir = "/tmp".into();
        form.model_index = Some(0); // opus
        form.effort_index = 3; // high
        match build_create_action(&form, &[]) {
            Ok(Action::Create { model, effort, .. }) => {
                assert_eq!(model.as_deref(), Some("opus"));
                assert_eq!(effort.as_deref(), Some("high"));
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn build_create_auto_model_has_no_flags() {
        let mut form = CreateForm::new("claude", &[]);
        form.name = "ok".into();
        form.dir = "/tmp".into();
        match build_create_action(&form, &[]) {
            Ok(Action::Create { model, effort, .. }) => {
                assert_eq!(model, None);
                assert_eq!(effort, None);
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }

    fn upd_info() -> crate::update::UpdateInfo {
        crate::update::UpdateInfo {
            version: "9.9.9".into(),
            url: "https://example.invalid/amux.tar.gz".into(),
        }
    }

    #[test]
    fn update_modal_offered_once_and_only_from_list() {
        let mut app = App::new(Config::default());
        app.update = Some(upd_info());
        app.mode = Mode::Help; // busy → no modal
        app.offer_update_if_idle();
        assert!(matches!(app.mode, Mode::Help));
        app.mode = Mode::List;
        app.offer_update_if_idle();
        assert!(matches!(app.mode, Mode::ConfirmUpdate(_)));
        // Declined → never offered again this run.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List));
        app.offer_update_if_idle();
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn update_y_starts_install_and_r_restarts_when_done() {
        use crate::update::UpdateStage as S;
        let mut app = App::new(Config::default());
        app.update = Some(upd_info());
        app.offer_update_if_idle();
        let act = app.handle_key(key('y'));
        assert!(matches!(act, Some(Action::StartUpdate(_))));
        match &app.mode {
            Mode::ConfirmUpdate(m) => assert_eq!(m.stage, Some(S::Downloading)),
            other => panic!("expected ConfirmUpdate, got {other:?}"),
        }
        app.set_update_stage(S::Done("9.9.9".into()));
        assert!(app.update.is_none(), "badge cleared once installed");
        let act = app.handle_key(key('r'));
        assert!(matches!(act, Some(Action::RestartSelf)));
    }

    #[test]
    fn update_esc_hides_progress_and_stage_updates_skip_closed_modal() {
        use crate::update::UpdateStage as S;
        let mut app = App::new(Config::default());
        app.update = Some(upd_info());
        app.offer_update_if_idle();
        app.handle_key(key('y'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::List), "install continues hidden");
        app.set_update_stage(S::Installing); // must not panic / change mode
        assert!(matches!(app.mode, Mode::List));
        app.set_update_stage(S::Done("9.9.9".into()));
        assert!(app.update.is_none(), "badge cleared even when hidden");
    }

    fn make_git_session(name: &str, dir: &str, worktree_repo: Option<&str>, branch: Option<&str>) -> crate::tmux::Session {
        crate::tmux::Session {
            name: name.into(),
            dir: dir.into(),
            cwd: dir.into(),
            created: 0,
            agent: String::new(),
            status: crate::tmux::Status::Idle,
            attached: false,
            git: branch.map(|b| crate::git::GitInfo { branch: b.into(), added: 0, removed: 0 }),
            worktree_repo: worktree_repo.map(|r| r.into()),
        }
    }

    #[test]
    fn dir_up_down_navigate_picker_not_fields() {
        let mut app = App::new(Config::default());
        // Open create form and manually put it on the Dir step with entries.
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Dir;
        // Seed two fake entries so the picker has something to walk.
        form.dir_entries = vec!["alpha".to_string(), "beta".to_string()];
        form.dir_selected = 0;
        app.mode = Mode::Create(form);

        // ↓ should move the picker selection, not advance the form field.
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        if let Mode::Create(ref f) = app.mode {
            assert_eq!(f.dir_selected, 1, "↓ moves picker selection");
            assert_eq!(f.field, CreateField::Dir, "↓ must not leave Dir step");
        } else {
            panic!("lost Create mode");
        }

        // ↑ wraps back.
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        if let Mode::Create(ref f) = app.mode {
            assert_eq!(f.dir_selected, 0, "↑ moves picker selection back");
            assert_eq!(f.field, CreateField::Dir, "↑ must not leave Dir step");
        } else {
            panic!("lost Create mode");
        }
    }

    #[test]
    fn ctrl_g_on_worktree_session_opens_promote_modal() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        let s = make_git_session("wt", "/proj/.worktrees/feat", Some("/proj"), Some("feat"));
        app.sessions = vec![s];
        app.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(
            matches!(&app.mode, Mode::Git(f) if f.action == GitAction::Promote && f.branch == "feat"),
            "Ctrl+g on worktree must open Promote modal, got {:?}", app.mode
        );
    }

    #[test]
    fn ctrl_g_on_normal_branch_session_opens_delete_modal() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        let s = make_git_session("br", "/proj", None, Some("feature/x"));
        app.sessions = vec![s];
        app.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(
            matches!(&app.mode, Mode::Git(f) if f.action == GitAction::DeleteBranch && f.branch == "feature/x"),
            "Ctrl+g on normal session must open DeleteBranch modal"
        );
    }

    #[test]
    fn ctrl_g_on_protected_branch_is_noop() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        let s = make_git_session("main-sess", "/proj", None, Some("main"));
        app.sessions = vec![s];
        app.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        app.handle_key(key);
        assert!(
            matches!(app.mode, Mode::List),
            "Ctrl+g on protected branch must stay in List"
        );
    }

    #[test]
    fn git_modal_y_returns_promote_action() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        app.mode = Mode::Git(GitForm {
            session_name: "wt".into(),
            branch: "feat".into(),
            repo_root: "/proj".into(),
            worktree_path: Some("/proj/.worktrees/feat".into()),
            has_stash: false,
            action: GitAction::Promote,
            branches: vec![],
            selected: std::collections::HashSet::new(),
            cursor: 0,
        });
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let action = app.handle_key(key);
        assert_eq!(
            action,
            Some(Action::PromoteWorktree { name: "wt".into(), branch: "feat".into(), has_stash: false })
        );
    }

    #[test]
    fn git_modal_n_returns_to_list() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        app.mode = Mode::Git(GitForm {
            session_name: "wt".into(),
            branch: "feat".into(),
            repo_root: "/proj".into(),
            worktree_path: Some("/proj/.worktrees/feat".into()),
            has_stash: false,
            action: GitAction::Promote,
            branches: vec![],
            selected: std::collections::HashSet::new(),
            cursor: 0,
        });
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        let action = app.handle_key(key);
        assert_eq!(action, None);
        assert!(matches!(app.mode, Mode::List));
    }

    #[test]
    fn cleanup_space_toggles_selection() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        let mut selected = std::collections::HashSet::new();
        selected.insert(0usize);
        app.mode = Mode::Git(GitForm {
            session_name: "s".into(),
            branch: String::new(),
            repo_root: "/proj".into(),
            worktree_path: None,
            has_stash: false,
            action: GitAction::BranchCleanup,
            branches: vec![BranchItem { name: "feat".into(), protected: false }],
            selected,
            cursor: 0,
        });
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        app.handle_key(key);
        let Mode::Git(f) = &app.mode else { panic!("must stay in Git mode") };
        assert!(!f.selected.contains(&0), "space must deselect");
    }

    #[test]
    fn build_create_action_no_worktree_when_flag_false() {
        let mut form = CreateForm::new("claude", &[]);
        form.name = "mysession".to_string();
        form.dir = "/tmp".to_string(); // must exist
        form.branches = vec!["main".to_string(), "dev".to_string()];
        form.current_branch = Some("main".to_string());
        form.refresh_branch_entries(); // populates branch_entries with main, dev
        // worktree is false (default) — no WorktreeSpec should be emitted
        let action = build_create_action(&form, &[]).unwrap();
        if let Action::Create { worktree, .. } = action {
            assert!(worktree.is_none(), "worktree must be None when form.worktree=false");
        } else {
            panic!("expected Action::Create");
        }
    }
}
