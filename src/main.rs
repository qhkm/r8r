use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "r8r")]
#[command(about = "Agent-first Rust workflow automation engine", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage workflows
    Workflows {
        #[command(subcommand)]
        action: WorkflowActions,
    },
    /// Start the server (API + scheduler)
    Server {
        #[arg(short, long, default_value = "8080")]
        port: u16,
        /// Disable web UI (API only)
        #[arg(long)]
        no_ui: bool,
    },
    /// Development mode (watch + hot reload)
    Dev {
        /// Path to workflow YAML file
        file: String,
    },
    /// Manage credentials
    Credentials {
        #[command(subcommand)]
        action: CredentialActions,
    },
    /// Database maintenance and checks
    Db {
        #[command(subcommand)]
        action: DbActions,
    },
}

#[derive(Subcommand)]
enum WorkflowActions {
    /// List all workflows
    List,
    /// Create a workflow from YAML file
    Create {
        /// Path to workflow YAML file
        file: String,
    },
    /// Run a workflow manually
    Run {
        /// Workflow name or ID
        name: String,
        /// JSON input data
        #[arg(short, long)]
        input: Option<String>,
        /// Wait for completion
        #[arg(short, long, default_value = "true")]
        wait: bool,
    },
    /// Show workflow execution logs
    Logs {
        /// Workflow name or ID
        name: String,
        /// Number of recent executions to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Search execution history with filters
    Search {
        /// Workflow name (optional)
        #[arg(long)]
        workflow: Option<String>,
        /// Status filter: pending|running|completed|failed|cancelled
        #[arg(long)]
        status: Option<String>,
        /// Trigger type filter (manual|replay|...)
        #[arg(long)]
        trigger: Option<String>,
        /// Search text in input/output/error
        #[arg(long)]
        search: Option<String>,
        /// RFC3339 timestamp lower bound
        #[arg(long)]
        started_after: Option<String>,
        /// RFC3339 timestamp upper bound
        #[arg(long)]
        started_before: Option<String>,
        /// Page size
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Offset for pagination
        #[arg(long, default_value = "0")]
        offset: usize,
    },
    /// Replay a previous execution
    Replay {
        /// Execution ID
        execution_id: String,
        /// Optional replacement JSON input
        #[arg(short, long)]
        input: Option<String>,
    },
    /// Show workflow version history
    History {
        /// Workflow name or ID
        name: String,
        /// Number of versions to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Roll back a workflow to a previous version
    Rollback {
        /// Workflow name or ID
        name: String,
        /// Version number to roll back to
        version: u32,
    },
    /// Show detailed node trace for an execution
    Trace {
        /// Execution ID
        execution_id: String,
    },
    /// Show workflow details
    Show {
        /// Workflow name or ID
        name: String,
    },
    /// Delete a workflow
    Delete {
        /// Workflow name or ID
        name: String,
    },
    /// Validate a workflow YAML file
    Validate {
        /// Path to workflow YAML file
        file: String,
    },
    /// Export a workflow to YAML file
    Export {
        /// Workflow name or ID
        name: String,
        /// Output file path (stdout if not specified)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Export all workflows to a directory
    ExportAll {
        /// Output directory path
        #[arg(short, long, default_value = "./workflows")]
        output: String,
    },
}

#[derive(Subcommand)]
enum CredentialActions {
    /// Set a credential
    Set {
        /// Service name (e.g., whatsapp, google-sheets)
        service: String,
        /// Credential key
        #[arg(short, long)]
        key: Option<String>,
        /// Credential value (or read from stdin)
        #[arg(short, long)]
        value: Option<String>,
    },
    /// List configured credentials
    List,
    /// Delete a credential
    Delete {
        /// Service name
        service: String,
    },
}

#[derive(Subcommand)]
enum DbActions {
    /// Run integrity and foreign-key health checks
    Check,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "r8r=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Workflows { action } => match action {
            WorkflowActions::List => cmd_workflows_list().await?,
            WorkflowActions::Create { file } => cmd_workflows_create(&file).await?,
            WorkflowActions::Run { name, input, wait } => {
                cmd_workflows_run(&name, input.as_deref(), wait).await?
            }
            WorkflowActions::Logs { name, limit } => cmd_workflows_logs(&name, limit).await?,
            WorkflowActions::Search {
                workflow,
                status,
                trigger,
                search,
                started_after,
                started_before,
                limit,
                offset,
            } => {
                cmd_workflows_search(
                    workflow.as_deref(),
                    status.as_deref(),
                    trigger.as_deref(),
                    search.as_deref(),
                    started_after.as_deref(),
                    started_before.as_deref(),
                    limit,
                    offset,
                )
                .await?
            }
            WorkflowActions::Replay {
                execution_id,
                input,
            } => cmd_workflows_replay(&execution_id, input.as_deref()).await?,
            WorkflowActions::History { name, limit } => cmd_workflows_history(&name, limit).await?,
            WorkflowActions::Rollback { name, version } => {
                cmd_workflows_rollback(&name, version).await?
            }
            WorkflowActions::Trace { execution_id } => cmd_workflows_trace(&execution_id).await?,
            WorkflowActions::Show { name } => cmd_workflows_show(&name).await?,
            WorkflowActions::Delete { name } => cmd_workflows_delete(&name).await?,
            WorkflowActions::Validate { file } => cmd_workflows_validate(&file).await?,
            WorkflowActions::Export { name, output } => {
                cmd_workflows_export(&name, output.as_deref()).await?
            }
            WorkflowActions::ExportAll { output } => cmd_workflows_export_all(&output).await?,
        },
        Commands::Server { port, no_ui } => cmd_server(port, no_ui).await?,
        Commands::Dev { file } => cmd_dev(&file).await?,
        Commands::Credentials { action } => match action {
            CredentialActions::Set {
                service,
                key,
                value,
            } => cmd_credentials_set(&service, key.as_deref(), value.as_deref()).await?,
            CredentialActions::List => cmd_credentials_list().await?,
            CredentialActions::Delete { service } => cmd_credentials_delete(&service).await?,
        },
        Commands::Db { action } => match action {
            DbActions::Check => cmd_db_check().await?,
        },
    }

    Ok(())
}

// ============================================================================
// Workflow Commands
// ============================================================================

async fn cmd_workflows_list() -> anyhow::Result<()> {
    let storage = get_storage()?;
    let workflows = storage.list_workflows().await?;

    if workflows.is_empty() {
        println!("No workflows found.");
        println!();
        println!("Create one with: r8r workflows create <file.yaml>");
        return Ok(());
    }

    println!("{:<30} {:<10} {:<20}", "NAME", "ENABLED", "UPDATED");
    println!("{}", "-".repeat(62));

    for wf in workflows {
        println!(
            "{:<30} {:<10} {:<20}",
            wf.name,
            if wf.enabled { "yes" } else { "no" },
            wf.updated_at.format("%Y-%m-%d %H:%M")
        );
    }

    Ok(())
}

async fn cmd_workflows_create(file: &str) -> anyhow::Result<()> {
    use r8r::storage::StoredWorkflow;
    use r8r::workflow::{parse_workflow_file, validate_workflow};
    use std::path::Path;

    let path = Path::new(file);
    if !path.exists() {
        anyhow::bail!("File not found: {}", file);
    }

    // Parse and validate
    let workflow = parse_workflow_file(path)?;
    validate_workflow(&workflow)?;

    // Read raw YAML
    let definition = std::fs::read_to_string(path)?;

    // Save to storage
    let storage = get_storage()?;
    let now = chrono::Utc::now();

    let stored = StoredWorkflow {
        id: uuid::Uuid::new_v4().to_string(),
        name: workflow.name.clone(),
        definition,
        enabled: true,
        created_at: now,
        updated_at: now,
    };

    storage.save_workflow(&stored).await?;

    println!("✓ Workflow '{}' created successfully", workflow.name);
    println!();
    println!("  Nodes: {}", workflow.nodes.len());
    println!("  Triggers: {}", workflow.triggers.len());
    println!();
    println!("Run with: r8r workflows run {}", workflow.name);

    Ok(())
}

async fn cmd_workflows_run(name: &str, input: Option<&str>, wait: bool) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::nodes::NodeRegistry;
    use r8r::workflow::parse_workflow;

    let storage = get_storage()?;

    // Get workflow
    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", name))?;

    let workflow = parse_workflow(&stored.definition)?;

    // Parse input
    let input_value: serde_json::Value = if let Some(input_str) = input {
        serde_json::from_str(input_str)?
    } else {
        serde_json::Value::Null
    };

    println!("Running workflow '{}'...", name);
    if !wait {
        println!("Note: --wait=false is not implemented yet; running synchronously.");
    }

    // Execute
    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage.clone());

    let execution = executor
        .execute(&workflow, &stored.id, "manual", input_value)
        .await?;

    // Print result
    println!();
    println!("Execution ID: {}", execution.id);
    println!("Status: {}", execution.status);

    if let Some(error) = &execution.error {
        println!("Error: {}", error);
    }

    if let Some(finished) = execution.finished_at {
        let duration = finished - execution.started_at;
        println!("Duration: {}ms", duration.num_milliseconds());
    }

    Ok(())
}

async fn cmd_workflows_logs(name: &str, limit: usize) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let executions = storage.list_executions(name, limit).await?;

    if executions.is_empty() {
        println!("No executions found for workflow '{}'", name);
        return Ok(());
    }

    println!(
        "{:<36} {:<12} {:<10} {:<20}",
        "EXECUTION ID", "STATUS", "TRIGGER", "STARTED"
    );
    println!("{}", "-".repeat(80));

    for exec in executions {
        println!(
            "{:<36} {:<12} {:<10} {:<20}",
            exec.id,
            exec.status.to_string(),
            exec.trigger_type,
            exec.started_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_workflows_search(
    workflow: Option<&str>,
    status: Option<&str>,
    trigger: Option<&str>,
    search: Option<&str>,
    started_after: Option<&str>,
    started_before: Option<&str>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<()> {
    use chrono::{DateTime, Utc};
    use r8r::storage::{ExecutionQuery, ExecutionStatus};

    let storage = get_storage()?;

    let status_filter = if let Some(status) = status {
        Some(status.parse::<ExecutionStatus>().map_err(|_| {
            anyhow::anyhow!(
                "Invalid status '{}'. Expected pending|running|completed|failed|cancelled",
                status
            )
        })?)
    } else {
        None
    };

    let started_after_filter = if let Some(raw) = started_after {
        Some(
            DateTime::parse_from_rfc3339(raw)
                .map_err(|e| anyhow::anyhow!("Invalid --started-after '{}': {}", raw, e))?
                .with_timezone(&Utc),
        )
    } else {
        None
    };

    let started_before_filter = if let Some(raw) = started_before {
        Some(
            DateTime::parse_from_rfc3339(raw)
                .map_err(|e| anyhow::anyhow!("Invalid --started-before '{}': {}", raw, e))?
                .with_timezone(&Utc),
        )
    } else {
        None
    };

    let query = ExecutionQuery {
        workflow_name: workflow.map(|s| s.to_string()),
        status: status_filter,
        trigger_type: trigger.map(|s| s.to_string()),
        search: search.map(|s| s.to_string()),
        started_after: started_after_filter,
        started_before: started_before_filter,
        limit,
        offset,
    };

    let executions = storage.query_executions(&query).await?;
    if executions.is_empty() {
        println!("No executions found for the provided filters.");
        return Ok(());
    }

    println!(
        "{:<36} {:<20} {:<10} {:<10} {:<20}",
        "EXECUTION ID", "WORKFLOW", "STATUS", "TRIGGER", "STARTED"
    );
    println!("{}", "-".repeat(104));

    for exec in executions {
        println!(
            "{:<36} {:<20} {:<10} {:<10} {:<20}",
            exec.id,
            exec.workflow_name,
            exec.status.to_string(),
            exec.trigger_type,
            exec.started_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(())
}

async fn cmd_workflows_replay(execution_id: &str, input: Option<&str>) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::nodes::NodeRegistry;

    let storage = get_storage()?;
    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage.clone());

    let input_value = if let Some(raw) = input {
        Some(serde_json::from_str(raw)?)
    } else {
        None
    };

    let execution = executor.replay(execution_id, input_value).await?;

    println!("Replay execution ID: {}", execution.id);
    println!("Status: {}", execution.status);
    if let Some(error) = execution.error {
        println!("Error: {}", error);
    }

    Ok(())
}

async fn cmd_workflows_history(name: &str, limit: usize) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let versions = storage.list_workflow_versions(name).await?;

    if versions.is_empty() {
        println!("No workflow versions found for {}", name);
        return Ok(());
    }

    println!("{:<8} {:<20} {:<12} CHANGELOG", "VERSION", "CREATED", "BY");
    println!("{}", "-".repeat(72));

    for version in versions.into_iter().take(limit) {
        println!(
            "{:<8} {:<20} {:<12} {}",
            version.version,
            version.created_at.format("%Y-%m-%d %H:%M:%S"),
            version.created_by.unwrap_or_else(|| "-".to_string()),
            version.changelog.unwrap_or_else(|| "-".to_string())
        );
    }

    Ok(())
}

async fn cmd_workflows_rollback(name: &str, version: u32) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let updated = storage
        .rollback_workflow(name, version, Some("cli"))
        .await?;

    println!("✓ Rolled back {} to version {}", name, version);
    println!(
        "Updated at: {}",
        updated.updated_at.format("%Y-%m-%d %H:%M:%S")
    );

    Ok(())
}

async fn cmd_workflows_trace(execution_id: &str) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let trace = storage
        .get_execution_trace(execution_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Execution not found: {}", execution_id))?;

    println!("Execution: {}", trace.execution.id);
    println!("Workflow: {}", trace.execution.workflow_name);
    println!("Status: {}", trace.execution.status);
    println!();

    for node in trace.nodes {
        let duration = node
            .finished_at
            .map(|f| (f - node.started_at).num_milliseconds())
            .unwrap_or(0);

        println!("- {} [{}] {}ms", node.node_id, node.status, duration);
        if let Some(error) = node.error {
            println!("  error: {}", error);
        }
    }

    Ok(())
}

async fn cmd_workflows_show(name: &str) -> anyhow::Result<()> {
    use r8r::workflow::parse_workflow;

    let storage = get_storage()?;

    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", name))?;

    let workflow = parse_workflow(&stored.definition)?;

    println!("Workflow: {}", workflow.name);
    println!("Description: {}", workflow.description);
    println!("Version: {}", workflow.version);
    println!("Enabled: {}", stored.enabled);
    println!();
    println!("Triggers:");
    for trigger in &workflow.triggers {
        println!("  - {:?}", trigger);
    }
    println!();
    println!("Nodes:");
    for node in &workflow.nodes {
        let deps = if node.depends_on.is_empty() {
            String::new()
        } else {
            format!(" (depends on: {})", node.depends_on.join(", "))
        };
        println!("  - {} [{}]{}", node.id, node.node_type, deps);
    }

    Ok(())
}

async fn cmd_workflows_delete(name: &str) -> anyhow::Result<()> {
    let storage = get_storage()?;
    storage.delete_workflow(name).await?;

    println!("✓ Workflow '{}' deleted", name);

    Ok(())
}

async fn cmd_workflows_validate(file: &str) -> anyhow::Result<()> {
    use r8r::workflow::{parse_workflow_file, validate_workflow};
    use std::path::Path;

    let path = Path::new(file);
    if !path.exists() {
        anyhow::bail!("File not found: {}", file);
    }

    let workflow = parse_workflow_file(path)?;
    validate_workflow(&workflow)?;

    println!("✓ Workflow '{}' is valid", workflow.name);
    println!();
    println!("  Nodes: {}", workflow.nodes.len());
    println!("  Triggers: {}", workflow.triggers.len());

    // Check for agent nodes
    let agent_nodes: Vec<_> = workflow
        .nodes
        .iter()
        .filter(|n| n.node_type == "agent")
        .collect();
    if !agent_nodes.is_empty() {
        println!("  Agent nodes: {} (requires ZeptoClaw)", agent_nodes.len());
    }

    Ok(())
}

async fn cmd_workflows_export(name: &str, output: Option<&str>) -> anyhow::Result<()> {
    let storage = get_storage()?;

    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", name))?;

    match output {
        Some(path) => {
            use std::path::Path;

            let path = Path::new(path);

            // Create parent directory if needed
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            std::fs::write(path, &stored.definition)?;
            println!("✓ Exported '{}' to {}", name, path.display());
        }
        None => {
            // Output to stdout
            print!("{}", stored.definition);
        }
    }

    Ok(())
}

async fn cmd_workflows_export_all(output_dir: &str) -> anyhow::Result<()> {
    use std::path::Path;

    let storage = get_storage()?;
    let workflows = storage.list_workflows().await?;

    if workflows.is_empty() {
        println!("No workflows to export.");
        return Ok(());
    }

    let dir = Path::new(output_dir);
    std::fs::create_dir_all(dir)?;

    let mut exported = 0;
    for wf in &workflows {
        let filename = format!("{}.yaml", wf.name);
        let path = dir.join(&filename);

        std::fs::write(&path, &wf.definition)?;
        println!("  Exported: {}", path.display());
        exported += 1;
    }

    println!();
    println!("✓ Exported {} workflow(s) to {}", exported, output_dir);

    Ok(())
}

// ============================================================================
// Server Commands
// ============================================================================

async fn cmd_server(port: u16, _no_ui: bool) -> anyhow::Result<()> {
    use r8r::api::{create_router, AppState};
    use r8r::nodes::NodeRegistry;
    use std::sync::Arc;

    let storage = get_storage()?;
    let registry = Arc::new(NodeRegistry::default());

    let state = AppState { storage, registry };
    let app = create_router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("r8r server running on http://0.0.0.0:{}", port);
    println!();
    println!("API endpoints:");
    println!("  GET  /api/health");
    println!("  GET  /api/workflows");
    println!("  GET  /api/workflows/:name");
    println!("  POST /api/workflows/:name/execute");
    println!("  GET  /api/executions/:id");
    println!("  GET  /api/executions/:id/trace");
    println!();
    println!("Press Ctrl+C to stop");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    println!("Server stopped.");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\nShutting down gracefully...");
}

async fn cmd_dev(file: &str) -> anyhow::Result<()> {
    anyhow::bail!(
        "Development mode is not implemented yet (requested file: {}).",
        file
    )
}

// ============================================================================
// Credentials Commands
// ============================================================================

async fn cmd_credentials_set(
    service: &str,
    key: Option<&str>,
    _value: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "Credential storage is not implemented yet (service='{}', key={:?}).",
        service,
        key
    )
}

async fn cmd_credentials_list() -> anyhow::Result<()> {
    anyhow::bail!("Credential listing is not implemented yet.")
}

async fn cmd_credentials_delete(service: &str) -> anyhow::Result<()> {
    anyhow::bail!(
        "Credential deletion is not implemented yet (service='{}').",
        service
    )
}

async fn cmd_db_check() -> anyhow::Result<()> {
    let storage = get_storage()?;
    let health = storage.check_health().await?;

    println!(
        "Foreign keys: {}",
        if health.foreign_keys_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Integrity check: {}", health.integrity_check);
    println!(
        "Foreign key violations: {}",
        health.foreign_key_violations.len()
    );
    for violation in &health.foreign_key_violations {
        println!("  - {}", violation);
    }
    println!(
        "Orphan rows: executions={} node_executions={} workflow_versions={}",
        health.orphaned_executions,
        health.orphaned_node_executions,
        health.orphaned_workflow_versions
    );

    let healthy = health.foreign_keys_enabled
        && health.integrity_check.eq_ignore_ascii_case("ok")
        && health.foreign_key_violations.is_empty()
        && health.orphaned_executions == 0
        && health.orphaned_node_executions == 0
        && health.orphaned_workflow_versions == 0;

    if healthy {
        println!("✓ Database health check passed");
        Ok(())
    } else {
        anyhow::bail!("Database health check failed")
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn get_storage() -> anyhow::Result<r8r::storage::SqliteStorage> {
    use r8r::storage::SqliteStorage;
    let config = r8r::config::Config::load();

    let db_path = config
        .storage
        .database_path
        .unwrap_or_else(|| r8r::config::Config::data_dir().join("r8r.db"));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(SqliteStorage::open(&db_path)?)
}
