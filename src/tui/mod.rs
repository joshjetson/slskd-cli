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

pub async fn run(api: Client) -> Result<()> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;

    let res = event_loop(&mut term, api).await;

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    res
}

async fn event_loop<B: ratatui::backend::Backend>(
    term: &mut Terminal<B>,
    api: Client,
) -> Result<()> {
    let mut app = App::new(api);
    app.refresh_status().await;
    app.refresh_transfers().await;

    let mut last_poll = std::time::Instant::now();

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        // Poll transfers on a timer, but stay responsive to keys in between.
        if last_poll.elapsed() >= Duration::from_secs(2) {
            app.refresh_transfers().await;
            last_poll = std::time::Instant::now();
        }

        if !event::poll(Duration::from_millis(120))? {
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
                    app.run_search().await;
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
                app.refresh_transfers().await;
            }
            (KeyCode::Char('a'), _) => app.toggle_any_format(),
            (KeyCode::Char('v'), _) => app.toggle_variants(),
            _ => {}
        }
    }
}
