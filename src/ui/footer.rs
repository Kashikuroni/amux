use crate::app::{App, Mode};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// (key, label, accent)
type Item = (&'static str, &'static str, bool);

fn items_for(mode: &Mode) -> Vec<Item> {
    match mode {
        Mode::Create(_) => vec![
            ("↵", "create", true),
            ("⇥", "next field", false),
            ("←→", "pick agent", false),
            ("esc", "cancel", false),
        ],
        Mode::ConfirmDelete(_) => vec![
            ("y", "yes, kill", true),
            ("n", "no", false),
            ("esc", "cancel", false),
        ],
        Mode::Help => vec![("esc", "close", true), ("q", "quit", false)],
        Mode::Rename(_) => vec![("↵", "rename", true), ("esc", "cancel", false)],
        Mode::Filter => vec![
            ("type", "filter", true),
            ("↑↓", "move", false),
            ("esc", "clear", false),
        ],
        Mode::List => vec![
            ("n", "new", true),
            ("↵", "attach", false),
            ("d", "kill", false),
            ("r", "rename", false),
            ("/", "filter", false),
            ("?", "help", false),
            ("q", "quit", false),
        ],
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize))
            .style(Style::default().fg(th::BORDER)),
        rows[0],
    );
    let mut spans: Vec<Span> = Vec::new();
    for (i, (k, label, accent)) in items_for(&app.mode).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", Style::default().fg(th::DIM)));
        }
        let key_style = if accent {
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(th::TEXT_BOLD)
        };
        spans.push(Span::styled(k, key_style));
        spans.push(Span::styled(format!(" {label}"), Style::default().fg(th::MUTED)));
    }
    if let Some(q) = &app.filter {
        spans.push(Span::styled(format!("    /{q}"), Style::default().fg(th::AMBER_HI)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}
