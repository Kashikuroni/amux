use crate::app::App;
use crate::theme as th;
use crate::usage::pretty_json;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    let entries = match app.usage_log.lock() {
        Ok(g) => g.iter().rev().cloned().collect::<Vec<_>>(),
        Err(_) => return,
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(
                "L Usage log",
                Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   ctrl+k/j scroll · ctrl+y copy · any key close",
                Style::default().fg(th::DIM),
            ),
        ]),
        Line::from(""),
    ];

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no requests yet",
            Style::default().fg(th::DIM),
        )));
    } else {
        for e in &entries {
            // ── separator with metadata ───────────────────────
            let status_str = if e.status == 0 {
                "–".to_string()
            } else {
                e.status.to_string()
            };
            let status_style = if e.status == 0 || e.status >= 400 {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            };
            let mut sep_spans = vec![
                Span::styled("── ", Style::default().fg(th::DIM)),
                Span::styled(e.hms.clone(), Style::default().fg(th::MUTED)),
                Span::styled("  ", Style::default()),
                Span::styled(e.path, Style::default().fg(th::AMBER_HI)),
                Span::styled("  ", Style::default()),
                Span::styled(status_str, status_style),
            ];
            if let Some(err) = &e.error {
                sep_spans.push(Span::styled("  ", Style::default()));
                sep_spans.push(Span::styled(
                    format!("[{err}]"),
                    Style::default().fg(Color::Red),
                ));
            }
            lines.push(Line::from(sep_spans));

            // Pretty-printed body
            let pretty = pretty_json(&e.raw);
            if pretty.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (empty body)",
                    Style::default().fg(th::DIM),
                )));
            } else {
                for json_line in pretty.lines() {
                    lines.push(Line::from(Span::styled(
                        json_line.to_string(),
                        Style::default().fg(th::TEXT),
                    )));
                }
            }
            lines.push(Line::from(""));
        }
    }

    let screen = f.area();
    // 96% wide, up to 90% tall
    let w = ((screen.width as u32 * 96 / 100) as u16).min(screen.width);
    let h = ((screen.height as u32 * 90 / 100) as u16).min(screen.height);
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(w) / 2,
        y: screen.y + screen.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };

    // Inner height (excluding 1-line top/bottom border)
    let inner_h = area.height.saturating_sub(2);
    let total_lines = lines.len() as u16;
    let max_scroll = total_lines.saturating_sub(inner_h);
    let scroll = app.usage_log_scroll.min(max_scroll);

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .block(
                th::panel()
                    .border_style(th::chrome(th::BORDER_HI))
                    .title(" usage log ")
                    .style(ratatui::style::Style::default().bg(th::BG_RAISED)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}
