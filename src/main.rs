use am::app::{Action, App};
use am::config::Config;
use am::state::State;
use am::{tmux, ui};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout, Stdout};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Restores the terminal on panic so a crash doesn't leave the shell unusable.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        original(info);
    }));
}

fn main() -> io::Result<()> {
    let config = Config::load();
    let refresh = Duration::from_millis(config.refresh_interval_ms.max(100));
    let mut app = App::new(config);
    let mut state = State::load();
    // Dead-project GC: a root whose directory is gone can never host a session
    // again — drop its note/name/order entries and persist the cleaned file.
    if state.prune_missing_projects(|root| std::path::Path::new(root).is_dir()) {
        state.save();
    }
    app.apply_state(state);
    // Read git off the UI thread so large/slow repos never stall rendering.
    app.attach_git_worker();
    if !tmux::is_available() {
        app.tmux_missing = true;
    } else {
        app.refresh();
    }

    install_panic_hook();
    // Poll Claude Code usage limits off the UI thread (a `curl` round-trip), and
    // hand fresh values to the loop over a channel so rendering never blocks.
    let usage_log = am::usage::new_log();
    app.usage_log = usage_log.clone();
    let usage_rx = spawn_usage_poller(usage_log);
    // One-shot release check; a found update arrives over this channel.
    let update_rx = am::update::spawn_check();

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, refresh, &usage_rx, &update_rx);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)?;
    if app.tmux_missing {
        std::process::exit(1);
    }
    Ok(())
}

/// Spawns a daemon thread that fetches account state (usage + plan) immediately
/// (so the header is populated at startup) and then on a slow interval. Failed
/// fields arrive as `None` so the loop keeps the last good value.
///
/// Cadence depends on the outcome, since the endpoint publishes no rate-limit
/// numbers (only an opaque `429` `rate_limit_error`):
///   - success            → `POLL_INTERVAL` (steady state; usage changes slowly)
///   - any failure/net    → `FAILURE_BACKOFF` (back off so we don't sustain it)
///
/// The first request still fires immediately at startup.
fn spawn_usage_poller(log: am::usage::UsageLog) -> mpsc::Receiver<am::usage::Account> {
    const POLL_INTERVAL: Duration = Duration::from_secs(300); // 5 min, steady state
    const FAILURE_BACKOFF: Duration = Duration::from_secs(600); // 10 min after any failure
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let acct = am::usage::fetch_account(&log);
            let last_ok = acct.usage.is_some();
            if tx.send(acct).is_err() {
                break; // receiver dropped → app is shutting down
            }
            thread::sleep(if last_ok {
                POLL_INTERVAL
            } else {
                FAILURE_BACKOFF
            });
        }
    });
    rx
}

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    // If any setup step fails after raw mode is on, undo everything so the user's
    // shell isn't left in raw mode / the alternate screen. The panic hook only
    // covers panics; this is the error path.
    if let Err(e) = execute!(
        out,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    ) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        return Err(e);
    }
    enable_key_disambiguation(&mut out);
    match Terminal::new(CrosstermBackend::new(out)) {
        Ok(term) => Ok(term),
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(
                stdout(),
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
            Err(e)
        }
    }
}

/// Asks the terminal (where supported) to report modified keys distinctly via
/// the kitty keyboard protocol, so chords like Shift+Enter arrive as a distinct
/// event rather than a bare Enter. A no-op on terminals without support.
fn enable_key_disambiguation<W: io::Write>(out: &mut W) {
    if matches!(supports_keyboard_enhancement(), Ok(true)) {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

fn restore_terminal(terminal: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    if matches!(supports_keyboard_enhancement(), Ok(true)) {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(
    terminal: &mut Term,
    app: &mut App,
    refresh: Duration,
    usage_rx: &mpsc::Receiver<am::usage::Account>,
    update_rx: &mpsc::Receiver<am::update::UpdateInfo>,
) -> io::Result<()> {
    let start = Instant::now();
    let tick = Duration::from_millis(80);
    let mut last_refresh = Instant::now();
    // Progress of an in-flight self-update install, if any.
    let mut install_rx: Option<mpsc::Receiver<am::update::UpdateStage>> = None;
    loop {
        app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
        // Drain account updates; keep the last good value on a failed fetch.
        while let Ok(acct) = usage_rx.try_recv() {
            if acct.usage.is_some() {
                app.usage = acct.usage;
                app.usage_error = None;
            } else if acct.usage_error.is_some() {
                app.usage_error = acct.usage_error;
            }
            if acct.plan.is_some() {
                app.plan = acct.plan;
            }
        }
        while let Ok(info) = update_rx.try_recv() {
            app.update = Some(info);
        }
        if let Some(rx) = &install_rx {
            while let Ok(stage) = rx.try_recv() {
                app.set_update_stage(stage);
            }
        }
        app.offer_update_if_idle();
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)? {
            let ev = event::read()?;
            if let Event::Paste(text) = &ev {
                if !app.tmux_missing {
                    app.handle_paste(text);
                }
                continue;
            }
            if let Event::Mouse(m) = &ev {
                match m.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        // Only scroll the preview when the cursor is over the right
                        // panel. The boundary is: x=2 margin + left-panel width.
                        let screen = terminal.size().unwrap_or_default();
                        let area_w = screen.width.saturating_sub(4);
                        let split_col = 2 + area_w * app.split_pct / 100 + 1;
                        if m.column >= split_col {
                            if m.kind == MouseEventKind::ScrollUp {
                                app.preview_scroll_up(3);
                            } else {
                                app.preview_scroll_down(3);
                            }
                        }
                    }
                    _ => {} // clicks/moves ignored
                }
                continue;
            }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if app.tmux_missing {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    continue;
                }
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action, &mut install_rx)?;
                }
                // Persist split width / session order if a key changed them.
                if app.dirty {
                    app.snapshot_state().save();
                    app.dirty = false;
                }
            }
        }

        if !app.tmux_missing && last_refresh.elapsed() >= refresh {
            app.refresh();
            last_refresh = Instant::now();

            // For sessions awaiting resume: once claude exits, the pane goes
            // dead (kept by remain-on-exit) with the `claude --resume <uuid>`
            // hint in its content — parse it and respawn the pane with that
            // command. Time out after 30 s (something went wrong — the dead
            // pane is left for inspection; kill with `d` or retry `u`).
            if !app.restarting.is_empty() {
                let mut to_clear: Vec<String> = Vec::new();
                for (name, &started) in &app.restarting {
                    if app.now_unix - started > 30 {
                        let _ = tmux::set_remain_on_exit(name, false);
                        to_clear.push(name.clone());
                        continue;
                    }
                    // The hint is printed by claude on exit; wait until the
                    // pane is actually dead (also guards against respawning —
                    // and killing — a still-live claude).
                    if !tmux::pane_dead(name).unwrap_or(false) {
                        continue;
                    }
                    if let Ok(pane) = tmux::capture_pane(name) {
                        if let Some(cmd) = tmux::parse_resume_command(&pane) {
                            let dir = app
                                .sessions
                                .iter()
                                .find(|s| s.name == *name)
                                .map(|s| s.dir.clone())
                                .unwrap_or_default();
                            if let Err(e) = tmux::respawn_pane(name, &dir, &cmd) {
                                app.error = Some(format!("resume: {e}"));
                            }
                            let _ = tmux::set_remain_on_exit(name, false);
                            to_clear.push(name.clone());
                        }
                    }
                }
                for name in &to_clear {
                    app.restarting.remove(name);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    // Flush state changed by background ticks (e.g. pruned drafts): the in-loop
    // save only runs on a keypress, so a quit right after a tick would lose it.
    if app.dirty {
        app.snapshot_state().save();
        app.dirty = false;
    }
    Ok(())
}

fn handle_action(
    terminal: &mut Term,
    app: &mut App,
    action: Action,
    install_rx: &mut Option<mpsc::Receiver<am::update::UpdateStage>>,
) -> io::Result<()> {
    match action {
        Action::Attach(name) => {
            if tmux::in_tmux() {
                app.error =
                    Some("detach from current tmux (Ctrl-B D) before attaching".to_string());
                return Ok(());
            }
            // Hand the terminal over to tmux, then take it back.
            restore_terminal(terminal)?;
            if let Err(e) = tmux::attach_session(&name) {
                app.error = Some(e.to_string());
            }
            enable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableMouseCapture
            )?;
            enable_key_disambiguation(terminal.backend_mut());
            terminal.clear()?;
            // Attaching reset the window to the full client size; force the next
            // refresh to re-fit it to the preview width.
            app.preview_sized = None;
            app.refresh();
        }
        Action::Create {
            name,
            dir,
            agent,
            worktree,
            terminal,
            model,
            effort,
        } => {
            let (command, label) = if terminal {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let label = tmux::shell_basename(&shell).to_string();
                (shell, label)
            } else {
                // Flags go into the command only; the label (@cm_agent, the
                // session list) stays the bare agent. The binary is resolved
                // to an absolute path so the tmux server's (possibly stale)
                // PATH cannot break the launch.
                (
                    resolve_agent_command_for_tmux(&am::app::compose_agent_command(
                        &agent,
                        model.as_deref(),
                        effort.as_deref(),
                    )),
                    agent.clone(),
                )
            };
            let result = match worktree {
                None => tmux::new_session(&name, &dir, &command, &label),
                Some(spec) => create_worktree_session(&name, &dir, &command, &label, &spec),
            };
            if let Err(e) = result {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::Kill {
            name,
            remove_worktree,
        } => {
            // Capture worktree info before the session disappears from the list.
            let wt = if remove_worktree {
                app.sessions
                    .iter()
                    .find(|s| s.name == name)
                    .and_then(|s| s.worktree_repo.clone().map(|repo| (repo, s.dir.clone())))
            } else {
                None
            };
            if let Err(e) = tmux::kill_session(&name) {
                app.error = Some(e.to_string());
            } else {
                if let Some((repo, path)) = wt {
                    if let Err(e) = am::git::remove_worktree(&repo, &path) {
                        app.error = Some(format!("session killed, worktree not removed: {e}"));
                    }
                }
                app.notes.remove(&name);
                app.drafts.remove(&name);
                app.dirty = true;
            }
            app.refresh();
        }
        Action::Rename { old, new } => {
            if let Err(e) = tmux::rename_session(&old, &new) {
                app.error = Some(e.to_string());
            } else {
                if let Some(text) = app.notes.remove(&old) {
                    app.notes.insert(new.clone(), text);
                    app.dirty = true;
                }
                if let Some(draft) = app.drafts.remove(&old) {
                    app.drafts.insert(new.clone(), draft);
                    app.dirty = true;
                }
            }
            app.refresh();
        }
        Action::SendChoice { name, digit } => {
            if let Err(e) = tmux::send_choice(&name, digit) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::SendText { name, text } => {
            if let Err(e) = tmux::send_text(&name, &text) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::SendShiftTab { name } => {
            if let Err(e) = tmux::send_shift_tab(&name) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::StartUpdate(info) => {
            // Idempotent: a second StartUpdate while one install is live would
            // orphan a swap thread — ignore it (the modal can't emit one, but
            // don't rely on UI invariants here).
            if install_rx.is_none() {
                *install_rx = Some(am::update::spawn_install(info));
            }
        }
        Action::RestartSelf => {
            restore_terminal(terminal)?;
            let err = am::update::restart(); // only returns on failure
                                             // exec failed — re-enter the TUI instead of dumping to a broken shell.
            enable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                EnableBracketedPaste,
                EnableMouseCapture
            )?;
            enable_key_disambiguation(terminal.backend_mut());
            terminal.clear()?;
            // Attaching reset the window to the full client size; force the next
            // refresh to re-fit it to the preview width.
            app.preview_sized = None;
            app.refresh();
            app.error = Some(format!("restart failed: {err}"));
        }
        Action::RestartAllClaude => {
            let now = app.now_unix;
            // is_claude: first whitespace token == "claude" (mirrors ui::preview::is_claude).
            let names: Vec<String> = app
                .sessions
                .iter()
                .filter(|s| s.agent.split_whitespace().next() == Some("claude"))
                .map(|s| s.name.clone())
                .collect();
            for name in names {
                // Safety net FIRST: sessions run the agent as the pane command,
                // so the session dies with the process. remain-on-exit keeps the
                // dead pane (and the session) alive with the --resume hint
                // readable. Never send Ctrl+C without it.
                if let Err(e) = tmux::set_remain_on_exit(&name, true) {
                    app.error = Some(format!("restart: {e}"));
                    continue;
                }
                if let Err(e) = tmux::send_ctrl_c(&name) {
                    app.error = Some(format!("restart: {e}"));
                    let _ = tmux::set_remain_on_exit(&name, false);
                } else {
                    app.restarting.insert(name, now);
                }
            }
            app.refresh();
        }
    }
    Ok(())
}

/// Creates the session for a branch choice that needs a worktree. `New` forks
/// a branch and adds a worktree (the original flow); `Existing` reuses the
/// worktree where the branch is already checked out, or adds one for it.
fn create_worktree_session(
    name: &str,
    dir: &str,
    command: &str,
    label: &str,
    spec: &am::app::WorktreeSpec,
) -> io::Result<()> {
    use am::app::WorktreeSpec;
    let repo = am::git::repo_root(dir)
        .ok_or_else(|| io::Error::other(format!("not a git repo: {dir}")))?;
    // .worktrees/<branch> path builder shared by both variants.
    let wt_for = |branch: &str| {
        std::path::Path::new(&repo)
            .join(".worktrees")
            .join(branch)
            .to_string_lossy()
            .to_string()
    };
    match spec {
        WorktreeSpec::New { base, branch } => {
            am::git::ensure_gitignore(&repo, ".worktrees/")?;
            let wt = wt_for(branch);
            am::git::prepare_worktree(&repo, &wt, branch, base)?;
            tmux::new_worktree_session(name, &wt, command, label, &repo)
        }
        WorktreeSpec::Existing { branch } => {
            match am::git::worktree_for_branch(&repo, branch) {
                // Checked out in the repo's main worktree → a plain session
                // there (not a removable worktree; no @cm_repo tag). Compare
                // canonicalized: porcelain paths are real, repo may be symlinked.
                Some(path) if same_dir(&path, &repo) => {
                    tmux::new_session(name, &repo, command, label)
                }
                // Already checked out in a linked worktree → open right there.
                Some(path) => tmux::new_worktree_session(name, &path, command, label, &repo),
                // Not checked out anywhere → add a worktree for it (no -b).
                None => {
                    am::git::ensure_gitignore(&repo, ".worktrees/")?;
                    let wt = wt_for(branch);
                    am::git::prepare_worktree_existing(&repo, &wt, branch)?;
                    tmux::new_worktree_session(name, &wt, command, label, &repo)
                }
            }
        }
    }
}

/// Rewrites the first word of an agent command to the absolute executable path
/// visible to amux itself. tmux panes inherit the tmux server's environment,
/// which may have an older PATH than the shell that launched amux.
fn resolve_agent_command_for_tmux(command: &str) -> String {
    let Some(bin) = command.split_whitespace().next() else {
        return command.to_string();
    };
    if bin.contains('/') {
        return command.to_string();
    }
    let Some(path) = am::app::resolve_agent_path(bin) else {
        return command.to_string();
    };
    let suffix = command.strip_prefix(bin).unwrap_or("");
    format!("{path}{suffix}")
}

/// Path equality robust to symlinks (macOS /var → /private/var) and trailing slashes.
fn same_dir(a: &str, b: &str) -> bool {
    let canon = |p: &str| {
        std::fs::canonicalize(p)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.trim_end_matches('/').to_string())
    };
    canon(a) == canon(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_command_keeps_absolute_path() {
        assert_eq!(
            resolve_agent_command_for_tmux("/usr/local/bin/opencode --flag"),
            "/usr/local/bin/opencode --flag"
        );
    }

    #[test]
    fn agent_command_keeps_relative_path() {
        assert_eq!(
            resolve_agent_command_for_tmux("./opencode --flag"),
            "./opencode --flag"
        );
    }

    #[test]
    fn agent_command_keeps_missing_binary() {
        let cmd = "definitely-not-an-amux-test-binary --flag";
        assert_eq!(resolve_agent_command_for_tmux(cmd), cmd);
    }

    #[test]
    fn agent_command_resolves_found_binary_and_keeps_args() {
        let resolved = resolve_agent_command_for_tmux("sh -c true");
        assert!(resolved.ends_with("sh -c true"), "{resolved}");
        assert!(resolved.starts_with('/'), "{resolved}");
    }
}
