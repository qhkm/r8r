//! Panel 1: Live node execution table with spinners.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::{App, NodeStatus};

pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(" [1] Live Executions ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if app.live_executions.is_empty() {
        let content = Paragraph::new(Line::from(vec![Span::styled(
            "  No active executions",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(block);
        f.render_widget(content, area);
        return;
    }

    let mut lines = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<20} {:<8} {}", "NODE", "TYPE", "STATUS"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    for exec in app.live_executions.iter() {
        // Execution header
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} ", &exec.execution_id[..8.min(exec.execution_id.len())]),
                Style::default().fg(Color::Blue),
            ),
            Span::styled(
                &exec.workflow_name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({})", exec.trigger_type),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        for node in &exec.nodes {
            let (icon, status_text, color) = match &node.status {
                NodeStatus::Running { started_at } => {
                    let elapsed = (chrono::Utc::now() - *started_at).num_milliseconds();
                    (
                        app.spinner_frame(),
                        format!("{}ms", elapsed),
                        Color::Yellow,
                    )
                }
                NodeStatus::Completed { duration_ms } => {
                    ("\u{2713}", format!("{}ms", duration_ms), Color::Green)
                }
                NodeStatus::Failed { error } => {
                    let short_err = if error.len() > 30 {
                        format!("{}...", &error[..27])
                    } else {
                        error.clone()
                    };
                    ("\u{2717}", short_err, Color::Red)
                }
            };

            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    format!("{:<20} ", node.id),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:<8} ", node.node_type),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::styled(status_text, Style::default().fg(color)),
            ]));
        }
    }

    let content = Paragraph::new(lines)
        .block(block)
        .scroll((app.live_scroll as u16, 0));
    f.render_widget(content, area);
}
