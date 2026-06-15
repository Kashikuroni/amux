use crate::app::{GitAction, GitForm};
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, form: &GitForm) {
    match form.action {
        GitAction::Promote => render_promote(f, form),
        GitAction::DeleteBranch => render_delete_branch(f, form),
        GitAction::BranchCleanup => render_cleanup(f, form),
    }
}

fn render_promote(f: &mut Frame, form: &GitForm) {
    let area = super::centered(56, 36, f.area());
    f.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("⎇ ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                "promote worktree",
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Branch:  ", Style::default().fg(th::MUTED)),
            Span::styled(
                form.branch.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Worktree: ", Style::default().fg(th::MUTED)),
            Span::styled(
                form.worktree_path.as_deref().unwrap_or("").to_string(),
                Style::default().fg(th::DIM),
            ),
        ]),
        Line::from(""),
    ];
    if form.has_stash {
        lines.push(Line::from(vec![Span::styled(
            "⚠ Unstaged changes — will git stash",
            Style::default().fg(th::YELLOW),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "  and restore after checkout",
            Style::default().fg(th::DIM),
        )]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![
        Span::styled(" y · Promote ", Style::default().bg(th::GREEN).fg(th::BG)),
        Span::raw("  "),
        Span::styled(" n · Cancel ", Style::default().fg(th::TEXT)),
        Span::styled("     esc to dismiss", Style::default().fg(th::DIM)),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(" git ")
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

fn render_delete_branch(f: &mut Frame, form: &GitForm) {
    let area = super::centered(56, 36, f.area());
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "✕ ",
                Style::default().fg(th::RED).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "delete branch",
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Branch:  ", Style::default().fg(th::MUTED)),
            Span::styled(
                form.branch.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Session: ", Style::default().fg(th::MUTED)),
            Span::styled(form.session_name.clone(), Style::default().fg(th::AMBER)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Runs: git branch -d",
            Style::default().fg(th::DIM),
        )),
        Line::from(Span::styled(
            "(refuses if not fully merged).",
            Style::default().fg(th::DIM),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y · Delete ", Style::default().bg(th::RED).fg(th::BG)),
            Span::raw("  "),
            Span::styled(" n · Cancel ", Style::default().fg(th::TEXT)),
            Span::styled("     esc to dismiss", Style::default().fg(th::DIM)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            th::panel()
                .border_style(th::chrome(th::RED))
                .title(" git ")
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

fn render_cleanup(f: &mut Frame, form: &GitForm) {
    use ratatui::layout::{Constraint, Layout};
    let area = super::centered(60, 70, f.area());
    f.render_widget(Clear, area);
    let block = th::panel()
        .border_style(th::chrome(th::BORDER_HI))
        .title(" git ")
        .style(Style::default().bg(th::BG_RAISED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let [header_area, list_area, hint_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Header
    let header_line = Line::from(vec![
        Span::styled(
            "branch cleanup",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({} branches)", form.branches.len()),
            Style::default().fg(th::DIM),
        ),
    ]);
    f.render_widget(
        Paragraph::new(vec![header_line, Line::from("")]),
        header_area,
    );

    // Branch list
    let mut list_lines: Vec<Line> = form
        .branches
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let checkbox = if b.protected {
                "[ ]"
            } else if form.selected.contains(&i) {
                "[✓]"
            } else {
                "[ ]"
            };
            let cursor_mark = if i == form.cursor { "▸ " } else { "  " };
            let name_style = if b.protected {
                Style::default().fg(th::DIM)
            } else if i == form.cursor {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut spans = vec![
                Span::styled(cursor_mark.to_string(), Style::default().fg(th::MUTED)),
                Span::styled(
                    format!("{checkbox} "),
                    Style::default().fg(if b.protected { th::DIM } else { th::AMBER }),
                ),
                Span::styled(b.name.clone(), name_style),
            ];
            if b.protected {
                spans.push(Span::styled("  (protected)", Style::default().fg(th::DIM)));
            }
            Line::from(spans)
        })
        .collect();
    // Clamp scroll so cursor is visible
    let visible = list_area.height as usize;
    let start = form.cursor.saturating_sub(visible.saturating_sub(1));
    let list_lines_display: Vec<Line> = list_lines.drain(start..).take(visible).collect();
    f.render_widget(Paragraph::new(list_lines_display), list_area);

    // Hints
    let hint = Line::from(vec![
        Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" toggle · ", Style::default().fg(th::DIM)),
        Span::styled("a", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" select all · ", Style::default().fg(th::DIM)),
        Span::styled("y", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" delete · ", Style::default().fg(th::DIM)),
        Span::styled("n/esc", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(" cancel", Style::default().fg(th::DIM)),
    ]);
    f.render_widget(Paragraph::new(vec![hint]), hint_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn promote_form(has_stash: bool) -> GitForm {
        GitForm {
            session_name: "wt".into(),
            branch: "feature/agent-x".into(),
            repo_root: "/home/user/proj".into(),
            worktree_path: Some("/home/user/proj/.worktrees/agent-x".into()),
            has_stash,
            action: GitAction::Promote,
            branches: vec![],
            selected: std::collections::HashSet::new(),
            cursor: 0,
        }
    }

    fn delete_form() -> GitForm {
        GitForm {
            session_name: "br".into(),
            branch: "feature/done".into(),
            repo_root: "/home/user/proj".into(),
            worktree_path: None,
            has_stash: false,
            action: GitAction::DeleteBranch,
            branches: vec![],
            selected: std::collections::HashSet::new(),
            cursor: 0,
        }
    }

    #[test]
    fn promote_modal_shows_branch_and_confirm() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &promote_form(false))).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("promote worktree"), "must show action title");
        assert!(s.contains("feature/agent-x"), "must show branch name");
        assert!(s.contains("Promote"), "must show confirm hint");
    }

    #[test]
    fn promote_modal_shows_stash_warning_when_dirty() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &promote_form(true))).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("stash"), "must mention stash when dirty");
    }

    #[test]
    fn promote_modal_no_stash_warning_when_clean() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &promote_form(false))).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("stash"), "must NOT mention stash when clean");
    }

    fn cleanup_form() -> GitForm {
        use crate::app::BranchItem;
        let mut selected = std::collections::HashSet::new();
        selected.insert(0usize);
        selected.insert(1usize);
        GitForm {
            session_name: "s".into(),
            branch: String::new(),
            repo_root: "/proj".into(),
            worktree_path: None,
            has_stash: false,
            action: GitAction::BranchCleanup,
            branches: vec![
                BranchItem {
                    name: "feature/agent-auth".into(),
                    protected: false,
                },
                BranchItem {
                    name: "fix/typo".into(),
                    protected: false,
                },
                BranchItem {
                    name: "main".into(),
                    protected: true,
                },
            ],
            selected,
            cursor: 0,
        }
    }

    #[test]
    fn cleanup_modal_shows_branches_and_hints() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &cleanup_form())).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("branch cleanup"), "must show title");
        assert!(s.contains("feature/agent-auth"), "must list branch");
        assert!(s.contains("fix/typo"), "must list branch");
        assert!(s.contains("main"), "must show protected branch");
        assert!(
            s.contains("Space") || s.contains("space"),
            "must show space hint"
        );
    }

    #[test]
    fn cleanup_modal_marks_selected_with_checkmark() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &cleanup_form())).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("✓"), "selected branches must show checkmark");
    }

    #[test]
    fn delete_branch_modal_shows_branch_and_session() {
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &delete_form())).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("delete branch"), "must show action title");
        assert!(s.contains("feature/done"), "must show branch name");
        assert!(s.contains("br"), "must show session name");
        assert!(s.contains("merged"), "must warn about merge check");
    }
}
