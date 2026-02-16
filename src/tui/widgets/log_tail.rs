//! Panel 4: Streaming event log with follow mode.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::event::LogLevel;

pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let follow_indicator = if app.log_follow { " [follow]" } else { "" };
    let title = format!(" [4] Event Log{} ", follow_indicator);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if app.log_lines.is_empty() {
        let content = Paragraph::new(Line::from(vec![Span::styled(
            "  Waiting for events...",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(block);
        f.render_widget(content, area);
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let total_lines = app.log_lines.len();

    // Calculate scroll position
    let scroll_offset = if app.log_follow {
        total_lines.saturating_sub(inner_height)
    } else {
        app.log_scroll.min(total_lines.saturating_sub(inner_height))
    };

    let mut lines = Vec::new();
    for log_line in app
        .log_lines
        .iter()
        .skip(scroll_offset)
        .take(inner_height)
    {
        let time_str = log_line.timestamp.format("%H:%M:%S").to_string();

        let (level_str, level_color) = match log_line.level {
            LogLevel::Info => ("INFO", Color::Blue),
            LogLevel::Warn => ("WARN", Color::Yellow),
            LogLevel::Error => ("ERR ", Color::Red),
        };

        lines.push(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("{} ", time_str),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{} ", level_str),
                Style::default()
                    .fg(level_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&log_line.message, Style::default().fg(Color::White)),
        ]));
    }

    let content = Paragraph::new(lines).block(block);
    f.render_widget(content, area);
}
