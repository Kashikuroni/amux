mod footer;
mod header;

use crate::app::{App, CreateField, Mode};
use crate::theme as th;
use crate::tmux::Status;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

const SPINNER: char = '\u{283B}'; // ⠻ : shown for Running
const READY: char = '\u{25CF}'; //   ● : shown for Waiting

pub fn draw(f: &mut Frame, app: &App) {
    let root = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2), // header + rule
        ratatui::layout::Constraint::Min(1),    // body
        ratatui::layout::Constraint::Length(2), // footer rule + keys
    ])
    .split(f.area());

    header::render(f, root[0], app);
    draw_body(f, root[1], app);
    footer::render(f, root[2], app);

    match &app.mode {
        Mode::Create(_) => draw_create_modal(f, app),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete(name) => draw_confirm_modal(f, name),
        Mode::Help => draw_help_modal(f),
        Mode::List | Mode::Filter => {}
    }
}

fn draw_body(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let cols = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Percentage(40),
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Min(0),
    ])
    .split(area);
    draw_sidebar(f, cols[0], app);
    // vertical separator
    f.render_widget(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::LEFT)
            .border_style(ratatui::style::Style::default().fg(th::BORDER)),
        cols[1],
    );
    draw_preview(f, cols[2], app);
}

fn draw_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .map(|s| {
            let marker = match s.status {
                Status::Running => SPINNER,
                Status::Idle => READY,
            };
            ListItem::new(Line::from(format!("{marker} {}", s.name)))
        })
        .collect();

    let sym = format!("{} ", th::SEL_BAR);
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("sessions ({})", app.sessions.len())),
        )
        .highlight_style(
            Style::default()
                .bg(th::SEL_BG)
                .fg(th::AMBER_HI)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(sym.as_str());

    let mut state = ListState::default();
    if !app.sessions.is_empty() {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(f: &mut Frame, area: Rect, app: &App) {
    let title = match app.sessions.get(app.selected) {
        Some(s) => format!("preview: {} · {}", s.name, s.dir),
        None => "preview".to_string(),
    };
    let para = Paragraph::new(app.preview.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, Style::default().fg(th::TEXT_BOLD))),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

/// A centered rectangle `pct_x`/`pct_y` percent of the screen.
/// Pass even percentages: `(100 - pct)` is halved with integer division, so
/// odd values leave the dialog 1 cell off-center.
pub(crate) fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(v[1])[1]
}

fn field_line(label: &str, value: &str, focused: bool) -> Line<'static> {
    let prefix = if focused { "> " } else { "  " };
    Line::from(format!("{prefix}{label}: {value}"))
}

fn draw_create_modal(f: &mut Frame, app: &App) {
    let Mode::Create(form) = &app.mode else { return };
    let area = centered(60, 60, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![
        field_line("name ", &form.name, form.field == CreateField::Name),
        field_line("dir  ", &form.dir, form.field == CreateField::Dir),
        field_line("agent", &form.agent, form.field == CreateField::Agent),
        Line::from(""),
    ];

    if form.field == CreateField::Dir {
        // Window the list so the highlighted row stays visible when entries overflow
        // the modal. inner height = area minus borders; reserve 4 header rows
        // (3 fields + blank) and 2 footer rows (blank + hint).
        let inner_h = area.height.saturating_sub(2) as usize;
        let cap = inner_h.saturating_sub(6).max(1);
        let total = form.dir_entries.len();
        let start = if form.dir_selected >= cap {
            form.dir_selected + 1 - cap
        } else {
            0
        };
        let end = (start + cap).min(total);
        for i in start..end {
            let selected = i == form.dir_selected;
            let text = format!(
                "{}{}/",
                if selected { "> " } else { "  " },
                form.dir_entries[i]
            );
            if selected {
                lines.push(Line::from(Span::styled(
                    text,
                    Style::default().add_modifier(Modifier::REVERSED),
                )));
            } else {
                lines.push(Line::from(text));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "↑↓ select · Tab/→ enter · Enter confirm · Esc cancel",
        ));
    } else {
        lines.push(Line::from(
            "Tab/Enter: next · Enter on agent: create · Esc: cancel",
        ));
    }

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("new session"));
    f.render_widget(para, area);
}

fn draw_rename_modal(f: &mut Frame, app: &App) {
    let Mode::Rename(form) = &app.mode else { return };
    let area = centered(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(vec![
        Line::from(format!("new name: {}", form.buffer)),
        Line::from("Enter: rename  ·  Esc: cancel"),
    ])
    .block(Block::default().borders(Borders::ALL).title("rename"));
    f.render_widget(para, area);
}

fn draw_confirm_modal(f: &mut Frame, name: &str) {
    let area = centered(50, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(format!("Kill session \"{name}\"? (y/n)"))
        .block(Block::default().borders(Borders::ALL).title("confirm"));
    f.render_widget(para, area);
}

fn draw_help_modal(f: &mut Frame) {
    let area = centered(50, 60, f.area());
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from("k / j        navigate up / down"),
        Line::from("Enter / o    attach to session"),
        Line::from("n            new session"),
        Line::from("d            kill session"),
        Line::from("r            rename session"),
        Line::from("q            quit (sessions keep running)"),
        Line::from("?            toggle this help"),
        Line::from(""),
        Line::from("any key to close"),
    ];
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("help"));
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
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

    #[test]
    fn renders_sidebar_and_footer() {
        let mut app = App::new(Config::default());
        app.sessions = vec![Session {
            name: "project-a".into(),
            dir: "/work/a".into(),
            created: 1,
            agent: "claude".into(),
            status: Status::Running,
            attached: false,
            git: None,
        }];

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("project-a"));
        assert!(text.contains(" cm"), "header logo must be present");
        assert!(text.contains("new"), "footer must show new key hint");
    }

    #[test]
    fn renders_error_footer_and_create_modal() {
        let mut app = App::new(Config::default());
        app.error = Some("boom".into());
        app.mode = crate::app::Mode::Create(crate::app::CreateForm::new("claude", &[]));

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("new session"), "create modal must be visible");
        assert!(text.contains("create"), "footer must show create key hint");
    }

    #[test]
    fn create_modal_renders_dir_entries_when_dir_focused() {
        use crate::app::{CreateField, CreateForm, Mode};

        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Dir;
        form.dir_entries = vec!["alpha".into(), "beta".into()];
        form.dir_selected = 0;
        app.mode = Mode::Create(form);

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("alpha/"));
        assert!(text.contains("beta/"));
        assert!(text.contains("new session"));
    }

    #[test]
    fn dir_list_keeps_selection_visible_when_scrolled() {
        use crate::app::{CreateField, CreateForm, Mode};

        let mut app = App::new(Config::default());
        let mut form = CreateForm::new("claude", &[]);
        form.field = CreateField::Dir;
        form.dir_entries = (0..40).map(|i| format!("entry{i:02}")).collect();
        form.dir_selected = 39; // last entry
        app.mode = Mode::Create(form);

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("entry39/"), "selected row must be visible");
        assert!(!text.contains("entry00/"), "top of list must scroll off");
    }
}
