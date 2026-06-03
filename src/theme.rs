//! Design tokens. The UI is intentionally colorless: every token resolves to
//! `Color::Reset` so text, backgrounds and borders use the terminal's own
//! default colors and font. Visual hierarchy is conveyed purely through font
//! attributes (`BOLD`/`DIM`/`REVERSED`), which the terminal also defines.
//! The tokens are kept (rather than deleted) so call sites stay unchanged and a
//! palette could be reintroduced in one place later.
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

pub const BG: Color = Color::Reset;
pub const BG_RAISED: Color = Color::Reset;
pub const BG_SUNKEN: Color = Color::Reset;
/// Selected-row highlight. Terminals have no true alpha, so this fakes a
/// "barely-there" fill with a very dark neutral gray — the light default text
/// color faded almost down to the background. Uses **truecolor RGB** (an
/// absolute value) on purpose: 256-color palette slots get remapped to a warm
/// orange on this user's terminal, RGB does not. Tune in one place; raise the
/// channels (44,44,44 → 56,56,56…) for a stronger band, lower them to fade more.
pub const SEL_BG: Color = Color::Rgb(42, 42, 42);
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
pub const IDLE_MARK: &str = "⏸"; // pause — agent is idle
pub const WAIT_MARK: &str = "●"; // solid dot — agent is waiting on the user
pub const BRANCH: &str = "⎇";
/// Branch marker for sessions running in a linked git worktree (vs the repo
/// root, which uses `BRANCH`). The two joined frames read as "linked checkout".
pub const WORKTREE: &str = "⧉";
pub const SEP: &str = "│";
pub const RULE_CHAR: &str = "━";

/// Base panel block: full rounded border. Callers chain `.border_style`,
/// `.title`, and `.style` to taste. Keeps the rounded-corner choice in one place.
pub fn panel() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
}

/// Style for structural chrome — borders, rules and separators. Dimmed so the
/// lines read as quiet scaffolding rather than bright white: in the colorless
/// theme, `DIM` (a font attribute) is how chrome recedes. `color` is the token
/// (all `Reset` today) so a future palette still flows through one place.
pub fn chrome(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::DIM)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_are_colorless() {
        // The UI must stay on the terminal's own colors.
        // SEL_BG is intentionally a subtle color (the selection highlight); all
        // other tokens stay on the terminal's own colors.
        for c in [
            BG, BG_RAISED, TEXT, TEXT_BOLD, MUTED, DIM, BORDER, AMBER, GREEN, RED,
        ] {
            assert_eq!(c, Color::Reset);
        }
    }
}
