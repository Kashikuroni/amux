use crate::theme as th;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame) {
    let v = Layout::vertical([Constraint::Percentage(30), Constraint::Min(0)]).split(f.area());
    let lines = vec![
        Line::from(Span::styled(
            th::LOGO,
            Style::default().fg(th::RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "tmux not found in PATH",
            Style::default().fg(th::TEXT_BOLD).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "cm manages tmux sessions and needs the tmux binary.",
            Style::default().fg(th::MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled("  macOS    brew install tmux", Style::default().fg(th::TEXT))),
        Line::from(Span::styled("  Ubuntu   sudo apt install tmux", Style::default().fg(th::TEXT))),
        Line::from(Span::styled("  Arch     sudo pacman -S tmux", Style::default().fg(th::TEXT))),
        Line::from(""),
        Line::from(Span::styled("  q to quit", Style::default().fg(th::DIM))),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        v[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    #[test]
    fn error_mentions_tmux_and_install() {
        let mut t = Terminal::new(TestBackend::new(60, 18)).unwrap();
        t.draw(render).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("tmux not found"));
        assert!(s.contains("brew install tmux"));
    }
}
