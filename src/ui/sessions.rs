use crate::app::App;
use crate::spinner;
use crate::theme as th;
use crate::timeutil;
use crate::tmux::{Session, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

fn card(s: &Session, now: i64, spinner_frame: usize, selected: bool) -> ListItem<'static> {
    // Line 1: name  ........  status. Colorless: running vs idle is told apart by
    // the spinner vs dot glyph, and the selected row by its bold name + bar.
    let (status_glyph, status_label) = match s.status {
        Status::Running => (spinner::glyph(spinner_frame).to_string(), "running"),
        Status::Idle => (th::IDLE_DOT.to_string(), "idle"),
    };
    let mut name_style = Style::default();
    if selected {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    let line1 = Line::from(vec![
        Span::styled(s.name.clone(), name_style),
        Span::raw("  "),
        Span::styled(status_glyph, Style::default().add_modifier(Modifier::DIM)),
        Span::styled(format!(" {status_label}"), Style::default().add_modifier(Modifier::DIM)),
    ]);

    // Line 2: dir
    let line2 = Line::from(Span::styled(
        s.dir.clone(),
        Style::default().fg(Color::Reset),
    ));

    // Line 3: ✻ agent · ⎇ branch · +a −d   age
    let mut l3 = vec![
        Span::styled(th::AGENT_MARK, Style::default().fg(Color::Reset).add_modifier(Modifier::DIM)),
        Span::styled(format!(" {}", s.agent), Style::default().fg(Color::Reset)),
    ];
    if let Some(g) = &s.git {
        l3.push(Span::styled(
            format!("   {} {}", th::BRANCH, g.branch),
            Style::default().fg(Color::Reset).add_modifier(Modifier::DIM),
        ));
        l3.push(Span::styled(format!("   +{}", g.added), Style::default()));
        l3.push(Span::styled(format!(" −{}", g.removed), Style::default()));
    }
    let age = timeutil::humanize_age(now - s.created);
    l3.push(Span::styled(
        format!("   {age}"),
        Style::default().fg(Color::Reset),
    ));
    let line3 = Line::from(l3);

    ListItem::new(vec![line1, line2, line3])
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Section label
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SESSIONS", Style::default().fg(Color::Reset)),
            Span::styled("   ↑↓ navigate", Style::default().fg(Color::Reset).add_modifier(Modifier::DIM)),
        ])),
        rows[0],
    );

    let vis = app.visible_indices();
    let sel = if vis.is_empty() {
        0
    } else {
        app.selected.min(vis.len() - 1)
    };
    let items: Vec<ListItem> = vis
        .iter()
        .enumerate()
        .map(|(pos, &i)| card(&app.sessions[i], app.now_unix, app.spinner_frame, pos == sel))
        .collect();

    // Selection is shown by the bar (highlight_symbol) + bold, in the terminal's
    // own colors — no foreground tint, no background.
    // bind sym so highlight_symbol can borrow a &str
    let sym = format!("{} ", th::SEL_BAR);
    let list = List::new(items)
        .highlight_symbol(sym.as_str())
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    let mut state = ListState::default();
    if !vis.is_empty() {
        state.select(Some(sel));
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
    fn list_uses_no_custom_colors() {
        // Every rendered cell must keep the terminal's default foreground/background.
        let mut app = App::new(Config::default());
        app.sessions = vec![
            sess(
                "project-a",
                Status::Running,
                Some(GitInfo { branch: "main".into(), added: 12, removed: 4 }),
            ),
            sess("project-b", Status::Idle, None),
        ];
        app.selected = 0;
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let st = buf[(x, y)].style();
                assert!(
                    st.fg.is_none() || st.fg == Some(Color::Reset),
                    "unexpected fg at ({x},{y}): {:?}",
                    st.fg
                );
                assert!(
                    st.bg.is_none() || st.bg == Some(Color::Reset),
                    "unexpected bg at ({x},{y}): {:?}",
                    st.bg
                );
            }
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
