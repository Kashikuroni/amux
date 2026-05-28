//! Braille spinner frames, animated at ~80 ms/frame off an elapsed clock.
pub const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Frame index (0..10) for a given elapsed time, 80 ms per frame.
pub fn frame_index(elapsed_ms: u128) -> usize {
    ((elapsed_ms / 80) % 10) as usize
}

/// Spinner glyph for a frame index (wraps).
pub fn glyph(frame: usize) -> &'static str {
    BRAILLE[frame % BRAILLE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_index_advances_every_80ms() {
        assert_eq!(frame_index(0), 0);
        assert_eq!(frame_index(79), 0);
        assert_eq!(frame_index(80), 1);
        assert_eq!(frame_index(159), 1);
        assert_eq!(frame_index(800), 0); // wraps after 10 frames
    }

    #[test]
    fn glyph_wraps() {
        assert_eq!(glyph(0), "⠋");
        assert_eq!(glyph(10), "⠋");
        assert_eq!(glyph(1), "⠙");
    }
}
