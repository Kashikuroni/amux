use crate::app::collapse_home;
use crate::theme as th;
use crate::tmux::Session;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, form: &crate::app::KillForm, session: Option<&Session>) {
    let area = super::centered(56, 36, f.area());
    f.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "✕ ",
                Style::default().fg(th::RED).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Kill session?",
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];
    // session header line: name [· status]
    let mut head = vec![Span::styled(
        form.name.to_string(),
        Style::default().fg(th::AMBER),
    )];
    if let Some(s) = session {
        let label = match s.status {
            crate::tmux::Status::Running => "running",
            crate::tmux::Status::Idle => "idle",
            crate::tmux::Status::Waiting => "waiting",
        };
        head.push(Span::styled(
            format!("  ·  {label}"),
            Style::default().fg(th::MUTED),
        ));
    }
    lines.push(Line::from(head));
    // path · ⎇ branch
    if let Some(s) = session {
        let mut sub = vec![Span::styled(
            collapse_home(&s.dir),
            Style::default().fg(th::MUTED),
        )];
        if let Some(g) = &s.git {
            sub.push(Span::styled(
                format!("  ·  {} {}", th::BRANCH, g.branch),
                Style::default().fg(th::DIM),
            ));
        }
        lines.push(Line::from(sub));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Stops the agent process and discards unsaved scratch. Files on disk are unaffected.",
            Style::default().fg(th::DIM),
        )),
        Line::from(""),
    ]);
    if form.worktree.is_some() {
        let mark = if form.remove_worktree { "[x]" } else { "[ ]" };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), Style::default().fg(th::AMBER)),
            Span::styled("also remove worktree", Style::default().fg(th::TEXT)),
            Span::styled("   space to toggle", Style::default().fg(th::DIM)),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![
        Span::styled(" y · yes, kill ", Style::default().bg(th::RED).fg(th::BG)),
        Span::raw("  "),
        Span::styled(" n · no ", Style::default().fg(th::TEXT)),
        Span::styled("     esc to dismiss", Style::default().fg(th::DIM)),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(
            th::panel()
                .border_style(th::chrome(th::RED))
                .title(" confirm ")
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn kill_modal_shows_name_and_warning() {
        // Use a tall-enough terminal so the 36%-height modal (≥9 rows for 7 lines + 2 borders)
        // can display all content. 70×30 gives ~10 inner rows at 36%.
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        let form = crate::app::KillForm {
            name: "project-a".into(),
            worktree: None,
            remove_worktree: false,
        };
        t.draw(|f| render(f, &form, None)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("Kill session?"));
        assert!(s.contains("project-a"));
        assert!(s.contains("yes, kill"));
    }
    #[test]
    fn kill_modal_shows_worktree_toggle_when_present() {
        use crate::app::KillForm;
        let form = KillForm {
            name: "wt".into(),
            worktree: Some(("/repo".into(), "/repo/.worktrees/wt".into())),
            remove_worktree: true,
        };
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, &form, None)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("remove worktree"));
        assert!(s.contains("[x]"));
    }
}
