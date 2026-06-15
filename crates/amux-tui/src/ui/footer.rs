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
            ("enter", "next", true),
            ("shift+enter", "create", false),
            ("↑↓", "move", false),
            ("h/l j/k", "agent", false),
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
        Mode::Help => vec![
            ("tab", "keys/changelog", true),
            ("esc", "close", false),
            ("q", "quit", false),
        ],
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
        // Slim on purpose: frequent direct keys only. Everything rarer lives in
        // the space-leader menu, which documents itself.
        Mode::List => vec![
            ("n", "new", true),
            ("enter", "attach", false),
            ("i", "reply", false),
            ("1-9", "answer", false),
            ("t", "notes", false),
            ("d", "kill", false),
            ("space", "menu", false),
            ("/", "filter", false),
            ("?", "help", false),
            ("q", "quit", false),
        ],
        Mode::UsageLog => vec![("any key", "close", true)],
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
                ("d", "delete", false),
                ("e", "edit", false),
                ("c", "clear", false),
                ("tab", "defocus", false),
                ("esc", "exit", false),
            ],
        },
        Mode::ConfirmUpdate(m) => match &m.stage {
            None => vec![
                ("y", "install", true),
                ("n", "not now", false),
                ("esc", "dismiss", false),
            ],
            Some(crate::update::UpdateStage::Done(_)) => {
                vec![("r", "restart", true), ("esc", "later", false)]
            }
            Some(crate::update::UpdateStage::Failed(_)) => vec![("esc", "close", false)],
            Some(_) => vec![("esc", "hide", false)],
        },
        Mode::Git(_) => vec![("esc", "cancel", true)],
        Mode::VerifyDetail(_) => vec![("esc", "close", true)],
        Mode::WhatsNew => vec![("ctrl+j/k", "scroll", true), ("any key", "close", false)],
        Mode::ForeignSessions(_) => vec![("any key", "close", true)],
        Mode::Leader(crate::app::LeaderMenu::Root) => {
            vec![("g/s/a", "group", true), ("esc", "close", false)]
        }
        Mode::Leader(_) => vec![("backspace", "back", true), ("esc", "close", false)],
        Mode::Issue(form) => match &form.stage {
            None => vec![
                ("shift+enter", "create issue", true),
                ("enter", "next/newline", false),
                ("tab", "field", false),
                ("esc", "cancel", false),
            ],
            Some(crate::git::IssueStage::Creating) => vec![("esc", "hide", true)],
            Some(_) => vec![("any key", "close", true)],
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
    fn update_footer_follows_stage() {
        use crate::update::{UpdateInfo, UpdateStage};
        let mk = |stage| {
            let mut app = App::new(Config::default());
            app.mode = Mode::ConfirmUpdate(crate::app::UpdateModal {
                info: UpdateInfo {
                    version: "9.9.9".into(),
                    url: String::new(),
                },
                stage,
            });
            let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
            t.draw(|f| render(f, f.area(), &app)).unwrap();
            buf_to_string(t.backend().buffer())
        };
        assert!(mk(None).contains("y install"));
        assert!(mk(Some(UpdateStage::Downloading)).contains("esc hide"));
        assert!(mk(Some(UpdateStage::Done("9.9.9".into()))).contains("r restart"));
        assert!(mk(Some(UpdateStage::Failed("x".into()))).contains("esc close"));
    }

    #[test]
    fn list_footer_is_slim_and_spells_keys_without_glyphs() {
        // Wide enough that the List hints don't clip before the assertions.
        let app = App::new(Config::default()); // defaults to Mode::List
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("enter attach"), "enter spelled:\n{s}");
        assert!(s.contains("space menu"), "leader entry point:\n{s}");
        // Rare ops moved to the leader: their hints must be gone from the footer.
        for gone in ["rename", "reorder", "agent mode", "verify"] {
            assert!(!s.contains(gone), "footer must stay slim ({gone}):\n{s}");
        }
        // The shift/tab/enter glyphs must be gone (arrows are kept elsewhere).
        for glyph in ["⇧", "⇥", "↵"] {
            assert!(!s.contains(glyph), "stale key glyph {glyph}:\n{s}");
        }
    }

    #[test]
    fn leader_footer_shows_groups_then_back() {
        let mut app = App::new(Config::default());
        app.mode = Mode::Leader(crate::app::LeaderMenu::Root);
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("g/s/a group"), "root hints:\n{s}");

        app.mode = Mode::Leader(crate::app::LeaderMenu::Git);
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("backspace back"), "group hints:\n{s}");
    }

    #[test]
    fn create_footer_lists_form_navigation() {
        let mut app = App::new(Config::default());
        app.mode = Mode::Create(crate::app::CreateForm::new("claude", &[]));
        let mut t = Terminal::new(TestBackend::new(120, 6)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("enter next"), "enter hint:\n{s}");
        assert!(s.contains("shift+enter create"), "submit hint:\n{s}");
        assert!(s.contains("move"), "arrow move hint:\n{s}");
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
