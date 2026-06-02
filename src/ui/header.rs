use crate::app::App;
use crate::theme as th;
use crate::tmux::Status;
use crate::usage::Window;
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
    // Subscription plan badge (e.g. "Max 5×"), bold so it reads as a chip.
    if let Some(plan) = &app.plan {
        spans.push(Span::styled(
            format!("  {plan}"),
            Style::default()
                .fg(th::TEXT_BOLD)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.extend([
        Span::styled(format!("  {}  ", th::SEP), Style::default().fg(th::DIM)),
        Span::styled(format!("{total}"), Style::default().fg(th::TEXT_BOLD)),
        Span::styled(" sessions   ", Style::default().fg(th::MUTED)),
        Span::styled(format!("{running} running"), status_style(Color::Blue)),
        Span::styled(format!("   {waiting} waiting"), status_style(INDIGO)),
        Span::styled(format!("   {idle} idle"), status_style(Color::Red)),
    ]);
    // Right cluster: Claude Code usage limits + clock, pushed to the top-right.
    let mut right: Vec<Span> = Vec::new();
    if let Some(u) = &app.usage {
        // 5h shows its reset time (resets intra-day); the weekly windows would be
        // ambiguous as bare HH:MM, so they stay just a percentage.
        let blocks: Vec<Vec<Span>> = [
            limit_spans("5h", u.five_hour.as_ref(), true),
            limit_spans("7d", u.seven_day.as_ref(), false),
            limit_spans("sonnet", u.seven_day_sonnet.as_ref(), false),
        ]
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect();
        // Join blocks with a " │ " divider.
        for (i, block) in blocks.into_iter().enumerate() {
            right.push(Span::styled(
                if i == 0 {
                    "  ".to_string()
                } else {
                    format!(" {} ", th::SEP)
                },
                Style::default().fg(th::DIM),
            ));
            right.extend(block);
        }
        if !right.is_empty() && !clock.is_empty() {
            right.push(Span::styled(
                format!(" {} ", th::SEP),
                Style::default().fg(th::DIM),
            ));
        }
    }
    if !clock.is_empty() {
        right.push(Span::styled(clock.clone(), Style::default().fg(th::MUTED)));
    }
    if !right.is_empty() {
        // All glyphs/text on this line are single-width, so a char count gives
        // the exact column of each run.
        let left_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let right_width: usize = right.iter().map(|s| s.content.chars().count()).sum();
        let pad = (area.width as usize)
            .saturating_sub(left_width + right_width)
            .max(1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.append(&mut right);
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[1],
    );
}

/// Traffic-light color for a utilization percentage: green below 50%, orange
/// 50–79%, red at 80%+. Bold is applied by the caller.
fn pct_color(p: i64) -> Color {
    if p >= 80 {
        Color::Red
    } else if p >= 50 {
        Color::Indexed(208) // orange
    } else {
        Color::Green
    }
}

/// Renders one usage window as `label NN%`, optionally with its reset time
/// (`resets HH:MM`). The percentage is bold and traffic-light colored (see
/// `pct_color`). Empty when the window is absent.
fn limit_spans(label: &str, window: Option<&Window>, show_reset: bool) -> Vec<Span<'static>> {
    let Some(w) = window else {
        return Vec::new();
    };
    let p = w.utilization.round() as i64;
    let value_style = Style::default()
        .fg(pct_color(p))
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(format!("{label} "), Style::default().fg(th::MUTED)),
        Span::styled(format!("{p}%"), value_style),
    ];
    if show_reset {
        if let Some(t) = &w.reset_hhmm {
            spans.push(Span::styled(
                format!(" resets {t}"),
                Style::default().fg(th::DIM),
            ));
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::usage::{Usage, Window};
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
    fn header_shows_usage_limits_when_present() {
        let mut app = App::new(Config::default());
        app.clock = String::new(); // isolate the limits cluster
        app.plan = Some("Max 5×".to_string());
        app.usage = Some(Usage {
            five_hour: Some(Window {
                utilization: 77.0,
                reset_unix: None,
                reset_hhmm: Some("14:40".to_string()),
            }),
            seven_day: Some(Window {
                utilization: 8.0,
                reset_unix: None,
                reset_hhmm: Some("04:00".to_string()),
            }),
            seven_day_sonnet: Some(Window {
                utilization: 1.0,
                reset_unix: None,
                reset_hhmm: Some("04:00".to_string()),
            }),
        });
        let mut t = Terminal::new(TestBackend::new(160, 2)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("Max 5×"), "missing plan badge:\n{s}");
        assert!(s.contains("5h 77%"), "missing 5h limit:\n{s}");
        assert!(s.contains("resets 14:40"), "missing 5h reset time:\n{s}");
        assert!(s.contains("7d 8%"), "missing 7d limit:\n{s}");
        assert!(s.contains("sonnet 1%"), "missing sonnet limit:\n{s}");
        assert!(
            !s.contains("7d sonnet"),
            "sonnet should drop the 7d prefix:\n{s}"
        );
        // Blocks are divided by the separator glyph.
        assert!(s.contains(th::SEP), "missing block divider:\n{s}");
        // The weekly windows must not show a reset time (ambiguous as bare HH:MM).
        assert!(
            !s.contains("resets 04:00"),
            "weekly windows should not show a reset time:\n{s}"
        );
    }

    #[test]
    fn pct_color_thresholds() {
        assert_eq!(pct_color(0), Color::Green);
        assert_eq!(pct_color(49), Color::Green);
        assert_eq!(pct_color(50), Color::Indexed(208));
        assert_eq!(pct_color(79), Color::Indexed(208));
        assert_eq!(pct_color(80), Color::Red);
        assert_eq!(pct_color(100), Color::Red);
    }

    #[test]
    fn header_omits_limits_when_absent() {
        let app = App::new(Config::default());
        let mut t = Terminal::new(TestBackend::new(140, 2)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains('%'), "should not render a percentage:\n{s}");
    }
}
