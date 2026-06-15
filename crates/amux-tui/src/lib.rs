// Re-export amux-core so `crate::config`, `crate::tmux`, etc. resolve
// inside this crate's modules without touching the moved files.
pub use amux_core::*;

pub mod app;
pub mod theme;
pub mod ui;
pub mod usage;
