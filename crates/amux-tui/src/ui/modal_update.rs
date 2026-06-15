use crate::theme as th;
use crate::update::UpdateStage;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, m: &crate::app::UpdateModal) {
    let area = super::centered(56, 30, f.area());
    f.render_widget(Clear, area);
    let current = env!("CARGO_PKG_VERSION");
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "↑ ",
                Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Update available",
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("amux v{}", m.info.version),
                Style::default().fg(th::AMBER),
            ),
            Span::styled(
                format!("  ·  current v{current}"),
                Style::default().fg(th::MUTED),
            ),
        ]),
        Line::from(""),
    ];
    match &m.stage {
        None => {
            lines.push(Line::from(Span::styled(
                "Sessions keep running in tmux; only the amux binary is replaced.",
                Style::default().fg(th::DIM),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" y · install ", Style::default().bg(th::AMBER).fg(th::BG)),
                Span::raw("  "),
                Span::styled(" n · not now ", Style::default().fg(th::TEXT)),
                Span::styled("     esc to dismiss", Style::default().fg(th::DIM)),
            ]));
        }
        Some(UpdateStage::Downloading) => lines.push(progress_line("downloading…")),
        Some(UpdateStage::Verifying) => lines.push(progress_line("verifying checksum…")),
        Some(UpdateStage::Installing) => lines.push(progress_line("installing…")),
        Some(UpdateStage::Done(v)) => {
            lines.push(Line::from(Span::styled(
                format!("updated to v{v}"),
                Style::default()
                    .fg(th::TEXT_BOLD)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" r · restart ", Style::default().bg(th::AMBER).fg(th::BG)),
                Span::styled("     esc · later", Style::default().fg(th::DIM)),
            ]));
        }
        Some(UpdateStage::Failed(reason)) => {
            lines.push(Line::from(Span::styled(
                format!("! {reason}"),
                Style::default().fg(th::RED).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "esc to close",
                Style::default().fg(th::DIM),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            th::panel()
                .border_style(th::chrome(th::BORDER_HI))
                .style(Style::default().bg(th::BG_RAISED)),
        ),
        area,
    );
}

fn progress_line(text: &str) -> Line<'_> {
    Line::from(Span::styled(text, Style::default().fg(th::MUTED)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn update_modal_renders_question_and_stages() {
        use crate::update::{UpdateInfo, UpdateStage};
        let info = UpdateInfo {
            version: "9.9.9".into(),
            url: String::new(),
        };
        let mut m = crate::app::UpdateModal { info, stage: None };
        let draw = |m: &crate::app::UpdateModal| {
            let mut t = Terminal::new(TestBackend::new(90, 50)).unwrap();
            t.draw(|f| render(f, m)).unwrap();
            buf_to_string(t.backend().buffer())
        };
        let s = draw(&m);
        assert!(s.contains("v9.9.9"), "offered version:\n{s}");
        assert!(s.contains("install"), "y hint:\n{s}");
        m.stage = Some(UpdateStage::Downloading);
        assert!(draw(&m).contains("downloading"));
        m.stage = Some(UpdateStage::Done("9.9.9".into()));
        let s = draw(&m);
        assert!(s.contains("updated"), "done text:\n{s}");
        assert!(s.contains("restart"), "r hint:\n{s}");
        // A long reason (the real ones embed MANUAL_HINT) must wrap, not clip.
        m.stage = Some(UpdateStage::Failed(format!(
            "no write access to /usr/local/bin — {}",
            crate::update::MANUAL_HINT
        )));
        let s = draw(&m);
        assert!(s.contains("no write access"), "reason start:\n{s}");
        assert!(
            s.contains("cargo binstall"),
            "recovery tail must be visible:\n{s}"
        );
    }
}
