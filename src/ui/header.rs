use crate::app::App;
use crate::spinner;
use crate::theme as th;
use crate::timeutil;
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let total = app.sessions.len();
    let running = app.sessions.iter().filter(|s| s.status == Status::Running).count();
    let idle = total - running;
    let spin = spinner::glyph(app.spinner_frame);
    // TODO(Task 11): main loop caches the clock once per minute in `app.clock`
    // — header will read that instead so we don't fork `date` on every draw.
    let clock = timeutil::clock_hhmm();

    let mut spans = vec![
        Span::styled(th::LOGO, Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
        Span::styled(" cm", Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)),
        Span::styled("  claude · session manager", Style::default().fg(th::DIM)),
        Span::styled(format!("  {}  ", th::SEP), Style::default().fg(th::DIM)),
        Span::styled(format!("{total}"), Style::default().fg(th::TEXT_BOLD)),
        Span::styled(" sessions   ", Style::default().fg(th::MUTED)),
        Span::styled(spin, Style::default().fg(th::AMBER_HI)),
        Span::styled(format!(" {running} running"), Style::default().fg(th::AMBER_HI)),
        Span::styled(format!("   {} {idle} idle", th::IDLE_DOT), Style::default().fg(th::DIM)),
    ];
    if !clock.is_empty() {
        spans.push(Span::styled(format!("    {clock}"), Style::default().fg(th::MUTED)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize))
            .style(Style::default().fg(th::BORDER)),
        rows[1],
    );
}
