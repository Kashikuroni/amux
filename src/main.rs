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
    // `amux doctor [--clean]` runs the diagnostic and exits; everything else is
    // the TUI. Kept argv-trivial — no parser dependency.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "doctor") {
        return am::doctor::run(&args[1..]);
    }

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
    // One-shot "What's New": diff the version recorded last run against this
    // build, then record the current version (saved immediately so the modal
    // shows exactly once, even if the app later crashes before a normal save).
    let current_version = env!("CARGO_PKG_VERSION");
    app.whats_new = am::changelog::whats_new_on_upgrade(
        app.last_version.as_deref(),
        current_version,
        am::changelog::raw(),
    );
    app.last_version = Some(current_version.to_string());
    app.snapshot_state().save();
    // Read git off the UI thread so large/slow repos never stall rendering.
    app.attach_git_worker();
    app.attach_verifier();
    if !tmux::is_available() {
        app.tmux_missing = true;
    } else {
        app.refresh();
        restore_sessions(&mut app);
        // Surface the upgrade notes over the live dashboard.
        if !app.whats_new.is_empty() {
            app.mode = am::app::Mode::WhatsNew;
        }
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

/// How long the event loop should block in `event::poll` this iteration.
/// Listed in the order the cases are checked:
///
/// - `tmux_missing` shows a static error screen → wake rarely (only a keypress
///   matters).
/// - A `Running` session animates the spinner → wake at the 80 ms frame cadence.
/// - Otherwise (idle) sleep until the next `refresh` is due, floored to 1 ms so a
///   just-due refresh doesn't busy-spin at a 0 ms timeout.
fn poll_timeout(
    any_running: bool,
    tmux_missing: bool,
    since_refresh: Duration,
    refresh: Duration,
) -> Duration {
    const SPINNER_TICK: Duration = Duration::from_millis(80);
    const FLOOR: Duration = Duration::from_millis(1);
    const IDLE_WAIT: Duration = Duration::from_secs(1);
    if tmux_missing {
        return IDLE_WAIT;
    }
    if any_running {
        return SPINNER_TICK;
    }
    refresh.saturating_sub(since_refresh).max(FLOOR)
}

/// How long the selected session must stay settled before the loop captures its
/// preview — coalesces a burst of `j`/`k` into a single tmux capture+resize (F4).
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

fn run(
    terminal: &mut Term,
    app: &mut App,
    refresh: Duration,
    usage_rx: &mpsc::Receiver<am::usage::Account>,
    update_rx: &mpsc::Receiver<am::update::UpdateInfo>,
) -> io::Result<()> {
    let start = Instant::now();
    let mut last_refresh = Instant::now();
    let mut install_rx: Option<mpsc::Receiver<am::update::UpdateStage>> = None;
    let mut issue_rx: Option<mpsc::Receiver<am::git::IssueStage>> = None;
    // Draw once on entry; thereafter only when something changed (F2).
    let mut needs_redraw = true;
    // When the selection changes, capture the new preview after it settles.
    let mut preview_deadline: Option<Instant> = None;
    loop {
        // Drain background channels; any applied message changes the view.
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
            needs_redraw = true;
        }
        while let Ok(info) = update_rx.try_recv() {
            app.update = Some(info);
            needs_redraw = true;
        }
        if let Some(rx) = &install_rx {
            while let Ok(stage) = rx.try_recv() {
                app.set_update_stage(stage);
                needs_redraw = true;
            }
        }
        if let Some(rx) = &issue_rx {
            while let Ok(stage) = rx.try_recv() {
                app.set_issue_stage(stage);
                needs_redraw = true;
            }
        }

        // Advance the spinner; only a running session animates, and only an
        // actual frame change needs a redraw.
        let prev_frame = app.spinner_frame;
        app.spinner_frame = am::spinner::frame_index(start.elapsed().as_millis());
        let any_running = app
            .sessions
            .iter()
            .any(|s| s.status == am::tmux::Status::Running);
        if any_running && app.spinner_frame != prev_frame {
            needs_redraw = true;
        }
        if app.offer_update_if_idle() {
            needs_redraw = true;
        }

        // Render only when something changed. The contract: every state
        // mutation above (and in the event block below) must set `needs_redraw`
        // — there is no wrapper enforcing it, so a new mutation that forgets it
        // will leave the screen stale.
        if needs_redraw {
            terminal.draw(|f| ui::draw(f, app))?;
            needs_redraw = false;
        }

        let mut timeout = poll_timeout(
            any_running,
            app.tmux_missing,
            last_refresh.elapsed(),
            refresh,
        );
        if let Some(deadline) = preview_deadline {
            // Don't sleep past a pending preview capture.
            let until = deadline.saturating_duration_since(Instant::now());
            timeout = timeout.min(until.max(Duration::from_millis(1)));
        }
        if event::poll(timeout)? {
            let ev = event::read()?;
            // Any event reaching the loop may change the view; ignored ones
            // (mouse moves, key-releases) cost at most one extra redraw.
            needs_redraw = true;
            if let Event::Paste(text) = &ev {
                if !app.tmux_missing {
                    app.handle_paste(text);
                }
                continue;
            }
            if let Event::Mouse(m) = &ev {
                if matches!(
                    m.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) {
                    let up = m.kind == MouseEventKind::ScrollUp;
                    // Wheel up → toward the top (smaller offset); down → further.
                    let wheel = |cur: u16| {
                        if up {
                            cur.saturating_sub(3)
                        } else {
                            cur.saturating_add(3)
                        }
                    };
                    // A scrollable modal captures the wheel so it never leaks to
                    // the preview underneath. Only the bare list scrolls the
                    // preview (and only when the cursor is over the right panel).
                    match &app.mode {
                        am::app::Mode::WhatsNew => {
                            app.whats_new_scroll = wheel(app.whats_new_scroll)
                        }
                        am::app::Mode::Help => app.help_scroll = wheel(app.help_scroll),
                        am::app::Mode::UsageLog => {
                            app.usage_log_scroll = wheel(app.usage_log_scroll)
                        }
                        am::app::Mode::List
                        | am::app::Mode::Filter
                        | am::app::Mode::SelectSession => {
                            let screen = terminal.size().unwrap_or_default();
                            let split_col = preview_boundary_col(screen.width, app.split_pct);
                            if m.column >= split_col {
                                if up {
                                    app.preview_scroll_up(3);
                                } else {
                                    app.preview_scroll_down(3);
                                }
                            }
                        }
                        // Other modals/overlays swallow the wheel.
                        _ => {}
                    }
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
                // Note the selected session before/after: a pure navigation key
                // (no Action) that changed it arms the preview-capture debounce.
                let prev_sel = app.selected_name();
                let action = app.handle_key(key);
                let nav_changed = action.is_none() && app.selected_name() != prev_sel;
                if let Some(action) = action {
                    handle_action(terminal, app, action, &mut install_rx, &mut issue_rx)?;
                }
                // Persist split width / session order if a key changed them.
                if app.dirty {
                    app.snapshot_state().save();
                    app.dirty = false;
                }
                if nav_changed {
                    // Re-arm on each move so a held key coalesces to one capture.
                    preview_deadline = Some(Instant::now() + PREVIEW_DEBOUNCE);
                }
            }
        }

        // Selection settled long enough → capture its preview once.
        if let Some(deadline) = preview_deadline {
            if Instant::now() >= deadline {
                app.update_preview();
                preview_deadline = None;
                needs_redraw = true;
            }
        }

        if !app.tmux_missing && last_refresh.elapsed() >= refresh {
            if app.refresh() {
                needs_redraw = true;
            }
            last_refresh = Instant::now();
            // refresh() just recaptured the selected session's preview, so any
            // pending debounce is redundant.
            preview_deadline = None;

            // For sessions awaiting resume: once claude exits, the pane goes
            // dead (kept by remain-on-exit) with the `claude --resume <uuid>`
            // hint in its content — parse it and respawn the pane with that
            // command. Time out after 30 s (something went wrong — the dead
            // pane is left for inspection; kill with `d` or retry `u`).
            if !app.restarting.is_empty() {
                // A pending restart mutates session state / errors as panes die
                // and respawn; always repaint while it's in flight.
                needs_redraw = true;
                let mut to_clear: Vec<String> = Vec::new();
                for (name, req) in &app.restarting {
                    if app.now_unix - req.started > 30 {
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
                    // Now that the agent has exited and no longer holds the
                    // worktree as its cwd, run any deferred promote git work
                    // (stash → remove worktree → checkout → pop) before the
                    // respawn drops Claude back in the repo root.
                    // If a promote git step fails, the worktree still exists —
                    // respawn the agent back in it (restore in place) rather
                    // than stranding it in the repo root.
                    let mut respawn_override: Option<String> = None;
                    if let Some(op) = &req.promote {
                        if let Err(e) =
                            am::git::promote_worktree(&op.repo_root, &op.worktree_dir, &op.branch)
                        {
                            app.error = Some(format!("promote failed, restored in worktree: {e}"));
                            respawn_override = Some(op.worktree_dir.clone());
                        }
                    }
                    if let Ok(pane) = tmux::capture_pane(name) {
                        if let Some(cmd) = tmux::parse_resume_command(&pane) {
                            let dir = respawn_override
                                .clone()
                                .unwrap_or_else(|| am::app::respawn_dir(req, &app.sessions, name));
                            if let Err(e) = tmux::respawn_pane(name, &dir, &cmd) {
                                app.error = Some(format!("resume: {e}"));
                            } else {
                                if let Some(ps) = app.session_persist.get_mut(name) {
                                    ps.resume_cmd = Some(cmd.clone());
                                    app.dirty = true;
                                }
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

/// Hands the terminal over to tmux for `name` and takes it back on detach.
/// Shared by Attach and OpenEditor.
fn attach_and_restore(terminal: &mut Term, app: &mut App, name: &str) -> io::Result<()> {
    restore_terminal(terminal)?;
    if let Err(e) = tmux::attach_session(name) {
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
    Ok(())
}

fn handle_action(
    terminal: &mut Term,
    app: &mut App,
    action: Action,
    install_rx: &mut Option<mpsc::Receiver<am::update::UpdateStage>>,
    issue_rx: &mut Option<mpsc::Receiver<am::git::IssueStage>>,
) -> io::Result<()> {
    match action {
        Action::Attach(name) => {
            if tmux::in_tmux() {
                app.error =
                    Some("detach from current tmux (Ctrl-B D) before attaching".to_string());
                return Ok(());
            }
            attach_and_restore(terminal, app, &name)?;
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
            let (command, label, resume_cmd) = if terminal {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let label = tmux::shell_basename(&shell).to_string();
                (shell, label, None)
            } else {
                // For Claude sessions inject --session-id <uuid> so the conversation
                // is resumable from cold start without needing a prior clean restart.
                let (base, resume_cmd) = if agent == "claude" {
                    match generate_uuid() {
                        Some(uuid) => {
                            let cmd = am::app::compose_agent_command(
                                &agent,
                                model.as_deref(),
                                effort.as_deref(),
                            ) + &format!(" --session-id {uuid}");
                            let rc = format!("claude --resume {uuid}");
                            (cmd, Some(rc))
                        }
                        None => (
                            am::app::compose_agent_command(
                                &agent,
                                model.as_deref(),
                                effort.as_deref(),
                            ),
                            None,
                        ),
                    }
                } else {
                    (
                        am::app::compose_agent_command(&agent, model.as_deref(), effort.as_deref()),
                        None,
                    )
                };
                // The binary is resolved to an absolute path so the tmux server's
                // (possibly stale) PATH cannot break the launch.
                (
                    resolve_agent_command_for_tmux(&base),
                    agent.clone(),
                    resume_cmd,
                )
            };
            let result = match worktree {
                None => tmux::new_session(&name, &dir, &command, &label),
                Some(spec) => create_worktree_session(&name, &dir, &command, &label, &spec),
            };
            if let Err(e) = result {
                app.error = Some(e.to_string());
            } else if !terminal {
                app.session_persist.insert(
                    name.clone(),
                    am::state::PersistedSession {
                        dir: dir.clone(),
                        agent: agent.clone(),
                        resume_cmd,
                    },
                );
                app.dirty = true;
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
                // Leave the `session_persist` entry: the upcoming refresh sees
                // the tmux session gone and moves it into recents (so a killed
                // agent can be re-spawned), then drops it from persist.
                app.dirty = true;
            }
            app.refresh();
        }
        Action::RestoreRecent { name } => {
            if let Some(pos) = app.recents.iter().position(|r| r.name == name) {
                let r = app.recents[pos].clone();
                // Resume the agent if we have a `--resume` command, else a fresh
                // session in the saved dir — same path as cold-start restore.
                let command = match &r.resume_cmd {
                    Some(cmd) => resolve_agent_command_for_tmux(cmd),
                    None => resolve_agent_command_for_tmux(&r.agent),
                };
                if let Err(e) = tmux::new_session(&r.name, &r.dir, &command, &r.agent) {
                    app.error = Some(format!("restore '{}': {e}", r.name));
                } else {
                    app.recents.remove(pos);
                    app.session_persist.insert(
                        r.name.clone(),
                        am::state::PersistedSession {
                            dir: r.dir.clone(),
                            agent: r.agent.clone(),
                            resume_cmd: r.resume_cmd.clone(),
                        },
                    );
                    app.left_tab = am::app::LeftTab::Current; // show the live result
                    app.dirty = true;
                }
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
                if let Some(ps) = app.session_persist.remove(&old) {
                    app.session_persist.insert(new.clone(), ps);
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
                    app.restarting.insert(
                        name,
                        am::app::RestartReq {
                            started: now,
                            root: None,
                            promote: None,
                        },
                    );
                }
            }
            app.refresh();
        }
        Action::ReturnToRoot { name, root } => {
            let is_claude = app
                .sessions
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.agent.split_whitespace().next() == Some("claude"))
                .unwrap_or(false);
            if is_claude {
                // Same pipeline as `u`, but the poll loop respawns in `root`:
                // remain-on-exit keeps the dead pane with the --resume hint,
                // Ctrl+C exits Claude, then it is respawned (resumed) in root.
                let now = app.now_unix;
                if let Err(e) = tmux::set_remain_on_exit(&name, true) {
                    app.error = Some(format!("return to root: {e}"));
                } else if let Err(e) = tmux::send_ctrl_c(&name) {
                    app.error = Some(format!("return to root: {e}"));
                    let _ = tmux::set_remain_on_exit(&name, false);
                } else {
                    app.restarting.insert(
                        name,
                        am::app::RestartReq {
                            started: now,
                            root: Some(root),
                            promote: None,
                        },
                    );
                }
            } else {
                // Plain shell: run the cd directly.
                let cmd = format!("cd {}", tmux::shell_single_quote(&root));
                if let Err(e) = tmux::send_text(&name, &cmd) {
                    app.error = Some(format!("return to root: {e}"));
                }
            }
            app.refresh();
        }
        Action::Verify { name } => {
            let dir = app
                .sessions
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.dir.clone());
            match dir {
                None => app.error = Some(format!("session '{name}' not found")),
                Some(dir) => match am::verify::load_contract(std::path::Path::new(&dir)) {
                    Ok(contract) => {
                        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        app.verify_cancel.insert(name.clone(), cancel.clone());
                        app.verification.insert(
                            name.clone(),
                            am::app::VerificationState::Running {
                                total: contract.gates.len(),
                                done: 0,
                                current: String::new(),
                            },
                        );
                        if let Some(w) = &app.verify_worker {
                            let _ = w.tx.send(am::verify::VerifyRequest {
                                name: name.clone(),
                                dir: std::path::PathBuf::from(&dir),
                                contract,
                                cancel,
                            });
                        }
                    }
                    Err(e) => app.error = Some(e),
                },
            }
            app.refresh();
        }
        Action::CancelVerify { name } => {
            if let Some(c) = app.verify_cancel.remove(&name) {
                c.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            app.verification.remove(&name);
            app.refresh();
        }
        Action::PromoteWorktree { name, branch } => {
            let session = app.sessions.iter().find(|s| s.name == name);
            let Some(s) = session else {
                app.error = Some(format!("session '{name}' not found"));
                app.refresh();
                return Ok(());
            };
            let worktree_dir = s.dir.clone();
            let is_claude = s.agent.split_whitespace().next() == Some("claude");
            let repo_root = match s
                .worktree_repo
                .clone()
                .or_else(|| am::git::repo_root(&s.dir))
            {
                Some(r) => r,
                None => {
                    app.error = Some("could not determine repo root".into());
                    app.refresh();
                    return Ok(());
                }
            };
            let promote = am::app::PromoteOp {
                repo_root: repo_root.clone(),
                worktree_dir: worktree_dir.clone(),
                branch: branch.clone(),
            };
            if is_claude {
                // The agent owns the pane, so the cd/checkout can't be typed —
                // it would land in Claude's prompt. Exit Claude first (the
                // proven return-to-root pipeline: remain-on-exit keeps the dead
                // pane with the `--resume` hint, Ctrl+C exits it), defer the git
                // work to the poll loop once the pane is dead, then respawn
                // Claude resumed in the repo root.
                let now = app.now_unix;
                if let Err(e) = tmux::set_remain_on_exit(&name, true) {
                    app.error = Some(format!("promote: {e}"));
                } else if let Err(e) = tmux::send_ctrl_c(&name) {
                    app.error = Some(format!("promote: {e}"));
                    let _ = tmux::set_remain_on_exit(&name, false);
                } else {
                    app.restarting.insert(
                        name,
                        am::app::RestartReq {
                            started: now,
                            root: Some(repo_root),
                            promote: Some(promote),
                        },
                    );
                }
            } else {
                // Plain shell: do the git work directly, then send a `cd` so the
                // shell leaves the now-removed worktree dir.
                if let Err(e) = am::git::promote_worktree(
                    &promote.repo_root,
                    &promote.worktree_dir,
                    &promote.branch,
                ) {
                    app.error = Some(format!("promote failed: {e}"));
                } else {
                    let cmd = format!("cd {}", tmux::shell_single_quote(&repo_root));
                    if let Err(e) = tmux::send_text(&name, &cmd) {
                        app.error = Some(format!("promote: cd failed: {e}"));
                    }
                }
            }
            app.refresh();
        }
        Action::DeleteBranch {
            branch, repo_root, ..
        } => {
            if let Err(e) = am::git::delete_branch(&repo_root, &branch) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::CleanupBranches {
            repo_root,
            branches,
        } => {
            let mut errors: Vec<String> = vec![];
            for branch in &branches {
                if let Err(e) = am::git::delete_branch(&repo_root, branch) {
                    errors.push(format!("{branch}: {e}"));
                }
            }
            if !errors.is_empty() {
                app.error = Some(errors.join(" | "));
            }
            app.refresh();
        }
        Action::OpenEditor { name } => {
            if tmux::in_tmux() {
                app.error =
                    Some("detach from current tmux (Ctrl-B D) before attaching".to_string());
                return Ok(());
            }
            let Some(src) = app.sessions.iter().find(|s| s.name == name).cloned() else {
                return Ok(());
            };
            // `e` on the editor session itself just re-enters it.
            let editor_name = if src.agent == "nvim" {
                src.name.clone()
            } else {
                format!("{}-nvim", src.name)
            };
            if !app.sessions.iter().any(|s| s.name == editor_name) {
                // `nvim .` so it opens the directory (netrw file explorer) rather
                // than an empty start screen. `.` resolves against the pane cwd,
                // which `new_session`'s `-c <dir>` pins to the agent's cwd below.
                let cmd = resolve_agent_command_for_tmux("nvim .");
                // The editor opens where the agent works *now* (live cwd). Tag
                // it with the project root whenever that differs (worktree or a
                // cd'd-into subdir) so it groups with the project in the list.
                let root = am::app::session_root(&src).to_string();
                let result = if src.cwd.trim_end_matches('/') != root {
                    tmux::new_worktree_session(&editor_name, &src.cwd, &cmd, "nvim", &root)
                } else {
                    tmux::new_session(&editor_name, &src.cwd, &cmd, "nvim")
                };
                if let Err(e) = result {
                    app.error = Some(format!("nvim: {e}"));
                    return Ok(());
                }
                // Not persisted (unlike agents): after a reboot an empty nvim
                // would be pointless — sessions are rebuilt for agents only.
            }
            attach_and_restore(terminal, app, &editor_name)?;
        }
        Action::CreateIssue {
            repo_root,
            title,
            body,
        } => {
            // Overwriting a still-live receiver is fine: the old thread's send
            // fails silently and the thread exits.
            *issue_rx = Some(am::git::spawn_issue_create(repo_root, title, body));
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

/// Generates a random UUID v4 via `uuidgen`, lowercased. Returns `None` if
/// `uuidgen` is unavailable or fails (session is still created, just without
/// a pre-assigned ID).
fn generate_uuid() -> Option<String> {
    let out = std::process::Command::new("uuidgen").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_lowercase())
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

/// Recreates any persisted agent sessions that are absent from the live tmux
/// server. Called once at startup: after a computer reboot the tmux server is
/// gone and all sessions need to be rebuilt from state.
///
/// For Claude Code: respawns with the saved `--resume <uuid>` command so the
/// conversation history is preserved. Falls back to a fresh session if no UUID
/// was saved (e.g. the session was never restarted cleanly while amux was open).
/// For Codex and other agents: starts a fresh session in the saved directory.
fn restore_sessions(app: &mut App) {
    let existing: std::collections::HashSet<String> =
        app.sessions.iter().map(|s| s.name.clone()).collect();
    let to_restore: Vec<(String, am::state::PersistedSession)> = app
        .session_persist
        .iter()
        .filter(|(name, _)| !existing.contains(*name))
        .map(|(n, ps)| (n.clone(), ps.clone()))
        .collect();
    if to_restore.is_empty() {
        return;
    }
    for (name, ps) in &to_restore {
        let command = match &ps.resume_cmd {
            Some(cmd) => resolve_agent_command_for_tmux(cmd),
            None => resolve_agent_command_for_tmux(&ps.agent),
        };
        if let Err(e) = tmux::new_session(name, &ps.dir, &command, &ps.agent) {
            eprintln!("am: cold-start restore '{name}': {e}");
        }
    }
    app.refresh();
}

/// First column of the right (preview) pane: 2-col left margin + left-pane
/// percentage of the body width + 1-col separator. Mirrors the body layout in
/// ui::draw_body. Widened to u32 so wide terminals can't overflow u16.
fn preview_boundary_col(screen_width: u16, split_pct: u16) -> u16 {
    let area_w = screen_width.saturating_sub(4) as u32;
    (2 + area_w * split_pct as u32 / 100 + 1) as u16
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

    #[test]
    fn preview_boundary_matches_layout_split() {
        // 120-col screen, 40% split: body=116, left=46 → boundary at 2+46+1.
        assert_eq!(preview_boundary_col(120, 40), 49);
        // Narrow floor.
        assert_eq!(preview_boundary_col(10, 40), 2 + 2 + 1);
    }

    #[test]
    fn preview_boundary_survives_wide_terminals() {
        // u16 math would overflow at width ≥ ~878 (panic in debug builds).
        assert_eq!(
            preview_boundary_col(2000, 75),
            (2u32 + 1996 * 75 / 100 + 1) as u16
        );
        assert_eq!(
            preview_boundary_col(u16::MAX, 75),
            (2u32 + 65531 * 75 / 100 + 1) as u16
        );
    }

    #[test]
    fn poll_timeout_animates_while_running() {
        assert_eq!(
            poll_timeout(
                true,
                false,
                Duration::from_millis(10),
                Duration::from_millis(1500)
            ),
            Duration::from_millis(80),
            "a running session animates the spinner at 80ms"
        );
    }

    #[test]
    fn poll_timeout_idle_sleeps_until_next_refresh() {
        // 500ms into a 1500ms cycle → ~1000ms remaining.
        assert_eq!(
            poll_timeout(
                false,
                false,
                Duration::from_millis(500),
                Duration::from_millis(1500)
            ),
            Duration::from_millis(1000)
        );
    }

    #[test]
    fn poll_timeout_idle_does_not_wake_at_spinner_rate() {
        // The key idle property: wait far longer than the 80ms spinner tick.
        let t = poll_timeout(false, false, Duration::ZERO, Duration::from_millis(1500));
        assert!(
            t > Duration::from_millis(80),
            "idle must not wake at 80ms, got {t:?}"
        );
    }

    #[test]
    fn poll_timeout_floors_when_refresh_overdue() {
        // Overdue refresh must not yield a 0ms timeout (busy spin).
        let t = poll_timeout(
            false,
            false,
            Duration::from_millis(2000),
            Duration::from_millis(1500),
        );
        assert_eq!(t, Duration::from_millis(1), "floored to 1ms, got {t:?}");
    }

    #[test]
    fn poll_timeout_tmux_missing_waits_long() {
        let t = poll_timeout(true, true, Duration::ZERO, Duration::from_millis(1500));
        assert!(
            t >= Duration::from_secs(1),
            "static error screen waits long, got {t:?}"
        );
    }
}
