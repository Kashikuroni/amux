use crate::app::{App, HelpTab};
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    // Tab strip: Keys | Changelog. The active tab bolds; the rest is dim.
    let dim = Style::default().fg(th::DIM);
    let active = Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD);
    let keys_style = if app.help_tab == HelpTab::Keys {
        active
    } else {
        dim
    };
    let cl_style = if app.help_tab == HelpTab::Changelog {
        active
    } else {
        dim
    };
    let header = Line::from(vec![
        Span::styled("KEYS", keys_style),
        Span::styled("  ·  ", dim),
        Span::styled("CHANGELOG", cl_style),
        Span::styled("    tab switch · esc close", dim),
    ]);

    let (mut lines, scroll) = match app.help_tab {
        HelpTab::Keys => (keys_lines(), 0),
        HelpTab::Changelog => {
            let entries = crate::changelog::parse(crate::changelog::raw());
            (
                super::modal_whatsnew::changelog_lines(&entries),
                app.help_scroll,
            )
        }
    };
    let mut all = vec![header, Line::from("")];
    all.append(&mut lines);

    // Fixed-size box that doesn't change between the Keys and Changelog tabs;
    // either tab scrolls (ctrl+j/k) if its content overflows.
    let area = super::centered(40, 80, f.area());
    let inner_h = area.height.saturating_sub(2);

    let para = Paragraph::new(all)
        .block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(" help ")
                .style(Style::default().bg(th::BG_RAISED)),
        )
        .wrap(Wrap { trim: false });
    // Clamp to the wrapped row count, not the logical line count — long lines in
    // this narrow box wrap, so `all.len()` would cap the scroll short of bottom.
    let total = para.line_count(area.width) as u16;
    let max_scroll = total.saturating_sub(inner_h);
    let scroll = scroll.min(max_scroll);

    f.render_widget(Clear, area);
    f.render_widget(para.scroll((scroll, 0)), area);
}

/// The keybinding reference as styled lines (the Keys tab body).
fn keys_lines() -> Vec<Line<'static>> {
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
                ("ctrl+r", "return to root (stale cwd)"),
                ("v", "verify session"),
                ("V", "verify details"),
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
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (title, items) in groups {
        lines.push(Line::from(Span::styled(
            title,
            Style::default().fg(th::MUTED).add_modifier(Modifier::BOLD),
        )));
        for (k, label) in items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::default().fg(th::AMBER_HI)),
                Span::styled(label.to_string(), Style::default().fg(th::TEXT)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    #[test]
    fn help_lists_groups_and_keys() {
        let app = App::new(Config::default()); // default tab: Keys
                                               // Wide+tall enough that the fixed (40%) box shows every Keys row at
                                               // scroll 0 without wrap splitting the labels across rows.
        let mut t = Terminal::new(TestBackend::new(140, 80)).unwrap();
        t.draw(|f| render(f, &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("KEYS"), "tab strip:\n{s}");
        assert!(s.contains("CHANGELOG"), "tab strip:\n{s}");
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
        assert!(
            s.contains("return to root"),
            "missing c/stale-cwd entry:\n{s}"
        );
        assert!(s.contains("verify session"), "missing v/verify entry:\n{s}");
        assert!(
            s.contains("verify details"),
            "missing V/verify-details entry:\n{s}"
        );
        // No modifier-key glyphs leak in (arrows are intentionally kept).
        for glyph in ["⇧", "⇥", "↵", "^"] {
            assert!(!s.contains(glyph), "stale key glyph {glyph}:\n{s}");
        }
    }

    #[test]
    fn help_changelog_tab_shows_versions() {
        let mut app = App::new(Config::default());
        app.help_tab = HelpTab::Changelog;
        let mut t = Terminal::new(TestBackend::new(80, 40)).unwrap();
        t.draw(|f| render(f, &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("CHANGELOG"), "tab strip:\n{s}");
        // Newest-first: the latest released version heads the changelog.
        assert!(s.contains("v0.5.1"), "changelog version header:\n{s}");
    }
}
