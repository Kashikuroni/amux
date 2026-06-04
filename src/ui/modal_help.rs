use crate::theme as th;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame) {
    let groups: [(&str, &[(&str, &str)]); 4] = [
        (
            "Navigation",
            &[
                ("k j / ↑↓", "move"),
                ("s 1-9", "jump to session"),
                ("g", "first session"),
                ("/", "filter"),
            ],
        ),
        (
            "Session",
            &[
                ("enter o", "attach"),
                ("n", "new"),
                ("N", "new in project"),
                ("i", "reply to agent"),
                ("1-9", "answer prompt"),
                ("d", "kill"),
                ("r", "rename"),
                ("R", "rename project"),
                ("shift+tab", "agent mode"),
                ("J/K", "reorder (project on edge)"),
                ("u", "restart all Claude sessions"),
            ],
        ),
        (
            "Preview",
            &[
                ("ctrl+k/j", "scroll up · down"),
                ("PgUp PgDn", "scroll up · down"),
                ("G", "jump to latest"),
                ("[ ] { }", "resize split"),
                ("ctrl+←→", "resize split (±8)"),
                ("auto", "refresh on interval"),
            ],
        ),
        (
            "App",
            &[
                ("?", "help"),
                ("L", "usage log"),
                ("q", "quit (sessions stay)"),
            ],
        ),
    ];
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "? Help",
                Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   keys & shortcuts", Style::default().fg(th::DIM)),
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
    // Size the panel to its content (lines + top/bottom border), centered and
    // clamped to the screen — so nothing clips and large terminals get no empty
    // box. Width is a fixed share wide enough for the longest "key  label" row.
    let screen = f.area();
    let h = (lines.len() as u16 + 2).min(screen.height);
    let w = ((screen.width as u32 * 60 / 100) as u16).min(screen.width);
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(w) / 2,
        y: screen.y + screen.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(" help ")
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    #[test]
    fn help_lists_groups_and_keys() {
        // Tall enough that the content-sized panel renders every group.
        let mut t = Terminal::new(TestBackend::new(80, 40)).unwrap();
        t.draw(render).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("Help"));
        assert!(s.contains("Navigation"));
        assert!(s.contains("attach"));
        assert!(s.contains("filter"));
        // The combos that were previously missing must all be listed.
        assert!(s.contains("new in project"), "missing N:\n{s}");
        assert!(s.contains("reply to agent"), "missing i:\n{s}");
        assert!(s.contains("answer prompt"), "missing 1-9:\n{s}");
        assert!(s.contains("resize split"), "missing split resize:\n{s}");
        assert!(
            s.contains("quit (sessions stay)"),
            "bottom group clipped:\n{s}"
        );
        assert!(s.contains("restart"), "missing u/restart entry:\n{s}");
        // No modifier-key glyphs leak in (arrows are intentionally kept).
        for glyph in ["⇧", "⇥", "↵", "^"] {
            assert!(!s.contains(glyph), "stale key glyph {glyph}:\n{s}");
        }
    }
}
