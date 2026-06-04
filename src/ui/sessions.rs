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
/// Branch-glyph tints (Pair C, "Cool & Clay"): cool slate-teal for the repo
/// root, warm terracotta for a linked worktree — so the marker's *color* alone
/// signals where work is happening, not just its shape (⎇ vs ⧉).
const BRANCH_FG: Color = Color::Indexed(66); // #5f8787 slate-teal — repo root
const WORKTREE_FG: Color = Color::Indexed(173); // #d7875f terracotta — worktree

/// Indent of session cards under their project header (the header is itself
/// indented from "SESSIONS" by the list's highlight-symbol gutter, so this gives
/// sessions the same step in again).
const INDENT: &str = "  ";

/// Always-on per-agent color (ANSI names track the terminal palette / theme).
fn agent_accent(agent: &str) -> Color {
    match agent.split_whitespace().next().unwrap_or("") {
        "claude" => Color::Yellow,
        "codex" => Color::White,
        "opencode" => Color::Green,
        "gemini" => Color::Blue,
        "aider" => Color::Magenta,
        _ => Color::Cyan,
    }
}

#[allow(clippy::too_many_arguments)]
fn card(
    s: &Session,
    spinner_frame: usize,
    selected: bool,
    prompt: Option<&[String]>,
    width: u16,
    num: usize,
    done: u32,
    total: u32,
    restarting: bool,
) -> ListItem<'static> {
    // Line 1: badge name ........... status. The status is pushed to the far
    // right so it sits in the card's top-right corner and catches the eye.
    // running/idle/waiting are told apart by glyph (spinner / pause / dot) + color.
    // A restarting session overrides its tmux-derived status: the card shows a
    // yellow spinner + "restarting" until the resume command is sent (or the
    // 30 s timeout clears it).
    let (status_glyph, status_label, status_color) = if restarting {
        (
            spinner::glyph(spinner_frame).to_string(),
            "restarting",
            Color::Yellow,
        )
    } else {
        match s.status {
            Status::Running => (
                spinner::glyph(spinner_frame).to_string(),
                "running",
                Color::Blue,
            ),
            Status::Idle => (th::IDLE_MARK.to_string(), "idle", Color::Red),
            Status::Waiting => (th::WAIT_MARK.to_string(), "waiting", INDIGO),
        }
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
        // Worktree sessions swap the branch glyph (⧉) for the repo-root one (⎇)
        // and tint it (terracotta vs slate-teal), so "running in a linked
        // worktree" reads from the marker's shape *and* color — no extra width or
        // trailing tag. Only the glyph is colored; the branch name stays neutral.
        let (glyph, glyph_fg) = if crate::app::is_worktree(s) {
            (th::WORKTREE, WORKTREE_FG)
        } else {
            (th::BRANCH, BRANCH_FG)
        };
        l2.push(Span::styled(
            format!("   {glyph} "),
            Style::default().fg(glyph_fg).add_modifier(Modifier::BOLD),
        ));
        l2.push(Span::styled(
            g.branch.clone(),
            Style::default()
                .fg(Color::Reset)
                .add_modifier(Modifier::DIM),
        ));
        // Task counter sits in the left run (so the diff still right-aligns).
        let counter = if total > 0 {
            format!("   {done}/{total}")
        } else {
            String::new()
        };
        if !counter.is_empty() {
            l2.push(Span::styled(
                counter.clone(),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        // Right-align the diff stat to the card's right edge so it lands directly
        // under the status on line 1 (same width-based padding as line 1).
        let added = format!("+{}", g.added);
        let removed = format!("−{}", g.removed);
        let left2 = INDENT.len()
            + 1                            // ✻ agent mark
            + 1 + s.agent.chars().count()  // " {agent}"
            + 5                            // "   {glyph} " (3 spaces + glyph + space)
            + g.branch.chars().count()
            + counter.chars().count();
        let diff_width = added.chars().count() + 1 + removed.chars().count();
        let pad2 = (width as usize).saturating_sub(left2 + diff_width).max(1);
        l2.push(Span::raw(" ".repeat(pad2)));
        l2.push(Span::styled(added, Style::default().fg(Color::Green)));
        l2.push(Span::styled(
            format!(" {removed}"),
            Style::default().fg(Color::Red),
        ));
    }
    // No git info: append the counter to the agent line if the note has tasks.
    if s.git.is_none() && total > 0 {
        l2.push(Span::styled(
            format!("   {done}/{total}"),
            Style::default().add_modifier(Modifier::DIM),
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

    // Leading frame row above the name: with the trailing reserved line it makes
    // the selection highlight a symmetric one-row border (top + bottom, plus the
    // left bar and right inner-pad). Carries the bar so the block stays continuous.
    // Card height is constant (4 rows) whether or not buttons show, so a prompt
    // appearing never shifts the cards below. When buttons DO occupy the reserved
    // line, the bottom frame is painted onto the existing blank row beneath the
    // card after the list renders — see `paint_prompt_frame`.
    let line0 = Line::from(lead());
    ListItem::new(vec![line0, line1, line2, line3])
}

/// Extends the selection highlight one row below the selected card when it shows
/// answer buttons, so the border still closes beneath them — without growing the
/// card (which would shift every card below while a prompt is up). Reuses the
/// already-blank row under the card; a no-op if there's no room.
fn paint_prompt_frame(buf: &mut ratatui::buffer::Buffer, area: Rect) {
    // The selected card is the only highlighted item; its bottommost SEL_BG row is
    // the buttons line. Mirror that row's highlighted span onto the row below.
    let cols = area.x..area.x + area.width;
    let Some(btn_y) = (area.y..area.y + area.height).rev().find(|&y| {
        cols.clone()
            .any(|x| buf[(x, y)].style().bg == Some(th::SEL_BG))
    }) else {
        return;
    };
    let below = btn_y + 1;
    if below >= area.y + area.height {
        return; // card sits at the very bottom — no row to extend into
    }
    for x in cols.clone() {
        if buf[(x, btn_y)].style().bg == Some(th::SEL_BG) {
            buf[(x, below)].set_bg(th::SEL_BG);
        }
    }
    // Carry the left selection bar so the border reads as one continuous block.
    buf[(area.x, below)]
        .set_symbol(th::SEL_BAR)
        .set_style(Style::default().bg(th::SEL_BG).add_modifier(Modifier::DIM));
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
                "   select a session 1-9",
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
        let dim = Style::default()
            .fg(Color::Reset)
            .add_modifier(Modifier::DIM);
        vec![
            Span::styled("SESSIONS", Style::default().fg(Color::Reset)),
            Span::styled("  j k or ↑↓ navigate · s select", dim),
            // Legend for the branch-glyph markers, tinted to match the cards.
            Span::styled("    ", dim),
            Span::styled(
                th::BRANCH,
                Style::default().fg(BRANCH_FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" repo · ", dim),
            Span::styled(
                th::WORKTREE,
                Style::default()
                    .fg(WORKTREE_FG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" worktree", dim),
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
    // Vertical rhythm. Each card now carries its OWN frame rows — a leading blank
    // above the name and a trailing reserved blank below — so the selection
    // highlight closes as a one-row border on all four sides. Those owned rows
    // replace the inter-card spacers, which drop to 0 to keep the same gap:
    //   between sessions  = trailing(1) + leading(1)        = 2 lines
    //   between projects  = trailing(1) + SPACER_PROJECTS(1) = 2 lines
    //   path → 1st session = leading(1)                     = 1 line
    //   above the first project (gap from "SESSIONS")       = SPACER_TOP
    const SPACER_BETWEEN_SESSIONS: usize = 0;
    const SPACER_BETWEEN_PROJECTS: usize = 1;
    const SPACER_PATH_TO_FIRST: usize = 0;
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
        // Task progress from the session's note (0/0 when it has no tasks).
        let (done, total) = app
            .notes
            .get(&s.name)
            .map(|t| crate::note::counts(t))
            .unwrap_or((0, 0));
        let restarting = app.restarting.contains_key(&s.name);
        items.push(card(
            s,
            app.spinner_frame,
            pos == sel,
            prompt,
            content_width,
            pos + 1,
            done,
            total,
            restarting,
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

    // If the selected card shows answer buttons, close the highlight with a frame
    // row beneath them (painted onto the existing blank row, so no card shifts).
    let selected_has_prompt = vis
        .get(sel)
        .map(|&i| app.prompts.contains_key(&app.sessions[i].name))
        .unwrap_or(false);
    if selected_has_prompt {
        paint_prompt_frame(f.buffer_mut(), list_area);
    }
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

    use crate::ui::testutil::buf_to_string;

    /// Buffer text from row `start_y` down — used to exclude the SESSIONS header
    /// (row 0), whose legend now contains both branch glyphs, when asserting on
    /// the card rows alone.
    fn buf_to_string_from(buf: &Buffer, start_y: u16) -> String {
        let mut s = String::new();
        for y in start_y..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    /// Foreground color of the first cell at or below `start_y` whose symbol
    /// equals `glyph`, or None. `start_y` skips the header legend.
    fn glyph_fg(buf: &Buffer, glyph: &str, start_y: u16) -> Option<Color> {
        for y in start_y..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() == glyph {
                    return buf[(x, y)].style().fg;
                }
            }
        }
        None
    }

    fn sess(name: &str, status: Status, git: Option<GitInfo>) -> Session {
        Session {
            name: name.into(),
            dir: "~/work/x".into(),
            cwd: "~/work/x".into(),
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
        assert_eq!(agent_accent("opencode"), Color::Green);
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

    /// Rows carrying at least one SEL_BG cell, as a set of y indices.
    fn highlighted_rows(buf: &Buffer) -> Vec<u16> {
        (0..buf.area.height)
            .filter(|&y| (0..buf.area.width).any(|x| buf[(x, y)].style().bg == Some(th::SEL_BG)))
            .collect()
    }

    #[test]
    fn selection_frames_card_with_blank_rows_top_and_bottom() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess(
            "proj",
            Status::Idle,
            Some(GitInfo {
                branch: "main".into(),
                added: 1,
                removed: 0,
            }),
        )];
        app.selected = 0;
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let rows = buf_to_string(buf);
        let name_y = rows.lines().position(|r| r.contains("proj")).unwrap() as u16;
        let hl = highlighted_rows(buf);
        // The card is a contiguous highlighted block: a blank frame row above the
        // name (top), the two data rows, and the reserved blank below (bottom).
        assert!(name_y >= 1, "name needs a row above it for the top frame");
        assert!(hl.contains(&(name_y - 1)), "top frame row not highlighted");
        assert!(hl.contains(&name_y), "name row not highlighted");
        assert!(
            hl.contains(&(name_y + 2)),
            "bottom frame row not highlighted"
        );
        // The top frame row is blank (only the selection bar / spaces, no text).
        let top = rows.lines().nth((name_y - 1) as usize).unwrap();
        assert!(
            top.chars()
                .all(|c| c == ' ' || c.to_string() == th::SEL_BAR),
            "top frame row must be blank: {top:?}"
        );
    }

    #[test]
    fn prompt_buttons_do_not_shift_cards_below() {
        // The same two-session list, rendered with and without a prompt on the
        // first card: the second card's name must land on the same row either way.
        let build = |with_prompt: bool| {
            let mut app = App::new(Config::default());
            app.sessions = vec![
                sess("first", Status::Waiting, None),
                sess("second", Status::Idle, None),
            ];
            app.selected = 0;
            if with_prompt {
                app.prompts
                    .insert("first".into(), vec!["yes".into(), "no".into()]);
            }
            let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
            t.draw(|f| render(f, f.area(), &app)).unwrap();
            buf_to_string(t.backend().buffer())
                .lines()
                .position(|r| r.contains("second"))
                .unwrap()
        };
        assert_eq!(
            build(false),
            build(true),
            "a prompt on the first card must not move the second card"
        );
    }

    #[test]
    fn prompt_buttons_keep_a_blank_frame_row_below() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Waiting, None)];
        app.selected = 0;
        app.prompts
            .insert("proj".into(), vec!["yes".into(), "no".into()]);
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let rows = buf_to_string(buf);
        let lines: Vec<&str> = rows.lines().collect();
        // The buttons row (carries an option label) must be followed by a blank,
        // highlighted frame row so the border closes beneath the buttons.
        let btn_y = lines.iter().position(|r| r.contains("yes")).unwrap();
        let below = lines[btn_y + 1];
        assert!(
            below
                .chars()
                .all(|c| c == ' ' || c.to_string() == th::SEL_BAR),
            "row under buttons must be blank: {below:?}"
        );
        assert!(
            highlighted_rows(buf).contains(&((btn_y + 1) as u16)),
            "row under buttons must be highlighted"
        );
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

    /// Rightmost column index holding a non-blank cell in row `y`, or None.
    fn last_col(buf: &Buffer, y: u16) -> Option<u16> {
        (0..buf.area.width)
            .rev()
            .find(|&x| buf[(x, y)].symbol() != " ")
    }

    #[test]
    fn diff_right_aligns_under_status() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess(
            "proj",
            Status::Idle,
            Some(GitInfo {
                branch: "main".into(),
                added: 12,
                removed: 4,
            }),
        )];
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let lines = buf_to_string(buf);
        let rows: Vec<&str> = lines.lines().collect();
        // Find the status row (line 1 of the card) and the diff row (line 2).
        let status_y = rows.iter().position(|r| r.contains("idle")).unwrap() as u16;
        let diff_y = rows.iter().position(|r| r.contains("−4")).unwrap() as u16;
        assert_eq!(
            last_col(buf, diff_y),
            last_col(buf, status_y),
            "diff's right edge must align under the status's right edge"
        );
    }

    #[test]
    fn card_shows_task_counter_when_note_has_tasks() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        app.notes.insert("proj".into(), "- [ ] a\n- [x] b".into());
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("1/2"), "counter missing:\n{s}");
    }

    #[test]
    fn card_has_no_counter_without_tasks() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        // A note with no tasks must not render a "0/0" counter.
        app.notes
            .insert("proj".into(), "just a thought, no checkboxes".into());
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("0/0"), "should not show a zero counter:\n{s}");
    }

    #[test]
    fn header_shows_branch_glyph_legend() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Idle, None)];
        let mut t = Terminal::new(TestBackend::new(80, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        // Only the header row (0) carries the legend explaining the markers.
        let header = buf_to_string(t.backend().buffer())
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(header.contains(th::BRANCH), "legend missing ⎇:\n{header}");
        assert!(header.contains("repo"), "legend missing 'repo':\n{header}");
        assert!(header.contains(th::WORKTREE), "legend missing ⧉:\n{header}");
        assert!(
            header.contains("worktree"),
            "legend missing 'worktree':\n{header}"
        );
        assert_eq!(
            glyph_fg(t.backend().buffer(), th::WORKTREE, 0),
            Some(WORKTREE_FG),
            "legend ⧉ must match the card tint"
        );
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
        // Skip row 0 (the SESSIONS header legend carries both glyphs).
        let out = buf_to_string_from(t.backend().buffer(), 1);
        // Worktree sessions carry the ⧉ marker (not the repo-root ⎇) plus the
        // branch name and diff; the old "worktree" word tag is gone.
        assert!(
            out.contains(th::WORKTREE),
            "missing ⧉ worktree glyph:\n{out}"
        );
        assert!(out.contains("feature-x"), "missing branch name:\n{out}");
        assert!(out.contains("+3"), "missing diff stat:\n{out}");
        assert!(!out.contains(th::BRANCH), "worktree must not use ⎇:\n{out}");
        assert!(!out.contains("worktree"), "stale word tag present:\n{out}");
        assert_eq!(
            glyph_fg(t.backend().buffer(), th::WORKTREE, 1),
            Some(WORKTREE_FG),
            "⧉ must be terracotta-tinted"
        );
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
        // Skip row 0 (the SESSIONS header legend carries both glyphs).
        let out = buf_to_string_from(t.backend().buffer(), 1);
        assert!(out.contains(th::BRANCH), "repo root must use ⎇:\n{out}");
        assert!(
            !out.contains(th::WORKTREE),
            "repo root must not use ⧉:\n{out}"
        );
        assert_eq!(
            glyph_fg(t.backend().buffer(), th::BRANCH, 1),
            Some(BRANCH_FG),
            "⎇ must be slate-teal-tinted"
        );
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

    #[test]
    fn restarting_card_shows_yellow_restarting_label() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Running, None)];
        app.restarting.insert("proj".into(), 0);
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let buf = t.backend().buffer();
        let s = buf_to_string(buf);
        assert!(
            s.contains("restarting"),
            "expected 'restarting' label:\n{s}"
        );
        // Must NOT show the normal Running status label.
        assert!(
            !s.contains(" running"),
            "must not show 'running' while restarting:\n{s}"
        );
        assert_eq!(
            glyph_fg(buf, crate::spinner::glyph(0), 1),
            Some(Color::Yellow),
            "restarting spinner must be yellow"
        );
    }

    #[test]
    fn non_restarting_card_shows_normal_status() {
        let mut app = App::new(Config::default());
        app.sessions = vec![sess("proj", Status::Running, None)];
        // restarting is empty — normal status shown
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        t.draw(|f| render(f, f.area(), &app)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(
            s.contains("running"),
            "expected normal 'running' label:\n{s}"
        );
        assert!(
            !s.contains("restarting"),
            "must not show 'restarting':\n{s}"
        );
    }
}
