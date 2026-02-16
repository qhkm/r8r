//! TUI monitor for live r8r server monitoring.
//!
//! Provides a ratatui-based terminal UI that connects to a running r8r server
//! via WebSocket and REST to display live execution state.

mod app;
mod event;
mod rest_client;
mod ui;
mod widgets;
mod ws_client;

use std::io;

use crossterm::{
    event::{self as crossterm_event, Event as CrosstermEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use app::App;
use event::TuiEvent;

/// Run the TUI monitor, connecting to the given server URL.
pub async fn run_monitor(base_url: &str, token: Option<&str>) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let result = run_app(&mut terminal, base_url, token).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    base_url: &str,
    token: Option<&str>,
) -> anyhow::Result<()> {
    let mut app = App::new(base_url.to_string());

    let (tx, mut rx) = mpsc::channel::<TuiEvent>(256);

    // 1. Terminal event reader (blocking crossterm reads on a dedicated thread)
    let term_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            match crossterm_event::read() {
                Ok(CrosstermEvent::Key(key)) => {
                    if term_tx.blocking_send(TuiEvent::Key(key)).is_err() {
                        break;
                    }
                }
                Ok(_) => {} // Ignore mouse/resize events
                Err(_) => break,
            }
        }
    });

    // 2. WebSocket client
    let ws_tx = tx.clone();
    let ws_url = base_url.to_string();
    let ws_token = token.map(|t| t.to_string());
    tokio::spawn(async move {
        ws_client::connect(&ws_url, ws_token.as_deref(), ws_tx).await;
    });

    // 3. Tick timer (250ms for spinner animation)
    let tick_tx = tx.clone();
    tokio::spawn(async move {
        let mut tick_interval = interval(Duration::from_millis(250));
        loop {
            tick_interval.tick().await;
            if tick_tx.send(TuiEvent::Tick).await.is_err() {
                break;
            }
        }
    });

    // 4. REST initial data loader
    let rest_tx = tx.clone();
    let rest_url = base_url.to_string();
    let rest_token = token.map(|t| t.to_string());
    tokio::spawn(async move {
        rest_client::load_initial_data(&rest_url, rest_token.as_deref(), rest_tx).await;
    });

    // Initial draw
    terminal.draw(|f| ui::render(f, &app))?;

    // Main event loop
    while let Some(event) = rx.recv().await {
        let needs_redraw = app.handle_event(event);
        if app.should_quit {
            break;
        }
        if needs_redraw {
            terminal.draw(|f| ui::render(f, &app))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ws_client::build_ws_url;

    #[test]
    fn test_ws_url_building() {
        assert_eq!(
            build_ws_url("http://localhost:8080", None),
            "ws://localhost:8080/api/monitor"
        );
        assert_eq!(
            build_ws_url("https://prod.example.com", Some("secret")),
            "wss://prod.example.com/api/monitor?token=secret"
        );
    }

    /// Renders a TUI frame with mock data to a test backend and prints the result.
    ///
    /// Run with: cargo test --lib tui::tests::render_tui_snapshot -- --nocapture
    #[test]
    fn render_tui_snapshot() {
        use super::app::App;
        use super::event::{ExecutionSummary, TuiEvent, WorkflowSummary};
        use super::ui;
        use crate::api::MonitorEvent;
        use ratatui::{backend::TestBackend, Terminal};

        let mut app = App::new("http://localhost:8080".to_string());

        // Populate mock data — workflows
        app.workflows = vec![WorkflowSummary {
            name: "durable-pipeline".to_string(),
            definition: r#"
name: durable-pipeline
description: Demo pipeline for monitoring
nodes:
  - id: fetch-orders
    type: http
    config:
      url: https://api.example.com/orders
      method: GET
  - id: transform
    type: transform
    depends_on: [fetch-orders]
    config:
      expression: "input"
  - id: notify
    type: agent
    depends_on: [transform]
    config:
      prompt: "Summarize: {{ input }}"
"#
            .to_string(),
        }];

        // Load DAG from the workflow
        app.handle_event(TuiEvent::InitialData(super::event::InitialData {
            executions: vec![
                ExecutionSummary {
                    id: "abc12def-3456-7890-abcd-ef1234567890".to_string(),
                    workflow_name: "durable-pipeline".to_string(),
                    status: "completed".to_string(),
                    trigger_type: "manual".to_string(),
                    started_at: chrono::Utc::now() - chrono::Duration::minutes(5),
                    finished_at: Some(chrono::Utc::now() - chrono::Duration::minutes(4)),
                    duration_ms: Some(1230),
                    error: None,
                },
                ExecutionSummary {
                    id: "def34abc-5678-1234-cdef-567890abcdef".to_string(),
                    workflow_name: "durable-pipeline".to_string(),
                    status: "failed".to_string(),
                    trigger_type: "cron".to_string(),
                    started_at: chrono::Utc::now() - chrono::Duration::minutes(10),
                    finished_at: Some(chrono::Utc::now() - chrono::Duration::minutes(9)),
                    duration_ms: Some(450),
                    error: Some("timeout".to_string()),
                },
                ExecutionSummary {
                    id: "1111aaaa-2222-3333-4444-555566667777".to_string(),
                    workflow_name: "durable-pipeline".to_string(),
                    status: "completed".to_string(),
                    trigger_type: "api".to_string(),
                    started_at: chrono::Utc::now() - chrono::Duration::hours(1),
                    finished_at: Some(chrono::Utc::now() - chrono::Duration::minutes(59)),
                    duration_ms: Some(890),
                    error: None,
                },
            ],
            workflows: app.workflows.clone(),
        }));

        // Simulate a live execution with node events
        app.handle_event(TuiEvent::Monitor(MonitorEvent::ExecutionStarted {
            execution_id: "live-exec-001".to_string(),
            workflow_name: "durable-pipeline".to_string(),
            trigger_type: "manual".to_string(),
        }));
        app.handle_event(TuiEvent::Monitor(MonitorEvent::NodeStarted {
            execution_id: "live-exec-001".to_string(),
            node_id: "fetch-orders".to_string(),
            node_type: "http".to_string(),
        }));
        app.handle_event(TuiEvent::Monitor(MonitorEvent::NodeCompleted {
            execution_id: "live-exec-001".to_string(),
            node_id: "fetch-orders".to_string(),
            status: "completed".to_string(),
            duration_ms: 45,
        }));
        app.handle_event(TuiEvent::Monitor(MonitorEvent::NodeStarted {
            execution_id: "live-exec-001".to_string(),
            node_id: "transform".to_string(),
            node_type: "transform".to_string(),
        }));

        // Add some log events
        app.handle_event(TuiEvent::WsConnected);

        // Mark connected
        app.connection_state = super::app::ConnectionState::Connected;

        // Set tick for spinner frame
        app.tick = 3;

        // Render to test backend (120 cols x 35 rows)
        let backend = TestBackend::new(120, 35);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui::render(f, &app)).unwrap();

        // Print the rendered buffer
        let buffer = terminal.backend().buffer().clone();
        println!("\n{}", "=".repeat(120));
        println!("  r8r monitor TUI snapshot (120x35)");
        println!("{}", "=".repeat(120));
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                line.push_str(cell.symbol());
            }
            println!("{}", line.trim_end());
        }
        println!("{}", "=".repeat(120));
    }
}
