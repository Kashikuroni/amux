//! Variant A (Studio) design tokens: warm dark palette + glyphs.
use ratatui::style::Color;

pub const BG: Color = Color::Rgb(0x1a, 0x17, 0x14);
pub const BG_RAISED: Color = Color::Rgb(0x21, 0x1d, 0x18);
pub const BG_SUNKEN: Color = Color::Rgb(0x16, 0x13, 0x0f);
/// Pre-blended amber-dim used as the selected-row background (terminals have no alpha).
pub const SEL_BG: Color = Color::Rgb(0x2a, 0x1d, 0x18);
pub const TEXT: Color = Color::Rgb(0xe8, 0xdf, 0xd1);
pub const TEXT_BOLD: Color = Color::Rgb(0xf4, 0xec, 0xdd);
pub const MUTED: Color = Color::Rgb(0x8a, 0x7f, 0x6e);
pub const DIM: Color = Color::Rgb(0x5c, 0x54, 0x4a);
pub const BORDER: Color = Color::Rgb(0x2f, 0x2a, 0x23);
pub const BORDER_HI: Color = Color::Rgb(0x40, 0x3a, 0x30);
pub const AMBER: Color = Color::Rgb(0xd9, 0x77, 0x57);
pub const AMBER_HI: Color = Color::Rgb(0xf4, 0xa3, 0x6a);
pub const GREEN: Color = Color::Rgb(0x7a, 0xb8, 0x7a);
pub const RED: Color = Color::Rgb(0xc7, 0x5d, 0x4a);
pub const YELLOW: Color = Color::Rgb(0xd6, 0xb2, 0x5f);
pub const BLUE: Color = Color::Rgb(0x6a, 0x9f, 0xb5);

// Glyphs
pub const LOGO: &str = "◆";
pub const PREVIEW_MARK: &str = "▸";
pub const AGENT_MARK: &str = "✻";
pub const SEL_BAR: &str = "▍";
pub const IDLE_DOT: &str = "·";
pub const BRANCH: &str = "⎇";
pub const SEP: &str = "│";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amber_token_matches_design() {
        assert_eq!(AMBER, Color::Rgb(0xd9, 0x77, 0x57));
    }
}
