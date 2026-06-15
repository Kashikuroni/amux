//! A small Markdown → ratatui renderer for the changelog (What's New + the
//! Help → Changelog tab). Uses `pulldown-cmark` purely as a parser and maps its
//! event stream to styled [`Line`]s: headings, **bold**, *italic*, `inline
//! code`, and bullet lists. Block-level only enough for release notes — no
//! tables/images/blockquotes (rendered as plain text if present).

use crate::theme as th;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Render markdown `md` to styled lines. Inline markers (`` ` ``, `**`) are
/// consumed by the parser, so the output carries styling, not raw syntax.
pub fn render(md: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut heading: Option<HeadingLevel> = None;
    let mut list_depth: usize = 0;
    let mut at_item_start = false;

    // Flush the in-progress span run as one line.
    fn flush(out: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
        if !spans.is_empty() {
            out.push(Line::from(std::mem::take(spans)));
        }
    }

    for ev in Parser::new(md) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut out, &mut spans);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut out, &mut spans);
                heading = None;
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut out, &mut spans);
                out.push(Line::from(""));
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 {
                    out.push(Line::from(""));
                }
            }
            Event::Start(Tag::Item) => at_item_start = true,
            Event::End(TagEnd::Item) => flush(&mut out, &mut spans),
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Text(t) => {
                start_item(&mut spans, &mut at_item_start, list_depth);
                spans.push(Span::styled(
                    t.into_string(),
                    inline_style(heading, bold, italic),
                ));
            }
            Event::Code(c) => {
                start_item(&mut spans, &mut at_item_start, list_depth);
                spans.push(Span::styled(
                    c.into_string(),
                    Style::default().fg(th::AMBER_HI),
                ));
            }
            // Within a block, breaks join with a space; ratatui wraps visually.
            Event::SoftBreak | Event::HardBreak => spans.push(Span::raw(" ")),
            _ => {}
        }
    }
    flush(&mut out, &mut spans);
    // Drop trailing blank lines left by block separators.
    while out.last().is_some_and(line_is_blank) {
        out.pop();
    }
    out
}

/// Emit the bullet prefix the first time content lands in a list item.
fn start_item(spans: &mut Vec<Span<'static>>, at_item_start: &mut bool, list_depth: usize) {
    if *at_item_start {
        let indent = "  ".repeat(list_depth.saturating_sub(1));
        spans.push(Span::styled(
            format!("  {indent}• "),
            Style::default().fg(th::MUTED),
        ));
        *at_item_start = false;
    }
}

/// Style for inline text: a heading style when inside a heading, else base text
/// with bold/italic toggles applied.
fn inline_style(heading: Option<HeadingLevel>, bold: bool, italic: bool) -> Style {
    if let Some(level) = heading {
        let base = Style::default().add_modifier(Modifier::BOLD);
        return match level {
            HeadingLevel::H1 | HeadingLevel::H2 => base.fg(th::AMBER),
            _ => base.fg(th::MUTED),
        };
    }
    let mut s = Style::default().fg(th::TEXT);
    if bold {
        s = s.add_modifier(Modifier::BOLD);
    }
    if italic {
        s = s.add_modifier(Modifier::ITALIC);
    }
    s
}

fn line_is_blank(line: &Line) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Concatenate a line's span contents into plain text.
    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
    fn all_text(lines: &[Line]) -> String {
        lines.iter().map(text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn inline_code_drops_backticks() {
        let lines = render("press `ctrl+r` now");
        assert_eq!(all_text(&lines), "press ctrl+r now");
    }

    #[test]
    fn bold_drops_markers_and_sets_modifier() {
        let lines = render("a **big** deal");
        assert_eq!(all_text(&lines), "a big deal");
        // The bolded span carries the BOLD modifier.
        let bold_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "big")
            .expect("bold span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn heading_drops_hashes() {
        let lines = render("### Added");
        assert_eq!(all_text(&lines).trim(), "Added");
    }

    #[test]
    fn bullet_list_gets_glyph() {
        let lines = render("- first\n- second");
        let joined = all_text(&lines);
        assert!(joined.contains("• first"), "bullet glyph + text:\n{joined}");
        assert!(joined.contains("• second"), "second bullet:\n{joined}");
        // No raw markdown dashes leak as list markers.
        assert!(!joined.contains("- first"));
    }
}
