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

    let title = format!(
        " query ({}{}) ",
        if app.any_format { "any format" } else { "mp3" },
        if app.allow_variants { ", +variants" } else { "" }
    );
    f.render_widget(
        Paragraph::new(Span::styled(text, style)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if app.mode == Mode::Editing { ACCENT } else { DIM }))
                .title(title),
        ),
        area,
    );
}

fn search_tab(f: &mut Frame, area: Rect, app: &mut App) {
    if app.results.is_empty() {
        let msg = if app.searching {
            "searching…"
        } else {
            "no results yet — press / and type a query, then Enter"
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(DIM))
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
            Row::new(vec![
                Cell::from(fmt::truncate(&c.username, 16)),
                Cell::from(slot).style(slot_style),
                Cell::from(c.queue_length.to_string()),
                Cell::from(fmt::speed(c.upload_speed as f64)),
                Cell::from(fmt::bitrate(c.file.bit_rate)),
                Cell::from(fmt::duration(c.file.length)),
                Cell::from(fmt::bytes(c.file.size)),
                Cell::from(fmt::truncate(c.name(), 44)),
                Cell::from(fmt::truncate(c.album(), 28)).style(Style::default().fg(DIM)),
            ])
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(app.search_selected));

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(28),
        ],
    )
    .header(
        Row::new(vec!["user", "slot", "queue", "speed", "rate", "len", "size", "file", "folder"])
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
            .title(format!(" transfers ({}) — c to clear completed ", app.transfers.len())),
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
            " {} — {} directories ",
            app.browse_user,
            app.browse_dirs.len()
        )));

    f.render_stateful_widget(table, area, &mut state);
}

fn status_bar(f: &mut Frame, area: Rect, app: &App) {
    let keys = " q quit · / search · jk move · d queue · b browse · c clear · a format · v variants";
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
