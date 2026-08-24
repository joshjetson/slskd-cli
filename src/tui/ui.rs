//! Rendering. Kept free of side effects so the layout can be reasoned about
//! independently of the event loop.

use super::app::{App, Mode, Tab};
use crate::format as fmt;
use crate::models::basename;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, TableState},
    Frame,
};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(3), // query / summary
            Constraint::Min(5),    // body
            Constraint::Length(1), // status
        ])
        .split(f.area());

    header(f, chunks[0], app);
    query_bar(f, chunks[1], app);
    match app.tab {
        Tab::Search => search_tab(f, chunks[2], app),
        Tab::Transfers => transfers_tab(f, chunks[2], app),
        Tab::Browse => browse_tab(f, chunks[2], app),
    }
    status_bar(f, chunks[3], app);
}

fn header(f: &mut Frame, area: Rect, app: &App) {
    let tabs = [
        (Tab::Search, "1 search"),
        (Tab::Transfers, "2 transfers"),
        (Tab::Browse, "3 browse"),
    ];
    let mut spans = vec![Span::styled(
        " slsk ",
        Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    for (tab, label) in tabs {
        let style = if app.tab == tab {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(label, style));
    }

    if let Some(s) = &app.app_state {
        let conn = if s.server.is_logged_in { "online" } else { "offline" };
        let color = if s.server.is_logged_in { Color::Green } else { Color::Red };
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("{} · {} · {} shared", s.user.username, conn, s.shares.files),
            Style::default().fg(color),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn query_bar(f: &mut Frame, area: Rect, app: &App) {
    let (text, style) = match app.mode {
        Mode::Editing => (
            format!("{}█", app.query),
            Style::default().fg(Color::White),
        ),
        Mode::Normal if app.query.is_empty() => (
            "press / to search".to_string(),
            Style::default().fg(DIM),
        ),
        Mode::Normal => (app.query.clone(), Style::default().fg(Color::White)),
    };

    let title = match app.search_elapsed() {
        Some(e) => format!(" query — {} searching {}s ", spinner_frame(e), e.as_secs()),
        None => format!(
            " query ({}{}) ",
            if app.any_format { "any format" } else { "mp3" },
            if app.allow_variants { ", +variants" } else { "" }
        ),
    };
    f.render_widget(
        Paragraph::new(Span::styled(text, style)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(
                    if app.mode == Mode::Editing || app.searching() { ACCENT } else { DIM },
                ))
                .title(title),
        ),
        area,
    );
}

/// Frames for the in-flight indicator. A Soulseek search takes roughly 35
/// seconds; without visible motion the app reads as hung.
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

fn spinner_frame(elapsed: std::time::Duration) -> &'static str {
    SPINNER[(elapsed.as_millis() / 120) as usize % SPINNER.len()]
}

/// One-glance state of a search result: has it been queued, how far along is
/// it, is it already on disk. Without this the only way to tell whether `d`
/// did anything was to leave the tab.
fn result_status(app: &App, c: &crate::models::Candidate) -> (String, Style) {
    if let Some(t) = app.transfer_for(c) {
        return if t.succeeded() {
            ("done".into(), Style::default().fg(Color::Green))
        } else if t.errored() {
            ("failed".into(), Style::default().fg(Color::Red))
        } else if t.state == "InProgress" {
            (
                format!("{:.0}%", t.percent_complete),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )
        } else {
            ("queued".into(), Style::default().fg(Color::Yellow))
        };
    }
    if app.already_have(c) {
        // Same filename already came down from a different peer.
        return ("have".into(), Style::default().fg(Color::Green).add_modifier(Modifier::DIM));
    }
    (String::new(), Style::default())
}

fn search_tab(f: &mut Frame, area: Rect, app: &mut App) {
    if app.results.is_empty() {
        let msg = match app.search_elapsed() {
            Some(e) => format!(
                "{} searching… {}s elapsed\n\n   Soulseek searches take about 35 seconds.\n                    The interface stays live — Esc cancels, q quits.",
                spinner_frame(e),
                e.as_secs()
            ),
            None => "no results yet — press / and type a query, then Enter".to_string(),
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(if app.searching() { ACCENT } else { DIM }))
                .block(Block::default().borders(Borders::ALL).title(" results ")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = app
        .results
        .iter()
        .map(|c| {
            let slot = if c.has_free_slot { "yes" } else { "no" };
            let slot_style = if c.has_free_slot {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            let (st, st_style) = result_status(app, c);
            Row::new(vec![
                Cell::from(st).style(st_style),
                Cell::from(fmt::truncate(&c.username, 16)),
                Cell::from(slot).style(slot_style),
                Cell::from(c.queue_length.to_string()),
                Cell::from(fmt::speed(c.upload_speed as f64)),
                Cell::from(fmt::bitrate(c.file.bit_rate)),
                Cell::from(fmt::duration(c.file.length)),
                Cell::from(fmt::bytes(c.file.size)),
                Cell::from(fmt::truncate(c.name(), 44)),
                Cell::from(fmt::truncate(c.album(), 18)).style(Style::default().fg(DIM)),
            ])
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(app.search_selected));

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(16),
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(24),
            Constraint::Length(18),
        ],
    )
    .header(
        Row::new(vec!["dl", "user", "slot", "queue", "speed", "rate", "len", "size", "file", "folder"])
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" results ({}) — d to queue, b to browse peer ", app.results.len())),
    );

    f.render_stateful_widget(table, area, &mut state);
}

fn transfers_tab(f: &mut Frame, area: Rect, app: &mut App) {
    if app.transfers.is_empty() {
        f.render_widget(
            Paragraph::new("no transfers")
                .style(Style::default().fg(DIM))
                .block(Block::default().borders(Borders::ALL).title(" transfers ")),
            area,
        );
        return;
    }

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let rows: Vec<Row> = app
        .transfers
        .iter()
        .map(|t| {
            let style = if t.succeeded() {
                Style::default().fg(Color::Green)
            } else if t.errored() {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Yellow)
            };
            Row::new(vec![
                Cell::from(fmt::truncate(&t.username, 16)),
                Cell::from(fmt::state(&t.state).to_string()).style(style),
                Cell::from(format!("{:.0}%", t.percent_complete)),
                Cell::from(fmt::speed(t.average_speed)),
                Cell::from(fmt::bytes(t.size)),
                Cell::from(fmt::truncate(t.name(), 44)),
            ])
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(app.transfer_selected));

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec!["user", "state", "pct", "speed", "size", "file"])
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " transfers ({}) — newest first · c clear finished · x remove selected ",
                app.transfers.len()
            )),
    );

    f.render_stateful_widget(table, split[0], &mut state);

    if let Some(t) = app.transfers.get(app.transfer_selected) {
        let pct = (t.percent_complete / 100.0).clamp(0.0, 1.0);
        let label = match &t.exception {
            Some(e) if t.errored() => fmt::truncate(e, 60),
            _ => format!("{} — {:.1}%", basename(&t.filename), t.percent_complete),
        };
        f.render_widget(
            Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(Style::default().fg(if t.errored() { Color::Red } else { ACCENT }))
                .ratio(pct)
                .label(label),
            split[1],
        );
    }
}

fn browse_tab(f: &mut Frame, area: Rect, app: &mut App) {
    if app.browse_dirs.is_empty() {
        f.render_widget(
            Paragraph::new("select a peer on the search or transfers tab and press b")
                .style(Style::default().fg(DIM))
                .block(Block::default().borders(Borders::ALL).title(" browse ")),
            area,
        );
        return;
    }

    let rows: Vec<Row> = app
        .browse_dirs
        .iter()
        .map(|d| {
            Row::new(vec![
                Cell::from(d.file_count.to_string()),
                Cell::from(fmt::truncate(&d.name, 100)),
            ])
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(app.browse_selected));

    let table = Table::new(rows, [Constraint::Length(7), Constraint::Min(20)])
        .header(
            Row::new(vec!["files", "directory"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title(format!(
            " {} — {} directories · c clear ",
            app.browse_user,
            app.browse_dirs.len()
        )));

    f.render_stateful_widget(table, area, &mut state);
}

fn status_bar(f: &mut Frame, area: Rect, app: &App) {
    // Per-tab, because a single global list either omits half the keys or is
    // too long to read. Dropping `c clear` from a shared list is exactly how it
    // became invisible.
    let keys = match app.tab {
        Tab::Search if app.searching() => " esc cancel · q quit",
        Tab::Search => " / search · d queue · b browse peer · a format · v variants · q quit",
        Tab::Transfers => " c clear finished · x remove · b browse peer · r refresh · q quit",
        Tab::Browse => " c clear list · jk move · r refresh · q quit",
    };
    let line = Line::from(vec![
        Span::styled(
            fmt::truncate(&app.status, area.width.saturating_sub(2) as usize),
            Style::default().fg(Color::White),
        ),
    ]);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    f.render_widget(Paragraph::new(line), chunks[0]);
    f.render_widget(
        Paragraph::new(Span::styled(keys, Style::default().fg(DIM))),
        chunks[1],
    );
}

#[cfg(test)]
pub(crate) mod tests_support {
    pub use super::tests::*;
}

#[cfg(test)]
mod tests {
    pub use super::*;
    use crate::client::Client;
    use crate::models::{BrowseDirectory, Candidate, File, Transfer};
    use crate::tui::app::App;
    use ratatui::{backend::TestBackend, Terminal};

    pub fn app() -> App {
        App::new(Client::new(reqwest::Client::new(), "http://localhost:5030", "k"))
    }

    pub fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn candidate(name: &str, user: &str) -> Candidate {
        Candidate {
            username: user.into(),
            has_free_slot: true,
            queue_length: 0,
            upload_speed: 8_000_000,
            file: File {
                filename: format!("@@peer\\Music\\Toto IV\\{name}"),
                size: 8_000_000,
                bit_rate: Some(320),
                length: Some(295),
                is_locked: false,
            },
        }
    }

    pub fn populated() -> App {
        let mut a = app();
        a.results = vec![candidate("01 Africa.mp3", "peer_one")];
        a.transfers = vec![Transfer {
            id: "1".into(), username: "peer_two".into(),
            filename: "@@peer\\Music\\Aja\\Peg.mp3".into(),
            state: "InProgress".into(), size: 8_000_000, bytes_transferred: 4_000_000,
            percent_complete: 50.0, average_speed: 1_048_576.0, exception: None,
            requested_at: Some("2026-08-24T05:10:40".into()), enqueued_at: None,
        }];
        a.browse_user = "peer_three".into();
        a.browse_dirs = vec![BrowseDirectory { name: "Classic Rock".into(), file_count: 57, files: vec![] }];
        a
    }

    #[test]
    fn renders_empty_state_without_panicking() {
        let mut a = app();
        let out = render(&mut a, 100, 24);
        assert!(out.contains("slsk"));
        assert!(out.contains("no results yet"));
    }

    #[test]
    fn renders_search_results() {
        let mut a = app();
        a.results = vec![candidate("01 Africa.mp3", "peer_one")];
        let out = render(&mut a, 120, 24);
        assert!(out.contains("peer_one"), "user column missing:\n{out}");
        assert!(out.contains("Africa"), "filename missing:\n{out}");
        assert!(out.contains("Toto IV"), "folder column missing:\n{out}");
    }

    fn xfer(user: &str, path: &str, state: &str, pct: f64) -> Transfer {
        Transfer {
            id: "1".into(),
            username: user.into(),
            filename: path.into(),
            state: state.into(),
            size: 8_000_000,
            bytes_transferred: 4_000_000,
            percent_complete: pct,
            average_speed: 1_048_576.0,
            exception: None,
            requested_at: Some("2026-08-24T05:10:40".into()),
            enqueued_at: None,
        }
    }

    #[test]
    fn search_rows_show_download_state_without_leaving_the_tab() {
        let mut a = app();
        let c = candidate("01 Africa.mp3", "peer_one");
        let path = c.file.filename.clone();
        a.results = vec![c];

        // nothing queued yet -> no marker
        assert!(!render(&mut a, 130, 12).contains("queued"));

        a.transfers = vec![xfer("peer_one", &path, "InProgress", 42.0)];
        a.reindex_for_test();
        assert!(render(&mut a, 130, 12).contains("42%"), "in-flight percent must show on the search tab");

        a.transfers = vec![xfer("peer_one", &path, "Completed, Succeeded", 100.0)];
        a.reindex_for_test();
        assert!(render(&mut a, 130, 12).contains("done"));

        a.transfers = vec![xfer("peer_one", &path, "Completed, Errored", 0.0)];
        a.reindex_for_test();
        assert!(render(&mut a, 130, 12).contains("failed"));
    }

    #[test]
    fn a_file_already_fetched_from_another_peer_is_marked_have() {
        // Guards against queueing the same song twice from different peers.
        let mut a = app();
        a.results = vec![candidate("01 Africa.mp3", "peer_two")];
        a.transfers = vec![xfer(
            "someone_else",
            "@@other\\Music\\Elsewhere\\01 Africa.mp3",
            "Completed, Succeeded",
            100.0,
        )];
        a.reindex_for_test();
        assert!(render(&mut a, 130, 12).contains("have"));
    }

    #[test]
    fn renders_transfers_with_a_progress_gauge() {
        let mut a = app();
        a.tab = Tab::Transfers;
        a.transfers = vec![Transfer {
            id: "1".into(),
            username: "peer_two".into(),
            filename: "@@peer\\Music\\Aja\\Peg.mp3".into(),
            state: "InProgress".into(),
            size: 8_000_000,
            bytes_transferred: 4_000_000,
            percent_complete: 50.0,
            average_speed: 1_048_576.0,
            exception: None,
            requested_at: Some("2026-08-24T05:10:40".into()),
            enqueued_at: None,
        }];
        let out = render(&mut a, 100, 24);
        assert!(out.contains("peer_two"));
        assert!(out.contains("downloading"));
    }

    #[test]
    fn renders_browse_listing() {
        let mut a = app();
        a.tab = Tab::Browse;
        a.browse_user = "peer_three".into();
        a.browse_dirs = vec![BrowseDirectory {
            name: "Classic Rock".into(),
            file_count: 57,
            files: vec![],
        }];
        let out = render(&mut a, 100, 24);
        assert!(out.contains("peer_three"));
        assert!(out.contains("Classic Rock"));
    }

    #[test]
    fn survives_an_absurdly_narrow_terminal() {
        // Layout maths that assumes width is a soft constraint panics here.
        let mut a = app();
        a.results = vec![candidate("01 Africa.mp3", "peer_one")];
        for w in [1u16, 5, 12, 30] {
            let _ = render(&mut a, w, 10);
        }
        for h in [4u16, 5, 6] {
            let _ = render(&mut a, 80, h);
        }
    }
}

#[cfg(test)]
mod size_tests {
    use super::tests_support::*;

    #[test]
    fn every_tab_survives_a_tiny_terminal() {
        for tab in [Tab::Search, Tab::Transfers, Tab::Browse] {
            let mut a = populated();
            a.tab = tab;
            for (w, h) in [(1u16, 1u16), (2, 3), (10, 4), (40, 5), (200, 60)] {
                let _ = render(&mut a, w, h);
            }
        }
    }
}
