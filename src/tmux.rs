use std::io;
use std::process::Command;

/// Tab-separated fields requested from `tmux list-sessions -F`.
/// Order: name, path, created, @cm_managed, @cm_agent, attached-client-count, @cm_repo.
pub const LIST_FORMAT: &str =
    "#{session_name}\t#{session_path}\t#{session_created}\t#{@cm_managed}\t#{@cm_agent}\t#{session_attached}\t#{@cm_repo}";

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Running,
    Idle,
    /// Agent is blocked on the user: a numbered prompt (digits 1–9) is on screen
    /// awaiting a choice or answer. Set from prompt detection, not pane diffing.
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub dir: String,
    pub created: i64,
    pub agent: String,
    pub status: Status,
    pub attached: bool,
    pub git: Option<crate::git::GitInfo>,
    /// Repo root if this session runs in a `cm`-created worktree; None otherwise.
    pub worktree_repo: Option<String>,
}

/// Parses `tmux list-sessions` output, keeping only sessions marked `@cm_managed=1`.
/// `status` defaults to `Idle`; the app overwrites it via capture-pane diffing.
pub fn parse_sessions(output: &str) -> Vec<Session> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Session> {
    let mut f = line.splitn(7, '\t');
    let name = f.next()?.to_string();
    let dir = f.next()?.to_string();
    let created = f.next()?.trim().parse::<i64>().ok()?;
    let managed = f.next()?;
    let agent = f.next()?.to_string();
    if managed != "1" {
        return None;
    }
    let attached = f
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|n| n > 0)
        .unwrap_or(false);
    let worktree_repo = match f.next().map(str::trim) {
        Some(r) if !r.is_empty() => Some(r.to_string()),
        _ => None,
    };
    Some(Session {
        name,
        dir,
        created,
        agent,
        status: Status::Idle,
        attached,
        git: None,
        worktree_repo,
    })
}

/// Dedicated tmux socket so `cm` sessions and key bindings stay isolated from
/// the user's default tmux server.
const SOCKET: &str = "cm";

/// A `tmux` command pre-pointed at our private socket.
fn tmux() -> Command {
    let mut c = Command::new("tmux");
    c.args(["-L", SOCKET]);
    c
}

/// Runs a tmux subcommand, returning an error containing stderr on failure.
fn run(args: &[&str]) -> io::Result<()> {
    let out = tmux().args(args).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// True if a `tmux` binary is callable.
pub fn is_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if `am` itself is running inside a tmux client (nested attach is unsafe).
pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Lists managed sessions. Any tmux failure (e.g. no server running) is treated as an empty list.
pub fn list_sessions() -> io::Result<Vec<Session>> {
    let out = tmux().args(["list-sessions", "-F", LIST_FORMAT]).output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_sessions(&String::from_utf8_lossy(&out.stdout)))
}

/// Server options that keep panes sized to the attaching client (avoids stale
/// redraw artifacts when am's terminal differs from the detached default size).
fn apply_resize_options() {
    let _ = run(&["set-option", "-g", "window-size", "latest"]);
    let _ = run(&["set-window-option", "-g", "aggressive-resize", "on"]);
}

/// Root-table key bindings for attached sessions (server-global, but our socket
/// only hosts am sessions). All best-effort.
///
/// - `Ctrl-q` detaches (returns to am).
/// - `Ctrl-k` enters copy-mode (history scrollback) — no `prefix [` dance.
/// - Inside copy-mode, `Ctrl-k`/`Ctrl-j` scroll up/down a few lines at a time
///   (gentle step, not whole pages); `q` or `Esc` returns to the live pane.
///
/// Note: `Ctrl-k` is captured before the agent sees it (and `Ctrl-j` while in
/// copy-mode), so those chords don't reach the program in those states. Each
/// binding is a single tmux command — chaining with `;` from the CLI would run
/// the second command immediately instead of binding it.
const SCROLL_STEP: &str = "3"; // lines per Ctrl-k / Ctrl-j press in copy-mode

fn apply_key_bindings() {
    let _ = run(&["bind-key", "-n", "C-q", "detach-client"]);
    let _ = run(&["bind-key", "-n", "C-k", "copy-mode"]);
    for table in ["copy-mode", "copy-mode-vi"] {
        let _ = run(&[
            "bind-key",
            "-T",
            table,
            "C-k",
            "send-keys",
            "-N",
            SCROLL_STEP,
            "-X",
            "scroll-up",
        ]);
        let _ = run(&[
            "bind-key",
            "-T",
            table,
            "C-j",
            "send-keys",
            "-N",
            SCROLL_STEP,
            "-X",
            "scroll-down",
        ]);
    }
}

/// Creates a detached session running `agent` in `dir` and tags it as managed.
pub fn new_session(name: &str, dir: &str, agent: &str) -> io::Result<()> {
    // Create at the current terminal size so the first attach needs no resize.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (cols, rows) = (cols.max(1).to_string(), rows.max(1).to_string());
    run(&[
        "new-session",
        "-d",
        "-s",
        name,
        "-x",
        &cols,
        "-y",
        &rows,
        "-c",
        dir,
        agent,
    ])?;
    apply_resize_options();
    // If tagging fails, the session would exist untagged (invisible to list_sessions);
    // kill it so creation is all-or-nothing.
    if let Err(e) = run(&["set-option", "-t", name, "@cm_managed", "1"])
        .and_then(|_| run(&["set-option", "-t", name, "@cm_agent", agent]))
    {
        let _ = run(&["kill-session", "-t", name]);
        return Err(e);
    }
    // Detach + scroll key bindings. Server-global, but our socket only ever
    // hosts am sessions, so it stays scoped to them. Best-effort.
    apply_key_bindings();
    // Hide tmux's status bar — am provides its own chrome. Best-effort.
    let _ = run(&["set-option", "-g", "status", "off"]);
    Ok(())
}

/// Like `new_session`, but also tags the session with `@cm_repo=<repo_root>` so
/// the UI knows it runs in a worktree (enables worktree-aware kill).
pub fn new_worktree_session(name: &str, dir: &str, agent: &str, repo_root: &str) -> io::Result<()> {
    new_session(name, dir, agent)?;
    if let Err(e) = run(&["set-option", "-t", name, "@cm_repo", repo_root]) {
        let _ = run(&["kill-session", "-t", name]);
        return Err(e);
    }
    Ok(())
}

pub fn kill_session(name: &str) -> io::Result<()> {
    run(&["kill-session", "-t", name])
}

/// Sends a single key (e.g. a menu digit) to a session, then Enter to confirm.
pub fn send_choice(name: &str, digit: char) -> io::Result<()> {
    let d = digit.to_string();
    run(&["send-keys", "-t", name, &d, "Enter"])
}

/// Sends a Shift+Tab keypress (tmux key name `BTab`) to the session — used to
/// cycle the agent's own mode (e.g. Claude Code normal/auto-accept/plan).
pub fn send_shift_tab(name: &str) -> io::Result<()> {
    run(&["send-keys", "-t", name, "BTab"])
}

/// Sends literal text followed by Enter (a free-text reply).
pub fn send_text(name: &str, text: &str) -> io::Result<()> {
    run(&["send-keys", "-t", name, "-l", text])?;
    run(&["send-keys", "-t", name, "Enter"])
}

pub fn rename_session(old: &str, new: &str) -> io::Result<()> {
    run(&["rename-session", "-t", old, new])
}

/// Resizes a session's window to `cols`×`rows` so a detached capture reflows to
/// the preview's width instead of the (wider) creation size — otherwise the
/// agent's full-width input box wraps to a second row in the narrow preview.
///
/// `resize-window` flips the window to `window-size manual`, so `attach_session`
/// resets it to `latest` to restore fill-the-client sizing on attach.
/// Best-effort: a failure just leaves the previous (possibly wrapping) capture.
pub fn resize_window(name: &str, cols: u16, rows: u16) {
    let (cols, rows) = (cols.max(1).to_string(), rows.max(1).to_string());
    let _ = run(&["resize-window", "-t", name, "-x", &cols, "-y", &rows]);
}

/// Captures the visible pane content of a session as plain text.
pub fn capture_pane(name: &str) -> io::Result<String> {
    let out = tmux()
        .args(["capture-pane", "-p", "-e", "-t", name])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Captures the pane plus `history` lines of scrollback, so the preview can be
/// scrolled back without attaching. Falls back to the visible pane on failure.
pub fn capture_scrollback(name: &str, history: u32) -> io::Result<String> {
    let start = format!("-{history}");
    let out = tmux()
        .args(["capture-pane", "-p", "-e", "-S", &start, "-t", name])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Attaches in the foreground (inherits stdio) and returns when the user detaches.
pub fn attach_session(name: &str) -> io::Result<()> {
    // Ensure chrome/sizing options are applied for sessions created before they
    // existed, so attaching an existing session resizes its pane to fill.
    let _ = run(&["set-option", "-g", "status", "off"]);
    apply_key_bindings();
    apply_resize_options();
    // The preview shrinks the window via `resize_window`, which sets a per-window
    // `window-size manual`. Reset it to `latest` so attaching fills the client.
    let _ = run(&["set-window-option", "-t", name, "window-size", "latest"]);
    tmux().args(["attach-session", "-t", name]).status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_managed_session() {
        let out = "proj-a\t/home/u/proj-a\t1716800000\t1\tclaude\t0\t";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "proj-a");
        assert_eq!(sessions[0].dir, "/home/u/proj-a");
        assert_eq!(sessions[0].created, 1716800000);
        assert_eq!(sessions[0].agent, "claude");
        assert_eq!(sessions[0].status, Status::Idle);
        assert!(!sessions[0].attached);
        assert_eq!(sessions[0].worktree_repo, None);
    }

    #[test]
    fn parses_worktree_repo() {
        let out = "wt\t/r/.worktrees/x\t1\t1\tclaude\t0\t/r";
        let s = &parse_sessions(out)[0];
        assert_eq!(s.worktree_repo.as_deref(), Some("/r"));
    }

    #[test]
    fn empty_worktree_repo_is_none() {
        let out = "plain\t/d\t1\t1\tclaude\t0\t";
        let s = &parse_sessions(out)[0];
        assert_eq!(s.worktree_repo, None);
    }

    #[test]
    fn filters_out_unmanaged_sessions() {
        let out = "mine\t/d\t1\t1\tclaude\t0\nother\t/d\t1\t\t\t1";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "mine");
    }

    #[test]
    fn marks_attached_when_client_count_positive() {
        let out = "live\t/d\t1\t1\tclaude\t1";
        let sessions = parse_sessions(out);
        assert!(sessions[0].attached);
    }

    #[test]
    fn attached_count_greater_than_one_is_attached() {
        let out = "multi\t/d\t1\t1\tclaude\t2";
        let sessions = parse_sessions(out);
        assert!(sessions[0].attached);
    }

    #[test]
    fn empty_input_yields_no_sessions() {
        assert!(parse_sessions("").is_empty());
    }

    #[test]
    fn trailing_newline_does_not_add_empty_session() {
        let out = "solo\t/d\t1\t1\tclaude\t0\n";
        let sessions = parse_sessions(out);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "solo");
    }
}
