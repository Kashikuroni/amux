mod empty;
mod error;
mod footer;
mod header;
mod modal_help;
mod modal_kill;
mod modal_new;
mod preview;
mod sessions;

use crate::app::{App, Mode, ReplyForm};
use crate::theme as th;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &App) {
    if app.tmux_missing {
        error::render(f);
        return;
    }
    // Small breathing room from the screen edges.
    let screen = f.area();
    let area = Rect {
        x: screen.x + 2,
        y: screen.y + 1,
        width: screen.width.saturating_sub(4),
        height: screen.height.saturating_sub(2),
    };
    let root = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2), // header + rule
        ratatui::layout::Constraint::Min(1),    // body
        ratatui::layout::Constraint::Length(2), // footer rule + keys
    ])
    .split(area);

    header::render(f, root[0], app);
    draw_body(f, root[1], app);
    footer::render(f, root[2], app);

    match &app.mode {
        Mode::Create(form) => modal_new::render(f, form, app.error.as_deref()),
        Mode::Rename(_) => draw_rename_modal(f, app),
        Mode::ConfirmDelete(form) => {
            let s = app.sessions.iter().find(|s| s.name == form.name);
            modal_kill::render(f, form, s);
        }
        Mode::Help => modal_help::render(f),
        Mode::Reply(form) => draw_reply_modal(f, form),
        Mode::RenameProject(form) => draw_project_rename_modal(f, form),
        // No modal: these render over the plain list (the SESSIONS label shows
        // the select-mode prompt).
        Mode::List | Mode::Filter | Mode::SelectSession => {}
    }
}

/// Multi-line reply composer: text wraps to the box width and the hardware
/// cursor tracks the edit position, so a long message is always fully visible.
fn draw_reply_modal(f: &mut Frame, form: &ReplyForm) {
    let area = centered(35, 33, f.area());
    f.render_widget(Clear, area);
    let block = th::panel()
        .border_style(th::chrome(th::BORDER_HI))
        .title(format!(" reply to {} ", form.name))
        .style(Style::default().bg(th::BG_RAISED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Reserve the bottom inner row for the key hint.
    let text_h = inner.height.saturating_sub(1);
    let text_area = Rect {
        height: text_h,
        ..inner
    };
    let hint_area = Rect {
        y: inner.y + text_h,
        height: 1,
        ..inner
    };

    let width = text_area.width as usize;
    let chars: Vec<char> = form.buffer.chars().collect();

    if chars.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "type your message…",
                Style::default().fg(th::MUTED).add_modifier(Modifier::DIM),
            ))),
            text_area,
        );
    } else {
        let rows = wrap_rows(&chars, width);
        let (crow, _) = cursor_rowcol(&rows, form.cursor);
        // Scroll vertically so the cursor row stays on screen.
        let visible = text_h as usize;
        let scroll = if crow >= visible {
            crow - visible + 1
        } else {
            0
        };
        let lines: Vec<Line> = rows
            .iter()
            .skip(scroll)
            .take(visible)
            .map(|(_, t)| Line::from(t.as_str()))
            .collect();
        f.render_widget(Paragraph::new(lines), text_area);
    }

    // Place the hardware cursor (works for the empty buffer too: row 0, col 0).
    let rows = wrap_rows(&chars, width);
    let (crow, ccol) = cursor_rowcol(&rows, form.cursor);
    let visible = text_h as usize;
    let scroll = if crow >= visible {
        crow - visible + 1
    } else {
        0
    };
    if crow >= scroll {
        let cx = text_area.x + ccol.min(width.saturating_sub(1)) as u16;
        let cy = text_area.y + (crow - scroll) as u16;
        f.set_cursor_position((cx, cy));
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            hint_key("↵"),
            hint_label(" send   "),
            hint_key("⇧↵"),
            hint_label(" newline   "),
            hint_key("esc"),
            hint_label(" cancel"),
        ])),
        hint_area,
    );
}

fn hint_key(k: &str) -> Span<'_> {
    Span::styled(
        k,
        Style::default()
            .fg(th::TEXT_BOLD)
            .add_modifier(Modifier::BOLD),
    )
}

fn hint_label(s: &str) -> Span<'_> {
    Span::styled(s, Style::default().fg(th::MUTED))
}

/// Word-wraps `chars` to `width` columns, honoring explicit newlines. Returns
/// `(start_char_index, row_text)` for each display row. Empty logical lines (and
/// a trailing newline) yield empty rows so the cursor can rest on them.
fn wrap_rows(chars: &[char], width: usize) -> Vec<(usize, String)> {
    let width = width.max(1);
    let mut rows: Vec<(usize, String)> = Vec::new();
    let n = chars.len();
    let mut line_start = 0;
    loop {
        let mut le = line_start;
        while le < n && chars[le] != '\n' {
            le += 1;
        }
        let seg = &chars[line_start..le];
        if seg.is_empty() {
            rows.push((line_start, String::new()));
        } else {
            let mut off = 0;
            while off < seg.len() {
                if seg.len() - off <= width {
                    rows.push((line_start + off, seg[off..].iter().collect()));
                    break;
                }
                // Break at the last space within the window, else hard-cut.
                let hard = off + width;
                let mut cut = hard;
                let mut k = hard;
                while k > off {
                    if seg[k - 1] == ' ' {
                        cut = k;
                        break;
                    }
                    k -= 1;
                }
                rows.push((line_start + off, seg[off..cut].iter().collect()));
                off = cut;
            }
        }
        if le >= n {
            break;
        }
        line_start = le + 1; // skip the '\n'
    }
    if rows.is_empty() {
        rows.push((0, String::new()));
    }
    rows
}

/// Maps a character cursor index onto a `(row, col)` in the wrapped layout.
fn cursor_rowcol(rows: &[(usize, String)], cursor: usize) -> (usize, usize) {
    for (ri, (start, text)) in rows.iter().enumerate() {
        let len = text.chars().count();
        let end = start + len;
        if cursor < end || ri == rows.len() - 1 {
            return (ri, cursor.saturating_sub(*start));
        }
        if cursor == end {
            // At a row boundary: if the next row begins at the same char index it
            // was a soft wrap (no separator consumed) → cursor belongs to the next
            // row's start; otherwise a newline was consumed → stay at this row end.
            if rows[ri + 1].0 == end {
                continue;
            }
            return (ri, len);
        }
    }
    let last = rows.len() - 1;
    (last, rows[last].1.chars().count())
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    if app.sessions.is_empty() {
        empty::render(f, area);
        return;
    }
    let cols = Layout::horizontal([
        Constraint::Percentage(app.split_pct),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area);
    sessions::render(f, cols[0], app);
    // vertical separator
    f.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(th::chrome(th::BORDER)),
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
    let Mode::Rename(form) = &app.mode else {
        return;
    };
    let area = centered(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(vec![
        Line::from(format!("new name: {}", form.buffer)),
        Line::from("Enter: rename  ·  Esc: cancel"),
    ])
    .block(
        th::panel()
            .border_style(th::chrome(th::AMBER))
            .title("rename"),
    );
    f.render_widget(para, area);
}

fn draw_project_rename_modal(f: &mut Frame, form: &crate::app::ProjectRenameForm) {
    let area = centered(60, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(vec![
        Line::from(format!("project name: {}", form.buffer)),
        Line::from("Enter: rename  ·  Esc: cancel  (display only)"),
    ])
    .block(
        th::panel()
            .border_style(th::chrome(th::AMBER))
            .title("rename project"),
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
    fn wrap_rows_breaks_on_words_and_newlines() {
        let chars: Vec<char> = "hello world foo".chars().collect();
        let rows = wrap_rows(&chars, 8);
        let texts: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["hello ", "world ", "foo"]);

        // Explicit newline yields its own (possibly empty) row.
        let chars: Vec<char> = "a\n\nb".chars().collect();
        let rows = wrap_rows(&chars, 8);
        let texts: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["a", "", "b"]);
    }

    #[test]
    fn cursor_rowcol_handles_soft_wrap_boundary() {
        // "abcdef" wrapped at width 3 → rows "abc","def" (soft wrap, no separator).
        let chars: Vec<char> = "abcdef".chars().collect();
        let rows = wrap_rows(&chars, 3);
        // cursor at index 3 (just after "abc") belongs to the start of row 1.
        assert_eq!(cursor_rowcol(&rows, 3), (1, 0));
        // cursor at end sits at the end of the last row.
        assert_eq!(cursor_rowcol(&rows, 6), (1, 3));
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
            worktree_repo: None,
        }];

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("project-a"));
        assert!(text.contains(" am"), "header logo must be present");
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
        assert!(text.contains("New session"), "create modal must be visible");
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

        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buf_to_string(terminal.backend().buffer());
        assert!(text.contains("alpha/"));
        assert!(text.contains("beta/"));
        assert!(text.contains("New session"));
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
