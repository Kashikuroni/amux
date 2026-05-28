use crate::app::{resolve_agent_path, CreateField, CreateForm};
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let marker = if focused { "▌ " } else { "  " };
    Line::from(vec![
        Span::styled(
            marker,
            Style::default().fg(if focused { th::AMBER } else { th::BORDER }),
        ),
        Span::styled(format!("{label}: "), Style::default().fg(th::MUTED)),
        Span::styled(value.to_string(), Style::default().fg(th::TEXT_BOLD)),
    ])
}

pub fn render(f: &mut Frame, form: &CreateForm) {
    let area = super::centered(64, 80, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![
        Line::from(Span::styled(
            "＋ New session",
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        field_line("name ", &form.name, form.field == CreateField::Name),
        field_line("dir  ", &form.dir, form.field == CreateField::Dir),
    ];

    if form.field == CreateField::Dir {
        // Live subdir picker (browse-aware). Window the selection to keep it visible.
        // 10 rows reserved: 4 above (title, blank, name, dir) + 6 below (blank,
        // agent label, segments, $ cmd, resolved-path hint, trailing blank).
        let h = area.height.saturating_sub(2) as usize;
        let cap = h.saturating_sub(10).max(1);
        let total = form.dir_entries.len();
        let start = if form.dir_selected >= cap {
            form.dir_selected + 1 - cap
        } else {
            0
        };
        let end = (start + cap).min(total);
        for i in start..end {
            let selected = i == form.dir_selected;
            let text = format!(
                "    {}{}/",
                if selected { "> " } else { "  " },
                form.dir_entries[i]
            );
            let style = if selected {
                Style::default().fg(th::AMBER_HI).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(th::MUTED)
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }

    // Agent segmented selector
    let mut seg: Vec<Span> = vec![Span::styled(
        "  agent: ",
        Style::default().fg(if form.field == CreateField::Agent { th::AMBER } else { th::MUTED }),
    )];
    for (i, choice) in form.agent_choices.iter().enumerate() {
        let sel = i == form.agent_index;
        let st = if sel {
            Style::default()
                .bg(th::AMBER)
                .fg(th::BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(th::MUTED)
        };
        seg.push(Span::styled(format!(" {choice} "), st));
        seg.push(Span::raw(" "));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(seg));

    // TODO: forks `sh -c "command -v ..."` on every redraw (incl. every keystroke
    // in the agent field). Modal is short-lived so fine for now; a
    // `(last_agent, resolved): (String, Option<String>)` cache on CreateForm
    // would eliminate it.
    // Resolved command line
    let resolved = resolve_agent_path(&form.agent);
    let cmd = if form.agent.is_empty() {
        "<type a command>".to_string()
    } else {
        form.agent.clone()
    };
    lines.push(Line::from(vec![
        Span::styled("  $ ", Style::default().fg(th::DIM)),
        Span::styled(cmd, Style::default().fg(th::TEXT_BOLD)),
    ]));
    lines.push(Line::from(Span::styled(
        match &resolved {
            Some(p) => format!("  found at {p}"),
            None => "  not found in PATH".to_string(),
        },
        Style::default().fg(if resolved.is_some() { th::DIM } else { th::YELLOW }),
    )));

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(th::AMBER))
                .title(" new session "),
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
    fn new_modal_shows_fields_and_agent_segments() {
        let form = CreateForm::new("claude", &["claude".into(), "codex".into()]);
        let mut t = Terminal::new(TestBackend::new(80, 22)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("New session"));
        assert!(s.contains("name"));
        assert!(s.contains("claude"));
        assert!(s.contains("codex"));
        assert!(s.contains("custom"));
    }
}
