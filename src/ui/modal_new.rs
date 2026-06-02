use crate::app::{
    abbreviate_path, agent_display_name, expand_tilde, resolve_agent_path, CreateField, CreateForm,
    CUSTOM_AGENT_SLOT,
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

/// Uppercase section label with light letter-spacing (e.g. "NAME" → "N A M E").
fn label(text: &str, color: ratatui::style::Color) -> Span<'static> {
    let spaced: String = text
        .chars()
        .flat_map(|c| [c, ' '])
        .collect::<String>()
        .trim_end()
        .to_string();
    Span::styled(
        spaced,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Filled input box: sunken background, an accent bar on the left, then the value.
fn input_box(f: &mut Frame, rect: Rect, value: &str, focused: bool) {
    let bar = if focused { th::AMBER } else { th::BORDER_HI };
    let line = Line::from(vec![
        Span::styled(th::SEL_BAR, Style::default().fg(bar)),
        Span::raw(" "),
        Span::styled(value.to_string(), Style::default().fg(th::TEXT_BOLD)),
    ]);
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(th::BG_SUNKEN)),
        rect,
    );
}

/// Max subdir rows shown in the live picker before it windows.
const PICKER_MAX: usize = 8;
/// Fixed content rows (everything except the variable-height picker and worktree extras).
/// Includes the always-visible WORKTREE label + toggle line (2 rows).
const BASE_ROWS: u16 = 20;
/// Extra rows when the worktree toggle is on (BASE label+row + BRANCH label+row).
/// The WORKTREE label + toggle line are always visible and already counted in `BASE_ROWS`.
const WORKTREE_ROWS: u16 = 4;

pub fn render(f: &mut Frame, form: &CreateForm) {
    let full = f.area();
    let picker_active = form.field == CreateField::Dir && !form.dir_entries.is_empty();
    let want_picker = if picker_active {
        form.dir_entries.len().min(PICKER_MAX) as u16
    } else {
        0
    };
    // Panel hugs its content: borders (2) + top/bottom inner padding (2) + rows.
    // Clamp to the terminal so a short window never produces a negative origin.
    let wt_extra = if form.worktree { WORKTREE_ROWS } else { 0 };
    let h = (BASE_ROWS + want_picker + wt_extra + 4).min(full.height);
    let w = ((full.width as u32 * 72 / 100) as u16).min(full.width);
    let area = Rect {
        x: full.x + (full.width.saturating_sub(w)) / 2,
        y: full.y + (full.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, area);

    // Outer rounded panel; the header lives inside so we leave the title empty.
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
        let title = Line::from(vec![
            Span::styled("＋ ", Style::default().fg(th::AMBER)),
            Span::styled(
                "New session",
                Style::default().fg(th::AMBER).add_modifier(Modifier::BOLD),
            ),
        ]);
        let step = Line::from(Span::styled(
            format!("{} of {}", form.step(), form.total_steps()),
            Style::default().fg(th::DIM),
        ));
        f.render_widget(Paragraph::new(step).alignment(Alignment::Right), r);
        f.render_widget(Paragraph::new(title), r);
    }
    y += 2;

    // Rule under the header.
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

    // NAME
    if let Some(r) = row(x, y, w, bottom) {
        f.render_widget(Paragraph::new(Line::from(label("NAME", th::MUTED))), r);
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        input_box(f, r, &form.name, form.field == CreateField::Name);
    }
    y += 2;

    // DIRECTORY
    if let Some(r) = row(x, y, w, bottom) {
        f.render_widget(Paragraph::new(Line::from(label("DIRECTORY", th::MUTED))), r);
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        input_box(f, r, &form.dir, form.field == CreateField::Dir);
    }
    y += 1;

    // Directory validation + git branch.
    // NOTE: this stats the path (and forks git) on every redraw, including each
    // keystroke. The modal is short-lived so it's acceptable; if it ever feels
    // sluggish on large repos, cache on (dir -> (exists, branch)) in CreateForm.
    if let Some(r) = row(x, y, w, bottom) {
        let expanded = expand_tilde(&form.dir);
        let exists = Path::new(&expanded).is_dir();
        let mut spans = if exists {
            vec![
                Span::styled("✓ ", Style::default().fg(th::GREEN)),
                Span::styled("directory exists", Style::default().fg(th::MUTED)),
            ]
        } else {
            vec![
                Span::styled("✗ ", Style::default().fg(th::RED)),
                Span::styled("directory not found", Style::default().fg(th::RED)),
            ]
        };
        if exists {
            if let Some(g) = crate::git::read(&expanded) {
                spans.push(Span::styled("  ·  ", Style::default().fg(th::DIM)));
                spans.push(Span::styled(
                    format!("{} ", th::BRANCH),
                    Style::default().fg(th::DIM),
                ));
                spans.push(Span::styled(g.branch, Style::default().fg(th::TEXT)));
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), r);
    }
    y += 1;

    // Live subdir picker (shown while editing the dir). Windowed so the
    // highlighted row stays visible. Leave headroom for the agent section below.
    if picker_active {
        let reserved_below = 9u16; // blank + AGENT label + segments + cmd + desc + rule + footer
        let avail = bottom.saturating_sub(y).saturating_sub(reserved_below) as usize;
        let cap = (want_picker as usize).min(avail).max(1);
        let total = form.dir_entries.len();
        let start = if form.dir_selected >= cap {
            form.dir_selected + 1 - cap
        } else {
            0
        };
        let end = (start + cap).min(total);
        for i in start..end {
            let Some(r) = row(x, y, w, bottom) else { break };
            let selected = i == form.dir_selected;
            let text = format!(
                "  {}{}/",
                if selected { "> " } else { "  " },
                form.dir_entries[i]
            );
            let style = if selected {
                Style::default()
                    .fg(th::AMBER_HI)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(th::MUTED)
            };
            f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), r);
            y += 1;
        }
    }
    y += 1;

    // WORKTREE (optional) — label + toggle line.
    if let Some(r) = row(x, y, w, bottom) {
        let lbl_color = if form.field == CreateField::Worktree {
            th::AMBER
        } else {
            th::MUTED
        };
        f.render_widget(Paragraph::new(Line::from(label("WORKTREE", lbl_color))), r);
    }
    y += 1;
    if let Some(r) = row(x, y, w, bottom) {
        let mark = if form.worktree { "[x]" } else { "[ ]" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(th::AMBER)),
                Span::styled("Create worktree", Style::default().fg(th::TEXT_BOLD)),
                Span::styled("   space to toggle", Style::default().fg(th::DIM)),
            ])),
            r,
        );
    }
    y += 1;
    if form.worktree {
        // BASE picker (segmented, cycled with ← →).
        if let Some(r) = row(x, y, w, bottom) {
            f.render_widget(Paragraph::new(Line::from(label("BASE", th::MUTED))), r);
        }
        y += 1;
        if let Some(r) = row(x, y, w, bottom) {
            let mut seg: Vec<Span> = Vec::new();
            if form.base_branches.is_empty() {
                seg.push(Span::styled(
                    " (no branches) ",
                    Style::default().fg(th::DIM),
                ));
            } else {
                for (i, b) in form.base_branches.iter().enumerate() {
                    if i > 0 {
                        seg.push(Span::raw("   "));
                    }
                    let sel = i == form.base_index;
                    let st = if sel {
                        Style::default()
                            .bg(th::AMBER)
                            .fg(th::BG)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(th::MUTED)
                    };
                    seg.push(Span::styled(format!(" {b} "), st));
                }
            }
            f.render_widget(
                Paragraph::new(Line::from(seg)).style(Style::default().bg(th::BG_SUNKEN)),
                r,
            );
        }
        y += 1;
        // NEW BRANCH input.
        if let Some(r) = row(x, y, w, bottom) {
            f.render_widget(Paragraph::new(Line::from(label("BRANCH", th::MUTED))), r);
        }
        y += 1;
        if let Some(r) = row(x, y, w, bottom) {
            input_box(f, r, &form.new_branch, form.field == CreateField::Branch);
        }
        y += 1;
    }
    y += 1;

    // AGENT label + navigation hint (right).
    if let Some(r) = row(x, y, w, bottom) {
        let hint = Line::from(Span::styled(
            "← → switch · type to override",
            Style::default().fg(th::DIM),
        ));
        f.render_widget(Paragraph::new(hint).alignment(Alignment::Right), r);
        f.render_widget(Paragraph::new(Line::from(label("AGENT", th::AMBER))), r);
    }
    y += 1;

    // Segmented agent selector, in a filled box.
    if let Some(r) = row(x, y, w, bottom) {
        let mut seg: Vec<Span> = Vec::new();
        for (i, choice) in form.agent_choices.iter().enumerate() {
            if i > 0 {
                seg.push(Span::raw("   "));
            }
            let sel = i == form.agent_index;
            let st = if sel {
                Style::default()
                    .bg(th::AMBER)
                    .fg(th::BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(th::MUTED)
            };
            seg.push(Span::styled(format!(" {choice} "), st));
        }
        f.render_widget(
            Paragraph::new(Line::from(seg)).style(Style::default().bg(th::BG_SUNKEN)),
            r,
        );
    }
    y += 1;

    // Resolved command box: "$ <cmd>" with a right-aligned caption.
    if let Some(r) = row(x, y, w, bottom) {
        let cmd = if form.agent.is_empty() {
            "<type a command>".to_string()
        } else {
            form.agent.clone()
        };
        let left = Line::from(vec![
            Span::styled("$ ", Style::default().fg(th::DIM)),
            Span::styled(cmd, Style::default().fg(th::TEXT_BOLD)),
        ]);
        let caption = Line::from(Span::styled(
            "resolved command",
            Style::default().fg(th::DIM),
        ));
        f.render_widget(
            Paragraph::new(caption)
                .alignment(Alignment::Right)
                .style(Style::default().bg(th::BG_SUNKEN)),
            r,
        );
        f.render_widget(
            Paragraph::new(left).style(Style::default().bg(th::BG_SUNKEN)),
            r,
        );
    }
    y += 1;

    // Agent description: friendly name + resolved location (or not-found).
    if let Some(r) = row(x, y, w, bottom) {
        let line = if form.agent.is_empty() {
            Line::from(Span::styled(
                "type a command to run as the agent",
                Style::default().fg(th::DIM),
            ))
        } else if let Some(p) = resolve_agent_path(&form.agent) {
            Line::from(vec![
                Span::styled(
                    format!("{} — found at ", agent_display_name(&form.agent)),
                    Style::default().fg(th::DIM),
                ),
                Span::styled(p, Style::default().fg(th::MUTED)),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{} — ", agent_display_name(&form.agent)),
                    Style::default().fg(th::DIM),
                ),
                Span::styled("not found in PATH", Style::default().fg(th::YELLOW)),
            ])
        };
        f.render_widget(Paragraph::new(line), r);
    }
    y += 2;

    // Footer rule + the exact tmux command that will run.
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
            Paragraph::new(Line::from(vec![
                Span::styled("will run ", Style::default().fg(th::DIM)),
                Span::styled(cmd, Style::default().fg(th::MUTED)),
            ])),
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
    fn new_modal_shows_worktree_rows_when_enabled() {
        let mut form = CreateForm::new("claude", &["claude".into()]);
        form.worktree = true;
        form.base_branches = vec!["main".into()];
        form.new_branch = "feature-x".into();
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("B A S E"), "BASE label");
        assert!(s.contains("B R A N C H"), "BRANCH label");
        assert!(s.contains("feature-x"));
        assert!(s.contains("of 5"), "dynamic step total");
    }

    #[test]
    fn new_modal_hides_worktree_rows_by_default() {
        let form = CreateForm::new("claude", &["claude".into()]);
        let mut t = Terminal::new(TestBackend::new(90, 40)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(!s.contains("B A S E"));
        assert!(s.contains("of 3"));
    }

    #[test]
    fn new_modal_shows_header_labels_and_agent_segments() {
        let form = CreateForm::new("claude", &["claude".into(), "codex".into()]);
        let mut t = Terminal::new(TestBackend::new(80, 30)).unwrap();
        t.draw(|f| render(f, &form)).unwrap();
        let s = buf_to_string(t.backend().buffer());
        assert!(s.contains("New session"), "header title");
        assert!(s.contains("of 3"), "step indicator");
        // Section labels are letter-spaced, so match the spaced forms.
        assert!(s.contains("N A M E"));
        assert!(s.contains("D I R E C T O R Y"));
        assert!(s.contains("A G E N T"));
        assert!(s.contains("claude"));
        assert!(s.contains("codex"));
        assert!(s.contains("custom"));
        assert!(s.contains("will run tmux new"), "command preview");
    }
}
