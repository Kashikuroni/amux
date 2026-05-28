use cm::app::{Action, App};
use cm::config::Config;
use cm::{tmux, ui};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, stdout, Stdout};
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
    if !tmux::is_available() {
        app.tmux_missing = true;
    } else {
        app.refresh();
    }

    install_panic_hook();
    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, refresh);
    let restore = restore_terminal(&mut terminal);
    result.and(restore)?;
    if app.tmux_missing {
        std::process::exit(1);
    }
    Ok(())
}

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

fn restore_terminal(terminal: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Term, app: &mut App, refresh: Duration) -> io::Result<()> {
    let start = Instant::now();
    let tick = Duration::from_millis(80);
    let mut last_refresh = Instant::now();
    loop {
        app.spinner_frame = cm::spinner::frame_index(start.elapsed().as_millis());
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
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
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
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
    }
    Ok(())
}
