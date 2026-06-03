use crate::app::{collapse_home, App};
use crate::theme as th;
use crate::timeutil;
use ansi_to_tui::IntoText;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

pub(crate) fn is_claude(agent: &str) -> bool {
    agent.split_whitespace().next() == Some("claude")
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // ▸ title          age
        Constraint::Length(1), // path · ⎇ branch
        Constraint::Length(1), // ━ rule
        Constraint::Min(0),    // ANSI content
    ])
    .split(area);

    let sel = app.selected_session();
    let title = sel.map(|s| s.name.as_str()).unwrap_or("preview");
    let age = sel
        .map(|s| timeutil::humanize_age(app.now_unix - s.created))
        .unwrap_or_default();

    let mut title_spans = vec![
        Span::styled(
            format!("{} ", th::PREVIEW_MARK),
            Style::default().fg(th::AMBER),
        ),
        Span::styled(title.to_string(), Style::default().fg(th::TEXT_BOLD)),
        Span::styled(format!("    {age}"), Style::default().fg(th::DIM)),
    ];
    let right = if sel.is_some_and(|s| is_claude(&s.agent)) {
        crate::usage::account_right_spans(
            app.usage.as_ref(),
            app.plan.as_deref(),
            app.usage_error.as_deref(),
        )
    } else {
        Vec::new()
    };
    if !right.is_empty() {
        // chars().count() assumes every character occupies one terminal column —
        // valid here because all span content is ASCII / single-glyph.
        let left_width: usize = title_spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        let right_width: usize = right.iter().map(|s| s.content.chars().count()).sum();
        let pad = (rows[0].width as usize)
            .saturating_sub(left_width + right_width)
            .max(1);
        title_spans.push(Span::raw(" ".repeat(pad)));
        title_spans.extend(right);
    }
    f.render_widget(Paragraph::new(Line::from(title_spans)), rows[0]);

    let mut sub = vec![Span::styled(
        sel.map(|s| collapse_home(&s.dir)).unwrap_or_default(),
        Style::default().fg(th::MUTED),
    )];
    if let Some(g) = sel.and_then(|s| s.git.as_ref()) {
        sub.push(Span::styled(
            format!(" · {} {}", th::BRANCH, g.branch),
            Style::default().fg(th::DIM),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(sub)), rows[1]);

    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[2],
    );

    // Record the content area so the capture logic can size the tmux window to
    // match — otherwise the agent's full-width input box wraps to two rows here.
    app.preview_dims.set((rows[3].width, rows[3].height));

    // Content: parse ANSI from capture-pane into styled Text; fall back to plain.
    // Drop trailing blank lines (tmux pads the pane), then scroll to the bottom so
    // the agent's latest output — and any pending prompt — is always visible.
    let trimmed = app.preview.trim_end_matches(['\n', ' ', '\t']);
    let text: Text = trimmed
        .into_text()
        .unwrap_or_else(|_| Text::raw(trimmed.to_string()));
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(th::TEXT));
    // Bottom-anchor against the *wrapped* display-row count (one logical line
    // may wrap to several rows), then offset upward by the user's manual scroll.
    let total = para.line_count(rows[3].width) as u16;
    let bottom = total.saturating_sub(rows[3].height);
    let scroll_y = bottom.saturating_sub(app.preview_scroll);
    f.render_widget(para.scroll((scroll_y, 0)), rows[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tmux::{Session, Status};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::ui::testutil::buf_to_string;

    #[test]
    fn renders_title_and_ansi_content() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(),
            dir: "~/work/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.now_unix = 0;
        // ANSI green "hello" — ansi-to-tui must not leave escape bytes in the buffer
        app.preview = "\u{1b}[32mhello\u{1b}[0m world".into();
        let mut t = Terminal::new(TestBackend::new(50, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("proj"), "missing title:\n{s}");
        assert!(s.contains("hello"), "missing 'hello':\n{s}");
        assert!(s.contains("world"), "missing 'world':\n{s}");
        assert!(!s.contains('\u{1b}'), "raw escape leaked into buffer:\n{s}");
    }

    #[test]
    fn records_content_dims_for_window_sizing() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(),
            dir: "~/work/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        let mut t = Terminal::new(TestBackend::new(50, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        // Three chrome rows (title, path, rule) sit above the content area, so the
        // recorded size is the full width × (height - 3) — what the tmux window
        // must reflow to for the capture to fit without wrapping.
        assert_eq!(app.preview_dims.get(), (50, 7));
    }

    #[test]
    fn wraps_wide_lines_instead_of_clipping() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(),
            dir: "~/work/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.now_unix = 0;
        // A line far wider than the 20-col backend; with wrapping the tail
        // ("zzz") must still appear in the buffer rather than being clipped.
        app.preview = "aaaaaaaaaaaaaaaaaaaa bbbbbbbbbb zzz".into();
        let mut t = Terminal::new(TestBackend::new(20, 12)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("zzz"), "wrapped tail must be visible:\n{s}");
    }

    #[test]
    fn preview_shows_limits_for_claude_session() {
        use crate::usage::{Usage, Window};
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "work".into(),
            dir: "~/work".into(),
            created: 0,
            agent: "claude".into(),
            status: crate::tmux::Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.usage = Some(Usage {
            five_hour: Some(Window {
                utilization: 77.0,
                reset_unix: None,
                reset_hhmm: None,
            }),
            seven_day: None,
            seven_day_sonnet: None,
        });
        let mut t = Terminal::new(TestBackend::new(80, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        // Only the title row (row 0) should contain the percentage.
        let row0 = buf_to_string(t.backend().buffer())
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(row0.contains("77%"), "limits must appear in title row:\n{row0}");
    }

    #[test]
    fn preview_shows_limits_for_claude_with_flags() {
        // "claude --dangerously-skip-permissions" must still match is_claude().
        use crate::usage::{Usage, Window};
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "work".into(),
            dir: "~/work".into(),
            created: 0,
            agent: "claude --dangerously-skip-permissions".into(),
            status: crate::tmux::Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.usage = Some(Usage {
            five_hour: Some(Window { utilization: 50.0, reset_unix: None, reset_hhmm: None }),
            seven_day: None,
            seven_day_sonnet: None,
        });
        let mut t = Terminal::new(TestBackend::new(80, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let row0 = buf_to_string(t.backend().buffer())
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(row0.contains("50%"), "claude with flags must show limits:\n{row0}");
    }

    #[test]
    fn preview_hides_limits_for_non_claude_session() {
        use crate::usage::{Usage, Window};
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "work".into(),
            dir: "~/work".into(),
            created: 0,
            agent: "codex".into(),
            status: crate::tmux::Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.usage = Some(Usage {
            five_hour: Some(Window { utilization: 77.0, reset_unix: None, reset_hhmm: None }),
            seven_day: None,
            seven_day_sonnet: None,
        });
        let mut t = Terminal::new(TestBackend::new(80, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains('%'), "non-claude session must not show limits:\n{s}");
    }

    #[test]
    fn preview_hides_limits_for_terminal_session() {
        use crate::usage::{Usage, Window};
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "shell".into(),
            dir: "~/work".into(),
            created: 0,
            agent: "$SHELL".into(),
            status: crate::tmux::Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.usage = Some(Usage {
            five_hour: Some(Window { utilization: 50.0, reset_unix: None, reset_hhmm: None }),
            seven_day: None,
            seven_day_sonnet: None,
        });
        let mut t = Terminal::new(TestBackend::new(80, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains('%'), "terminal session must not show limits:\n{s}");
    }

    #[test]
    fn bottom_anchors_newest_line_when_wrapped_content_overflows() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "proj".into(),
            dir: "~/work/proj".into(),
            created: 0,
            agent: "claude".into(),
            status: Status::Idle,
            attached: false,
            git: None,
            worktree_repo: None,
        }];
        app.now_unix = 0;
        app.preview_scroll = 0;
        // Many lines: far more than the content area (height 8 - 3 chrome = 5 rows).
        app.preview = (0..40)
            .map(|i| format!("line{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = Terminal::new(TestBackend::new(20, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(
            s.contains("line39"),
            "newest line must be anchored visible:\n{s}"
        );
        assert!(
            !s.contains("line00"),
            "oldest line must have scrolled off:\n{s}"
        );
    }
}
