use crate::app::LeaderMenu;
use crate::theme as th;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

/// `(key, label)` rows of one leader group.
const GIT: &[(&str, &str)] = &[
    ("i", "create github issue"),
    ("p", "promote worktree to root"),
    ("b", "delete session branch"),
    ("c", "cleanup merged branches"),
];
const SESSION: &[(&str, &str)] = &[
    ("r", "rename session"),
    ("R", "rename project"),
    ("v", "verify / cancel"),
    ("V", "verification details"),
    ("e", "nvim in agent dir"),
];
const APP: &[(&str, &str)] = &[
    ("l", "usage log"),
    ("o", "other tmux sessions"),
    ("u", "restart all claude sessions"),
];

/// The root row of a group: its key, name, and a one-line command preview.
fn group_preview(items: &[(&str, &str)]) -> String {
    items
        .iter()
        .map(|(k, label)| format!("{k} {label}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Which-key panel for the space leader: anchored above the footer, the root
/// shows every group with its commands inline; a group view spells them out
/// one per row. Purely informative — keys are handled in `App::handle_leader_key`.
pub fn render(f: &mut Frame, menu: LeaderMenu) {
    let rows: Vec<Line> = match menu {
        LeaderMenu::Root => [
            ("g", "git", GIT),
            ("s", "session", SESSION),
            ("a", "app", APP),
        ]
        .into_iter()
        .map(|(key, name, items)| {
            Line::from(vec![
                key_span(key),
                Span::styled(
                    format!("  {name:<8}"),
                    Style::default()
                        .fg(th::TEXT_BOLD)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    group_preview(items),
                    Style::default().fg(th::MUTED).add_modifier(Modifier::DIM),
                ),
            ])
        })
        .collect(),
        LeaderMenu::Git | LeaderMenu::Session | LeaderMenu::App => {
            let items = match menu {
                LeaderMenu::Git => GIT,
                LeaderMenu::Session => SESSION,
                _ => APP,
            };
            items
                .iter()
                .map(|(key, label)| {
                    Line::from(vec![
                        key_span(key),
                        Span::styled(format!("  {label}"), Style::default().fg(th::TEXT)),
                    ])
                })
                .collect()
        }
    };

    let title = match menu {
        LeaderMenu::Root => " space ".to_string(),
        LeaderMenu::Git => " space g · git ".to_string(),
        LeaderMenu::Session => " space s · session ".to_string(),
        LeaderMenu::App => " space a · app ".to_string(),
    };

    // Anchor just above the footer, mirroring ui::draw's outer margins.
    let screen = f.area();
    let outer = Rect {
        x: screen.x + 2,
        y: screen.y + 1,
        width: screen.width.saturating_sub(4),
        height: screen.height.saturating_sub(2),
    };
    let h = (rows.len() as u16 + 2).min(outer.height);
    let area = Rect {
        x: outer.x,
        y: outer.y + outer.height.saturating_sub(2 + h),
        width: outer.width,
        height: h,
    };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(rows).block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(title)
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

fn key_span(k: &str) -> Span<'static> {
    Span::styled(
        format!("  {k}"),
        Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn root_lists_all_groups_with_command_previews() {
        let mut t = Terminal::new(TestBackend::new(140, 24)).unwrap();
        t.draw(|f| render(f, LeaderMenu::Root)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("space"), "panel title:\n{s}");
        for group in ["git", "session", "app"] {
            assert!(s.contains(group), "group {group} listed:\n{s}");
        }
        assert!(s.contains("i create github issue"), "git preview:\n{s}");
        assert!(s.contains("u restart all claude"), "app preview:\n{s}");
    }

    #[test]
    fn git_group_spells_out_commands() {
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
        t.draw(|f| render(f, LeaderMenu::Git)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("space g · git"), "breadcrumb title:\n{s}");
        assert!(s.contains("create github issue"), "issue row:\n{s}");
        assert!(s.contains("promote worktree to root"), "promote row:\n{s}");
        assert!(s.contains("delete session branch"), "delete row:\n{s}");
        assert!(s.contains("cleanup merged branches"), "cleanup row:\n{s}");
    }

    #[test]
    fn session_group_keeps_old_direct_letters() {
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
        t.draw(|f| render(f, LeaderMenu::Session)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        for row in [
            "r  rename session",
            "R  rename project",
            "v  verify / cancel",
            "V  verification details",
            "e  nvim in agent dir",
        ] {
            assert!(s.contains(row), "row {row:?} present:\n{s}");
        }
    }
}
