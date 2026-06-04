use crate::app::{App, Mode, NoteSub, NoteTarget, RightPane};
use crate::note::{self, NoteLine};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The note target currently shown in the pane (focused note wins; otherwise the
/// pane's view mode + selection decide). None => nothing to show.
fn shown_target(app: &App) -> Option<NoteTarget> {
    if let Mode::Note(ns) = &app.mode {
        return Some(ns.target.clone());
    }
    match app.right_pane {
        RightPane::ProjectNote => app
            .selected_session()
            .map(|s| NoteTarget::Project(crate::app::session_root(s).to_string())),
        RightPane::SessionNote => app.selected_name().map(NoteTarget::Session),
        RightPane::Preview => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // rule
        Constraint::Min(0),    // body
    ])
    .split(area);

    let Some(target) = shown_target(app) else {
        return;
    };
    let text = app.note_text(&target).to_string();
    let (done, total) = note::counts(&text);
    let title = match &target {
        NoteTarget::Project(root) => app.project_display_name(root),
        NoteTarget::Session(name) => name.clone(),
    };
    let hint = match &app.mode {
        Mode::Note(ns) if ns.confirm_clear => "clear note? y / n",
        Mode::Note(ns) if ns.sub == NoteSub::Edit => "edit — esc done",
        Mode::Note(_) => "j/k · space · V · y · e edit · c clear · tab defocus · esc exit",
        _ => "Tab to edit",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("   {done}/{total}"),
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::styled(
                format!("   {hint}"),
                Style::default().add_modifier(Modifier::DIM),
            ),
        ])),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[1],
    );

    // Edit mode: raw editor with wrapped text + hardware cursor.
    if let Mode::Note(ns) = &app.mode {
        if ns.sub == NoteSub::Edit {
            super::render_editor(f, rows[2], &ns.editor);
            return;
        }
    }

    // Render mode: styled markdown with cursor/selection highlight over tasks.
    let focused = matches!(&app.mode, Mode::Note(_));
    let (cur, sel) = match &app.mode {
        Mode::Note(ns) => (Some(ns.cursor), crate::app::selection_set(ns)),
        _ => (None, std::collections::HashSet::new()),
    };
    let mut task_ord = 0usize;
    let lines: Vec<Line> = note::parse(&text)
        .into_iter()
        .map(|nl| {
            let line = render_line(&nl, task_ord, focused, cur, &sel);
            if matches!(nl, NoteLine::Task { .. }) {
                task_ord += 1;
            }
            line
        })
        .collect();
    f.render_widget(Paragraph::new(lines), rows[2]);
}

fn render_line(
    nl: &NoteLine,
    ord: usize,
    focused: bool,
    cursor: Option<usize>,
    sel: &std::collections::HashSet<usize>,
) -> Line<'static> {
    match nl {
        NoteLine::Task { done, text } => {
            let box_glyph = if *done { "☑" } else { "☐" };
            let on_cursor = focused && cursor == Some(ord);
            let selected = sel.contains(&ord);
            let mut style = Style::default();
            if *done {
                style = style.add_modifier(Modifier::DIM | Modifier::CROSSED_OUT);
            }
            if selected {
                style = style.bg(th::SEL_BG);
            }
            let bar = if on_cursor { "› " } else { "  " };
            Line::from(vec![
                Span::styled(
                    bar.to_string(),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(format!("{box_glyph} {text}"), style),
            ])
        }
        NoteLine::Heading { text, .. } => Line::from(Span::styled(
            text.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        NoteLine::Bullet(text) => Line::from(format!("  • {text}")),
        NoteLine::Text(t) => Line::from(format!("  {t}")),
        NoteLine::Blank => Line::from(""),
    }
}

#[cfg(test)]
mod tests {
    use super::*; // brings in App, RightPane, render, …
    use crate::config::Config;
    use crate::tmux::{Session, Status};
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn renders_checkboxes_and_counter() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "s".into(),
            dir: "/home/u/proj".into(),
            cwd: "/home/u/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.project_notes.insert(
            "/home/u/proj".into(),
            "# Today\n- [ ] open\n- [x] done".into(),
        );
        app.right_pane = RightPane::ProjectNote;
        let mut t = Terminal::new(TestBackend::new(40, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("proj"), "title is the project name:\n{s}");
        assert!(s.contains("1/2"), "counter:\n{s}");
        assert!(s.contains("☐ open"), "open box:\n{s}");
        assert!(s.contains("☑ done"), "done box:\n{s}");
    }
}
