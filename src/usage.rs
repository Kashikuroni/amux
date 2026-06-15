// Re-export the pure layer from amux-core so `crate::usage::Usage` still works
pub use amux_core::usage::*;

use crate::theme as th;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

fn pct_color(p: i64) -> Color {
    if p >= 80 {
        Color::Red
    } else if p >= 50 {
        Color::Indexed(208) // orange
    } else {
        Color::Green
    }
}

pub(crate) fn limit_spans(
    label: &str,
    window: Option<&Window>,
    show_reset: bool,
) -> Vec<Span<'static>> {
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

pub(crate) fn account_right_spans(
    usage: Option<&Usage>,
    plan: Option<&str>,
    usage_error: Option<&str>,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span> = Vec::new();

    if let Some(p) = plan {
        out.push(Span::styled(
            p.to_string(),
            Style::default()
                .fg(th::TEXT_BOLD)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(u) = usage {
        let blocks: Vec<Vec<Span>> = [
            limit_spans("5h", u.five_hour.as_ref(), true),
            limit_spans("7d", u.seven_day.as_ref(), false),
            limit_spans("sonnet", u.seven_day_sonnet.as_ref(), false),
        ]
        .into_iter()
        .filter(|b| !b.is_empty())
        .collect();

        for (i, block) in blocks.into_iter().enumerate() {
            if i > 0 || !out.is_empty() {
                out.push(Span::styled(
                    format!(" {} ", th::SEP),
                    Style::default().fg(th::DIM),
                ));
            }
            out.extend(block);
        }
    } else if let Some(err) = usage_error {
        if !out.is_empty() {
            out.push(Span::styled(
                format!(" {} ", th::SEP),
                Style::default().fg(th::DIM),
            ));
        }
        out.push(Span::styled(
            format!("limits ✗ {err}"),
            Style::default().fg(th::DIM),
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_spans_renders_label_pct_and_reset() {
        let w = Window {
            utilization: 77.0,
            reset_unix: None,
            reset_hhmm: Some("14:40".to_string()),
        };
        let spans = limit_spans("5h", Some(&w), true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("5h"), "missing label: {text}");
        assert!(text.contains("77%"), "missing pct: {text}");
        assert!(text.contains("resets 14:40"), "missing reset: {text}");
    }

    #[test]
    fn limit_spans_omits_reset_when_show_reset_false() {
        let w = Window {
            utilization: 8.0,
            reset_unix: None,
            reset_hhmm: Some("04:00".to_string()),
        };
        let spans = limit_spans("7d", Some(&w), false);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("7d"), "missing label: {text}");
        assert!(text.contains("8%"), "missing pct: {text}");
        assert!(!text.contains("resets"), "must not show reset: {text}");
    }

    #[test]
    fn limit_spans_empty_when_window_absent() {
        assert!(limit_spans("5h", None, true).is_empty());
    }

    #[test]
    fn pct_color_thresholds() {
        use ratatui::style::Color;
        assert_eq!(pct_color(0), Color::Green);
        assert_eq!(pct_color(49), Color::Green);
        assert_eq!(pct_color(50), Color::Indexed(208));
        assert_eq!(pct_color(79), Color::Indexed(208));
        assert_eq!(pct_color(80), Color::Red);
        assert_eq!(pct_color(100), Color::Red);
    }

    #[test]
    fn account_right_spans_empty_when_no_data() {
        assert!(account_right_spans(None, None, None).is_empty());
    }

    #[test]
    fn account_right_spans_includes_plan_and_usage() {
        let usage = Usage {
            five_hour: Some(Window {
                utilization: 77.0,
                reset_unix: None,
                reset_hhmm: Some("14:40".to_string()),
            }),
            seven_day: Some(Window {
                utilization: 8.0,
                reset_unix: None,
                reset_hhmm: None,
            }),
            seven_day_sonnet: None,
        };
        let spans = account_right_spans(Some(&usage), Some("Max 5×"), None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Max 5×"), "missing plan: {text}");
        assert!(text.contains("5h"), "missing 5h label: {text}");
        assert!(text.contains("77%"), "missing 5h pct: {text}");
        assert!(text.contains("7d"), "missing 7d label: {text}");
        assert!(text.contains("8%"), "missing 7d pct: {text}");
    }

    #[test]
    fn account_right_spans_shows_error_when_no_usage() {
        let spans = account_right_spans(None, None, Some("429"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("limits"), "missing 'limits': {text}");
        assert!(text.contains("429"), "missing error code: {text}");
    }

    #[test]
    fn account_right_spans_plan_with_error() {
        let spans = account_right_spans(None, Some("Pro"), Some("net"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Pro"), "missing plan: {text}");
        assert!(text.contains("net"), "missing error: {text}");
    }
}
