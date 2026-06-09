use crate::app::App;
use crate::changelog::Entry;
use crate::theme as th;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Renders parsed changelog [`Entry`]s as styled lines: a bold version header
/// (with date) followed by the section body rendered through the Markdown
/// renderer (headings, **bold**, `code`, bullets). Shared by the What's New
/// modal and the Help → Changelog tab.
pub(crate) fn changelog_lines(entries: &[Entry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for e in entries {
        let header = match &e.date {
            Some(d) => format!("v{}  —  {d}", e.version),
            None => format!("v{}", e.version),
        };
        lines.push(Line::from(Span::styled(
            header,
            Style::default()
                .fg(th::AMBER)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        lines.extend(super::markdown::render(&e.body));
        lines.push(Line::from(""));
    }
    lines
}

pub fn render(f: &mut Frame, app: &App) {
    // Header kept short: the narrow box would wrap a long hint, and the footer
    // already shows the scroll/close keys.
    let mut lines = vec![
        Line::from(Span::styled(
            "✨ What's New",
            Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(changelog_lines(&app.whats_new));

    // Exactly the `i` reply composer's width (shared constant); the content
    // scrolls (keys or mouse wheel) rather than the box growing wider.
    let area = super::centered(super::NARROW_MODAL_PCT, 72, f.area());
    let inner_h = area.height.saturating_sub(2);

    let para = Paragraph::new(lines)
        .block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .title(" what's new ")
                .style(Style::default().bg(th::BG_RAISED)),
        )
        .wrap(Wrap { trim: false });
    // Clamp to the *wrapped* height: in this narrow box long lines wrap to many
    // visual rows, so `lines.len()` under-counts and would cap the scroll short
    // of the real bottom. `line_count` reports the post-wrap row count.
    let total = para.line_count(area.width) as u16;
    let max_scroll = total.saturating_sub(inner_h);
    let scroll = app.whats_new_scroll.min(max_scroll);

    f.render_widget(Clear, area);
    f.render_widget(para.scroll((scroll, 0)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn whats_new_shows_version_and_notes() {
        let mut app = App::new(Config::default());
        app.whats_new = vec![Entry {
            version: "0.6.0".into(),
            date: Some("2026-07-01".into()),
            body: "### Added\n- A shiny feature.".into(),
        }];
        // Normal-size terminal: the box is a fixed narrow share of it.
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| render(f, &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("What's New"), "title:\n{s}");
        assert!(s.contains("v0.6.0"), "version header:\n{s}");
        assert!(s.contains("shiny feature"), "notes body:\n{s}");
    }

    #[test]
    fn whats_new_scrolls_to_bottom_despite_wrapping() {
        // A long first entry that wraps to many rows in the narrow box, then a
        // short final entry. The scroll cap must use the wrapped row count, not
        // the logical line count — else it clamps short and the bottom entry is
        // unreachable (the reported "hangs partway" bug).
        let mut app = App::new(Config::default());
        app.whats_new = vec![
            Entry {
                version: "0.9.0".into(),
                date: None,
                body: format!("- {}", "wrap ".repeat(60)),
            },
            Entry {
                version: "0.1.0".into(),
                date: None,
                body: "- final entry".into(),
            },
        ];
        app.whats_new_scroll = u16::MAX; // way past bottom; render clamps to max
        let mut t = Terminal::new(TestBackend::new(80, 16)).unwrap();
        t.draw(|f| render(f, &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(
            s.contains("v0.1.0") && s.contains("final entry"),
            "max scroll must reveal the last entry:\n{s}"
        );
    }
}
