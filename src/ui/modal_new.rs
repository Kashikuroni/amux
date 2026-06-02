use crate::app::{
    abbreviate_path, expand_tilde, resolve_agent_path, CreateField, CreateForm, CUSTOM_AGENT_SLOT,
};
use crate::theme as th;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::path::Path;

/// A 1-row rect at (x, y) of width w, clamped so nothing draws past `bottom`.
/// Returns None when there's no room left, so callers can stop early.
fn row(x: u16, y: u16, w: u16, bottom: u16) -> Option<Rect> {
    if y >= bottom {
        return None;
    }
    Some(Rect {
        x,
        y,
        width: w,
        height: 1,
    })
}

/// Width of the left label column (longest label, "directory" = 9, plus gutter).
const LABEL_W: usize = 11;
/// Columns a sub-line indents to align under the value column (label + "▍ ").
const VALUE_INDENT: u16 = LABEL_W as u16 + 2;

/// Quiet, lowercase left label, padded to the column width.
fn lbl(text: &str) -> Span<'static> {
    Span::styled(
        format!("{text:<LABEL_W$}"),
        Style::default().add_modifier(Modifier::DIM),
    )
}

/// Paints the focused row with a faint full-width band (the same cue the session
/// list uses for selection); leaves unfocused rows on the terminal background.
fn band(p: Paragraph<'_>, focused: bool) -> Paragraph<'_> {
    if focused {
        p.style(Style::default().bg(th::SEL_BG))
    } else {
        p
    }
}

/// Inline text-input row: `label  ▍ value`. Focused rows get the band, a bold
/// bar, and a bold value; empty fields show a dim placeholder.
fn input_row(
    f: &mut Frame,
    rect: Rect,
    label: &str,
    value: &str,
    placeholder: &str,
    focused: bool,
) {
    let bar = if focused {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        th::chrome(th::BORDER_HI)
    };
    let val = if value.is_empty() {
        Span::styled(
            placeholder.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        )
    } else {
        let st = if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Span::styled(value.to_string(), st)
    };
    let line = Line::from(vec![
        lbl(label),
        Span::styled(format!("{} ", th::SEL_BAR), bar),
        val,
    ]);
    f.render_widget(band(Paragraph::new(line), focused), rect);
}

/// Inline picker row: `label  ‹selected›  other  other`. The selected choice is
/// bold and bracketed; the rest are dim. Focused rows get the band.
fn segment_row(
    f: &mut Frame,
    rect: Rect,
    label: &str,
    choices: &[String],
    selected: usize,
    focused: bool,
    empty_hint: &str,
) {
    let mut spans = vec![lbl(label)];
    if choices.is_empty() {
        spans.push(Span::styled(
            empty_hint.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        ));
    } else {
        // Uniform ` choice ` cells (even gaps); the selected one is bold, the
        // rest dim — weight alone marks the choice in the colorless theme.
        for (i, c) in choices.iter().enumerate() {
            let st = if i == selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            spans.push(Span::styled(format!(" {c} "), st));
        }
    }
    f.render_widget(band(Paragraph::new(Line::from(spans)), focused), rect);
}

/// Max subdir rows shown in the live picker before it windows.
const PICKER_MAX: usize = 8;
/// Fixed content rows (header, rule, name, dir + validation, worktree, agent,
/// rule, command, and the blanks between groups) when the worktree is off.
const BASE_ROWS: u16 = 13;
/// Extra rows when the worktree toggle is on (base picker + branch input).
const WORKTREE_ROWS: u16 = 2;

pub fn render(f: &mut Frame, form: &CreateForm) {
    let full = f.area();
    let picker_active = form.field == CreateField::Dir && !form.dir_entries.is_empty();
    let want_picker = if picker_active {
        form.dir_entries.len().min(PICKER_MAX) as u16
    } else {
        0
    };
    // The agent-not-found warning takes one extra row only when it's shown.
    let agent_warn = !form.agent.is_empty() && resolve_agent_path(&form.agent).is_none();
    let warn_extra = u16::from(agent_warn);
    // Panel hugs its content: border (2) + top/bottom inner padding (2) + rows.
    let wt_extra = if form.worktree { WORKTREE_ROWS } else { 0 };
    let h = (BASE_ROWS + want_picker + wt_extra + warn_extra + 4).min(full.height);
    let w = ((full.width as u32 * 70 / 100) as u16).min(full.width);
    let area = Rect {
        x: full.x + (full.width.saturating_sub(w)) / 2,
        y: full.y + (full.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, area);

    f.render_widget(
        th::panel()
            .border_style(th::chrome(th::BORDER_HI))
            .style(Style::default().bg(th::BG_RAISED)),
        area,
    );

    let pad: u16 = 2;
    let x = area.x + 1 + pad;
    let w = area.width.saturating_sub(2 + pad * 2);
    let bottom = area.y + area.height.saturating_sub(1);
    let mut y = area.y + 2;

    // Header: title (left) + step indicator (right).
    if let Some(r) = row(x, y, w, bottom) {
        let title = Line::from(Span::styled(
            "New session",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        let step = Line::from(Span::styled(
            format!("{} of {}", form.step(), form.total_steps()),
            Style::default().add_modifier(Modifier::DIM),
        ));
        f.render_widget(Paragraph::new(step).alignment(Alignment::Right), r);
        f.render_widget(Paragraph::new(title), r);
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(w as usize),
                th::chrome(th::BORDER),
            ))),
            r,
        );
    }
    y += 2;

    // name.
    if let Some(r) = row(x, y, w, bottom) {
        input_row(
            f,
            r,
            "name",
            &form.name,
            "session name",
            form.field == CreateField::Name,
        );
    }
    y += 1;

    // directory + a quiet validation sub-line (exists / not found · branch).
    if let Some(r) = row(x, y, w, bottom) {
        input_row(
            f,
            r,
            "directory",
            &form.dir,
            "~/",
            form.field == CreateField::Dir,
        );
    }
    y += 1;
    if let Some(r) = row(x + VALUE_INDENT, y, w.saturating_sub(VALUE_INDENT), bottom) {
        let expanded = expand_tilde(&form.dir);
        let spans = if Path::new(&expanded).is_dir() {
            let mut v = vec![Span::styled(
                "exists",
                Style::default().add_modifier(Modifier::DIM),
            )];
            if let Some(g) = crate::git::read(&expanded) {
                v.push(Span::styled(
                    "  ·  ",
                    Style::default().add_modifier(Modifier::DIM),
                ));
                v.push(Span::styled(
                    format!("{} {}", th::BRANCH, g.branch),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            v
        } else {
            vec![Span::styled(
                "not found",
                Style::default().add_modifier(Modifier::DIM | Modifier::BOLD),
            )]
        };
        f.render_widget(Paragraph::new(Line::from(spans)), r);
    }
    y += 1;

    // Live subdir picker (while editing the dir), indented under the value.
    if picker_active {
        let total = form.dir_entries.len();
        let cap = want_picker as usize;
        let start = if form.dir_selected >= cap {
            form.dir_selected + 1 - cap
        } else {
            0
        };
        let end = (start + cap).min(total);
        for i in start..end {
            let Some(r) = row(x + VALUE_INDENT, y, w.saturating_sub(VALUE_INDENT), bottom) else {
                break;
            };
            let selected = i == form.dir_selected;
            let text = format!(
                "{}{}/",
                if selected { "‹ " } else { "  " },
                form.dir_entries[i]
            );
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), r);
            y += 1;
        }
    }
    y += 1;

    // worktree toggle.
    if let Some(r) = row(x, y, w, bottom) {
        let focused = form.field == CreateField::Worktree;
        let line = if !form.dir_is_repo() {
            Line::from(vec![
                lbl("worktree"),
                Span::styled(
                    "[ ] needs a git repo",
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ])
        } else {
            let mark = if form.worktree { "[x]" } else { "[ ]" };
            Line::from(vec![
                lbl("worktree"),
                Span::styled(format!("{mark} create worktree"), Style::default()),
                Span::styled("   space", Style::default().add_modifier(Modifier::DIM)),
            ])
        };
        f.render_widget(band(Paragraph::new(line), focused), r);
    }
    y += 1;

    if form.worktree {
        if let Some(r) = row(x, y, w, bottom) {
            segment_row(
                f,
                r,
                "base",
                &form.base_branches,
                form.base_index,
                form.field == CreateField::Base,
                "(no branches)",
            );
        }
        y += 1;
        if let Some(r) = row(x, y, w, bottom) {
            input_row(
                f,
                r,
                "branch",
                &form.new_branch,
                "new branch name",
                form.field == CreateField::Branch,
            );
        }
        y += 1;
    }
    y += 1;

    // agent picker (+ a quiet warning sub-line when the command isn't in PATH).
    if let Some(r) = row(x, y, w, bottom) {
        segment_row(
            f,
            r,
            "agent",
            &form.agent_choices,
            form.agent_index,
            form.field == CreateField::Agent,
            "",
        );
    }
    y += 1;
    if agent_warn {
        if let Some(r) = row(x + VALUE_INDENT, y, w.saturating_sub(VALUE_INDENT), bottom) {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "not found in PATH",
                    Style::default().add_modifier(Modifier::DIM | Modifier::BOLD),
                ))),
                r,
            );
        }
        y += 1;
    }
    y += 1;

    // Footer rule + the single command that will run.
    if let Some(r) = row(x, y, w, bottom) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(w as usize),
                th::chrome(th::BORDER),
            ))),
            r,
        );
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        let agent = if form.agent.is_empty() {
            CUSTOM_AGENT_SLOT
        } else {
            form.agent.as_str()
        };
        let name = if form.name.is_empty() {
            "<name>"
        } else {
            form.name.as_str()
        };
        let cmd = format!(
            "tmux new -s {} -c {} \"{}\"",
            name,
            abbreviate_path(&form.dir),
            agent,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                cmd,
                Style::default().add_modifier(Modifier::DIM),
            ))),
            r,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn new_modal_shows_inline_labels_and_agent_segments() {
        let form = CreateForm::new("claude", &["claude".into(), "codex".into()]);
        let mut t = Terminal::new(TestBackend::new(80, 30)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("New session"), "header title");
        assert!(s.contains("of 3"), "step indicator");
        // Inline lowercase labels (no letter-spacing).
        assert!(s.contains("name"));
        assert!(s.contains("directory"));
        assert!(s.contains("agent"));
        assert!(s.contains("claude"));
        assert!(s.contains("codex"));
        assert!(s.contains("custom"));
        assert!(s.contains("tmux new"), "command preview");
        // The old letter-spaced caps labels are gone.
        assert!(!s.contains("N A M E"));
    }

    #[test]
    fn new_modal_shows_worktree_rows_when_enabled() {
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.new_branch = "feature-x".into();
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("base"), "base label");
        assert!(s.contains("branch"), "branch label");
        assert!(s.contains("feature-x"));
        assert!(s.contains("of 5"), "dynamic step total");
    }

    #[test]
    fn new_modal_hides_worktree_rows_by_default() {
        let form = CreateForm::new("claude", &["claude".into()]);
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("base"));
        assert!(s.contains("of 3"));
    }

    #[test]
    fn prefilled_modal_shows_two_steps_and_project_values() {
        let form = CreateForm::for_project("/home/u/proj", "claude", &["claude".into()]);
        let mut t = Terminal::new(TestBackend::new(80, 30)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("of 2"), "streamlined step total:\n{s}");
        assert!(s.contains("proj"), "project path on directory row:\n{s}");
        assert!(s.contains("claude"), "project agent shown:\n{s}");
        assert!(s.contains("name"), "name row present:\n{s}");
    }
}
