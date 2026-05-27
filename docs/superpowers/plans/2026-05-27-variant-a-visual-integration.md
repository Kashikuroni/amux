# Variant A Visual Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the existing `cm` TUI to the hi-fi Variant A (Studio) design: warm dark theme, header status bar, bottom footer, 40/60 layout, multi-line session cards, animated braille spinner, colored preview, restyled modals + agent selector, fuzzy filter, g/G, empty + tmux-error screens.

**Architecture:** Keep the existing tmux-source-of-truth, stateless, IO-free-`App`+`Action` design. Add pure helper modules (`theme`, `spinner`, `timeutil`, `git`), extend `Session`/`App`/`CreateForm`, split the single `ui.rs` into a focused `ui/` module tree, and convert the event loop to an ~80 ms tick with throttled refresh so the spinner animates without `tokio`.

**Tech Stack:** Rust 2021, ratatui 0.29, crossterm 0.28, serde+toml, plus `ansi-to-tui` (new, colored preview). Read-only git/clock via shelling out to `git`/`date`. No tokio/chrono/git2/directories.

**Reference spec:** `docs/superpowers/specs/2026-05-27-variant-a-visual-integration-design.md`

**Ratatui note for every UI task:** if any ratatui 0.29 / crossterm 0.28 API differs from a snippet (e.g. `Line::styled`, `Span::styled`, `f.area()`, `frame.set_cursor_position`), adapt to the correct API keeping behavior identical, and report the change. Snapshot tests lock the rendered output.

---

## Task 1: `theme` + `spinner` modules

**Files:**
- Create: `src/theme.rs`
- Create: `src/spinner.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Register modules in `src/lib.rs`** (keep alphabetical):

```rust
pub mod app;
pub mod browse;
pub mod config;
pub mod git;
pub mod spinner;
pub mod theme;
pub mod timeutil;
pub mod tmux;
pub mod ui;
```
(The `git`/`timeutil` modules are created in Tasks 2–3; if `cargo build` is run before those exist it will fail — that's expected mid-plan. To keep this task self-contained, add only `pub mod spinner;` and `pub mod theme;` now, and add `git`/`timeutil` lines in their tasks.)

So for THIS task, `src/lib.rs` becomes:
```rust
pub mod app;
pub mod browse;
pub mod config;
pub mod spinner;
pub mod theme;
pub mod tmux;
pub mod ui;
```

- [ ] **Step 2: Create `src/theme.rs`**

```rust
//! Variant A (Studio) design tokens: warm dark palette + glyphs.
use ratatui::style::Color;

pub const BG: Color = Color::Rgb(0x1a, 0x17, 0x14);
pub const BG_RAISED: Color = Color::Rgb(0x21, 0x1d, 0x18);
pub const BG_SUNKEN: Color = Color::Rgb(0x16, 0x13, 0x0f);
/// Pre-blended amber-dim used as the selected-row background (terminals have no alpha).
pub const SEL_BG: Color = Color::Rgb(0x2a, 0x1d, 0x18);
pub const TEXT: Color = Color::Rgb(0xe8, 0xdf, 0xd1);
pub const TEXT_BOLD: Color = Color::Rgb(0xf4, 0xec, 0xdd);
pub const MUTED: Color = Color::Rgb(0x8a, 0x7f, 0x6e);
pub const DIM: Color = Color::Rgb(0x5c, 0x54, 0x4a);
pub const BORDER: Color = Color::Rgb(0x2f, 0x2a, 0x23);
pub const BORDER_HI: Color = Color::Rgb(0x40, 0x3a, 0x30);
pub const AMBER: Color = Color::Rgb(0xd9, 0x77, 0x57);
pub const AMBER_HI: Color = Color::Rgb(0xf4, 0xa3, 0x6a);
pub const GREEN: Color = Color::Rgb(0x7a, 0xb8, 0x7a);
pub const RED: Color = Color::Rgb(0xc7, 0x5d, 0x4a);
pub const YELLOW: Color = Color::Rgb(0xd6, 0xb2, 0x5f);
pub const BLUE: Color = Color::Rgb(0x6a, 0x9f, 0xb5);

// Glyphs
pub const LOGO: &str = "◆";
pub const PREVIEW_MARK: &str = "▸";
pub const AGENT_MARK: &str = "✻";
pub const SEL_BAR: &str = "▍";
pub const IDLE_DOT: &str = "·";
pub const BRANCH: &str = "⎇";
pub const SEP: &str = "│";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amber_token_matches_design() {
        assert_eq!(AMBER, Color::Rgb(0xd9, 0x77, 0x57));
    }
}
```

- [ ] **Step 3: Create `src/spinner.rs`**

```rust
//! Braille spinner frames, animated at ~80 ms/frame off an elapsed clock.
pub const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Frame index (0..10) for a given elapsed time, 80 ms per frame.
pub fn frame_index(elapsed_ms: u128) -> usize {
    ((elapsed_ms / 80) % 10) as usize
}

/// Spinner glyph for a frame index (wraps).
pub fn glyph(frame: usize) -> &'static str {
    BRAILLE[frame % BRAILLE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_index_advances_every_80ms() {
        assert_eq!(frame_index(0), 0);
        assert_eq!(frame_index(79), 0);
        assert_eq!(frame_index(80), 1);
        assert_eq!(frame_index(159), 1);
        assert_eq!(frame_index(800), 0); // wraps after 10 frames
    }

    #[test]
    fn glyph_wraps() {
        assert_eq!(glyph(0), "⠋");
        assert_eq!(glyph(10), "⠋");
        assert_eq!(glyph(1), "⠙");
    }
}
```

- [ ] **Step 4: Test + commit**

Run: `cargo test --lib spinner && cargo test --lib theme`
Expected: pass.

```bash
git add src/theme.rs src/spinner.rs src/lib.rs
git commit -m "feat: theme palette/glyphs and braille spinner module"
```

---

## Task 2: `timeutil` module

**Files:**
- Create: `src/timeutil.rs`
- Modify: `src/lib.rs` (add `pub mod timeutil;` alphabetically — after `tmux`? no: after `theme`, before `tmux`)

- [ ] **Step 1: Add `pub mod timeutil;` to `src/lib.rs`** (place between `theme` and `tmux`):
```rust
pub mod theme;
pub mod timeutil;
pub mod tmux;
```

- [ ] **Step 2: Create `src/timeutil.rs`**

```rust
//! Dependency-free time helpers: age humanizer + local HH:MM via `date`.
use std::process::Command;

/// Human age from a duration in seconds: "0s","45s","3m","2h","5d".
pub fn humanize_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// Current local time as "HH:MM" via `date`. Empty string if it fails.
pub fn clock_hhmm() -> String {
    Command::new("date")
        .args(["+%H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Unix seconds now (for computing session age). 0 on failure.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(0), "0s");
        assert_eq!(humanize_age(45), "45s");
        assert_eq!(humanize_age(60), "1m");
        assert_eq!(humanize_age(185), "3m");
        assert_eq!(humanize_age(3600), "1h");
        assert_eq!(humanize_age(90_000), "1d");
        assert_eq!(humanize_age(-5), "0s");
    }
}
```

- [ ] **Step 3: Test + commit**

Run: `cargo test --lib timeutil`
Expected: pass.

```bash
git add src/timeutil.rs src/lib.rs
git commit -m "feat: timeutil (humanize_age, clock, now_unix)"
```

---

## Task 3: `git` module (read-only)

**Files:**
- Create: `src/git.rs`
- Modify: `src/lib.rs` (add `pub mod git;` after `config`)

- [ ] **Step 1: Add `pub mod git;` to `src/lib.rs`** (between `config` and `spinner`):
```rust
pub mod config;
pub mod git;
pub mod spinner;
```

- [ ] **Step 2: Create `src/git.rs`**

```rust
//! Read-only git info for a directory (branch + uncommitted diff stat).
//! Never mutates the repo; non-repo dirs return None.
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub added: u32,
    pub removed: u32,
}

/// Parse `git diff --shortstat` output → (insertions, deletions).
/// Examples:
///   "" → (0,0)
///   " 3 files changed, 12 insertions(+), 4 deletions(-)" → (12,4)
///   " 1 file changed, 5 insertions(+)" → (5,0)
///   " 1 file changed, 2 deletions(-)" → (0,2)
pub fn parse_shortstat(s: &str) -> (u32, u32) {
    let mut added = 0;
    let mut removed = 0;
    for part in s.split(',') {
        let p = part.trim();
        if let Some(n) = p.split_whitespace().next().and_then(|w| w.parse::<u32>().ok()) {
            if p.contains("insertion") {
                added = n;
            } else if p.contains("deletion") {
                removed = n;
            }
        }
    }
    (added, removed)
}

fn git_out(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Branch + working-tree diff stat for `dir`, or None if not a git repo.
pub fn read(dir: &str) -> Option<GitInfo> {
    let branch = git_out(dir, &["symbolic-ref", "--short", "HEAD"])
        .or_else(|| git_out(dir, &["rev-parse", "--short", "HEAD"]))?;
    let shortstat = git_out(dir, &["diff", "--shortstat"]).unwrap_or_default();
    let (added, removed) = parse_shortstat(&shortstat);
    Some(GitInfo { branch, added, removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_cases() {
        assert_eq!(parse_shortstat(""), (0, 0));
        assert_eq!(
            parse_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)"),
            (12, 4)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 5 insertions(+)"), (5, 0));
        assert_eq!(parse_shortstat(" 1 file changed, 2 deletions(-)"), (0, 2));
    }

    #[test]
    fn read_non_repo_is_none() {
        let dir = std::env::temp_dir().join(format!("cm_git_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read(dir.to_str().unwrap()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_repo_reports_branch_and_diff() {
        // Skip if git missing.
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("cm_git_repo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let run = |args: &[&str]| {
            Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a\nb\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        // modify tracked file → uncommitted diff
        std::fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();

        let info = read(d).expect("repo");
        assert_eq!(info.branch, "main");
        assert!(info.added >= 1, "expected at least one insertion, got {info:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 3: Test + commit**

Run: `cargo test --lib git`
Expected: pass (the repo test self-skips if `git` is absent).

```bash
git add src/git.rs src/lib.rs
git commit -m "feat: read-only git info (branch + diff shortstat)"
```

---

## Task 4: tmux `Status::Idle` rename, `Session.git`, colored capture; app wiring

**Files:**
- Modify: `src/tmux.rs`
- Modify: `src/app.rs`
- Modify: `src/ui.rs` (minimal: keep it compiling)

- [ ] **Step 1: In `src/tmux.rs`, rename the status enum and add the git field.**

Replace:
```rust
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
```
with:
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Running,
    Idle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub dir: String,
    pub created: i64,
    pub agent: String,
    pub status: Status,
    pub attached: bool,
    pub git: Option<crate::git::GitInfo>,
}
```

- [ ] **Step 2: In `parse_line` (src/tmux.rs), set the new fields.** The `Some(Session { ... status: Status::Waiting, attached: ... })` block becomes:
```rust
    Some(Session {
        name,
        dir,
        created,
        agent,
        status: Status::Idle,
        attached: f
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|n| n > 0)
            .unwrap_or(false),
        git: None,
    })
```
Also update the doc comment above `parse_sessions` ("status defaults to `Waiting`") to say `Idle`.

- [ ] **Step 3: Colored capture.** In `capture_pane`, change the args line
`.args(["capture-pane", "-p", "-t", name])` to
`.args(["capture-pane", "-p", "-e", "-t", name])`.

- [ ] **Step 4: Update any tmux test fixtures** that construct `Session` literally or reference `Status::Waiting` in `src/tmux.rs` (none currently build `Session` directly there, but `parses_managed_session` checks `status == Status::Waiting`). Change that assertion to `Status::Idle`. Search: `grep -n "Status::Waiting" src/tmux.rs`.

- [ ] **Step 5: In `src/app.rs`, update `compute_status` and its tests.**
Replace the body:
```rust
pub fn compute_status(prev: Option<u64>, current: u64) -> Status {
    match prev {
        Some(p) if p != current => Status::Running,
        _ => Status::Idle, // first observation OR content unchanged
    }
}
```
In the `#[cfg(test)] mod tests`, replace the three status tests' `Status::Waiting` with `Status::Idle` and rename them for clarity:
```rust
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
```
Also update `app_with_two_sessions()` test helper: both `status: Status::Waiting` → `Status::Idle`, and add `git: None,` to each `Session { ... }` literal.

- [ ] **Step 6: Add new `App` fields.** In the `App` struct add after `should_quit`:
```rust
    pub filter: Option<String>,
    pub spinner_frame: usize,
    pub now_unix: i64,
```
In `App::new`, initialize them:
```rust
            should_quit: false,
            filter: None,
            spinner_frame: 0,
            now_unix: crate::timeutil::now_unix(),
```

- [ ] **Step 7: Populate git + now_unix in `refresh`.** In `App::refresh`, inside `Ok(mut sessions) => {`, right after `let selected_name = self.selected_name();` add:
```rust
                self.now_unix = crate::timeutil::now_unix();
```
and inside the `for s in &mut sessions {` loop, after the `if let Ok(content) = ... { ... }` block (still inside the for), add:
```rust
                    s.git = crate::git::read(&s.dir);
```
(Place it as a separate statement inside the `for`, after the capture `if let`, so git is read for every session regardless of capture success.)

- [ ] **Step 8: Keep `src/ui.rs` compiling.** It currently matches `Status::Running`/`Status::Waiting` in `draw_sidebar`. Change the `Status::Waiting` arm to `Status::Idle`:
```rust
            let marker = match s.status {
                Status::Running => SPINNER,
                Status::Idle => READY,
            };
```
Also any `Session { ... }` literal in `src/ui.rs` tests must get `git: None,` and `status: Status::Idle` (the `renders_sidebar_and_footer` test builds a `Session`). Search: `grep -n "Status::\|Session {" src/ui.rs` and fix each.

- [ ] **Step 9: Build, test, commit.**

Run: `cargo build && cargo test 2>&1 | grep "test result"`
Expected: compiles; all tests pass (status tests renamed, git field threaded).

```bash
git add src/tmux.rs src/app.rs src/ui.rs
git commit -m "refactor: Status::Idle, Session.git, colored capture, app wiring"
```

---

## Task 5: fuzzy filter + g/G navigation (app logic)

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add a `Filter` mode.** In `enum Mode` add `Filter` (no payload — text lives in `App.filter`):
```rust
pub enum Mode {
    List,
    Create(CreateForm),
    Rename(RenameForm),
    ConfirmDelete(String),
    Help,
    Filter,
}
```
In `enum ModeKind` add `Filter`, and in `mode_kind()` add `Mode::Filter => ModeKind::Filter,`. In `handle_key`, add `ModeKind::Filter => self.handle_filter_key(key),`.

- [ ] **Step 2: Add visible-list helpers.** Add these methods to `impl App` (after `selected_name`):
```rust
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
```
Then replace `selected_name` to go through the visible mapping:
```rust
    pub fn selected_name(&self) -> Option<String> {
        self.selected_session().map(|s| s.name.clone())
    }
```

- [ ] **Step 3: Make navigation operate over the visible list.** Replace `select_next`, `select_prev`, `clamp_selection`:
```rust
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
```

- [ ] **Step 4: Fix the preview mapping in `refresh`.** It currently compares `selected_name` by name — that still works because `selected_name()` now maps through the filter. No change needed there beyond Task 4. Verify `refresh` still references `self.selected_name()` (it does).

- [ ] **Step 5: Wire `/`, `g`, `G` in `handle_list_key`.** Add arms (before the `_ => {}`):
```rust
            KeyCode::Char('/') => {
                self.filter = Some(String::new());
                self.selected = 0;
                self.mode = Mode::Filter;
            }
            KeyCode::Char('g') => self.select_first(),
            KeyCode::Char('G') => self.select_last(),
```

- [ ] **Step 6: Add `handle_filter_key`.** Add this method to `impl App`:
```rust
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
```

- [ ] **Step 7: Tests.** Add to `#[cfg(test)] mod tests` in `src/app.rs` (the `app_with_two_sessions` helper exists; sessions are named "a" and "b"):
```rust
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
```

- [ ] **Step 8: Build, test, commit.**

Run: `cargo test --lib app 2>&1 | grep "test result"`
Expected: pass.

```bash
git add src/app.rs
git commit -m "feat: substring filter mode and g/G navigation"
```

---

## Task 6: agent selector in `CreateForm` (app logic)

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Extend `CreateForm`.** Add fields:
```rust
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
```

- [ ] **Step 2: Change `CreateForm::new` to take presets and build choices.** Replace the function:
```rust
    pub fn new(default_agent: &str, presets: &[String]) -> Self {
        // choices = configured presets (deduped, default first) + a custom slot
        let mut choices: Vec<String> = Vec::new();
        choices.push(default_agent.to_string());
        for p in presets {
            if !choices.contains(p) {
                choices.push(p.clone());
            }
        }
        choices.push("custom…".to_string());
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
```

- [ ] **Step 3: Add agent-cycling + the "custom" sentinel.** Add methods to `impl CreateForm`:
```rust
    /// True when the current choice is the free-text "custom…" slot.
    fn agent_is_custom(&self) -> bool {
        self.agent_choices
            .get(self.agent_index)
            .map(|c| c == "custom…")
            .unwrap_or(false)
    }

    /// Move agent selection by `delta` (wraps); sets `agent` to the chosen command,
    /// or clears it for the custom slot so the user can type a command.
    fn cycle_agent(&mut self, delta: isize) {
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
```

- [ ] **Step 4: Handle ←/→ and typing in the agent step.** In `handle_create_key`, the "Name / agent steps" match currently treats `Char(c)` and `Tab`/`Enter` generically. We need: in the Agent field, `Left`/`Right` cycle the agent, and typing only edits `agent` when on the custom slot. Replace the `KeyCode::Char(c) => form.current_mut().push(c),` arm and add Left/Right handling — restructure that match to:
```rust
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Left if form.field == CreateField::Agent => form.cycle_agent(-1),
            KeyCode::Right if form.field == CreateField::Agent => form.cycle_agent(1),
            KeyCode::Backspace => {
                if form.field == CreateField::Agent && !form.agent_is_custom() {
                    // editing a preset turns it into a custom command
                    form.agent_index = form.agent_choices.len().saturating_sub(1);
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
                // (unchanged Agent-submit / Name-advance logic below)
```
Keep the rest of the `Enter` arm and the trailing `self.mode = Mode::Create(form); None` exactly as they are.

- [ ] **Step 5: Add a command resolver (display only).** Add a module-level function near `expand_tilde`:
```rust
/// Resolves the first word of `cmd` on PATH via `command -v`. Returns the path,
/// or None if not found / empty.
pub fn resolve_agent_path(cmd: &str) -> Option<String> {
    let bin = cmd.split_whitespace().next()?;
    if bin.is_empty() {
        return None;
    }
    let out = std::process::Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
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
```

- [ ] **Step 6: Update the `CreateForm::new` call site.** In `handle_list_key`, the `'n'` arm:
```rust
            KeyCode::Char('n') => {
                self.error = None;
                self.mode = Mode::Create(CreateForm::new(
                    &self.config.default_agent,
                    &self.config.agent_presets,
                ));
            }
```

- [ ] **Step 7: Fix the existing `n_opens_create_form_prefilled_with_default_agent` test** if it calls `CreateForm::new("...")` with one arg — it goes through `handle_key(key('n'))` so it's fine; but `create_form_starts_in_home_dir` calls `CreateForm::new("claude")`. Update those direct calls to `CreateForm::new("claude", &[])`. Search: `grep -n "CreateForm::new(" src/app.rs` and fix test call sites.

- [ ] **Step 8: Add agent-selector tests.**
```rust
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
    }
```

- [ ] **Step 9: Build, test, commit.**

Run: `cargo test --lib app 2>&1 | grep "test result"`
Expected: pass.

```bash
git add src/app.rs
git commit -m "feat: agent segmented selector + command resolver in create form"
```

---

## Task 7: `ui/` scaffold — theme, header, footer, 40/60 layout

**Files:**
- Delete: `src/ui.rs`
- Create: `src/ui/mod.rs`, `src/ui/header.rs`, `src/ui/footer.rs`
- (sessions/preview/modals temporarily live as functions inside `mod.rs`, ported from old `ui.rs`, then split out in later tasks)

This task reorganizes without losing behavior, applies theme colors, and adds the header + bottom footer + 40/60 split. The old modal/list/preview rendering is ported into `mod.rs` (themed) and split into files in Tasks 8–10.

- [ ] **Step 1: Move the file.** `git mv src/ui.rs src/ui/mod.rs` (create the `src/ui/` dir). `lib.rs` already says `pub mod ui;` which now resolves to `ui/mod.rs`.

- [ ] **Step 2: At the top of `src/ui/mod.rs`, declare submodules and import theme:**
```rust
mod header;
mod footer;

use crate::app::{App, CreateField, Mode};
use crate::theme as th;
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
```

- [ ] **Step 3: Rewrite `draw` to the header/body/footer layout.** Replace the existing `pub fn draw` with:
```rust
pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::vertical([
        Constraint::Length(2), // header + rule
        Constraint::Min(1),    // body
        Constraint::Length(2), // footer rule + keys
    ])
    .split(f.area());

    header::render(f, root[0], app);
    draw_body(f, root[1], app);
    footer::render(f, root[2], app);

    match &app.mode {
        Mode::Create(_) => draw_create_modal(f, app),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete(name) => draw_confirm_modal(f, name),
        Mode::Help => draw_help_modal(f),
        Mode::List | Mode::Filter => {}
    }
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);
    draw_sessions(f, cols[0], app);
    // vertical separator
    f.render_widget(
        Block::default().borders(Borders::LEFT).border_style(Style::default().fg(th::BORDER)),
        cols[1],
    );
    draw_preview(f, cols[2], app);
}
```
Rename the old `draw_sidebar` to `draw_sessions` and the old `draw_preview` stays named `draw_preview` (both are restyled in Tasks 8–9; for now port them with theme colors — minimal: set list/paragraph `Style::default().fg(th::TEXT)` and selected highlight to `Style::default().bg(th::SEL_BG).fg(th::AMBER_HI).add_modifier(Modifier::BOLD)` with `highlight_symbol(format!("{} ", th::SEL_BAR))`). Delete the old standalone `draw_footer` (footer is now `footer::render`).

- [ ] **Step 4: Create `src/ui/header.rs`:**
```rust
use crate::app::App;
use crate::spinner;
use crate::theme as th;
use crate::timeutil;
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let total = app.sessions.len();
    let running = app.sessions.iter().filter(|s| s.status == Status::Running).count();
    let idle = total - running;
    let spin = spinner::glyph(app.spinner_frame);
    let clock = timeutil::clock_hhmm();

    let mut spans = vec![
        Span::styled(th::LOGO, Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
        Span::styled(" cm", Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
        Span::styled("  claude · session manager", Style::default().fg(th::DIM)),
        Span::styled(format!("  {}  ", th::SEP), Style::default().fg(th::DIM)),
        Span::styled(format!("{total}"), Style::default().fg(th::TEXT_BOLD)),
        Span::styled(" sessions   ", Style::default().fg(th::MUTED)),
        Span::styled(spin, Style::default().fg(th::AMBER_HI)),
        Span::styled(format!(" {running} running", ), Style::default().fg(th::AMBER_HI)),
        Span::styled(format!("   {} {idle} idle", th::IDLE_DOT), Style::default().fg(th::DIM)),
    ];
    if !clock.is_empty() {
        // right-pad-ish: just append; alignment to the right edge is approximate.
        spans.push(Span::styled(format!("    {clock}"), Style::default().fg(th::MUTED)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize))
            .style(Style::default().fg(th::BORDER)),
        rows[1],
    );
}
```
Add to `theme.rs`: `pub const RULE_CHAR: &str = "━";` (if not already present — Task 1 didn't add it; add it now in this task or Task 1; ensure it exists).

- [ ] **Step 5: Create `src/ui/footer.rs`:**
```rust
use crate::app::{App, Mode};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// (key, label, accent)
type Item = (&'static str, &'static str, bool);

fn items_for(mode: &Mode) -> Vec<Item> {
    match mode {
        Mode::Create(_) => vec![
            ("↵", "create", true), ("⇥", "next field", false),
            ("←→", "pick agent", false), ("esc", "cancel", false),
        ],
        Mode::ConfirmDelete(_) => vec![
            ("y", "yes, kill", true), ("n", "no", false), ("esc", "cancel", false),
        ],
        Mode::Help => vec![("esc", "close", true), ("q", "quit", false)],
        Mode::Rename(_) => vec![("↵", "rename", true), ("esc", "cancel", false)],
        Mode::Filter => vec![
            ("type", "filter", true), ("↑↓", "move", false), ("esc", "clear", false),
        ],
        Mode::List => vec![
            ("n", "new", true), ("↵", "attach", false), ("d", "kill", false),
            ("r", "rename", false), ("/", "filter", false), ("?", "help", false),
            ("q", "quit", false),
        ],
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize))
            .style(Style::default().fg(th::BORDER)),
        rows[0],
    );
    let mut spans: Vec<Span> = Vec::new();
    for (i, (k, label, accent)) in items_for(&app.mode).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", Style::default().fg(th::DIM)));
        }
        let key_style = if accent {
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(th::TEXT_BOLD)
        };
        spans.push(Span::styled(k, key_style));
        spans.push(Span::styled(format!(" {label}"), Style::default().fg(th::MUTED)));
    }
    if let Some(q) = &app.filter {
        spans.push(Span::styled(format!("    /{q}"), Style::default().fg(th::AMBER_HI)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}
```

- [ ] **Step 6: Update `src/ui/mod.rs` tests.** The existing snapshot tests (`renders_sidebar_and_footer`, etc.) assert old footer text like `"[n] new"`. Update assertions to the new footer (`"n"` + `"new"` present, `"sessions"` present from header). Keep `buf_to_string` helper. Adjust each test to render via `draw` and assert on themed strings (e.g. `assert!(text.contains("cm"))`, `assert!(text.contains("new"))`).

- [ ] **Step 7: Build, test, commit.**

Run: `cargo build && cargo test --lib ui 2>&1 | grep "test result"`
Expected: compiles; ui tests pass.

```bash
git add src/ui/ && git rm --cached src/ui.rs 2>/dev/null; git add -A src/ui*
git commit -m "feat: ui module split with themed header + bottom footer + 40/60"
```

---

## Task 8: `ui/sessions.rs` — multi-line session cards

**Files:**
- Create: `src/ui/sessions.rs`
- Modify: `src/ui/mod.rs` (declare `mod sessions;`, call `sessions::render`, remove the ported `draw_sessions`)

- [ ] **Step 1: Create `src/ui/sessions.rs`:**
```rust
use crate::app::App;
use crate::spinner;
use crate::theme as th;
use crate::timeutil;
use crate::tmux::{Session, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

fn card(s: &Session, now: i64, spinner_frame: usize) -> ListItem<'static> {
    // Line 1: name ......... status
    let (status_glyph, status_label, status_color) = match s.status {
        Status::Running => (spinner::glyph(spinner_frame).to_string(), "running", th::AMBER_HI),
        Status::Idle => (th::IDLE_DOT.to_string(), "idle", th::MUTED),
    };
    let line1 = Line::from(vec![
        Span::styled(s.name.clone(), Style::default().fg(th::TEXT_BOLD)),
        Span::raw("  "),
        Span::styled(status_glyph, Style::default().fg(status_color)),
        Span::styled(format!(" {status_label}"), Style::default().fg(status_color)),
    ]);
    // Line 2: dir
    let line2 = Line::from(Span::styled(s.dir.clone(), Style::default().fg(th::MUTED)));
    // Line 3: ✻ agent · ⎇ branch · +a −d   age
    let mut l3 = vec![
        Span::styled(th::AGENT_MARK, Style::default().fg(th::DIM)),
        Span::styled(format!(" {}", s.agent), Style::default().fg(th::MUTED)),
    ];
    if let Some(g) = &s.git {
        l3.push(Span::styled(format!("   {} {}", th::BRANCH, g.branch), Style::default().fg(th::DIM)));
        l3.push(Span::styled(format!("   +{}", g.added), Style::default().fg(th::GREEN)));
        l3.push(Span::styled(format!(" −{}", g.removed), Style::default().fg(th::RED)));
    }
    let age = timeutil::humanize_age(now - s.created);
    l3.push(Span::styled(format!("   {age}"), Style::default().fg(th::MUTED)));
    let line3 = Line::from(l3);
    ListItem::new(vec![line1, line2, line3])
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    // Section label
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SESSIONS", Style::default().fg(th::MUTED)),
            Span::styled("   ↑↓ navigate", Style::default().fg(th::DIM)),
        ])),
        rows[0],
    );

    let vis = app.visible_indices();
    let items: Vec<ListItem> = vis
        .iter()
        .map(|&i| card(&app.sessions[i], app.now_unix, app.spinner_frame))
        .collect();

    let list = List::new(items)
        .highlight_symbol(&format!("{} ", th::SEL_BAR))
        .highlight_style(
            Style::default().bg(th::SEL_BG).fg(th::AMBER_HI).add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    if !vis.is_empty() {
        state.select(Some(app.selected.min(vis.len() - 1)));
    }
    f.render_stateful_widget(list, rows[1], &mut state);
}
```
> Note: if `highlight_symbol` requires a `&'static str` / owned-string lifetime issue arises with the `format!`, bind it to a `let sym = format!(...)` before `List::new(...)` and pass `sym.as_str()`. Adapt as needed.

- [ ] **Step 2: Wire it in `src/ui/mod.rs`.** Add `mod sessions;` near the other `mod` lines; in `draw_body`, replace the `draw_sessions(f, cols[0], app);` call site to `sessions::render(f, cols[0], app);` and delete the old ported `draw_sessions` fn (and the now-unused `Status`/`List` imports if they cause unused warnings — keep what preview/modals still use).

- [ ] **Step 3: Snapshot tests** in `src/ui/sessions.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::git::GitInfo;
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

    fn sess(name: &str, status: Status, git: Option<GitInfo>) -> Session {
        Session {
            name: name.into(), dir: "~/work/x".into(), created: 0,
            agent: "claude".into(), status, attached: false, git,
        }
    }

    #[test]
    fn renders_running_and_idle_cards_with_git() {
        let mut app = App::new(Config::default());
        app.now_unix = 185; // → "3m" age
        app.spinner_frame = 0;
        app.sessions = vec![
            sess("project-a", Status::Running, Some(GitInfo { branch: "main".into(), added: 12, removed: 4 })),
            sess("project-b", Status::Idle, None),
        ];
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("project-a"));
        assert!(s.contains("running"));
        assert!(s.contains("⠋")); // spinner frame 0
        assert!(s.contains("main"));
        assert!(s.contains("+12"));
        assert!(s.contains("idle"));
        assert!(s.contains("3m"));
    }
}
```

- [ ] **Step 4: Build, test, commit.**

Run: `cargo test --lib ui 2>&1 | grep "test result"`
Expected: pass.

```bash
git add src/ui/sessions.rs src/ui/mod.rs
git commit -m "feat: multi-line session cards (status, agent, git, age)"
```

---

## Task 9: `ui/preview.rs` — colored preview via ansi-to-tui

**Files:**
- Modify: `Cargo.toml` (add `ansi-to-tui`)
- Create: `src/ui/preview.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Add the dependency.** In `Cargo.toml` `[dependencies]` add:
```toml
ansi-to-tui = "7"
```
Run `cargo build` once to resolve (it pulls a compatible 7.x). If 7 fails to resolve, report the available version rather than guessing.

- [ ] **Step 2: Create `src/ui/preview.rs`:**
```rust
use crate::app::App;
use crate::theme as th;
use crate::timeutil;
use ansi_to_tui::IntoText;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // path · branch
        Constraint::Length(1), // rule
        Constraint::Min(0),    // content
    ])
    .split(area);

    let sel = app.selected_session();
    let title = sel.map(|s| s.name.as_str()).unwrap_or("preview");
    let age = sel.map(|s| timeutil::humanize_age(app.now_unix - s.created)).unwrap_or_default();

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{} ", th::PREVIEW_MARK), Style::default().fg(th::AMBER)),
            Span::styled(title.to_string(), Style::default().fg(th::TEXT_BOLD)),
            Span::styled(format!("    {age}"), Style::default().fg(th::DIM)),
        ])),
        rows[0],
    );

    let mut sub = vec![Span::styled(
        sel.map(|s| s.dir.clone()).unwrap_or_default(),
        Style::default().fg(th::MUTED),
    )];
    if let Some(g) = sel.and_then(|s| s.git.as_ref()) {
        sub.push(Span::styled(format!(" {} {} {}", th::IDLE_DOT, th::BRANCH, g.branch), Style::default().fg(th::DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(sub)), rows[1]);

    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(Style::default().fg(th::BORDER)),
        rows[2],
    );

    // Content: parse ANSI from capture-pane into styled Text; fall back to plain.
    let text: Text = app.preview.as_str().into_text().unwrap_or_else(|_| Text::raw(app.preview.clone()));
    f.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).style(Style::default().fg(th::TEXT)),
        rows[3],
    );
}
```

- [ ] **Step 3: Wire it.** In `src/ui/mod.rs` add `mod preview;`, replace the `draw_preview(...)` call in `draw_body` with `preview::render(f, cols[2], app);`, and delete the old ported `draw_preview` fn + its now-unused imports.

- [ ] **Step 4: Snapshot test** in `src/ui/preview.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tmux::{Session, Status};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn buf_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf[(x, y)].symbol()); }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_title_and_ansi_content() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(), dir: "~/work/proj".into(), created: 0,
            agent: "claude".into(), status: Status::Idle, attached: false, git: None,
        }];
        app.now_unix = 0;
        // ANSI green "hello" — ansi-to-tui must not leave escape bytes in the buffer
        app.preview = "\u{1b}[32mhello\u{1b}[0m world".into();
        let mut t = Terminal::new(TestBackend::new(50, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("proj"));
        assert!(s.contains("hello"));
        assert!(s.contains("world"));
        assert!(!s.contains('\u{1b}')); // escapes consumed, not rendered literally
    }
}
```

- [ ] **Step 5: Build, test, commit.**

Run: `cargo build && cargo test --lib ui::preview 2>&1 | grep "test result"`
Expected: pass.

```bash
git add Cargo.toml Cargo.lock src/ui/preview.rs src/ui/mod.rs
git commit -m "feat: colored preview via ansi-to-tui"
```

---

## Task 10: restyled modals — new / kill / help

**Files:**
- Create: `src/ui/modal_new.rs`, `src/ui/modal_kill.rs`, `src/ui/modal_help.rs`
- Modify: `src/ui/mod.rs` (declare modules; delegate; keep `centered`)

- [ ] **Step 1: Keep `centered` in `mod.rs`** (it already exists). Ensure it's `pub(crate)` so submodules can use it: change `fn centered` → `pub(crate) fn centered`.

- [ ] **Step 2: Create `src/ui/modal_help.rs`** (simplest first — port the 4-group help, themed):
```rust
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame) {
    let area = super::centered(70, 70, f.area());
    f.render_widget(Clear, area);
    let groups: [(&str, &[(&str, &str)]); 4] = [
        ("Navigation", &[("k j / ↑↓", "move"), ("g G", "first · last"), ("/", "filter")]),
        ("Session", &[("↵ o", "attach"), ("n", "new"), ("d", "kill"), ("r", "rename")]),
        ("Preview", &[("auto", "refresh on interval")]),
        ("App", &[("?", "help"), ("q", "quit (sessions stay)")]),
    ];
    let mut lines = vec![Line::from(vec![
        Span::styled("? Help", Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
        Span::styled("   keys & shortcuts", Style::default().fg(th::MUTED)),
    ]), Line::from("")];
    for (title, items) in groups {
        lines.push(Line::from(Span::styled(title, Style::default().fg(th::MUTED).add_modifier(Modifier::BOLD))));
        for (k, label) in items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::default().fg(th::AMBER_HI)),
                Span::styled(*label, Style::default().fg(th::TEXT)),
            ]));
        }
        lines.push(Line::from(""));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(th::BORDER_HI)).title(" help "),
        ),
        area,
    );
}
```

- [ ] **Step 3: Create `src/ui/modal_kill.rs`:**
```rust
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, name: &str) {
    let area = super::centered(56, 36, f.area());
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![
            Span::styled("✕ ", Style::default().fg(th::RED).add_modifier(Modifier::BOLD)),
            Span::styled("Kill session?", Style::default().fg(th::TEXT_BOLD).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled(name.to_string(), Style::default().fg(th::AMBER))),
        Line::from(""),
        Line::from(Span::styled(
            "Stops the agent process and discards unsaved scratch. Files on disk are unaffected.",
            Style::default().fg(th::DIM),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y · yes, kill ", Style::default().bg(th::RED).fg(th::BG)),
            Span::raw("  "),
            Span::styled(" n · no ", Style::default().fg(th::TEXT)),
            Span::styled("     esc to dismiss", Style::default().fg(th::DIM)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(th::RED)).title(" confirm "),
        ),
        area,
    );
}
```

- [ ] **Step 4: Create `src/ui/modal_new.rs`** (name/dir + dir picker list + agent selector). It renders the `CreateForm`:
```rust
use crate::app::{resolve_agent_path, CreateField, CreateForm};
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let marker = if focused { "▌ " } else { "  " };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(if focused { th::AMBER } else { th::BORDER })),
        Span::styled(format!("{label}: "), Style::default().fg(th::MUTED)),
        Span::styled(value.to_string(), Style::default().fg(th::TEXT_BOLD)),
    ])
}

pub fn render(f: &mut Frame, form: &CreateForm) {
    let area = super::centered(64, 80, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![
        Line::from(Span::styled("＋ New session", Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD))),
        Line::from(""),
        field_line("name ", &form.name, form.field == CreateField::Name),
        field_line("dir  ", &form.dir, form.field == CreateField::Dir),
    ];

    if form.field == CreateField::Dir {
        // windowed subdir list (selection visible) — same approach as before
        for (i, name) in form.dir_entries.iter().enumerate() {
            let selected = i == form.dir_selected;
            let text = format!("    {}{}/", if selected { "> " } else { "  " }, name);
            let style = if selected {
                Style::default().fg(th::AMBER_HI).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(th::MUTED)
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }

    // Agent segmented selector
    let mut seg: Vec<Span> = vec![Span::styled("  agent: ", Style::default().fg(th::AMBER))];
    for (i, choice) in form.agent_choices.iter().enumerate() {
        let sel = i == form.agent_index;
        let st = if sel {
            Style::default().bg(th::AMBER).fg(th::BG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(th::MUTED)
        };
        seg.push(Span::styled(format!(" {choice} "), st));
        seg.push(Span::raw(" "));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(seg));

    // Resolved command line
    let resolved = resolve_agent_path(&form.agent);
    let cmd = if form.agent.is_empty() { "<type a command>".to_string() } else { form.agent.clone() };
    lines.push(Line::from(vec![
        Span::styled("  $ ", Style::default().fg(th::DIM)),
        Span::styled(cmd, Style::default().fg(th::TEXT_BOLD)),
    ]));
    lines.push(Line::from(Span::styled(
        match &resolved {
            Some(p) => format!("  found at {p}"),
            None => "  not found in PATH".to_string(),
        },
        Style::default().fg(if resolved.is_some() { th::DIM } else { th::YELLOW }),
    )));

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(th::AMBER)).title(" new session "),
        ),
        area,
    );
}
```

- [ ] **Step 5: Wire modals in `src/ui/mod.rs`.** Add `mod modal_new; mod modal_kill; mod modal_help;`. In `draw`, replace the modal dispatch:
```rust
    match &app.mode {
        Mode::Create(form) => modal_new::render(f, form),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete(name) => modal_kill::render(f, name),
        Mode::Help => modal_help::render(f),
        Mode::List | Mode::Filter => {}
    }
```
Keep the existing `draw_rename_modal` in `mod.rs` (themed: amber border). Delete the old `draw_create_modal`/`draw_confirm_modal`/`draw_help_modal` ported functions.

- [ ] **Step 6: Snapshot tests** (add to each modal file). Example for `modal_new.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn buf_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width { s.push_str(buf[(x, y)].symbol()); }
            s.push('\n');
        }
        s
    }

    #[test]
    fn new_modal_shows_fields_and_agent_segments() {
        let form = CreateForm::new("claude", &["claude".into(), "codex".into()]);
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("New session"));
        assert!(s.contains("name"));
        assert!(s.contains("claude"));
        assert!(s.contains("codex"));
        assert!(s.contains("custom"));
    }
}
```
Add equivalent small tests for `modal_kill` (`assert s.contains("Kill session?")` and the name) and `modal_help` (`assert s.contains("Help")` and `"attach"`).

- [ ] **Step 7: Build, test, commit.**

Run: `cargo test --lib ui 2>&1 | grep "test result"`
Expected: pass.

```bash
git add src/ui/modal_new.rs src/ui/modal_kill.rs src/ui/modal_help.rs src/ui/mod.rs
git commit -m "feat: restyled new/kill/help modals + agent selector render"
```

---

## Task 11: empty + tmux-error screens; event loop tick/throttle/spinner

**Files:**
- Create: `src/ui/empty.rs`, `src/ui/error.rs`
- Modify: `src/ui/mod.rs`, `src/app.rs`, `src/main.rs`

- [ ] **Step 1: Add a fatal/error state to `App`.** In `src/app.rs`, add to `App`: `pub tmux_missing: bool,` and init `tmux_missing: false` in `new`. (When set, `main` shows the error screen and only `q` quits.)

- [ ] **Step 2: Create `src/ui/empty.rs`:**
```rust
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect) {
    let v = Layout::vertical([Constraint::Percentage(38), Constraint::Min(0)]).split(area);
    let lines = vec![
        Line::from(Span::styled(th::LOGO, Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("No sessions yet", Style::default().fg(th::TEXT_BOLD).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("Spin up an agent in any directory. Sessions keep running after you quit.", Style::default().fg(th::MUTED))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" n ", Style::default().bg(th::AMBER).fg(th::BG).add_modifier(Modifier::BOLD)),
            Span::styled("  start your first session", Style::default().fg(th::TEXT)),
        ]),
    ];
    let p = Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(p, v[1]);
}
```

- [ ] **Step 3: Create `src/ui/error.rs`:**
```rust
use crate::theme as th;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame) {
    let v = Layout::vertical([Constraint::Percentage(30), Constraint::Min(0)]).split(f.area());
    let lines = vec![
        Line::from(Span::styled(th::LOGO, Style::default().fg(th::RED).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("tmux not found in PATH", Style::default().fg(th::TEXT_BOLD).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("cm manages tmux sessions and needs the tmux binary.", Style::default().fg(th::MUTED))),
        Line::from(""),
        Line::from(Span::styled("  macOS    brew install tmux", Style::default().fg(th::TEXT))),
        Line::from(Span::styled("  Ubuntu   sudo apt install tmux", Style::default().fg(th::TEXT))),
        Line::from(Span::styled("  Arch     sudo pacman -S tmux", Style::default().fg(th::TEXT))),
        Line::from(""),
        Line::from(Span::styled("  q to quit", Style::default().fg(th::DIM))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center), v[1]);
}
```

- [ ] **Step 4: Wire empty + error into `draw`.** In `src/ui/mod.rs` add `mod empty; mod error;`. At the top of `draw`, before the layout:
```rust
    if app.tmux_missing {
        error::render(f);
        return;
    }
```
In `draw_body`, when there are no sessions show the empty state instead of the split:
```rust
fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    if app.sessions.is_empty() {
        empty::render(f, area);
        return;
    }
    // ... existing split ...
}
```

- [ ] **Step 5: Rewrite the `main.rs` loop for ticking + throttle + spinner + error.** Replace `main`, `run`:
```rust
use std::time::{Duration, Instant};

fn main() -> io::Result<()> {
    let config = Config::load();
    let refresh = Duration::from_millis(config.refresh_interval_ms.max(100));
    let mut app = App::new(config);
    if !tmux::is_available() {
        app.tmux_missing = true;
    } else {
        app.refresh();
    }

    install_panic_hook();
    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, refresh);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)
}

fn run(terminal: &mut Term, app: &mut App, refresh: Duration) -> io::Result<()> {
    let start = Instant::now();
    let tick = Duration::from_millis(80);
    let mut last_refresh = Instant::now();
    loop {
        app.spinner_frame = cm::spinner::frame_index(start.elapsed().as_millis());
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if app.tmux_missing {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    continue;
                }
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action)?;
                }
            }
        }

        if !app.tmux_missing && last_refresh.elapsed() >= refresh {
            app.refresh();
            last_refresh = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
```
(`handle_action`, `init_terminal`, `restore_terminal`, `install_panic_hook` stay as-is. Add `use std::time::Instant;` — replace the existing `use std::time::Duration;` line.)
Exit code on tmux-missing: after the loop, if `app.tmux_missing` you may `std::process::exit(1)` from `main` after restoring; simplest: in `main`, change the tail to:
```rust
    let restore = restore_terminal(&mut terminal);
    result.and(restore)?;
    if app.tmux_missing {
        std::process::exit(1);
    }
    Ok(())
```

- [ ] **Step 6: Empty/error snapshot tests** (add to `empty.rs`/`error.rs`):
```rust
// empty.rs
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    #[test]
    fn empty_mentions_no_sessions() {
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area())).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height { for x in 0..b.area.width { s.push_str(b[(x,y)].symbol()); } }
        assert!(s.contains("No sessions yet"));
    }
}
```
```rust
// error.rs
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    #[test]
    fn error_mentions_tmux_and_install() {
        let mut t = Terminal::new(TestBackend::new(60, 18)).unwrap();
        t.draw(|f| render(f)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height { for x in 0..b.area.width { s.push_str(b[(x,y)].symbol()); } }
        assert!(s.contains("tmux not found"));
        assert!(s.contains("brew install tmux"));
    }
}
```

- [ ] **Step 7: Full build, lint, test.**

Run: `cargo build --release && cargo clippy --all-targets -- -D warnings 2>&1 | tail -5 && cargo test 2>&1 | grep "test result"`
Expected: release build clean, clippy clean, all tests pass.

- [ ] **Step 8: Manual smoke test** (real TTY, by the human): `cargo run` → warm theme, header with counts + spinner, 40/60, multi-line cards, `/` filter, `g`/`G`, `n` modal with agent segments, `d` kill, `?` help, empty state when no sessions. (Spinner animation and colors can't be verified in a non-TTY; defer to human.)

- [ ] **Step 9: Commit.**

```bash
git add src/ui/empty.rs src/ui/error.rs src/ui/mod.rs src/app.rs src/main.rs
git commit -m "feat: empty + tmux-error screens; animated tick loop with throttled refresh"
```

---

## Self-Review Notes (for the implementer)

- **Spec coverage:** theme/palette (T1), glyphs (T1), spinner anim (T1+T11), header counts/clock/spinner (T7), bottom footer (T7), 40/60 (T7), multi-line cards w/ selection bar+agent+git+age (T8), colored preview (T9), Status→Idle 2-state (T4), git read-only (T3+T4), new modal + agent selector + resolved command (T6+T10), kill modal (T10), help modal (T10), empty (T11), tmux-error screen (T11), `/` filter (T5), g/G (T5), throttled refresh + 80ms tick (T11). Out-of-scope (overlay, 3rd status, c/space/p/shift+r/ctrl-r) correctly excluded.
- **Type consistency:** `Status::{Running,Idle}`, `Session.git: Option<GitInfo>`, `GitInfo{branch,added,removed}`, `App.{filter,spinner_frame,now_unix,tmux_missing}`, `CreateForm.{agent_choices,agent_index}` + `new(default, presets)`, `resolve_agent_path`, `spinner::{frame_index,glyph}`, `timeutil::{humanize_age,clock_hhmm,now_unix}`, `theme::*`, ui submodule `render` fns — all used consistently across tasks.
- **Build-green ordering:** logic/modules (T1–T6) land before the UI rewrite (T7–T11); T4 keeps old `ui.rs` compiling via the minimal `Status::Idle` arm fix; T7 moves `ui.rs`→`ui/mod.rs` wholesale; later UI tasks delete ported fns as they split them out.
- **Known adapt points:** ratatui 0.29 `highlight_symbol` lifetime (T8 note), `into_text()` import path for `ansi-to-tui` v7 (T9), right-alignment of header clock is approximate (acceptable).
```
