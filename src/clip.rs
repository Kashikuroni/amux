//! Clipboard via macOS `pbcopy`. Best-effort: any failure is silently ignored,
//! same as the other shell-outs in this app. Formatting is done by the caller
//! (`note::selected_as_numbered`); this only pipes a string to the clipboard.
use std::io::Write;
use std::process::{Command, Stdio};

/// Copy `text` to the system clipboard via `pbcopy`. No-op on failure.
pub fn copy(text: &str) {
    let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}
