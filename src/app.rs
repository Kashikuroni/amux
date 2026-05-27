use crate::config::Config;
use crate::tmux::{Session, Status};
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};

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
}

impl CreateForm {
    pub fn new(default_agent: &str) -> Self {
        Self {
            name: String::new(),
            dir: String::new(),
            agent: default_agent.to_string(),
            field: CreateField::Name,
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
        }
    }

    pub fn selected_name(&self) -> Option<String> {
        self.sessions.get(self.selected).map(|s| s.name.clone())
    }

    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.sessions.len();
    }

    pub fn select_prev(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.sessions.len() - 1
        } else {
            self.selected - 1
        };
    }

    fn clamp_selection(&mut self) {
        if self.sessions.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.sessions.len() {
            self.selected = self.sessions.len() - 1;
        }
    }

    fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::List => ModeKind::List,
            Mode::Create(_) => ModeKind::Create,
            Mode::Rename(_) => ModeKind::Rename,
            Mode::ConfirmDelete(_) => ModeKind::ConfirmDelete,
            Mode::Help => ModeKind::Help,
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
        }
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.select_prev(),
            KeyCode::Char('n') => {
                self.error = None;
                self.mode = Mode::Create(CreateForm::new(&self.config.default_agent));
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
        match key.code {
            KeyCode::Esc => return None, // mode already reset to List
            KeyCode::Backspace => {
                form.current_mut().pop();
            }
            KeyCode::Char(c) => form.current_mut().push(c),
            KeyCode::Tab => form.field = form.next_field(),
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
                    form.field = form.next_field();
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

    /// Re-derives sessions from tmux and recomputes statuses + preview.
    pub fn refresh(&mut self) {
        match crate::tmux::list_sessions() {
            Ok(mut sessions) => {
                let selected_name = self.selected_name();
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

/// First observation (no previous snapshot) is `Waiting`; changed → `Running`.
pub fn compute_status(prev: Option<u64>, current: u64) -> Status {
    match prev {
        Some(p) if p != current => Status::Running,
        _ => Status::Waiting, // first observation OR content unchanged
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
                status: Status::Waiting,
                attached: false,
            },
            Session {
                name: "b".into(),
                dir: "/b".into(),
                created: 2,
                agent: "claude".into(),
                status: Status::Waiting,
                attached: false,
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
    fn status_is_waiting_on_first_observation() {
        assert_eq!(compute_status(None, 42), Status::Waiting);
    }

    #[test]
    fn status_is_running_when_content_changed() {
        assert_eq!(compute_status(Some(1), 2), Status::Running);
    }

    #[test]
    fn status_is_waiting_when_content_unchanged() {
        assert_eq!(compute_status(Some(7), 7), Status::Waiting);
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
}
