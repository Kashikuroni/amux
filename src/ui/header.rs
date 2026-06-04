use crate::app::App;
use crate::theme as th;
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Indigo accent for "waiting on user" — matches the session list.
const INDIGO: Color = Color::Indexed(105);

/// Status accent: bold but not bright (BOLD + DIM), matching the session list.
fn status_style(color: Color) -> Style {
    Style::default()
        .fg(color)
        .add_modifier(Modifier::BOLD | Modifier::DIM)
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let total = app.sessions.len();
    let count = |st: Status| app.sessions.iter().filter(|s| s.status == st).count();
    let running = count(Status::Running);
    let waiting = count(Status::Waiting);
    let idle = count(Status::Idle);
    let clock = &app.clock;

    let mut spans = vec![
        Span::styled(
            th::LOGO,
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " am",
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  agent multiplexer", Style::default().fg(th::DIM)),
    ];
    spans.extend([
        Span::styled(format!("  {}  ", th::SEP), Style::default().fg(th::DIM)),
        Span::styled(format!("{total}"), Style::default().fg(th::TEXT_BOLD)),
        Span::styled(" sessions   ", Style::default().fg(th::MUTED)),
        Span::styled(format!("{running} running"), status_style(Color::Blue)),
        Span::styled(format!("   {waiting} waiting"), status_style(INDIGO)),
        Span::styled(format!("   {idle} idle"), status_style(Color::Red)),
    ]);
    if let Some(u) = &app.update {
        spans.push(Span::styled(
            format!("  {}  ", th::SEP),
            Style::default().fg(th::DIM),
        ));
        spans.push(Span::styled(
            format!("↑ v{}", u.version),
            Style::default().fg(th::AMBER).add_modifier(Modifier::DIM),
        ));
    }
    if !clock.is_empty() {
        let left_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let right_width = clock.chars().count();
        let pad = (area.width as usize)
            .saturating_sub(left_width + right_width)
            .max(1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(clock.clone(), Style::default().fg(th::MUTED)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::usage::{Usage, Window};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::ui::testutil::buf_to_string;

    #[test]
    fn header_shows_update_badge() {
        let mut app = App::new(crate::config::Config::default());
        app.update = Some(crate::update::UpdateInfo {
            version: "9.9.9".into(),
            url: String::new(),
        });
        let mut t = Terminal::new(TestBackend::new(120, 4)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("↑ v9.9.9"), "update badge:\n{s}");
    }

    #[test]
    fn header_omits_limits_always() {
        // Limits have moved to the preview panel — the header must never show them.
        let mut app = App::new(Config::default());
        app.usage = Some(Usage {
            five_hour: Some(Window {
                utilization: 99.0,
                reset_unix: None,
                reset_hhmm: None,
            }),
            seven_day: None,
            seven_day_sonnet: None,
        });
        app.plan = Some("Max 5×".to_string());
        let mut t = Terminal::new(TestBackend::new(140, 2)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains('%'), "header must not show a percentage:\n{s}");
        assert!(
            !s.contains("Max 5×"),
            "plan badge must not appear in header:\n{s}"
        );
    }
}
