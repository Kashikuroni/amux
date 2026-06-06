//! Contract-based verification runner for amux sessions.
//!
//! Parses `.amux/verify.toml` (the *contract*: an ordered list of *gates*,
//! one command each), runs the gates in a worktree without a shell, and
//! reports a *verdict*. Knows nothing about amux itself — also usable from
//! CI or by hand via the `amux-verify` binary.
