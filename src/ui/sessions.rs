use crate::app::App;
use crate::spinner;
use crate::theme as th;
use crate::timeutil;
use crate::tmux::{Session, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

fn card(s: &Session, now: i64, spinner_frame: usize) -> ListItem<'static> {
    // Line 1: name  ........  status
    let (status_glyph, status_label, status_color) = match s.status {
        Status::Running => (
            spinner::glyph(spinner_frame).to_string(),
            "running",
            th::AMBER_HI,
        ),
        Status::Idle => (th::IDLE_DOT.to_string(), "idle", th::MUTED),
    };
    let line1 = Line::from(vec![
        Span::styled(s.name.clone(), Style::default().fg(th::TEXT_BOLD)),
        Span::raw("  "),
        Span::styled(status_glyph, Style::default().fg(status_color)),
        Span::styled(format!(" {status_label}"), Style::default().fg(status_color)),
    ]);

    // Line 2: dir
    let line2 = Line::from(Span::styled(
        s.dir.clone(),
        Style::default().fg(th::MUTED),
    ));

    // Line 3: ✻ agent · ⎇ branch · +a −d   age
    let mut l3 = vec![
        Span::styled(th::AGENT_MARK, Style::default().fg(th::DIM)),
        Span::styled(format!(" {}", s.agent), Style::default().fg(th::MUTED)),
    ];
    if let Some(g) = &s.git {
        l3.push(Span::styled(
            format!("   {} {}", th::BRANCH, g.branch),
            Style::default().fg(th::DIM),
        ));
        l3.push(Span::styled(
            format!("   +{}", g.added),
            Style::default().fg(th::GREEN),
        ));
        l3.push(Span::styled(
            format!(" −{}", g.removed),
            Style::default().fg(th::RED),
        ));
    }
    let age = timeutil::humanize_age(now - s.created);
    l3.push(Span::styled(
        format!("   {age}"),
        Style::default().fg(th::MUTED),
    ));
    let line3 = Line::from(l3);

    ListItem::new(vec![line1, line2, line3])
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Section label
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SESSIONS", Style::default().fg(th::MUTED)),
            Span::styled("   ↑↓ navigate", Style::default().fg(th::DIM)),
        ])),
        rows[0],
    );

    let vis = app.visible_indices();
    let items: Vec<ListItem> = vis
        .iter()
        .map(|&i| card(&app.sessions[i], app.now_unix, app.spinner_frame))
        .collect();

    // bind sym so highlight_symbol can borrow a &str
    let sym = format!("{} ", th::SEL_BAR);
    let list = List::new(items)
        .highlight_symbol(sym.as_str())
        .highlight_style(
            Style::default()
                .bg(th::SEL_BG)
                .fg(th::AMBER_HI)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    if !vis.is_empty() {
        state.select(Some(app.selected.min(vis.len() - 1)));
    }
    f.render_stateful_widget(list, rows[1], &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::git::GitInfo;
    use crate::tmux::Session;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn buf_to_string(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn sess(name: &str, status: Status, git: Option<GitInfo>) -> Session {
        Session {
            name: name.into(),
            dir: "~/work/x".into(),
            created: 0,
            agent: "claude".into(),
            status,
            attached: false,
            git,
        }
    }

    #[test]
    fn renders_running_and_idle_cards_with_git() {
        let mut app = App::new(Config::default());
        app.now_unix = 185; // → "3m" age
        app.spinner_frame = 0;
        app.sessions = vec![
            sess(
                "project-a",
                Status::Running,
                Some(GitInfo { branch: "main".into(), added: 12, removed: 4 }),
            ),
            sess("project-b", Status::Idle, None),
        ];
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("project-a"), "missing project-a:\n{s}");
        assert!(s.contains("running"), "missing 'running':\n{s}");
        assert!(s.contains("⠋"), "missing spinner frame 0:\n{s}");
        assert!(s.contains("main"), "missing branch:\n{s}");
        assert!(s.contains("+12"), "missing +12:\n{s}");
        assert!(s.contains("idle"), "missing 'idle':\n{s}");
        assert!(s.contains("3m"), "missing age '3m':\n{s}");
    }
}
