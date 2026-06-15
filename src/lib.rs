// Pure business logic — provided by amux-core
pub use amux_core::browse;
pub use amux_core::changelog;
pub use amux_core::clip;
pub use amux_core::config;
pub use amux_core::doctor;
pub use amux_core::editor;
pub use amux_core::git;
pub use amux_core::note;
pub use amux_core::spinner;
pub use amux_core::state;
pub use amux_core::timeutil;
pub use amux_core::tmux;
pub use amux_core::update;
pub use amux_core::verify;

// TUI-specific (still in src/)
pub mod app;
pub mod theme;
pub mod ui;
pub mod usage; // src/usage.rs — ratatui rendering + re-exports amux_core::usage::*
