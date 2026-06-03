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

/// Indent of session cards under their project header (the header is itself
/// indented from "SESSIONS" by the list's highlight-symbol gutter, so this gives
/// sessions the same step in again).
const INDENT: &str = "  ";

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
    let left = INDENT.len() + badge.chars().count() + s.name.chars().count();
    let status_width = 1 + 1 + status_label.chars().count(); // glyph + space + label
    let pad = (width as usize).saturating_sub(left + status_width).max(1);
    // Leading gutter (same width as INDENT): the selection bar on the selected
    // card, drawn inside the row on the SEL_BG background, else plain indent.
    let lead = || -> Span<'static> {
        if selected {
            Span::styled(
                format!("{} ", th::SEL_BAR),
                Style::default().add_modifier(Modifier::DIM),
            )
        } else {
            Span::raw(INDENT)
        }
    };
    let line1 = Line::from(vec![
        lead(),
        Span::styled(badge, badge_style),
        Span::styled(s.name.clone(), name_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(status_glyph, status_style),
        Span::styled(format!(" {status_label}"), status_style),
    ]);

    // Line 2: ✻ agent · ⎇/⧉ branch · +a −d. The branch glyph is ⧉ for worktree
    // sessions, ⎇ for the repo root. (The path lives on the project header now —
    // every session in a project shares it.)
    let accent = agent_accent(&s.agent);
    let mut l2 = vec![
        lead(),
        Span::styled(th::AGENT_MARK, Style::default().fg(accent)),
        Span::styled(format!(" {}", s.agent), Style::default().fg(accent)),
    ];
    if let Some(g) = &s.git {
        // Worktree sessions swap the branch glyph (⧉) for the repo-root one (⎇),
        // so "running in a linked worktree" reads from the marker itself with no
        // extra width or trailing tag.
        let glyph = if crate::app::is_worktree(s) {
            th::WORKTREE
        } else {
            th::BRANCH
        };
        l2.push(Span::styled(
            format!("   {} {}", glyph, g.branch),
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ));
        l2.push(Span::styled(
            format!("   +{}", g.added),
            Style::default().fg(Color::Green),
        ));
        l2.push(Span::styled(
            format!(" −{}", g.removed),
            Style::default().fg(Color::Red),
        ));
    }
    let line2 = Line::from(l2);

    // Line 3: reserved. Holds the answer buttons when a numbered prompt is
    // detected, otherwise stays blank — so buttons appearing never shift the
    // cards below, and there's always a gap between sessions.
    let line3 = match prompt {
        Some(opts) => {
            let mut btns: Vec<Span> = vec![
                lead(),
                Span::styled(
                    format!("{} ", th::PREVIEW_MARK),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ];
            for (i, label) in opts.iter().enumerate().take(9) {
                let short: String = label.chars().take(16).collect();
                btns.push(Span::styled(
                    format!(" {} {} ", i + 1, short),
                    Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                ));
                btns.push(Span::raw(" "));
            }
            Line::from(btns)
        }
        // Blank reserved line still carries the bar so the highlight reads as one
        // continuous block on the selected card.
        None => Line::from(lead()),
    };

    ListItem::new(vec![line1, line2, line3])
}

/// Project group header: the display name (bold) with the shared path beneath it
/// in DIM. Inter-group spacing is handled by the caller's spacer items.
fn project_header(name: &str, root: &str) -> ListItem<'static> {
    ListItem::new(vec![
        Line::from(Span::styled(
            name.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            collapse_home(root),
            Style::default().add_modifier(Modifier::DIM),
        )),
    ])
}

/// A non-selectable spacer of `n` blank lines (none if `n == 0`).
fn spacer(n: usize) -> Option<ListItem<'static>> {
    (n > 0).then(|| ListItem::new(vec![Line::from(""); n]))
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Section label. In session-select mode the hint becomes a prompt for a digit.
    let label = if matches!(app.mode, crate::app::Mode::SelectSession) {
        vec![
            Span::styled("SESSIONS", Style::default().fg(Color::Reset)),
            Span::styled(
                "   Выберите сессию 1-9",
                Style::default().fg(INDIGO).add_modifier(Modifier::BOLD),
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
    f.render_widget(Paragraph::new(Line::from(label)), rows[0]);

    let vis = app.visible_indices();
    let sel = if vis.is_empty() {
        0
    } else {
        app.selected.min(vis.len() - 1)
    };
    // Inset the list on the right so the SEL_BG background doesn't butt against
    // the divider. No highlight-symbol gutter: the list sits flush with the
    // "SESSIONS" label and the selection bar is drawn inside each card.
    const RIGHT_PAD: u16 = 2;
    // Inner pad so the status ends short of the background's right edge, mirroring
    // the left gutter (bar + space) — the highlight gets symmetric inner padding.
    const INNER_RIGHT_PAD: u16 = 2;
    let list_area = Rect {
        width: rows[1].width.saturating_sub(RIGHT_PAD),
        ..rows[1]
    };
    let content_width = list_area.width.saturating_sub(INNER_RIGHT_PAD);
    // Build items grouped by project: a header precedes each project's sessions.
    // Headers aren't selectable, so the ListState index is the selected session's
    // *item* index (which differs from its session index by the headers above it).
    // Vertical rhythm (the card's trailing reserved line already adds 1 line):
    //   between sessions  = reserved(1) + 1 spacer  = 2 lines
    //   between projects  = reserved(1) + 1 spacer  = 2 lines (same as sessions)
    //   path → 1st session = 1 spacer               = 1 line  (half)
    //   above the first project (gap from "SESSIONS")  = SPACER_TOP
    const SPACER_BETWEEN_SESSIONS: usize = 1;
    const SPACER_BETWEEN_PROJECTS: usize = 1;
    const SPACER_PATH_TO_FIRST: usize = 1;
    const SPACER_TOP: usize = 1;
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item: Option<usize> = None;
    let mut prev_root: Option<String> = None;
    for (pos, &i) in vis.iter().enumerate() {
        let s = &app.sessions[i];
        let root = crate::app::session_root(s).to_string();
        if prev_root.as_deref() != Some(root.as_str()) {
            // Gap between projects, or above the very first project.
            items.extend(spacer(if prev_root.is_some() {
                SPACER_BETWEEN_PROJECTS
            } else {
                SPACER_TOP
            }));
            items.push(project_header(&app.project_display_name(&root), &root));
            items.extend(spacer(SPACER_PATH_TO_FIRST));
            prev_root = Some(root);
        } else {
            items.extend(spacer(SPACER_BETWEEN_SESSIONS));
        }
        if pos == sel {
            selected_item = Some(items.len());
        }
        let prompt = app.prompts.get(&s.name).map(|v| v.as_slice());
        items.push(card(
            s,
            app.spinner_frame,
            pos == sel,
            prompt,
            content_width,
            pos + 1,
        ));
    }

    // Selection is a background-only cue: a faint full-row background (th::SEL_BG),
    // no left gutter so the list aligns with the "SESSIONS" label. The selected
    // name bolds via its own name_style; we deliberately do NOT add BOLD here, so
    // selecting a row never changes any text's weight (the status stays bold).
    let list = List::new(items).highlight_style(Style::default().bg(th::SEL_BG));
    let mut state = ListState::default();
    state.select(selected_item);
    f.render_stateful_widget(list, list_area, &mut state);
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
            worktree_repo: None,
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
    fn worktree_row_uses_worktree_branch_glyph() {
        let mut app = App::new(Config::default());
        let mut s = sess(
            "feature-x",
            Status::Idle,
            Some(GitInfo {
                branch: "feature-x".into(),
                added: 3,
                removed: 1,
            }),
        );
        // Make is_worktree(s) true: dir differs from the worktree's repo root.
        s.dir = "~/work/x/.worktrees/feature-x".into();
        s.worktree_repo = Some("~/work/x".into());
        app.sessions = vec![s];
        let mut t = Terminal::new(TestBackend::new(80, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let out = buf_to_string(t.backend().buffer());
        // Worktree sessions carry the ⧉ marker (not the repo-root ⎇) plus the
        // branch name and diff; the old "worktree" word tag is gone.
        assert!(out.contains(th::WORKTREE), "missing ⧉ worktree glyph:\n{out}");
        assert!(out.contains("feature-x"), "missing branch name:\n{out}");
        assert!(out.contains("+3"), "missing diff stat:\n{out}");
        assert!(!out.contains(th::BRANCH), "worktree must not use ⎇:\n{out}");
        assert!(!out.contains("worktree"), "stale word tag present:\n{out}");
    }

    #[test]
    fn repo_root_row_uses_plain_branch_glyph() {
        let mut app = App::new(Config::default());
        // Default sess() has worktree_repo None and dir == its own root → not a
        // worktree, so it keeps the ⎇ marker.
        app.sessions = vec![sess(
            "main-sess",
            Status::Idle,
            Some(GitInfo {
                branch: "main".into(),
                added: 0,
                removed: 0,
            }),
        )];
        let mut t = Terminal::new(TestBackend::new(80, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let out = buf_to_string(t.backend().buffer());
        assert!(out.contains(th::BRANCH), "repo root must use ⎇:\n{out}");
        assert!(!out.contains(th::WORKTREE), "repo root must not use ⧉:\n{out}");
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
        let indigo = (0..buf.area.height)
            .any(|y| (0..buf.area.width).any(|x| buf[(x, y)].style().fg == Some(INDIGO)));
        assert!(indigo, "no indigo cell found:\n{s}");
    }

    #[test]
    fn status_is_pushed_to_the_right_edge() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        // Find whichever row carries the status label; it should sit in the right
        // half of the 40-col width, not next to the name.
        let row = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|r| r.contains("idle"))
            .expect("status 'idle' not found");
        let col = row.find("idle").unwrap();
        assert!(col > 20, "status not right-aligned (col {col}):\n{row}");
    }
}
