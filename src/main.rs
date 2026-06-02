use am::app::{Action, App};
use am::config::Config;
use am::state::State;
use am::{tmux, ui};

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
        let _ = execute!(stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

fn main() -> io::Result<()> {
    let config = Config::load();
    let refresh = Duration::from_millis(config.refresh_interval_ms.max(100));
    let mut app = App::new(config);
    app.apply_state(State::load());
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

/// Spawns a daemon thread that fetches account state (usage + plan) now and
/// every 60s, sending each snapshot down a channel. Failed fields arrive as
/// `None` so the loop can keep the last good value.
fn spawn_usage_poller() -> mpsc::Receiver<am::usage::Account> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || loop {
        if tx.send(am::usage::fetch_account()).is_err() {
            break; // receiver dropped → app is shutting down
        }
        thread::sleep(Duration::from_secs(60));
    });
    rx
}

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    enable_key_disambiguation(&mut out);
    Terminal::new(CrosstermBackend::new(out))
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
                EnableBracketedPaste
            )?;
            enable_key_disambiguation(terminal.backend_mut());
            terminal.clear()?;
            app.refresh();
        }
        Action::Create { name, dir, agent } => {
            if let Err(e) = tmux::new_session(&name, &dir, &agent) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::Kill(name) => {
            if let Err(e) = tmux::kill_session(&name) {
                app.error = Some(e.to_string());
            }
            app.refresh();
        }
        Action::Rename { old, new } => {
            if let Err(e) = tmux::rename_session(&old, &new) {
                app.error = Some(e.to_string());
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
    }
    Ok(())
}
