use am::app::{Action, App};
use am::config::Config;
use am::state::State;
use am::{tmux, ui};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
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
    app.apply_state(State::load());
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
    let usage_rx = spawn_usage_poller();

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, refresh, &usage_rx);
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
///   - HTTP 429           → `RATE_LIMIT_BACKOFF` (back off so we don't sustain it)
///   - other failure/net  → `RETRY_INTERVAL` (recover quickly from a transient error)
///
/// The first request still fires immediately at startup.
fn spawn_usage_poller() -> mpsc::Receiver<am::usage::Account> {
    const POLL_INTERVAL: Duration = Duration::from_secs(180); // 3 min, steady state
    const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(180); // 3 min after a 429
    const RETRY_INTERVAL: Duration = Duration::from_secs(30); // transient error (net/token)
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut got_usage = false;
        loop {
            let acct = am::usage::fetch_account();
            got_usage |= acct.usage.is_some();
            let rate_limited = acct.usage_error.as_deref() == Some("429");
            if tx.send(acct).is_err() {
                break; // receiver dropped → app is shutting down
            }
            let wait = if got_usage {
                POLL_INTERVAL
            } else if rate_limited {
                RATE_LIMIT_BACKOFF
            } else {
                RETRY_INTERVAL
            };
            thread::sleep(wait);
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
) -> io::Result<()> {
    let start = Instant::now();
    let tick = Duration::from_millis(80);
    let mut last_refresh = Instant::now();
    loop {
        app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
        // Drain account updates; keep the last good value on a failed fetch.
        while let Ok(acct) = usage_rx.try_recv() {
            if acct.usage.is_some() {
                app.usage = acct.usage;
            }
            if acct.plan.is_some() {
                app.plan = acct.plan;
            }
            // Track the latest fetch outcome (cleared to None on success).
            app.usage_error = acct.usage_error;
        }
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)? {
            let ev = event::read()?;
            if let Event::Paste(text) = &ev {
                if !app.tmux_missing {
                    app.handle_paste(text);
                }
                continue;
            }
            if let Event::Mouse(_) = &ev {
                continue; // wheel/click do nothing — kills accidental list scroll
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
                    handle_action(terminal, app, action)?;
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
    Ok(())
}

fn handle_action(terminal: &mut Term, app: &mut App, action: Action) -> io::Result<()> {
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
        } => {
            let (command, label) = if terminal {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let label = tmux::shell_basename(&shell).to_string();
                (shell, label)
            } else {
                (agent.clone(), agent.clone())
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

/// Creates a git worktree under `<repo>/.worktrees/<branch>` then starts a tmux
/// session in it. Any git step failing aborts before the session is created.
fn create_worktree_session(
    name: &str,
    dir: &str,
    command: &str,
    label: &str,
    spec: &am::app::WorktreeSpec,
) -> io::Result<()> {
    let repo = am::git::repo_root(dir)
        .ok_or_else(|| io::Error::other(format!("not a git repo: {dir}")))?;
    am::git::ensure_gitignore(&repo, ".worktrees/")?;
    let wt_path = std::path::Path::new(&repo)
        .join(".worktrees")
        .join(&spec.new_branch);
    let wt_str = wt_path.to_string_lossy().to_string();
    am::git::prepare_worktree(&repo, &wt_str, &spec.new_branch, &spec.base)?;
    tmux::new_worktree_session(name, &wt_str, command, label, &repo)
}
