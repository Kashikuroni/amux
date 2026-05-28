mod footer;
mod header;
mod modal_help;
mod modal_kill;
mod modal_new;
mod preview;
mod sessions;

use crate::app::{App, Mode};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

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
        Mode::Create(form) => modal_new::render(f, form),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete(name) => modal_kill::render(f, name),
        Mode::Help => modal_help::render(f),
        Mode::List | Mode::Filter => {}
    }
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);
    sessions::render(f, cols[0], app);
    // vertical separator
    f.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(th::BORDER)),
        cols[1],
    );
    preview::render(f, cols[2], app);
}

/// Centered Rect helper shared with modal submodules (Task 10).
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

fn draw_rename_modal(f: &mut Frame, app: &App) {
    let Mode::Rename(form) = &app.mode else { return };
    let area = centered(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(vec![
        Line::from(format!("new name: {}", form.buffer)),
        Line::from("Enter: rename  ·  Esc: cancel"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(th::AMBER))
            .title("rename"),
    );
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::tmux::{Session, Status};
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
    fn renders_create_modal() {
        let mut app = App::new(Config::default());
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
