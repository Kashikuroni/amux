use crate::app::{collapse_home, App};
use crate::spinner;
use crate::theme as th;
use crate::tmux::{Session, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// Indigo accent for the "waiting on user" status — mirrors Claude Code's
/// "accept edits on" indicator. A 256-color index so it tracks the palette.
const INDIGO: Color = Color::Indexed(105);

/// Always-on per-agent color (ANSI names track the terminal palette / theme).
fn agent_accent(agent: &str) -> Color {
    match agent.split_whitespace().next().unwrap_or("") {
        "claude" => Color::Yellow,
        "codex" => Color::White,
        "gemini" => Color::Blue,
        "aider" => Color::Magenta,
        _ => Color::Cyan,
    }
}

fn card(
    s: &Session,
    spinner_frame: usize,
    selected: bool,
    prompt: Option<&[String]>,
    width: u16,
    num: usize,
) -> ListItem<'static> {
    // Line 1: badge name ........... status. The status is pushed to the far
    // right so it sits in the card's top-right corner and catches the eye.
    // running/idle/waiting are told apart by glyph (spinner / pause / dot) + color.
    let (status_glyph, status_label, status_color) = match s.status {
        Status::Running => (
            spinner::glyph(spinner_frame).to_string(),
            "running",
            Color::Blue,
        ),
        Status::Idle => (th::IDLE_MARK.to_string(), "idle", Color::Red),
        Status::Waiting => (th::WAIT_MARK.to_string(), "waiting", INDIGO),
    };
    // Always solid bold (no DIM): the status should read at full strength whether
    // or not the row is selected.
    let status_style = Style::default()
        .fg(status_color)
        .add_modifier(Modifier::BOLD);
    let mut name_style = Style::default();
    if selected {
        name_style = name_style.add_modifier(Modifier::BOLD);
    }
    // Leading list-number badge (s + N quick-jump). Dim so it reads as a quiet
    // index; only 1..9 are jumpable but every card shows its number.
    let badge = format!("{num} ");
    let badge_style = Style::default().add_modifier(Modifier::DIM);
    // Pad between the left run (badge + name) and the right-aligned status. Glyphs
    // and ASCII labels are single-width, so a char count is exact here.
    let left = badge.chars().count() + s.name.chars().count();
    let status_width = 1 + 1 + status_label.chars().count(); // glyph + space + label
    let pad = (width as usize).saturating_sub(left + status_width).max(1);
    let line1 = Line::from(vec![
        Span::styled(badge, badge_style),
        Span::styled(s.name.clone(), name_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(status_glyph, status_style),
        Span::styled(format!(" {status_label}"), status_style),
    ]);

    // Line 2: dir
    let line2 = Line::from(Span::styled(
        collapse_home(&s.dir),
        Style::default().fg(Color::Reset),
    ));

    // Line 3: ✻ agent · ⎇ branch · +a −d
    let accent = agent_accent(&s.agent);
    let mut l3 = vec![
        Span::styled(th::AGENT_MARK, Style::default().fg(accent)),
        Span::styled(format!(" {}", s.agent), Style::default().fg(accent)),
    ];
    if let Some(g) = &s.git {
        l3.push(Span::styled(
            format!("   {} {}", th::BRANCH, g.branch),
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ));
        l3.push(Span::styled(
            format!("   +{}", g.added),
            Style::default().fg(Color::Green),
        ));
        l3.push(Span::styled(
            format!(" −{}", g.removed),
            Style::default().fg(Color::Red),
        ));
    }
    let line3 = Line::from(l3);

    let mut lines = vec![line1, line2, line3];
    // Line 4 (optional): answer buttons for a detected numbered prompt.
    if let Some(opts) = prompt {
        let mut btns: Vec<Span> = vec![Span::styled(
            format!("{} ", th::PREVIEW_MARK),
            Style::default().add_modifier(Modifier::DIM),
        )];
        for (i, label) in opts.iter().enumerate().take(9) {
            let short: String = label.chars().take(16).collect();
            btns.push(Span::styled(
                format!(" {} {} ", i + 1, short),
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
            btns.push(Span::raw(" "));
        }
        lines.push(Line::from(btns));
    }

    ListItem::new(lines)
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Section label. In session-select mode the hint becomes a prompt for a digit.
    let label = if matches!(app.mode, crate::app::Mode::SelectSession) {
        vec![
            Span::styled("SESSIONS", Style::default().fg(Color::Reset)),
            Span::styled(
                "   Выберите сессию 1-9",
                Style::default()
                    .fg(INDIGO)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   esc cancel",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
        ]
    } else {
        vec![
            Span::styled("SESSIONS", Style::default().fg(Color::Reset)),
            Span::styled(
                "   ↑↓ navigate · s select",
                Style::default()
                    .fg(Color::Reset)
                    .add_modifier(Modifier::DIM),
            ),
        ]
    };
    f.render_widget(
        Paragraph::new(Line::from(label)),
        rows[0],
    );

    let vis = app.visible_indices();
    let sel = if vis.is_empty() {
        0
    } else {
        app.selected.min(vis.len() - 1)
    };
    // The list reserves the highlight_symbol ("▍ ", 2 cols) on the left of every
    // row, so card content is laid out in the remaining width.
    let content_width = rows[1].width.saturating_sub(2);
    let items: Vec<ListItem> = vis
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let s = &app.sessions[i];
            let prompt = app.prompts.get(&s.name).map(|v| v.as_slice());
            card(s, app.spinner_frame, pos == sel, prompt, content_width, pos + 1)
        })
        .collect();

    // Selection is a background-only cue: the bar (highlight_symbol) + a faint
    // full-row background (th::SEL_BG). The selected name bolds via its own
    // name_style; we deliberately do NOT add BOLD here, so selecting a row never
    // changes any text's weight (the status stays consistently bold).
    // bind sym so highlight_symbol can borrow a &str
    let sym = format!("{} ", th::SEL_BAR);
    let list = List::new(items)
        .highlight_symbol(sym.as_str())
        .highlight_style(Style::default().bg(th::SEL_BG));
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
    fn agent_accent_maps_presets() {
        assert_eq!(agent_accent("claude"), Color::Yellow);
        assert_eq!(agent_accent("codex --yolo"), Color::White);
        assert_eq!(agent_accent("gemini"), Color::Blue);
        assert_eq!(agent_accent("other"), Color::Cyan);
    }

    #[test]
    fn only_selection_uses_a_background() {
        // The selected row carries the faint SEL_BG highlight; every other cell
        // stays on the terminal's own background (no arbitrary color fills).
        let mut app = App::new(Config::default());
        app.sessions = vec![
            sess(
                "project-a",
                Status::Running,
                Some(GitInfo {
                    branch: "main".into(),
                    added: 12,
                    removed: 4,
                }),
            ),
            sess("project-b", Status::Idle, None),
        ];
        app.selected = 0;
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let mut saw_sel_bg = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let bg = buf[(x, y)].style().bg;
                if bg == Some(th::SEL_BG) {
                    saw_sel_bg = true;
                }
                assert!(
                    bg.is_none() || bg == Some(Color::Reset) || bg == Some(th::SEL_BG),
                    "unexpected bg at ({x},{y}): {bg:?}"
                );
            }
        }
        assert!(saw_sel_bg, "selected row should carry the SEL_BG highlight");
    }

    #[test]
    fn renders_running_and_idle_cards_with_git() {
        let mut app = App::new(Config::default());
        app.spinner_frame = 0;
        app.sessions = vec![
            sess(
                "project-a",
                Status::Running,
                Some(GitInfo {
                    branch: "main".into(),
                    added: 12,
                    removed: 4,
                }),
            ),
            sess("project-b", Status::Idle, None),
        ];
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("project-a"), "missing project-a:\n{s}");
        // Quick-jump number badges (Alt+N) lead each card.
        assert!(s.contains("1 project-a"), "missing badge '1':\n{s}");
        assert!(s.contains("2 project-b"), "missing badge '2':\n{s}");
        assert!(s.contains("running"), "missing 'running':\n{s}");
        assert!(s.contains("⠋"), "missing spinner frame 0:\n{s}");
        assert!(s.contains("main"), "missing branch:\n{s}");
        assert!(s.contains("+12"), "missing +12:\n{s}");
        assert!(s.contains("idle"), "missing 'idle':\n{s}");
    }

    #[test]
    fn waiting_status_renders_indigo_label_and_glyph() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("needs-input", Status::Waiting, None)];
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let s = buf_to_string(buf);
        assert!(s.contains("waiting"), "missing 'waiting':\n{s}");
        assert!(s.contains(th::WAIT_MARK), "missing wait glyph:\n{s}");
        // The label must be indigo (Indexed(105)).
        let indigo = (0..buf.area.height).any(|y| {
            (0..buf.area.width).any(|x| buf[(x, y)].style().fg == Some(INDIGO))
        });
        assert!(indigo, "no indigo cell found:\n{s}");
    }

    #[test]
    fn status_is_pushed_to_the_right_edge() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        // Row 0 is the "SESSIONS" header; row 1 is the first card line
        // (badge + name … status). The status label "idle" should sit in the
        // right half of the 40-col width, not next to the name.
        let row1: String = (0..buf.area.width)
            .map(|x| buf[(x, 1)].symbol().to_string())
            .collect();
        let col = row1.find("idle").expect("status 'idle' not on first card line");
        assert!(col > 20, "status not right-aligned (col {col}):\n{row1}");
    }
}
