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
use std::time::Duration;

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> io::Result<()> {
    if !tmux::is_available() {
        eprintln!("error: `tmux` not found in PATH. Install tmux and try again.");
        std::process::exit(1);
    }

    let config = Config::load();
    let interval = Duration::from_millis(config.refresh_interval_ms.max(100));
    let mut app = App::new(config);
    app.refresh();

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, &mut app, interval);
    restore_terminal(&mut terminal)?;
    result
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

fn run(terminal: &mut Term, app: &mut App, interval: Duration) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(interval)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Ctrl-C always quits (raw mode suppresses SIGINT).
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if let Some(action) = app.handle_key(key) {
                    handle_action(terminal, app, action)?;
                }
            }
        } else {
            app.refresh();
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
            let _ = tmux::attach_session(&name);
            enable_raw_mode()?;
            execute!(terminal.backend_mut(), EnterAlternateScreen)?;
            terminal.clear()?;
            app.error = None;
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
