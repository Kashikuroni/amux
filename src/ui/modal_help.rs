use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame) {
    let area = super::centered(70, 70, f.area());
    f.render_widget(Clear, area);
    let groups: [(&str, &[(&str, &str)]); 4] = [
        ("Navigation", &[("k j / ↑↓", "move"), ("g G", "first · last"), ("/", "filter")]),
        ("Session", &[("↵ o", "attach"), ("n", "new"), ("d", "kill"), ("r", "rename")]),
        ("Preview", &[("auto", "refresh on interval")]),
        ("App", &[("?", "help"), ("q", "quit (sessions stay)")]),
    ];
    let mut lines = vec![
        Line::from(vec![
            Span::styled("? Help", Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
            Span::styled("   keys & shortcuts", Style::default().fg(th::MUTED)),
        ]),
        Line::from(""),
    ];
    for (title, items) in groups {
        lines.push(Line::from(Span::styled(
            title,
            Style::default().fg(th::MUTED).add_modifier(Modifier::BOLD),
        )));
        for (k, label) in items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::default().fg(th::AMBER_HI)),
                Span::styled(*label, Style::default().fg(th::TEXT)),
            ]));
        }
        lines.push(Line::from(""));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(th::BORDER_HI))
                .title(" help "),
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
    fn help_lists_groups_and_keys() {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(render).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("Help"));
        assert!(s.contains("Navigation"));
        assert!(s.contains("attach"));
        assert!(s.contains("filter"));
    }
}
