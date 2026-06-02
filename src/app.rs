use crate::config::Config;
use crate::tmux::{Session, Status};
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Sentinel label for the free-text agent slot in `CreateForm::agent_choices`.
pub const CUSTOM_AGENT_SLOT: &str = "custom\u{2026}"; // "custom…"

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateField {
    Name,
    Dir,
    Worktree,
    Base,
    Branch,
    Agent,
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
    pub worktree: bool,
    pub base_branches: Vec<String>,
    pub base_index: usize,
    pub new_branch: String,
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
            worktree: false,
            base_branches: Vec::new(),
            base_index: 0,
            new_branch: String::new(),
        }
    }

    fn current_mut(&mut self) -> &mut String {
        match self.field {
            CreateField::Name => &mut self.name,
            CreateField::Dir => &mut self.dir,
            CreateField::Branch => &mut self.new_branch,
            CreateField::Agent => &mut self.agent,
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
        if self.worktree {
            5
        } else {
            3
        }
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
    }

    /// Appends pasted text to the focused field (reloads dir listing if on Dir).
    pub fn paste(&mut self, text: &str) {
        self.current_mut().push_str(text);
        if self.field == CreateField::Dir {
            self.refresh_dir_entries();
        }
    }

    /// 1-based position of the focused field, for the `N of M` step indicator.
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

/// Free-text reply being composed for a specific session.
///
/// `cursor` is a *character* index into `buffer` (0..=char_count), so editing
/// stays correct with multi-byte input (e.g. Cyrillic). All byte offsets are
/// derived from it on demand.
#[derive(Debug, Clone)]
pub struct ReplyForm {
    pub name: String,
    pub buffer: String,
    pub cursor: usize,
}

impl ReplyForm {
    fn new(name: String) -> Self {
        Self {
            name,
            buffer: String::new(),
            cursor: 0,
        }
    }

    fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    /// Byte offset of character `idx` (or end of buffer if out of range).
    fn byte_at(&self, idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    fn insert_char(&mut self, c: char) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert(b, c);
        self.cursor += 1;
    }

    fn insert_str(&mut self, s: &str) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let b = self.byte_at(self.cursor - 1);
        self.buffer.remove(b);
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let b = self.byte_at(self.cursor);
        self.buffer.remove(b);
    }

    fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Start/end character index of the logical line the cursor sits on.
    fn line_bounds(&self) -> (usize, usize) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut start = self.cursor.min(chars.len());
        while start > 0 && chars[start - 1] != '\n' {
            start -= 1;
        }
        let mut end = self.cursor.min(chars.len());
        while end < chars.len() && chars[end] != '\n' {
            end += 1;
        }
        (start, end)
    }

    fn home(&mut self) {
        self.cursor = self.line_bounds().0;
    }

    fn end(&mut self) {
        self.cursor = self.line_bounds().1;
    }

    /// Move up one logical line, preserving the column where possible.
    fn up(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let (start, _) = self.line_bounds();
        if start == 0 {
            self.cursor = 0;
            return;
        }
        let col = self.cursor - start;
        let prev_end = start - 1; // the '\n'
        let mut prev_start = prev_end;
        while prev_start > 0 && chars[prev_start - 1] != '\n' {
            prev_start -= 1;
        }
        let prev_len = prev_end - prev_start;
        self.cursor = prev_start + col.min(prev_len);
    }

    /// Move down one logical line, preserving the column where possible.
    fn down(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let (start, end) = self.line_bounds();
        if end >= chars.len() {
            self.cursor = chars.len();
            return;
        }
        let col = self.cursor - start;
        let next_start = end + 1;
        let mut next_end = next_start;
        while next_end < chars.len() && chars[next_end] != '\n' {
            next_end += 1;
        }
        let next_len = next_end - next_start;
        self.cursor = next_start + col.min(next_len);
    }

    /// Delete the word (and any trailing spaces) before the cursor (Ctrl+W).
    fn delete_word(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && chars[i - 1] != ' ' && chars[i - 1] != '\n' {
            i -= 1;
        }
        let (sb, eb) = (self.byte_at(i), self.byte_at(self.cursor));
        self.buffer.replace_range(sb..eb, "");
        self.cursor = i;
    }

    /// Delete from the start of the current line to the cursor (Ctrl+U).
    fn delete_to_line_start(&mut self) {
        let (start, _) = self.line_bounds();
        let (sb, eb) = (self.byte_at(start), self.byte_at(self.cursor));
        self.buffer.replace_range(sb..eb, "");
        self.cursor = start;
    }
}

#[derive(Debug, Clone)]
pub enum Mode {
    List,
    Create(CreateForm),
    Rename(RenameForm),
    ConfirmDelete(String),
    Help,
    Filter,
    Reply(ReplyForm),
    /// Awaiting a 1–9 digit to jump to that session (entered with `s`).
    SelectSession,
    /// Editing a project's display name (entered with Shift+R). Display-only —
    /// never renames the directory.
    RenameProject(ProjectRenameForm),
}

/// Display-name editor for a project, keyed by its root path.
#[derive(Debug, Clone)]
pub struct ProjectRenameForm {
    pub root: String,
    pub buffer: String,
}

/// Worktree parameters carried by `Action::Create` when the worktree toggle is on.
#[derive(Debug, Clone, PartialEq)]
pub struct WorktreeSpec {
    pub base: String,
    pub new_branch: String,
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
    },
    Kill(String),
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
}

#[derive(Copy, Clone)]
enum ModeKind {
    List,
    Create,
    Rename,
    ConfirmDelete,
    Help,
    Filter,
    Reply,
    SelectSession,
    RenameProject,
}

pub struct App {
    pub config: Config,
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub mode: Mode,
    pub preview: String,
    pub snapshots: HashMap<String, u64>,
    pub error: Option<String>,
    pub should_quit: bool,
    pub filter: Option<String>,
    pub spinner_frame: usize,
    pub now_unix: i64,
    pub tmux_missing: bool,
    pub clock: String,
    /// Detected numbered prompt per session: option labels for digits 1..N.
    pub prompts: HashMap<String, Vec<String>>,
    /// Lines the preview is scrolled up from the bottom (0 = latest/bottom).
    pub preview_scroll: u16,
    /// Left (sessions) pane width as a percentage of the body.
    pub split_pct: u16,
    /// Latest Claude Code subscription usage (5h / 7d), shown in the header.
    /// `None` until the first successful fetch (or if unauthenticated).
    pub usage: Option<crate::usage::Usage>,
    /// Subscription plan badge (e.g. "Max 5×"), shown in the header.
    pub plan: Option<String>,
    /// User's custom session order *within projects* (by name). Empty = tmux order.
    pub order: Vec<String>,
    /// User's custom project (group) order, by project root path.
    pub project_order: Vec<String>,
    /// Display-name overrides for projects, keyed by project root path.
    pub project_names: std::collections::BTreeMap<String, String>,
    /// Set when persisted state (split width / order / names) changed and needs
    /// saving. The event loop saves and clears it; keeps `App` itself IO-free.
    pub dirty: bool,
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
            error: None,
            should_quit: false,
            filter: None,
            spinner_frame: 0,
            now_unix: crate::timeutil::now_unix(),
            tmux_missing: false,
            clock: crate::timeutil::clock_hhmm(),
            prompts: HashMap::new(),
            preview_scroll: 0,
            split_pct: 40,
            usage: None,
            plan: None,
            order: Vec::new(),
            project_order: Vec::new(),
            project_names: std::collections::BTreeMap::new(),
            dirty: false,
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
    }

    /// Snapshots the persistable UI state for saving to disk.
    pub fn snapshot_state(&self) -> crate::state::State {
        crate::state::State {
            split_pct: Some(self.split_pct),
            order: self.order.clone(),
            project_order: self.project_order.clone(),
            project_names: self.project_names.clone(),
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
    fn preview_scroll_up(&mut self, n: u16) {
        self.preview_scroll = self.preview_scroll.saturating_add(n).min(5000);
    }
    fn preview_scroll_down(&mut self, n: u16) {
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
            Mode::Help => ModeKind::Help,
            Mode::Filter => ModeKind::Filter,
            Mode::Reply(_) => ModeKind::Reply,
            Mode::SelectSession => ModeKind::SelectSession,
            Mode::RenameProject(_) => ModeKind::RenameProject,
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
            ModeKind::Create => self.handle_create_key(key),
            ModeKind::Rename => self.handle_rename_key(key),
            ModeKind::Filter => self.handle_filter_key(key),
            ModeKind::Reply => self.handle_reply_key(key),
            ModeKind::SelectSession => self.handle_select_session_key(key),
            ModeKind::RenameProject => self.handle_rename_project_key(key),
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
                self.preview = crate::tmux::capture_scrollback(&name, 500)
                    .or_else(|_| crate::tmux::capture_pane(&name))
                    .unwrap_or_default();
            }
            None => self.preview.clear(),
        }
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
            KeyCode::Char('d') => {
                if let Some(name) = self.selected_name() {
                    self.mode = Mode::ConfirmDelete(name);
                }
            }
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
            KeyCode::Char('g') => {
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
            // Free-text reply to the selected session.
            KeyCode::Char('i') => {
                if let Some(name) = self.selected_name() {
                    self.mode = Mode::Reply(ReplyForm::new(name));
                }
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

    fn handle_reply_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::Reply(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return None, // mode already reset to List → cancels
            // Newline on Shift+Enter; plain Enter sends (see below). Shift+Enter
            // requires the kitty keyboard protocol to be reported distinctly
            // (enabled in main); Alt+Enter is a fallback on terminals without it.
            KeyCode::Enter if shift || alt => form.insert_char('\n'),
            // Editing chords (readline-ish).
            KeyCode::Char('w') if ctrl => form.delete_word(),
            KeyCode::Char('u') if ctrl => form.delete_to_line_start(),
            KeyCode::Char('a') if ctrl => form.home(),
            KeyCode::Char('e') if ctrl => form.end(),
            // Plain text entry — guard against control chords leaking through.
            KeyCode::Char(c) if !ctrl => form.insert_char(c),
            // Plain Enter sends the composed message.
            KeyCode::Enter => {
                let text = form.buffer.trim().to_string();
                if text.is_empty() {
                    return None;
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

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::ConfirmDelete(name) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match latin_code(key.code) {
            KeyCode::Char('y') => return Some(Action::Kill(name)),
            KeyCode::Char('n') | KeyCode::Esc => {} // mode already reset to List
            _ => self.mode = Mode::ConfirmDelete(name), // unknown key: stay in confirm
        }
        None
    }

    fn handle_create_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Take the form out so we can borrow `self.sessions` for validation.
        let Mode::Create(mut form) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };

        // Dir step: interactive picker (live subdir list).
        if form.field == CreateField::Dir {
            match key.code {
                KeyCode::Esc => return None, // mode already reset to List
                KeyCode::Backspace => {
                    form.dir.pop();
                    form.refresh_dir_entries();
                }
                KeyCode::Char(c) => {
                    form.dir.push(c);
                    form.refresh_dir_entries();
                }
                KeyCode::Up => form.dir_select_prev(),
                KeyCode::Down => form.dir_select_next(),
                KeyCode::Tab | KeyCode::Right => form.enter_selected_dir(),
                KeyCode::Enter => {
                    let existing: Vec<String> =
                        self.sessions.iter().map(|s| s.name.clone()).collect();
                    match validate_create(&form.name, &form.dir, &existing) {
                        Ok(()) => {
                            self.error = None;
                            form.field = CreateField::Agent;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                _ => {}
            }
            self.mode = Mode::Create(form);
            return None;
        }

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

        // Name / agent / branch steps: plain text fields.
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Left if form.field == CreateField::Agent => form.cycle_agent(-1),
            KeyCode::Right if form.field == CreateField::Agent => form.cycle_agent(1),
            KeyCode::Backspace => {
                if form.field == CreateField::Agent && !form.agent_is_custom() {
                    // Backspace off a preset: jump to custom and start fresh (matches Char).
                    form.agent_index = form.agent_choices.len().saturating_sub(1);
                    form.agent.clear();
                }
                form.current_mut().pop();
            }
            KeyCode::Char(c) => {
                if form.field == CreateField::Agent && !form.agent_is_custom() {
                    form.agent_index = form.agent_choices.len().saturating_sub(1);
                    form.agent.clear();
                }
                form.current_mut().push(c);
            }
            KeyCode::Tab => form.advance(),
            KeyCode::Enter => {
                if form.field == CreateField::Agent {
                    let existing: Vec<String> =
                        self.sessions.iter().map(|s| s.name.clone()).collect();
                    match validate_create(&form.name, &form.dir, &existing) {
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
                        Err(e) => {
                            self.error = Some(e);
                            self.mode = Mode::Create(form);
                            return None;
                        }
                    }
                } else {
                    // Non-Agent step → advance (handles Dir refresh when needed).
                    form.advance();
                }
            }
            _ => {}
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

    /// Re-derives sessions from tmux and recomputes statuses + preview.
    pub fn refresh(&mut self) {
        match crate::tmux::list_sessions() {
            Ok(mut sessions) => {
                let selected_name = self.selected_name();
                self.now_unix = crate::timeutil::now_unix();
                self.clock = crate::timeutil::clock_hhmm();
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
                    // TODO(perf): git::read shells out to `git` per session per
                    // tick on the main thread. Fine for local repos / few
                    // sessions; move to a background thread if it ever stalls
                    // the UI on slow filesystems.
                    s.git = crate::git::read(&s.dir);
                }
                self.snapshots = new_snaps;
                self.prompts = new_prompts;
                self.sessions = apply_grouped_order(&self.project_order, &self.order, sessions);
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

/// Removes ANSI/CSI escape sequences so captured pane text can be matched.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip the escape and everything up to its final byte (a letter).
            while let Some(n) = chars.next() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
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

    fn app_with_two_sessions() -> App {
        let mut app = App::new(Config::default());
        app.sessions = vec![
            Session {
                name: "a".into(),
                dir: "/a".into(),
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
        assert_eq!(action, Some(Action::Kill("a".into())));
        assert!(matches!(app.mode, Mode::List));
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
        let mut form = CreateForm::new("claude", &[]);
        assert_eq!(form.step(), 1);
        form.field = CreateField::Dir;
        assert_eq!(form.step(), 2);
        form.field = CreateField::Agent;
        assert_eq!(form.step(), 3);
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
        assert_eq!(f.buffer, "привет");
        assert_eq!(f.cursor, 6);
        // Move into the middle and insert (cursor lands before "е").
        f.left();
        f.left();
        f.insert_char('Х');
        assert_eq!(f.buffer, "привХет");
        // Backspace removes the char we just inserted.
        f.backspace();
        assert_eq!(f.buffer, "привет");
        // Delete (forward) removes "е".
        f.delete();
        assert_eq!(f.buffer, "привт");
    }

    #[test]
    fn reply_delete_word_and_line_start() {
        let mut f = ReplyForm::new("s".into());
        f.insert_str("hello world foo");
        f.delete_word();
        assert_eq!(f.buffer, "hello world ");
        f.delete_to_line_start();
        assert_eq!(f.buffer, "");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn reply_up_down_preserve_column_within_lines() {
        let mut f = ReplyForm::new("s".into());
        f.insert_str("abcd\nef\nghij");
        // cursor at end (line "ghij", col 4)
        assert_eq!(f.cursor, 12);
        f.up(); // onto "ef" (len 2) → column clamps to 2
        let (start, _) = f.line_bounds();
        assert_eq!(f.cursor - start, 2);
        f.up(); // onto "abcd", same column 2
        let (start, _) = f.line_bounds();
        assert_eq!(f.cursor - start, 2);
        f.home();
        assert_eq!(f.cursor, 0);
        f.end();
        assert_eq!(f.cursor, 4); // end of first logical line, before '\n'
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

    #[test]
    fn worktree_off_skips_base_and_branch() {
        let mut form = CreateForm::new("claude", &[]);
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

    #[test]
    fn create_action_carries_worktree_spec() {
        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.name = "iso".into();
        form.dir = "/tmp".into(); // exists as a dir
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.base_index = 0;
        form.new_branch = "iso-branch".into();
        form.field = CreateField::Agent;
        app.mode = Mode::Create(form);
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            Some(Action::Create {
                worktree: Some(spec),
                ..
            }) => {
                assert_eq!(spec.base, "main");
                assert_eq!(spec.new_branch, "iso-branch");
            }
            other => panic!("expected Create with worktree, got {other:?}"),
        }
    }
}
