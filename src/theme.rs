//! Design tokens. The UI is intentionally colorless: every token resolves to
//! `Color::Reset` so text, backgrounds and borders use the terminal's own
//! default colors and font. Visual hierarchy is conveyed purely through font
//! attributes (`BOLD`/`DIM`/`REVERSED`), which the terminal also defines.
//! The tokens are kept (rather than deleted) so call sites stay unchanged and a
//! palette could be reintroduced in one place later.
use ratatui::style::Color;
use ratatui::widgets::{Block, BorderType, Borders};

pub const BG: Color = Color::Reset;
pub const BG_RAISED: Color = Color::Reset;
pub const BG_SUNKEN: Color = Color::Reset;
pub const SEL_BG: Color = Color::Reset;
pub const TEXT: Color = Color::Reset;
pub const TEXT_BOLD: Color = Color::Reset;
pub const MUTED: Color = Color::Reset;
pub const DIM: Color = Color::Reset;
pub const BORDER: Color = Color::Reset;
pub const BORDER_HI: Color = Color::Reset;
pub const AMBER: Color = Color::Reset;
pub const AMBER_HI: Color = Color::Reset;
pub const GREEN: Color = Color::Reset;
pub const RED: Color = Color::Reset;
pub const YELLOW: Color = Color::Reset;
pub const BLUE: Color = Color::Reset;

// Glyphs
pub const LOGO: &str = "◆";
pub const PREVIEW_MARK: &str = "▸";
pub const AGENT_MARK: &str = "✻";
pub const SEL_BAR: &str = "▍";
pub const IDLE_DOT: &str = "·";
pub const BRANCH: &str = "⎇";
pub const SEP: &str = "│";
pub const RULE_CHAR: &str = "━";

/// Base panel block: full rounded border. Callers chain `.border_style`,
/// `.title`, and `.style` to taste. Keeps the rounded-corner choice in one place.
pub fn panel() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_are_colorless() {
        // The UI must stay on the terminal's own colors.
        for c in [BG, BG_RAISED, SEL_BG, TEXT, TEXT_BOLD, MUTED, DIM, BORDER, AMBER, GREEN, RED] {
            assert_eq!(c, Color::Reset);
        }
    }
}
