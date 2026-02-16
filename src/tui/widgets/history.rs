//! Panel 2: Scrollable execution history table.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(" [2] Execution History ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if app.history.is_empty() {
        let content = Paragraph::new(Line::from(vec![Span::styled(
            "  No execution history",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(block);
        f.render_widget(content, area);
        return;
    }

    let mut lines = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  {:<10} {:<18} {:<10} {:<8} {:<10}",
            "ID", "WORKFLOW", "STATUS", "TRIGGER", "DURATION"
        ),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));

    let inner_height = area.height.saturating_sub(3) as usize; // border + header
    let visible_start = app.history_scroll;
    let visible_end = (visible_start + inner_height).min(app.history.len());

    for (i, exec) in app
        .history
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_end - visible_start)
    {
        let is_selected = app.history_selected == Some(i);

        let status_color = match exec.status.as_str() {
            "completed" => Color::Green,
            "failed" => Color::Red,
            "running" => Color::Yellow,
            "pending" => Color::DarkGray,
            "cancelled" => Color::Magenta,
            _ => Color::White,
        };

        let status_icon = match exec.status.as_str() {
            "completed" => "\u{2713}",
            "failed" => "\u{2717}",
            "running" => "\u{2022}",
            _ => "\u{25CB}",
        };

        let duration = exec
            .duration_ms
            .map(|ms| format!("{}ms", ms))
            .unwrap_or_else(|| "-".to_string());

        let id_short = if exec.id.len() > 8 {
            &exec.id[..8]
        } else {
            &exec.id
        };

        let wf_name = if exec.workflow_name.len() > 16 {
            format!("{}...", &exec.workflow_name[..13])
        } else {
            exec.workflow_name.clone()
        };

        let bg_style = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled("  ", bg_style),
            Span::styled(
                format!("{:<10} ", id_short),
                bg_style.fg(Color::Blue),
            ),
            Span::styled(format!("{:<18} ", wf_name), bg_style.fg(Color::White)),
            Span::styled(
                format!("{}{:<9} ", status_icon, exec.status),
                bg_style.fg(status_color),
            ),
            Span::styled(
                format!("{:<8} ", exec.trigger_type),
                bg_style.fg(Color::DarkGray),
            ),
            Span::styled(format!("{:<10}", duration), bg_style.fg(Color::White)),
        ]));
    }

    let content = Paragraph::new(lines).block(block);
    f.render_widget(content, area);
}
