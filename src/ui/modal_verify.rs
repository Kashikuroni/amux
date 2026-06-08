use crate::app::{App, VerificationState};
use crate::theme as th;
use amux_verify::GateStatus;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, name: &str) {
    let Some(VerificationState::Done(verdict)) = app.verification.get(name) else {
        return;
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("verify · {name}"),
        Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for g in &verdict.gates {
        let (glyph, color) = match g.status {
            GateStatus::Passed => ("✓", Color::Green),
            GateStatus::Failed => ("✗", Color::Red),
            GateStatus::TimedOut => ("⏱", Color::Red),
            GateStatus::Skipped => ("⊘", Color::Reset),
        };
        let secs = g.duration_ms as f64 / 1000.0;
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::raw(format!("{:<14} ({secs:.1}s)", g.name)),
        ]));
    }
    for g in verdict
        .gates
        .iter()
        .filter(|g| g.status != GateStatus::Passed)
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("repro: {}", g.repro),
            Style::default().add_modifier(Modifier::DIM),
        )));
        for tail in g.stderr_tail.lines().chain(g.stdout_tail.lines()) {
            lines.push(Line::from(Span::styled(
                format!("  {tail}"),
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
    }

    // Centered box — same approach as modal_help::render.
    let area = centered(f.area(), 72, (lines.len() as u16 + 2).min(f.area().height));
    f.render_widget(Clear, area);
    let block = th::panel()
        .border_style(th::chrome(th::BORDER_HI))
        .title(" verify ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Centers a `w`×`h` rect inside `area` (clamped to `area`).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ui::testutil::buf_to_string;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn lists_gates_and_failure_details() {
        let mut app = App::new(Config::default());
        let verdict = amux_verify::Verdict {
            task_id: None,
            passed: false,
            gates: vec![amux_verify::GateResult {
                name: "clippy".into(),
                status: GateStatus::Failed,
                exit_code: Some(1),
                duration_ms: 1200,
                stdout_tail: String::new(),
                stderr_tail: "error: bad thing".into(),
                repro: "cargo clippy".into(),
            }],
        };
        app.verification
            .insert("feat".into(), VerificationState::Done(verdict));
        let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
        t.draw(|f| render(f, &app, "feat")).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("verify · feat"), "{s}");
        assert!(s.contains("clippy"), "{s}");
        assert!(s.contains("repro: cargo clippy"), "{s}");
        assert!(s.contains("error: bad thing"), "{s}");
    }
}
