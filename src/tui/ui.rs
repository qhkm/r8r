//! TUI layout: 4-panel split with status bar.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::app::{ActivePanel, App, ConnectionState};
use super::widgets;

/// Render the complete TUI layout.
pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    // Main split: content area + status bar (1 line)
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // content
            Constraint::Length(1), // status bar
        ])
        .split(size);

    let content_area = main_chunks[0];
    let status_area = main_chunks[1];

    // Content: top half + bottom half
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // top
            Constraint::Percentage(50), // bottom
        ])
        .split(content_area);

    // Top: live view (55%) + DAG (45%)
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(vertical_chunks[0]);

    // Bottom: history (55%) + log tail (45%)
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(vertical_chunks[1]);

    // Render panels
    widgets::live_view::render(
        f,
        top_chunks[0],
        app,
        app.active_panel == ActivePanel::LiveView,
    );
    widgets::dag_view::render(
        f,
        top_chunks[1],
        app,
        app.active_panel == ActivePanel::DagView,
    );
    widgets::history::render(
        f,
        bottom_chunks[0],
        app,
        app.active_panel == ActivePanel::History,
    );
    widgets::log_tail::render(
        f,
        bottom_chunks[1],
        app,
        app.active_panel == ActivePanel::LogTail,
    );

    // Status bar
    render_status_bar(f, status_area, app);
}

fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let (conn_icon, conn_color) = match app.connection_state {
        ConnectionState::Connected => ("\u{25CF}", Color::Green),
        ConnectionState::Connecting => ("\u{25CB}", Color::Yellow),
        ConnectionState::Disconnected => ("\u{25CF}", Color::Red),
    };

    let conn_label = match app.connection_state {
        ConnectionState::Connected => "Connected",
        ConnectionState::Connecting => "Connecting...",
        ConnectionState::Disconnected => "Disconnected",
    };

    // Build WS URL display from server URL
    let ws_display = app.server_url.replacen("http", "ws", 1);

    let status_line = Line::from(vec![
        Span::styled(format!(" {} ", conn_icon), Style::default().fg(conn_color)),
        Span::styled(
            format!("{} {} ", conn_label, ws_display),
            Style::default().fg(Color::White),
        ),
        Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Tab",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(": switch ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "j/k",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(": scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Enter",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(": select ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "q",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(": quit", Style::default().fg(Color::DarkGray)),
    ]);

    let status = Paragraph::new(status_line).style(Style::default().bg(Color::Black));
    f.render_widget(status, area);
}
