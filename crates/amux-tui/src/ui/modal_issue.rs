use crate::app::{IssueField, IssueForm};
use crate::git::IssueStage;
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

/// GitHub-issue composer: a title line and a multi-line body, filed with the
/// `gh` CLI in the form's repo. After submit the same box shows the async
/// stage (creating… / the new issue's URL / gh's error).
pub fn render(f: &mut Frame, form: &IssueForm) {
    let area = super::centered(50, 50, f.area());
    f.render_widget(Clear, area);
    let block = th::panel()
        .border_style(th::chrome(th::BORDER_HI))
        .title(" new issue ")
        .style(Style::default().bg(th::BG_RAISED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height < 4 {
        return;
    }

    let repo = crate::app::project_default_name(&form.repo_root).to_string();
    let dim = Style::default().fg(th::MUTED).add_modifier(Modifier::DIM);

    if let Some(stage) = &form.stage {
        render_stage(f, inner, stage, &repo);
        return;
    }

    let [head, title_row, body_label, body_area, hint] = Layout::vertical([
        Constraint::Length(2), // repo line + blank
        Constraint::Length(2), // title field + blank
        Constraint::Length(1), // body label
        Constraint::Min(1),    // body editor
        Constraint::Length(1), // keys
    ])
    .areas(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("repo: ", dim),
            Span::styled(repo, Style::default().add_modifier(Modifier::BOLD)),
        ])),
        head,
    );

    let focus = |on: bool| {
        if on {
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)
        } else {
            dim
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("title ", focus(form.field == IssueField::Title)),
            Span::styled(th::SEP.to_string(), dim),
            Span::raw(format!(" {}", form.title)),
        ])),
        title_row,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "body",
            focus(form.field == IssueField::Body),
        ))),
        body_label,
    );

    match form.field {
        // Hardware cursor sits in the active field; the body keeps its text
        // rendered (read-only) while the title is focused.
        IssueField::Title => {
            f.render_widget(
                Paragraph::new(form.body.buffer.clone()).wrap(Wrap { trim: false }),
                body_area,
            );
            let x = title_row.x + 8 + form.title.chars().count() as u16;
            f.set_cursor_position((x.min(title_row.right().saturating_sub(1)), title_row.y));
        }
        IssueField::Body => super::render_editor(f, body_area, &form.body),
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            super::hint_key("shift+enter"),
            super::hint_label(" create  "),
            super::hint_key("enter"),
            super::hint_label(" next/newline  "),
            super::hint_key("tab"),
            super::hint_label(" field  "),
            super::hint_key("esc"),
            super::hint_label(" cancel"),
        ])),
        hint,
    );
}

fn render_stage(f: &mut Frame, inner: Rect, stage: &IssueStage, repo: &str) {
    let dim = Style::default().fg(th::MUTED).add_modifier(Modifier::DIM);
    let lines = match stage {
        IssueStage::Creating => vec![
            Line::from(Span::styled(
                format!("creating issue in {repo}…"),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled("esc hide — gh keeps running", dim)),
        ],
        IssueStage::Done(url) => vec![
            Line::from(vec![
                Span::styled("✓ ", Style::default().fg(th::GREEN)),
                Span::styled(
                    "issue created",
                    Style::default()
                        .fg(th::TEXT_BOLD)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(url.clone(), Style::default().fg(th::TEXT))),
            Line::from(Span::styled("(copied to clipboard)", dim)),
            Line::from(""),
            Line::from(Span::styled("any key to close", dim)),
        ],
        IssueStage::Failed(err) => vec![
            Line::from(vec![
                Span::styled("✕ ", Style::default().fg(th::RED)),
                Span::styled(
                    "issue not created",
                    Style::default()
                        .fg(th::TEXT_BOLD)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(err.clone(), Style::default().fg(th::RED))),
            Line::from(""),
            Line::from(Span::styled("any key to close", dim)),
        ],
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn form() -> IssueForm {
        IssueForm {
            repo_root: "/work/amux".into(),
            title: "broken thing".into(),
            body: crate::editor::TextArea::new("details"),
            field: IssueField::Title,
            stage: None,
        }
    }

    #[test]
    fn renders_fields_repo_and_hints() {
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| render(f, &form())).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("new issue"), "title:\n{s}");
        assert!(s.contains("repo: amux"), "repo shortname:\n{s}");
        assert!(s.contains("broken thing"), "typed title:\n{s}");
        assert!(s.contains("details"), "body text stays visible:\n{s}");
        assert!(s.contains("shift+enter create"), "submit hint:\n{s}");
    }

    #[test]
    fn renders_done_stage_with_url() {
        let mut f = form();
        f.stage = Some(IssueStage::Done("https://github.com/u/r/issues/7".into()));
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|fr| render(fr, &f)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("issue created"), "done headline:\n{s}");
        assert!(s.contains("issues/7"), "url shown:\n{s}");
        assert!(s.contains("copied to clipboard"), "clipboard note:\n{s}");
    }

    #[test]
    fn renders_failed_stage_with_error() {
        let mut f = form();
        f.stage = Some(IssueStage::Failed("gh CLI not found".into()));
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|fr| render(fr, &f)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("issue not created"), "failed headline:\n{s}");
        assert!(s.contains("gh CLI not found"), "error shown:\n{s}");
    }
}
