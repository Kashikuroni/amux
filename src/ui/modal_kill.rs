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
            Span::styled(
                "Kill session?",
                Style::default().fg(th::TEXT_BOLD).add_modifier(Modifier::BOLD),
            ),
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
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(th::RED))
                .title(" confirm "),
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
            for x in 0..buf.area.width { s.push_str(buf[(x, y)].symbol()); }
            s.push('\n');
        }
        s
    }
    #[test]
    fn kill_modal_shows_name_and_warning() {
        // Use a tall-enough terminal so the 36%-height modal (≥9 rows for 7 lines + 2 borders)
        // can display all content. 70×30 gives ~10 inner rows at 36%.
        let mut t = Terminal::new(TestBackend::new(70, 30)).unwrap();
        t.draw(|f| render(f, "project-a")).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("Kill session?"));
        assert!(s.contains("project-a"));
        assert!(s.contains("yes, kill"));
    }
}
