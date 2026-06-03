//! Pure markdown-note logic: parse lines, count tasks, toggle checkboxes, and
//! extract selected tasks as a numbered list. No UI, no IO — all functions are
//! deterministic and unit-tested.

/// A single parsed line of a note. Only the subset we render specially is
/// distinguished; everything else is `Text`.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteLine {
    /// `- [ ] text` (open) or `- [x] text` (done).
    Task { done: bool, text: String },
    /// `# text` .. `###### text`; `level` is the number of leading `#`.
    Heading { level: u8, text: String },
    /// `- text` or `* text` (a non-task bullet).
    Bullet(String),
    /// Any other non-empty line.
    Text(String),
    /// An empty line.
    Blank,
}

/// If `line` is a checkbox task, returns `(done, text)` with the `- [ ] ` prefix
/// stripped. Leading whitespace is allowed before the dash.
fn parse_task(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim_start();
    let b = trimmed.as_bytes();
    // Need at least "- [x]" (5 bytes) then the body.
    if b.len() >= 5 && trimmed.starts_with("- [") && b[4] == b']' {
        let done = match b[3] {
            b' ' => false,
            b'x' | b'X' => true,
            _ => return None,
        };
        return Some((done, trimmed[5..].trim_start().to_string()));
    }
    None
}

/// Parse a whole note buffer into typed lines (split on `\n`).
pub fn parse(buf: &str) -> Vec<NoteLine> {
    buf.split('\n').map(parse_line).collect()
}

fn parse_line(line: &str) -> NoteLine {
    if let Some((done, text)) = parse_task(line) {
        return NoteLine::Task { done, text };
    }
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return NoteLine::Blank;
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        let extra = rest.chars().take_while(|&c| c == '#').count();
        let level = (1 + extra).min(6) as u8;
        let text = trimmed[level as usize..].trim_start().to_string();
        return NoteLine::Heading { level, text };
    }
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return NoteLine::Bullet(rest.to_string());
    }
    NoteLine::Text(line.to_string())
}

/// `(done, total)` task counts for the card progress indicator.
pub fn counts(buf: &str) -> (u32, u32) {
    let mut done = 0;
    let mut total = 0;
    for line in buf.split('\n') {
        if let Some((d, _)) = parse_task(line) {
            total += 1;
            if d {
                done += 1;
            }
        }
    }
    (done, total)
}

/// Buffer line indices (0-based, over `split('\n')`) that are tasks, in order.
/// The position in the returned vec is the task ordinal.
pub fn task_line_indices(buf: &str) -> Vec<usize> {
    buf.split('\n')
        .enumerate()
        .filter(|(_, l)| parse_task(l).is_some())
        .map(|(i, _)| i)
        .collect()
}

/// Flip the `[ ]` <-> `[x]` checkbox of the `ord`-th task (0-based). No-op if
/// `ord` is out of range. Preserves all other text and line structure.
pub fn toggle(buf: &mut String, ord: usize) {
    let mut seen = 0;
    let lines: Vec<String> = buf
        .split('\n')
        .map(|line| {
            if let Some((done, _)) = parse_task(line) {
                let out = if seen == ord {
                    let lead_len = line.len() - line.trim_start().len();
                    let (lead, rest) = line.split_at(lead_len);
                    let mark = if done { ' ' } else { 'x' };
                    // `rest` starts with "- [x]" (5 ASCII bytes).
                    format!("{lead}- [{mark}]{}", &rest[5..])
                } else {
                    line.to_string()
                };
                seen += 1;
                out
            } else {
                line.to_string()
            }
        })
        .collect();
    *buf = lines.join("\n");
}

/// Render the given task ordinals as a numbered list (`"1. text\n2. text"`),
/// stripping the `- [ ]` prefix. Includes every requested task regardless of
/// done state, renumbered from 1 in the given order. Unknown ordinals skipped.
pub fn selected_as_numbered(buf: &str, ords: &[usize]) -> String {
    let texts: Vec<String> = buf
        .split('\n')
        .filter_map(|l| parse_task(l).map(|(_, t)| t))
        .collect();
    let mut out = String::new();
    let mut n = 1;
    for &ord in ords {
        if let Some(t) = texts.get(ord) {
            if n > 1 {
                out.push('\n');
            }
            out.push_str(&format!("{n}. {t}"));
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_line_kind() {
        let buf = "# Title\n- [ ] open\n- [x] done\n- bullet\nplain\n";
        let lines = parse(buf);
        assert_eq!(
            lines[0],
            NoteLine::Heading {
                level: 1,
                text: "Title".into()
            }
        );
        assert_eq!(
            lines[1],
            NoteLine::Task {
                done: false,
                text: "open".into()
            }
        );
        assert_eq!(
            lines[2],
            NoteLine::Task {
                done: true,
                text: "done".into()
            }
        );
        assert_eq!(lines[3], NoteLine::Bullet("bullet".into()));
        assert_eq!(lines[4], NoteLine::Text("plain".into()));
        assert_eq!(lines[5], NoteLine::Blank); // trailing newline => empty last line
    }

    #[test]
    fn uppercase_x_is_done() {
        assert_eq!(
            parse("- [X] hi")[0],
            NoteLine::Task {
                done: true,
                text: "hi".into()
            }
        );
    }

    #[test]
    fn counts_tasks() {
        assert_eq!(counts("- [ ] a\n- [x] b\ntext\n- [x] c"), (2, 3));
        assert_eq!(counts("no tasks here"), (0, 0));
    }

    #[test]
    fn task_line_indices_maps_ordinals_to_lines() {
        // lines: 0 heading, 1 task, 2 blank, 3 task
        assert_eq!(task_line_indices("# h\n- [ ] a\n\n- [x] b"), vec![1, 3]);
    }

    #[test]
    fn toggle_flips_the_nth_task_only() {
        let mut buf = "- [ ] a\n- [ ] b".to_string();
        toggle(&mut buf, 1);
        assert_eq!(buf, "- [ ] a\n- [x] b");
        toggle(&mut buf, 1); // idempotent flip back
        assert_eq!(buf, "- [ ] a\n- [ ] b");
    }

    #[test]
    fn toggle_preserves_leading_whitespace() {
        let mut buf = "  - [ ] indented".to_string();
        toggle(&mut buf, 0);
        assert_eq!(buf, "  - [x] indented");
    }

    #[test]
    fn selected_as_numbered_strips_prefix_and_renumbers() {
        let buf = "- [ ] first\ntext\n- [x] second\n- [ ] third";
        // ords are task ordinals: 0=first, 1=second, 2=third
        assert_eq!(selected_as_numbered(buf, &[0, 2]), "1. first\n2. third");
        assert_eq!(selected_as_numbered(buf, &[1]), "1. second");
    }
}
