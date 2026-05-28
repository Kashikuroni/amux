use crate::config::Config;
use crate::tmux::{Session, Status};
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};

/// Sentinel label for the free-text agent slot in `CreateForm::agent_choices`.
pub const CUSTOM_AGENT_SLOT: &str = "custom\u{2026}"; // "custom…"

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreateField {
    Name,
    Dir,
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
        }
    }

    fn current_mut(&mut self) -> &mut String {
        match self.field {
            CreateField::Name => &mut self.name,
            CreateField::Dir => &mut self.dir,
            CreateField::Agent => &mut self.agent,
        }
    }

    fn next_field(&self) -> CreateField {
        match self.field {
            CreateField::Name => CreateField::Dir,
            CreateField::Dir => CreateField::Agent,
            CreateField::Agent => CreateField::Name,
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

#[derive(Debug, Clone)]
pub enum Mode {
    List,
    Create(CreateForm),
    Rename(RenameForm),
    ConfirmDelete(String),
    Help,
    Filter,
}

/// Side effects the event loop must perform (kept out of `App` so it stays IO-free).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Attach(String),
    Create { name: String, dir: String, agent: String },
    Kill(String),
    Rename { old: String, new: String },
}

#[derive(Copy, Clone)]
enum ModeKind {
    List,
    Create,
    Rename,
    ConfirmDelete,
    Help,
    Filter,
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
        }
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
        self.selected = if self.selected == 0 { n - 1 } else { self.selected - 1 };
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

    fn select_last(&mut self) {
        let n = self.visible_indices().len();
        self.selected = n.saturating_sub(1);
    }

    fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::List => ModeKind::List,
            Mode::Create(_) => ModeKind::Create,
            Mode::Rename(_) => ModeKind::Rename,
            Mode::ConfirmDelete(_) => ModeKind::ConfirmDelete,
            Mode::Help => ModeKind::Help,
            Mode::Filter => ModeKind::Filter,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.mode_kind() {
            ModeKind::List => self.handle_list_key(key),
            ModeKind::Help => {
                self.mode = Mode::List;
                None
            }
            ModeKind::ConfirmDelete => self.handle_confirm_key(key),
            ModeKind::Create => self.handle_create_key(key),
            ModeKind::Rename => self.handle_rename_key(key),
            ModeKind::Filter => self.handle_filter_key(key),
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
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
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),
            _ => {}
        }
        None
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Mode::ConfirmDelete(name) = std::mem::replace(&mut self.mode, Mode::List) else {
            return None;
        };
        match key.code {
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

        // Name / agent steps: plain text fields.
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
            KeyCode::Tab => {
                form.field = form.next_field();
                if form.field == CreateField::Dir {
                    form.refresh_dir_entries();
                }
            }
            KeyCode::Enter => {
                if form.field == CreateField::Agent {
                    let existing: Vec<String> =
                        self.sessions.iter().map(|s| s.name.clone()).collect();
                    match validate_create(&form.name, &form.dir, &existing) {
                        Ok(()) => {
                            self.error = None;
                            return Some(Action::Create {
                                name: form.name.trim().to_string(),
                                dir: expand_tilde(&form.dir),
                                agent: form.agent.clone(),
                            });
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.mode = Mode::Create(form);
                            return None;
                        }
                    }
                } else {
                    // Name step → advance to dir and load its listing.
                    form.field = form.next_field();
                    form.refresh_dir_entries();
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
                let mut new_snaps = HashMap::new();
                let mut new_preview = None;
                for s in &mut sessions {
                    if let Ok(content) = crate::tmux::capture_pane(&s.name) {
                        let h = content_hash(&content);
                        s.status = compute_status(self.snapshots.get(&s.name).copied(), h);
                        new_snaps.insert(s.name.clone(), h);
                        if selected_name.as_deref() == Some(s.name.as_str()) {
                            new_preview = Some(content);
                        }
                    }
                    // TODO(perf): git::read shells out to `git` per session per
                    // tick on the main thread. Fine for local repos / few
                    // sessions; move to a background thread if it ever stalls
                    // the UI on slow filesystems.
                    s.git = crate::git::read(&s.dir);
                }
                self.snapshots = new_snaps;
                self.sessions = sessions;
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
            },
            Session {
                name: "b".into(),
                dir: "/b".into(),
                created: 2,
                agent: "claude".into(),
                status: Status::Idle,
                attached: false,
                git: None,
            },
        ];
        app
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
    fn g_and_shift_g_jump_first_last() {
        let mut app = app_with_two_sessions();
        app.handle_key(key('G'));
        assert_eq!(app.selected, 1);
        app.handle_key(key('g'));
        assert_eq!(app.selected, 0);
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
}
