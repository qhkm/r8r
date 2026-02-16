//! Panel 3: ASCII workflow DAG renderer.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::{App, DagNode, NodeStatus};

pub fn render(f: &mut Frame, area: Rect, app: &App, is_active: bool) {
    let border_color = if is_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title = match &app.dag_workflow_name {
        Some(name) => format!(" [3] Workflow DAG: {} ", name),
        None => " [3] Workflow DAG ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if app.dag_nodes.is_empty() {
        let content = Paragraph::new(Line::from(vec![Span::styled(
            "  No workflow loaded",
            Style::default().fg(Color::DarkGray),
        )]))
        .block(block);
        f.render_widget(content, area);
        return;
    }

    let lines = render_dag_lines(&app.dag_nodes, app);

    let content = Paragraph::new(lines)
        .block(block)
        .scroll((app.dag_scroll as u16, 0));
    f.render_widget(content, area);
}

fn render_dag_lines<'a>(nodes: &'a [DagNode], app: &App) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    // Group nodes by depth
    let max_depth = nodes.iter().map(|n| n.depth).max().unwrap_or(0);

    for depth in 0..=max_depth {
        let nodes_at_depth: Vec<&DagNode> = nodes.iter().filter(|n| n.depth == depth).collect();

        // Draw connector from previous level
        if depth > 0 {
            // Find which nodes at this depth have dependencies on previous levels
            let has_connections = !nodes_at_depth.is_empty();
            if has_connections {
                lines.push(Line::from(vec![Span::styled(
                    "      \u{2502}",
                    Style::default().fg(Color::DarkGray),
                )]));
            }
        }

        for (i, node) in nodes_at_depth.iter().enumerate() {
            let (status_icon, status_color) = match &node.status {
                None => ("\u{25CB}", Color::DarkGray),
                Some(NodeStatus::Running { .. }) => (app.spinner_frame(), Color::Yellow),
                Some(NodeStatus::Completed { .. }) => ("\u{2713}", Color::Green),
                Some(NodeStatus::Failed { .. }) => ("\u{2717}", Color::Red),
            };

            let status_detail = match &node.status {
                Some(NodeStatus::Completed { duration_ms }) => format!(" {}ms", duration_ms),
                Some(NodeStatus::Running { started_at }) => {
                    let elapsed = (chrono::Utc::now() - *started_at).num_milliseconds();
                    format!(" {}ms", elapsed)
                }
                _ => String::new(),
            };

            // Draw connector
            let connector = if i == 0 && depth > 0 {
                "\u{250C}\u{2500}"
            } else if i > 0 {
                "\u{251C}\u{2500}"
            } else {
                "  "
            };

            // Node box
            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled(connector, Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("\u{250C}{}\u{2510}", "\u{2500}".repeat(node.id.len() + 2)),
                    Style::default().fg(Color::White),
                ),
            ]));

            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("  ", Style::default()),
                Span::styled("\u{2502} ", Style::default().fg(Color::White)),
                Span::styled(
                    &node.id,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" \u{2502}", Style::default().fg(Color::White)),
                Span::styled(
                    format!(" {} {}", status_icon, node.node_type),
                    Style::default().fg(status_color),
                ),
                Span::styled(status_detail, Style::default().fg(status_color)),
            ]));

            lines.push(Line::from(vec![
                Span::styled("   ", Style::default()),
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("\u{2514}{}\u{2518}", "\u{2500}".repeat(node.id.len() + 2)),
                    Style::default().fg(Color::White),
                ),
            ]));
        }
    }

    lines
}
