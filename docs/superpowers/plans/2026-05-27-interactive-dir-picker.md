# Interactive Directory Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the free-text `dir` step of `cm`'s create form with an interactive picker: editable path + a live list of the current path's subdirectories that you filter by typing, navigate with ↑↓, descend into with Tab/→, and confirm with Enter.

**Architecture:** A new `browse` module isolates pure logic (`split_path`, `filter_subdirs`) from filesystem IO (`read_subdirs`). `CreateForm` gains `dir_entries`/`dir_selected` plus methods that recompute the listing whenever the path text changes; `handle_create_key` gets a dedicated branch for the dir step. `ui::draw_create_modal` renders the list when the dir field is focused.

**Tech Stack:** Rust (edition 2021), ratatui 0.29, crossterm 0.28. Builds on the existing `cm` crate (`src/app.rs`, `src/ui.rs`, `src/lib.rs`).

**Reference spec:** `docs/superpowers/specs/2026-05-27-interactive-dir-picker-design.md`

---

## Task 1: `browse` module — path splitting, filtering, subdir listing

**Files:**
- Create: `src/browse.rs`
- Modify: `src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/browse.rs`

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add `pub mod browse;` so the file reads (keep alphabetical):
```rust
pub mod app;
pub mod browse;
pub mod config;
pub mod tmux;
pub mod ui;
```

- [ ] **Step 2: Write `src/browse.rs` with implementation + failing tests**

Create `src/browse.rs`:
```rust
use std::path::Path;

/// Splits a path into (directory-part-including-trailing-slash, trailing-segment).
/// No '/' present → ("", text).
pub fn split_path(text: &str) -> (String, String) {
    match text.rfind('/') {
        Some(i) => (text[..=i].to_string(), text[i + 1..].to_string()),
        None => (String::new(), text.to_string()),
    }
}

/// Case-insensitive prefix filter over directory names. Hides names starting with
/// '.' unless `filter` itself starts with '.'. Sorted case-insensitively.
pub fn filter_subdirs(names: &[String], filter: &str) -> Vec<String> {
    let lower = filter.to_lowercase();
    let show_hidden = filter.starts_with('.');
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| show_hidden || !n.starts_with('.'))
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .cloned()
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Immediate subdirectory names of `base`. Missing/unreadable path → empty list.
pub fn read_subdirs(base: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Convenience: subdirectories of `base` filtered by `filter`.
pub fn list(base: &str, filter: &str) -> Vec<String> {
    filter_subdirs(&read_subdirs(Path::new(base)), filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_handles_mid_trailing_and_none() {
        assert_eq!(
            split_path("~/projects/pets/ag"),
            ("~/projects/pets/".to_string(), "ag".to_string())
        );
        assert_eq!(
            split_path("~/projects/pets/"),
            ("~/projects/pets/".to_string(), "".to_string())
        );
        assert_eq!(split_path("ag"), ("".to_string(), "ag".to_string()));
    }

    #[test]
    fn filter_subdirs_prefix_case_insensitive_sorted() {
        let names = vec![
            "notes".to_string(),
            "agents".to_string(),
            "Apps".to_string(),
            ".git".to_string(),
        ];
        // case-insensitive prefix "a" matches agents + Apps, sorted by lowercase
        assert_eq!(
            filter_subdirs(&names, "a"),
            vec!["agents".to_string(), "Apps".to_string()]
        );
    }

    #[test]
    fn filter_subdirs_hides_dotdirs_unless_filter_starts_with_dot() {
        let names = vec!["src".to_string(), ".git".to_string(), ".config".to_string()];
        // empty filter hides dotdirs
        assert_eq!(filter_subdirs(&names, ""), vec!["src".to_string()]);
        // dot-prefixed filter reveals matching dotdirs
        assert_eq!(filter_subdirs(&names, ".g"), vec![".git".to_string()]);
    }

    #[test]
    fn read_subdirs_returns_only_directories() {
        let base = std::env::temp_dir().join(format!("cm_browse_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("alpha")).unwrap();
        std::fs::create_dir_all(base.join("beta")).unwrap();
        std::fs::write(base.join("file.txt"), b"x").unwrap();

        let mut got = read_subdirs(&base);
        got.sort();
        assert_eq!(got, vec!["alpha".to_string(), "beta".to_string()]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn read_subdirs_missing_path_is_empty() {
        assert!(read_subdirs(Path::new("/no/such/path/xyz123")).is_empty());
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib browse`
Expected: 5 tests pass (the implementation ships with the tests; this confirms the logic).

- [ ] **Step 4: Lint**

Run: `cargo clippy --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/browse.rs src/lib.rs
git commit -m "feat: browse module for subdir listing (split/filter/read)"
```

---

## Task 2: extend `CreateForm` and rewrite the dir-step key handling

**Files:**
- Modify: `src/app.rs` (the `CreateForm` struct + impl near lines 14–45, and `handle_create_key` near lines 209–248)
- Test: inline `#[cfg(test)]` in `src/app.rs`

- [ ] **Step 1: Replace the `CreateForm` struct and its impl block**

In `src/app.rs`, replace the current struct + impl (the block starting `#[derive(Debug, Clone)] pub struct CreateForm {` through the closing `}` of `impl CreateForm`, i.e. the `name`/`dir`/`agent`/`field` struct and its `new`/`current_mut`/`next_field`) with:

```rust
#[derive(Debug, Clone)]
pub struct CreateForm {
    pub name: String,
    pub dir: String,
    pub agent: String,
    pub field: CreateField,
    pub dir_entries: Vec<String>,
    pub dir_selected: usize,
}

impl CreateForm {
    pub fn new(default_agent: &str) -> Self {
        Self {
            name: String::new(),
            dir: "~/".to_string(),
            agent: default_agent.to_string(),
            field: CreateField::Name,
            dir_entries: Vec::new(),
            dir_selected: 0,
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
```

- [ ] **Step 2: Replace `handle_create_key` with a dir-aware version**

Replace the entire existing `handle_create_key` function (lines ~209–248) with:

```rust
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
            KeyCode::Backspace => {
                form.current_mut().pop();
            }
            KeyCode::Char(c) => form.current_mut().push(c),
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
```

- [ ] **Step 3: Add tests to the existing `#[cfg(test)] mod tests` block in `src/app.rs`**

Append these tests inside that module (after the existing key-handling tests):

```rust
    #[test]
    fn dir_list_navigation_wraps() {
        let mut form = CreateForm::new("claude");
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

        let mut form = CreateForm::new("claude");
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
        let form = CreateForm::new("claude");
        assert_eq!(form.dir, "~/");
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib app`
Expected: all previous app tests plus the 3 new ones pass.

- [ ] **Step 5: Build + lint**

Run: `cargo build && cargo clippy --all-targets 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: interactive dir picker state + key handling in create form"
```

---

## Task 3: render the subdir list in the create modal

**Files:**
- Modify: `src/ui.rs` (`draw_create_modal`, lines ~98–112)
- Test: inline `#[cfg(test)]` in `src/ui.rs`

- [ ] **Step 1: Add the `Span` import**

At the top of `src/ui.rs`, the line `use ratatui::text::Line;` should become:
```rust
use ratatui::text::{Line, Span};
```
(`Style` and `Modifier` are already imported via `use ratatui::style::{Modifier, Style};` — confirm before adding duplicates.)

- [ ] **Step 2: Replace `draw_create_modal`**

Replace the whole `draw_create_modal` function with:

```rust
fn draw_create_modal(f: &mut Frame, app: &App) {
    let Mode::Create(form) = &app.mode else { return };
    let area = centered(60, 60, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![
        field_line("name ", &form.name, form.field == CreateField::Name),
        field_line("dir  ", &form.dir, form.field == CreateField::Dir),
        field_line("agent", &form.agent, form.field == CreateField::Agent),
        Line::from(""),
    ];

    if form.field == CreateField::Dir {
        for (i, name) in form.dir_entries.iter().enumerate() {
            let text = format!("{}{}/", if i == form.dir_selected { "> " } else { "  " }, name);
            if i == form.dir_selected {
                lines.push(Line::from(Span::styled(
                    text,
                    Style::default().add_modifier(Modifier::REVERSED),
                )));
            } else {
                lines.push(Line::from(text));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "↑↓ select · Tab/→ enter · Enter confirm · Esc cancel",
        ));
    } else {
        lines.push(Line::from(
            "Tab/Enter: next · Enter on agent: create · Esc: cancel",
        ));
    }

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("new session"));
    f.render_widget(para, area);
}
```

- [ ] **Step 3: Add a snapshot test**

Add to the existing `#[cfg(test)] mod tests` block in `src/ui.rs` (after the existing tests):

```rust
    #[test]
    fn create_modal_renders_dir_entries_when_dir_focused() {
        use crate::app::{CreateField, CreateForm, Mode};

        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude");
        form.field = CreateField::Dir;
        form.dir_entries = vec!["alpha".into(), "beta".into()];
        form.dir_selected = 0;
        app.mode = Mode::Create(form);

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("alpha/"));
        assert!(text.contains("beta/"));
        assert!(text.contains("new session"));
    }
```

(The `buf_to_string` helper and the `TestBackend`/`Terminal`/`Buffer` imports already exist in this test module from earlier work.)

- [ ] **Step 4: Run tests**

Run: `cargo test --lib ui`
Expected: existing ui tests plus the new one pass.

- [ ] **Step 5: Full build, lint, test**

Run: `cargo build --release && cargo clippy --all-targets -- -D warnings 2>&1 | tail -5 && cargo test 2>&1 | grep "test result"`
Expected: release build clean, clippy clean, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/ui.rs
git commit -m "feat: render live subdir list in create modal dir step"
```

---

## Self-Review Notes (for the implementer)

- **Spec coverage:** live subdir list in dir step (Task 3 render + Task 2 state); typing filters (Task 2 `refresh_dir_entries` on Char/Backspace + Task 1 `filter_subdirs`); ↑↓ navigate with wrap (Task 2 `dir_select_*`); Tab/→ descend (Task 2 `enter_selected_dir`); Enter confirm→agent (Task 2 dir-branch Enter); Esc cancel (Task 2); dotdir hiding (Task 1 `filter_subdirs`); `~` expanded for FS read but preserved in text (Task 2 `refresh_dir_entries`/`enter_selected_dir` keep `base`+name, expand only for `list`); missing base → empty list (Task 1 `read_subdirs`); name/agent steps unchanged (Task 2 second branch preserves prior logic).
- **Out of scope confirmed absent:** recursive/fuzzy search, Left=parent, longest-common-prefix completion.
- **Type consistency:** `CreateForm` fields `dir_entries: Vec<String>`/`dir_selected: usize` used identically in app.rs and ui.rs; `browse::{split_path, filter_subdirs, read_subdirs, list}` signatures match their call sites in `refresh_dir_entries`/`enter_selected_dir`.
```
