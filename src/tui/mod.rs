//! Interactive terminal UI.
//!
//! slskd 0.22 exposes SignalR hubs for search and logs, but not for transfers,
//! so this polls instead. At a two-second interval the difference is not
//! perceptible, and it keeps the client to plain HTTP.

mod app;
mod ui;

use crate::client::Client;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, time::Duration};

pub use app::{App, Mode, Tab};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

/// Put the terminal back before a panic reaches the default handler.
///
/// Without this, any panic leaves the shell in raw mode on the alternate
/// screen: no echo, no line editing, and the backtrace invisible. That is
/// indistinguishable from the program "crashing the terminal", and the only way
/// out is to blind-type `reset`.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default(info);
    }));
}

pub async fn run(api: Client) -> Result<()> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;

    let res = event_loop(&mut term, api).await;

    restore_terminal();
    let _ = term.show_cursor();
    res
}

async fn event_loop<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    api: Client,
) -> Result<()> {
    let mut app = App::new(api);
    app.refresh_status().await;
    app.refresh_transfers();

    let mut last_poll = std::time::Instant::now();

    loop {
        // Pick up anything that finished in the background, then draw. Neither
        // of these blocks, so the interface stays live while a search runs.
        app.poll_tasks().await;
        term.draw(|f| ui::draw(f, &mut app))?;

        if last_poll.elapsed() >= Duration::from_secs(2) {
            app.refresh_transfers();
            last_poll = std::time::Instant::now();
        }

        // A short tick keeps the spinner moving while a search is in flight.
        let tick = if app.searching() { 100 } else { 250 };
        if !event::poll(Duration::from_millis(tick))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }

        // Text entry swallows most keys, so handle it first.
        if app.mode == Mode::Editing {
            match k.code {
                KeyCode::Esc => app.mode = Mode::Normal,
                KeyCode::Enter => {
                    app.mode = Mode::Normal;
                    app.start_search();
                }
                KeyCode::Backspace => {
                    app.query.pop();
                }
                KeyCode::Char(c) => app.query.push(c),
                _ => {}
            }
            continue;
        }

        match (k.code, k.modifiers) {
            (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
            (KeyCode::Esc, _) => app.cancel_search(),
            (KeyCode::Char('1'), _) => app.tab = Tab::Search,
            (KeyCode::Char('2'), _) => app.tab = Tab::Transfers,
            (KeyCode::Char('3'), _) => app.tab = Tab::Browse,
            (KeyCode::Tab, _) => app.next_tab(),
            (KeyCode::Char('/'), _) => {
                app.mode = Mode::Editing;
                app.query.clear();
            }
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => app.move_selection(1),
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => app.move_selection(-1),
            (KeyCode::Char('g'), _) | (KeyCode::Home, _) => app.select_first(),
            (KeyCode::Char('G'), _) | (KeyCode::End, _) => app.select_last(),
            (KeyCode::Enter, _) => app.activate().await,
            (KeyCode::Char('d'), _) => app.enqueue_selected().await,
            (KeyCode::Char('b'), _) => app.browse_selected().await,
            (KeyCode::Char('c'), _) => app.clear_completed().await,
            (KeyCode::Char('r'), _) => {
                app.refresh_status().await;
                app.refresh_transfers();
            }
            (KeyCode::Char('a'), _) => app.toggle_any_format(),
            (KeyCode::Char('v'), _) => app.toggle_variants(),
            _ => {}
        }
    }
}
