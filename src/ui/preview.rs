use crate::app::{collapse_home, App};
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

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} ", th::PREVIEW_MARK),
                Style::default().fg(th::AMBER),
            ),
            Span::styled(title.to_string(), Style::default().fg(th::TEXT_BOLD)),
            Span::styled(format!("    {age}"), Style::default().fg(th::DIM)),
        ])),
        rows[0],
    );

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
}
