# cm — Session Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `cm`, a fullscreen Rust TUI that manages multiple CLI-agent tmux sessions (claude-squad-like UX, no git), runnable from any directory.

**Architecture:** Single process. The tmux server is the source of truth — `cm` is stateless on disk (except a config file) and re-derives everything from `tmux` each tick. Per-session metadata (managed-marker, agent command) lives in tmux user options (`@cm_*`). The crate is split into pure logic (config parsing, list parsing, navigation, status-diff, form validation, `handle_key`) that is unit-tested, plus thin IO wrappers around `tmux` that are integration-tested. UI is a layout split 25% sidebar / 75% preview with a full-width footer; status `Running`/`Waiting` is computed by diffing `capture-pane` snapshots between ticks.

**Tech Stack:** Rust (edition 2021), `ratatui` 0.29, `crossterm` 0.28, `serde` 1, `toml` 0.8. External runtime dependency: `tmux` in PATH.

**Reference spec:** `docs/superpowers/specs/2026-05-27-cm-session-manager-design.md`

---

## Task 1: Cargo project scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "cm"
version = "0.1.0"
edition = "2021"

[lib]
name = "cm"
path = "src/lib.rs"

[[bin]]
name = "cm"
path = "src/main.rs"

[dependencies]
ratatui = "0.29"
crossterm = "0.28"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

- [ ] **Step 2: Create `.gitignore`**

```gitignore
/target
```

- [ ] **Step 3: Create placeholder `src/lib.rs`**

```rust
pub mod app;
pub mod config;
pub mod tmux;
pub mod ui;
```

(Modules are empty stubs until later tasks; create them now so `cargo build` works.)

- [ ] **Step 4: Create empty module files**

Create `src/config.rs`, `src/tmux.rs`, `src/app.rs`, `src/ui.rs`, each containing only a comment:

```rust
// implemented in a later task
```

- [ ] **Step 5: Create minimal `src/main.rs`**

```rust
fn main() {
    println!("cm");
}
```

- [ ] **Step 6: Build**

Run: `cargo build`
Expected: compiles successfully (warnings about unused modules are OK).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/
git commit -m "chore: scaffold cm cargo project"
```

---

## Task 2: Config loading

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing tests**

Replace `src/config.rs` contents with the struct skeleton plus tests:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_agent: String,
    pub refresh_interval_ms: u64,
    pub agent_presets: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_agent: "claude".to_string(),
            refresh_interval_ms: 1500,
            agent_presets: vec!["claude".into(), "aider".into(), "codex".into()],
        }
    }
}

impl Config {
    pub fn parse(toml_str: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(toml_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let cfg = Config::parse(
            "default_agent = \"aider\"\nrefresh_interval_ms = 500\nagent_presets = [\"aider\"]",
        )
        .unwrap();
        assert_eq!(cfg.default_agent, "aider");
        assert_eq!(cfg.refresh_interval_ms, 500);
        assert_eq!(cfg.agent_presets, vec!["aider".to_string()]);
    }

    #[test]
    fn empty_config_uses_defaults() {
        let cfg = Config::parse("").unwrap();
        assert_eq!(cfg.default_agent, "claude");
        assert_eq!(cfg.refresh_interval_ms, 1500);
    }

    #[test]
    fn partial_config_fills_missing_with_defaults() {
        let cfg = Config::parse("default_agent = \"codex\"").unwrap();
        assert_eq!(cfg.default_agent, "codex");
        assert_eq!(cfg.refresh_interval_ms, 1500);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib config`
Expected: 3 tests pass (the implementation is already present — this confirms serde defaults behave correctly).

- [ ] **Step 3: Add `load()` for the real config path**

Append to the `impl Config` block (above the `#[cfg(test)]` module):

```rust
    /// Loads `~/.claude-manager/config.toml`. Missing file or parse error → defaults.
    pub fn load() -> Config {
        let Ok(home) = std::env::var("HOME") else {
            return Config::default();
        };
        let path = std::path::Path::new(&home)
            .join(".claude-manager")
            .join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(contents) => Config::parse(&contents).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
```

- [ ] **Step 4: Build to verify it compiles**

Run: `cargo build`
Expected: compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: config parsing with defaults"
```

---

## Task 3: tmux session struct and list parsing (pure)

**Files:**
- Modify: `src/tmux.rs`

- [ ] **Step 1: Write the failing tests**

Replace `src/tmux.rs` contents with the types, the format constant, the parser, and tests:

```rust
/// Tab-separated fields requested from `tmux list-sessions -F`.
/// Order: name, path, created, @cm_managed, @cm_agent, attached-client-count.
pub const LIST_FORMAT: &str =
    "#{session_name}\t#{session_path}\t#{session_created}\t#{@cm_managed}\t#{@cm_agent}\t#{session_attached}";

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Running,
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub dir: String,
    pub created: i64,
    pub agent: String,
    pub status: Status,
    pub attached: bool,
}

/// Parses `tmux list-sessions` output, keeping only sessions marked `@cm_managed=1`.
/// `status` defaults to `Waiting`; the app overwrites it via capture-pane diffing.
pub fn parse_sessions(output: &str) -> Vec<Session> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Session> {
    let mut f = line.splitn(6, '\t');
    let name = f.next()?.to_string();
    let dir = f.next()?.to_string();
    let created = f.next()?.trim().parse::<i64>().ok()?;
    let managed = f.next()?;
    let agent = f.next()?.to_string();
    let attached = f.next().unwrap_or("0").trim();
    if managed != "1" {
        return None;
    }
    Some(Session {
        name,
        dir,
        created,
        agent,
        status: Status::Waiting,
        attached: attached != "0" && !attached.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_managed_session() {
        let out = "proj-a\t/home/u/proj-a\t1716800000\t1\tclaude\t0";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "proj-a");
        assert_eq!(sessions[0].dir, "/home/u/proj-a");
        assert_eq!(sessions[0].created, 1716800000);
        assert_eq!(sessions[0].agent, "claude");
        assert_eq!(sessions[0].status, Status::Waiting);
        assert!(!sessions[0].attached);
    }

    #[test]
    fn filters_out_unmanaged_sessions() {
        let out = "mine\t/d\t1\t1\tclaude\t0\nother\t/d\t1\t\t\t1";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "mine");
    }

    #[test]
    fn marks_attached_when_client_count_positive() {
        let out = "live\t/d\t1\t1\tclaude\t1";
        let sessions = parse_sessions(out);
        assert!(sessions[0].attached);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib tmux::tests`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/tmux.rs
git commit -m "feat: tmux session struct and list-sessions parsing"
```

---

## Task 4: tmux command wrappers (IO)

**Files:**
- Modify: `src/tmux.rs`
- Create: `tests/tmux_integration.rs`

- [ ] **Step 1: Add command wrappers**

Add these imports at the top of `src/tmux.rs` (above `LIST_FORMAT`):

```rust
use std::io;
use std::process::Command;
```

Add the following functions after `parse_line` (before the `#[cfg(test)]` module):

```rust
/// Runs a tmux subcommand, returning an error containing stderr on failure.
fn run(args: &[&str]) -> io::Result<()> {
    let out = Command::new("tmux").args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// True if a `tmux` binary is callable.
pub fn is_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if `cm` itself is running inside a tmux client (nested attach is unsafe).
pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Lists managed sessions. "No server running" is treated as an empty list.
pub fn list_sessions() -> io::Result<Vec<Session>> {
    let out = Command::new("tmux")
        .args(["list-sessions", "-F", LIST_FORMAT])
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_sessions(&String::from_utf8_lossy(&out.stdout)))
}

/// Creates a detached session running `agent` in `dir` and tags it as managed.
pub fn new_session(name: &str, dir: &str, agent: &str) -> io::Result<()> {
    run(&["new-session", "-d", "-s", name, "-c", dir, agent])?;
    run(&["set-option", "-t", name, "@cm_managed", "1"])?;
    run(&["set-option", "-t", name, "@cm_agent", agent])?;
    Ok(())
}

pub fn kill_session(name: &str) -> io::Result<()> {
    run(&["kill-session", "-t", name])
}

pub fn rename_session(old: &str, new: &str) -> io::Result<()> {
    run(&["rename-session", "-t", old, new])
}

/// Captures the visible pane content of a session as plain text.
pub fn capture_pane(name: &str) -> io::Result<String> {
    let out = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", name])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Attaches in the foreground (inherits stdio) and returns when the user detaches.
pub fn attach_session(name: &str) -> io::Result<()> {
    Command::new("tmux")
        .args(["attach-session", "-t", name])
        .status()?;
    Ok(())
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build`
Expected: compiles successfully.

- [ ] **Step 3: Write the integration test**

Create `tests/tmux_integration.rs`:

```rust
use cm::tmux::{self, Status};

/// Full round-trip against a real tmux server. Skipped if tmux is unavailable.
#[test]
fn new_list_rename_capture_kill_roundtrip() {
    if !tmux::is_available() {
        eprintln!("skipping: tmux not available");
        return;
    }

    let name = format!("cm_it_{}", std::process::id());
    let renamed = format!("{name}_r");
    // Clean any leftovers from a previous failed run.
    let _ = tmux::kill_session(&name);
    let _ = tmux::kill_session(&renamed);

    let dir = std::env::temp_dir();
    let dir = dir.to_str().unwrap();

    tmux::new_session(&name, dir, "bash").expect("new_session");

    let sessions = tmux::list_sessions().expect("list_sessions");
    let found = sessions.iter().find(|s| s.name == name).expect("session present");
    assert_eq!(found.agent, "bash");
    assert_eq!(found.status, Status::Waiting);

    tmux::rename_session(&name, &renamed).expect("rename_session");
    let sessions = tmux::list_sessions().expect("list after rename");
    assert!(sessions.iter().any(|s| s.name == renamed));
    assert!(!sessions.iter().any(|s| s.name == name));

    let _capture = tmux::capture_pane(&renamed).expect("capture_pane");

    tmux::kill_session(&renamed).expect("kill_session");
    let sessions = tmux::list_sessions().expect("list after kill");
    assert!(!sessions.iter().any(|s| s.name == renamed));
}
```

- [ ] **Step 4: Run the integration test**

Run: `cargo test --test tmux_integration`
Expected: PASS if tmux is installed; prints "skipping" and passes otherwise.

- [ ] **Step 5: Commit**

```bash
git add src/tmux.rs tests/tmux_integration.rs
git commit -m "feat: tmux command wrappers with integration test"
```

---

## Task 5: App state and pure logic helpers

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Write the failing tests for pure helpers**

Replace `src/app.rs` contents with the types, helpers, and tests below:

```rust
use crate::config::Config;
use crate::tmux::{Session, Status};
use std::collections::HashMap;

/// Stable hash of pane content, used to detect changes between ticks.
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
        _ => Status::Waiting,
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
        std::env::set_var("HOME", "/home/u");
        assert_eq!(expand_tilde("~/proj"), "/home/u/proj");
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
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib app::tests`
Expected: 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat: app pure helpers (status diff, tilde, validation)"
```

---

## Task 6: App modes, actions, and key handling

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add modes, forms, actions, and the App struct**

Insert the following into `src/app.rs` immediately after the `use` lines at the top (before `content_hash`):

```rust
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
    ConfirmDelete,
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
            Mode::ConfirmDelete => ModeKind::ConfirmDelete,
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
                if !self.sessions.is_empty() {
                    self.mode = Mode::ConfirmDelete;
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
        match key.code {
            KeyCode::Char('y') => {
                self.mode = Mode::List;
                return self.selected_name().map(Action::Kill);
            }
            KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::List,
            _ => {}
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
```

- [ ] **Step 2: Add the import for `KeyModifiers` used only in tests, plus key-handling tests**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `src/app.rs` (after the validation tests):

```rust
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
        assert!(matches!(app.mode, Mode::ConfirmDelete));
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
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --lib app`
Expected: all app tests pass (6 helper tests + 5 key-handling tests).

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat: app modes, actions, key handling, refresh"
```

---

## Task 7: TUI rendering

**Files:**
- Modify: `src/ui.rs`

- [ ] **Step 1: Implement `draw` and modals**

Replace `src/ui.rs` contents with:

```rust
use crate::app::{App, CreateField, Mode};
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

const SPINNER: char = '\u{283B}'; // ⠻ : shown for Running
const READY: char = '\u{25CF}'; //   ● : shown for Waiting

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());
    let body = Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(root[0]);

    draw_sidebar(f, app, body[0]);
    draw_preview(f, app, body[1]);
    draw_footer(f, app, root[1]);

    match &app.mode {
        Mode::Create(_) => draw_create_modal(f, app),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete => draw_confirm_modal(f, app),
        Mode::Help => draw_help_modal(f),
        Mode::List => {}
    }
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .map(|s| {
            let marker = match s.status {
                Status::Running => SPINNER,
                Status::Waiting => READY,
            };
            ListItem::new(Line::from(format!("{marker} {}", s.name)))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(format!("sessions ({})", app.sessions.len())))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.sessions.is_empty() {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.sessions.get(app.selected) {
        Some(s) => format!("preview: {} · {}", s.name, s.dir),
        None => "preview".to_string(),
    };
    let para = Paragraph::new(app.preview.as_str())
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = match &app.error {
        Some(e) => format!("error: {e}"),
        None => "sessions: [n] new  [d] kill  [r] rename   actions: [↵/o] attach   nav: [j/k] move  [q] quit  [?] help".to_string(),
    };
    f.render_widget(Paragraph::new(text), area);
}

/// A centered rectangle `pct_x`/`pct_y` percent of the screen.
fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(v[1])[1]
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let prefix = if focused { "> " } else { "  " };
    Line::from(format!("{prefix}{label}: {value}"))
}

fn draw_create_modal(f: &mut Frame, app: &App) {
    let Mode::Create(form) = &app.mode else { return };
    let area = centered(60, 40, f.area());
    f.render_widget(Clear, area);
    let lines = vec![
        field_line("name ", &form.name, form.field == CreateField::Name),
        field_line("dir  ", &form.dir, form.field == CreateField::Dir),
        field_line("agent", &form.agent, form.field == CreateField::Agent),
        Line::from(""),
        Line::from("Tab/Enter: next  ·  Enter on agent: create  ·  Esc: cancel"),
    ];
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("new session"));
    f.render_widget(para, area);
}

fn draw_rename_modal(f: &mut Frame, app: &App) {
    let Mode::Rename(form) = &app.mode else { return };
    let area = centered(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(vec![
        Line::from(format!("new name: {}", form.buffer)),
        Line::from("Enter: rename  ·  Esc: cancel"),
    ])
    .block(Block::default().borders(Borders::ALL).title("rename"));
    f.render_widget(para, area);
}

fn draw_confirm_modal(f: &mut Frame, app: &App) {
    let name = app.selected_name().unwrap_or_default();
    let area = centered(50, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(format!("Kill session \"{name}\"? (y/n)"))
        .block(Block::default().borders(Borders::ALL).title("confirm"));
    f.render_widget(para, area);
}

fn draw_help_modal(f: &mut Frame) {
    let area = centered(50, 60, f.area());
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from("k / j        navigate up / down"),
        Line::from("h / l        scroll preview"),
        Line::from("Enter / o    attach to session"),
        Line::from("n            new session"),
        Line::from("d            kill session"),
        Line::from("r            rename session"),
        Line::from("q            quit (sessions keep running)"),
        Line::from("?            toggle this help"),
        Line::from(""),
        Line::from("any key to close"),
    ];
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("help"));
    f.render_widget(para, area);
}
```

> Note: `h`/`l` preview scrolling is listed in help but is a nice-to-have; it is not wired in v1 (the preview shows the latest captured frame). Leave the help text as documentation of intent.

- [ ] **Step 2: Write a snapshot test**

Add to the bottom of `src/ui.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tmux::Session;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn buf_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_sidebar_and_footer() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "project-a".into(),
            dir: "/work/a".into(),
            created: 1,
            agent: "claude".into(),
            status: Status::Running,
            attached: false,
        }];

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("project-a"));
        assert!(text.contains("sessions (1)"));
        assert!(text.contains("[n] new"));
    }
}
```

- [ ] **Step 3: Run the UI test**

Run: `cargo test --lib ui`
Expected: PASS.

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add src/ui.rs
git commit -m "feat: TUI rendering with sidebar, preview, footer, modals"
```

---

## Task 8: Wire up main (event loop, terminal, attach handoff)

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement `main.rs`**

Replace `src/main.rs` contents with:

```rust
use cm::app::{Action, App};
use cm::config::Config;
use cm::{tmux, ui};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout, Stdout};
use std::time::Duration;

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> io::Result<()> {
    if !tmux::is_available() {
        eprintln!("error: `tmux` not found in PATH. Install tmux and try again.");
        std::process::exit(1);
    }

    let config = Config::load();
    let interval = Duration::from_millis(config.refresh_interval_ms.max(100));
    let mut app = App::new(config);
    app.refresh();

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, interval);
    restore_terminal(&mut terminal)?;
    result
}

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

fn restore_terminal(terminal: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Term, app: &mut App, interval: Duration) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(interval)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action)?;
                }
            }
        } else {
            app.refresh();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_action(terminal: &mut Term, app: &mut App, action: Action) -> io::Result<()> {
    match action {
        Action::Attach(name) => {
            if tmux::in_tmux() {
                app.error =
                    Some("detach from current tmux (Ctrl-B D) before attaching".to_string());
                return Ok(());
            }
            // Hand the terminal over to tmux, then take it back.
            restore_terminal(terminal)?;
            let _ = tmux::attach_session(&name);
            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            terminal.clear()?;
            app.error = None;
            app.refresh();
        }
        Action::Create { name, dir, agent } => {
            if let Err(e) = tmux::new_session(&name, &dir, &agent) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::Kill(name) => {
            if let Err(e) = tmux::kill_session(&name) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::Rename { old, new } => {
            if let Err(e) = tmux::rename_session(&old, &new) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build the whole crate**

Run: `cargo build --release`
Expected: compiles successfully with no errors.

- [ ] **Step 3: Lint**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. Fix any that appear (common: needless clones, unused imports).

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: all unit, integration, and UI tests pass.

- [ ] **Step 5: Manual smoke test** (requires tmux)

Run: `cargo run`
Verify by hand:
1. Press `n`, fill name `smoke`, dir `~`, accept agent `claude` (or `bash` if claude is not installed), Enter on the agent field → a row appears in the sidebar.
2. Selected row shows a preview on the right; status flips to `Running` while the agent prints, `Waiting` when idle.
3. Press `Enter`/`o` to attach; detach with `Ctrl-B D` → returns to the manager.
4. Press `r`, change the name, Enter → row renamed.
5. Press `d`, then `y` → row removed.
6. Press `q` → manager exits; `tmux ls` still lists any remaining `smoke`-style sessions (they keep running).
7. Clean up: `tmux kill-session -t <name>` for any leftovers.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire up main event loop and attach handoff"
```

---

## Self-Review Notes (for the implementer)

- **Spec coverage:** main screen list (Task 7), preview (Task 7 + refresh in Task 6), status running/waiting via diff (Tasks 5–6), create with name/dir/agent + `~` expansion (Tasks 5–6), per-session agent in `@cm_agent` (Task 4), attach/detach (Task 8), kill with confirm (Tasks 6, 8), rename (Tasks 6, 8), quit leaves sessions running (Task 8), config file (Task 2), tmux-missing error (Task 8), `@cm_managed` filtering (Tasks 3–4), runnable from any directory (no cwd dependence anywhere), claude-squad-like fullscreen UI (Task 7).
- **Out of scope confirmed absent:** git/worktree, diff tab, multi-agent orchestration, state file, dir tab-completion.
- **Type consistency:** `Session`/`Status` defined in `tmux.rs` and reused everywhere; `Action`/`Mode`/`App` in `app.rs`; `ui.rs` and `main.rs` consume them by the same names.
```
