use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect) {
    let v = Layout::vertical([Constraint::Percentage(38), Constraint::Min(0)]).split(area);
    let lines = vec![
        Line::from(Span::styled(
            th::LOGO,
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "No sessions yet",
            Style::default()
                .fg(th::TEXT_BOLD)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Spin up an agent in any directory. Sessions keep running after you quit.",
            Style::default().fg(th::MUTED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " n ",
                Style::default()
                    .bg(th::AMBER)
                    .fg(th::BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  start your first session", Style::default().fg(th::TEXT)),
        ]),
    ];
    let p = Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(p, v[1]);
}

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
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("No sessions yet"), "missing in:\n{s}");
    }
}
