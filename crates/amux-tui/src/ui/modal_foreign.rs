use crate::theme as th;
use crate::tmux::ForeignSession;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

/// `$HOME`-prefixed paths shortened to `~/…` so the rows stay readable.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Read-only list of tmux sessions am does not manage: sessions on the user's
/// own servers (`default` and any named socket) plus untagged ones on the am
/// socket. Snapshotted when the modal opened; any key closes it.
pub fn render(f: &mut Frame, sessions: &[ForeignSession]) {
    let area = super::centered(60, 60, f.area());
    f.render_widget(Clear, area);
    let dim = Style::default().fg(th::MUTED).add_modifier(Modifier::DIM);
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(
                "other tmux sessions",
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  ({})", sessions.len()), dim),
        ]),
        Line::from(Span::styled(
            "sessions on this machine that am does not manage",
            dim,
        )),
        Line::from(""),
    ];
    if sessions.is_empty() {
        lines.push(Line::from(Span::styled(
            "none — every tmux session here is visible in am",
            Style::default().fg(th::MUTED),
        )));
    } else {
        for s in sessions {
            let mut spans = vec![
                Span::styled(format!("{:>8} ", s.socket), dim),
                Span::styled(th::SEP.to_string(), dim),
                Span::styled(
                    format!(" {}", s.name),
                    Style::default()
                        .fg(th::TEXT_BOLD)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}w", s.windows), dim),
            ];
            if s.attached {
                spans.push(Span::styled("  attached", Style::default().fg(th::GREEN)));
            }
            spans.push(Span::styled(format!("  {}", tilde(&s.dir)), dim));
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "attach from a terminal: tmux -L <socket> attach -t <name>",
            dim,
        )));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(" tmux ")
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fs(socket: &str, name: &str, attached: bool) -> ForeignSession {
        ForeignSession {
            socket: socket.into(),
            name: name.into(),
            dir: "/work/x".into(),
            attached,
            windows: 2,
        }
    }

    #[test]
    fn lists_foreign_sessions_with_socket_and_attach_hint() {
        let sessions = vec![fs("default", "scratch", true), fs("cm", "stray", false)];
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
        t.draw(|f| render(f, &sessions)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("other tmux sessions"), "title:\n{s}");
        assert!(s.contains("(2)"), "count:\n{s}");
        assert!(s.contains("scratch"), "session name:\n{s}");
        assert!(s.contains("default"), "socket name:\n{s}");
        assert!(s.contains("attached"), "attached marker:\n{s}");
        assert!(s.contains("tmux -L <socket> attach"), "attach hint:\n{s}");
    }

    #[test]
    fn empty_list_shows_all_visible_note() {
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
        t.draw(|f| render(f, &[])).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("none — every tmux session"), "empty note:\n{s}");
    }
}
