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
            ("enter", "create", true),
            ("↑↓ j/k", "move", false),
            ("←→ h/l", "pick", false),
            ("tab", "complete", false),
            ("esc", "cancel", false),
        ],
        Mode::ConfirmDelete(_) => vec![
            ("y", "yes, kill", true),
            ("n", "no", false),
            ("esc", "cancel", false),
        ],
        Mode::ConfirmRestart(_) => vec![
            ("yes", "type to confirm", true),
            ("enter", "restart", false),
            ("esc", "cancel", false),
        ],
        Mode::Help => vec![("esc", "close", true), ("q", "quit", false)],
        Mode::Rename(_) => vec![("enter", "rename", true), ("esc", "cancel", false)],
        Mode::RenameProject(_) => {
            vec![("enter", "rename project", true), ("esc", "cancel", false)]
        }
        Mode::Reply(_) => vec![
            ("enter", "send", true),
            ("shift+enter", "newline", false),
            ("esc", "cancel", false),
        ],
        Mode::Filter => vec![
            ("type", "filter", true),
            ("↑↓", "move", false),
            ("esc", "clear", false),
        ],
        Mode::SelectSession => vec![("1-9", "select session", true), ("esc", "cancel", false)],
        Mode::List => vec![
            ("n", "new", true),
            ("N", "new in proj", false),
            ("enter", "attach", false),
            ("J/K", "reorder", false),
            ("shift+tab", "agent mode", false),
            ("1-9", "answer", false),
            ("i", "reply", false),
            ("t", "notes", false),
            ("d", "kill", false),
            ("r", "rename", false),
            ("R", "rename proj", false),
            ("/", "filter", false),
            ("?", "help", false),
            ("q", "quit", false),
        ],
        Mode::Note(ns) if ns.confirm_clear => {
            vec![("y", "clear note", true), ("n", "cancel", false)]
        }
        Mode::Note(ns) => match ns.sub {
            crate::app::NoteSub::Edit => vec![("esc", "done", true), ("enter", "newline", false)],
            crate::app::NoteSub::Render => vec![
                ("j/k", "task", true),
                ("space", "toggle", false),
                ("V", "select", false),
                ("y", "copy", false),
                ("e", "edit", false),
                ("c", "clear", false),
                ("tab", "defocus", false),
                ("esc", "exit", false),
            ],
        },
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    f.render_widget(
        Paragraph::new(th::RULE_CHAR.repeat(area.width as usize)).style(th::chrome(th::BORDER)),
        rows[0],
    );
    let mut spans: Vec<Span> = Vec::new();
    for (i, (k, label, accent)) in items_for(&app.mode).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("   ", Style::default().fg(th::DIM)));
        }
        // Keys read at full strength (BOLD) so the chords catch the eye in
        // peripheral vision; the labels recede to DIM so what they do is only
        // legible on a deliberate look.
        let key_style = if accent {
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(th::TEXT_BOLD)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(k, key_style));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(th::MUTED).add_modifier(Modifier::DIM),
        ));
    }
    if let Some(q) = &app.filter {
        // Appended inline; clips silently at narrow widths (ratatui won't wrap a Line).
        spans.push(Span::styled(
            format!("    /{q}"),
            Style::default().fg(th::AMBER_HI),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn list_footer_spells_keys_without_modifier_glyphs() {
        // Wide enough that the List hints don't clip before the assertions.
        let app = App::new(Config::default()); // defaults to Mode::List
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        // shift+letter collapses to the capital letter; named keys are words.
        assert!(s.contains("N new in proj"), "shift+N → N:\n{s}");
        assert!(s.contains("J/K reorder"), "shift+JK → J/K:\n{s}");
        assert!(
            s.contains("shift+tab agent mode"),
            "shift+tab spelled:\n{s}"
        );
        assert!(s.contains("enter attach"), "enter spelled:\n{s}");
        // The shift/tab/enter glyphs must be gone (arrows are kept elsewhere).
        for glyph in ["⇧", "⇥", "↵"] {
            assert!(!s.contains(glyph), "stale key glyph {glyph}:\n{s}");
        }
    }

    #[test]
    fn create_footer_lists_form_navigation() {
        let mut app = App::new(Config::default());
        app.mode = Mode::Create(crate::app::CreateForm::new("claude", &[]));
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("enter create"), "enter hint:\n{s}");
        assert!(s.contains("move"), "j/k move hint:\n{s}");
        assert!(s.contains("tab complete"), "tab hint:\n{s}");
        assert!(!s.contains("next field"), "old wizard hint gone:\n{s}");
    }

    #[test]
    fn keys_are_bold_labels_are_dim() {
        let app = App::new(Config::default());
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let mut bold_key = false; // a key cell: BOLD, not DIM
        let mut dim_label = false; // a label cell: DIM, not BOLD
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() == " " {
                    continue;
                }
                let m = buf[(x, y)].style().add_modifier;
                if m.contains(Modifier::BOLD) && !m.contains(Modifier::DIM) {
                    bold_key = true;
                }
                if m.contains(Modifier::DIM) && !m.contains(Modifier::BOLD) {
                    dim_label = true;
                }
            }
        }
        assert!(bold_key, "key chords must render BOLD");
        assert!(dim_label, "labels must render DIM");
    }
}
