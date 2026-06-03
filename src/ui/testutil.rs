//! Shared test helpers for the `ui` modules (compiled only under `cfg(test)`).
use ratatui::buffer::Buffer;

/// Dump a rendered buffer to a newline-joined string of cell symbols, for
/// substring assertions in snapshot-style tests.
pub(crate) fn buf_to_string(buf: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}
