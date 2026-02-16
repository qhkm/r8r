use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
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
    /// Manage workflow templates
    Templates {
        #[command(subcommand)]
        action: TemplateActions,
    },
    /// Live TUI monitor for a running r8r server
    Monitor {
        /// Server URL to connect to
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,
        /// Authentication token
        #[arg(short, long)]
        token: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: CompletionShell,
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
        /// Parameter values (key=value)
        #[arg(short, long = "param", value_parser = parse_var)]
        params: Vec<(String, String)>,
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
    /// Resume a failed execution from checkpoint
    Resume {
        /// Execution ID of the failed execution
        execution_id: String,
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
    /// Show workflow dependency graph (DAG)
    Dag {
        /// Workflow name or ID
        name: String,
        /// Show execution order instead of tree
        #[arg(long)]
        order: bool,
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

#[derive(Subcommand)]
enum TemplateActions {
    /// List available workflow templates
    List {
        /// Show templates by category
        #[arg(long)]
        by_category: bool,
    },
    /// Show details of a specific template
    Show {
        /// Template name
        name: String,
    },
    /// Create a workflow from a template
    Use {
        /// Template name
        name: String,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Variable assignments (key=value)
        #[arg(short, long = "var", value_parser = parse_var)]
        vars: Vec<(String, String)>,
        /// Create workflow in database after generating
        #[arg(long)]
        create: bool,
    },
}

fn parse_var(s: &str) -> std::result::Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("Invalid variable format '{}'. Expected key=value", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
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
            WorkflowActions::Run {
                name,
                input,
                params,
                wait,
            } => cmd_workflows_run(&name, input.as_deref(), &params, wait).await?,
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
            WorkflowActions::Resume { execution_id } => cmd_workflows_resume(&execution_id).await?,
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
            WorkflowActions::Dag { name, order } => cmd_workflows_dag(&name, order).await?,
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
        Commands::Templates { action } => match action {
            TemplateActions::List { by_category } => cmd_templates_list(by_category).await?,
            TemplateActions::Show { name } => cmd_templates_show(&name).await?,
            TemplateActions::Use {
                name,
                output,
                vars,
                create,
            } => cmd_templates_use(&name, output.as_deref(), &vars, create).await?,
        },
        Commands::Monitor { url, token } => cmd_monitor(&url, token.as_deref()).await?,
        Commands::Completions { shell } => {
            cmd_completions(shell)?;
        }
    }

    Ok(())
}

/// Shell completion variants
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum CompletionShell {
    /// Bash shell
    Bash,
    /// Zsh shell
    Zsh,
    /// Fish shell
    Fish,
    /// PowerShell
    PowerShell,
    /// Elvish shell
    Elvish,
}

impl From<CompletionShell> for Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
            CompletionShell::Fish => Shell::Fish,
            CompletionShell::PowerShell => Shell::PowerShell,
            CompletionShell::Elvish => Shell::Elvish,
        }
    }
}

/// Generate shell completions
fn cmd_completions(shell: CompletionShell) -> anyhow::Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let shell: Shell = shell.into();
    generate(shell, &mut cmd, name, &mut std::io::stdout());
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

async fn cmd_workflows_run(
    name: &str,
    input: Option<&str>,
    params: &[(String, String)],
    wait: bool,
) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::nodes::NodeRegistry;
    use r8r::workflow::{get_parameter_info, merge_params, parse_cli_params, parse_workflow};

    let storage = get_storage()?;

    // Get workflow
    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", name))?;

    let workflow = parse_workflow(&stored.definition)?;

    // Show parameter info if workflow has parameters and none provided
    if !workflow.inputs.is_empty() && params.is_empty() && input.is_none() {
        let param_info = get_parameter_info(&workflow);
        let required: Vec<_> = param_info
            .iter()
            .filter(|p| p.required && p.default.is_none())
            .collect();

        if !required.is_empty() {
            println!("Workflow '{}' requires parameters:", name);
            for p in &param_info {
                let req = if p.required && p.default.is_none() {
                    " (required)"
                } else {
                    ""
                };
                let def = p
                    .default
                    .as_ref()
                    .map(|d| format!(" [default: {}]", d))
                    .unwrap_or_default();
                println!(
                    "  --param {}=<{}>{}{}  {}",
                    p.name, p.input_type, req, def, p.description
                );
            }
            anyhow::bail!("Missing required parameters");
        }
    }

    // Parse input JSON
    let base_input: serde_json::Value = if let Some(input_str) = input {
        serde_json::from_str(input_str)?
    } else {
        serde_json::json!({})
    };

    // Parse CLI params and merge with input
    let cli_params = parse_cli_params(params)?;
    let input_value = merge_params(&base_input, &cli_params);

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

async fn cmd_workflows_resume(execution_id: &str) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::nodes::NodeRegistry;

    let storage = get_storage()?;
    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage.clone());

    // Get original execution info for display
    let original = storage.get_execution(execution_id).await?;
    let original =
        original.ok_or_else(|| anyhow::anyhow!("Execution not found: {}", execution_id))?;

    // Get completed nodes count
    let node_execs = storage.get_node_executions(execution_id).await?;
    let completed_count = node_execs
        .iter()
        .filter(|n| n.status == r8r::storage::ExecutionStatus::Completed)
        .count();

    println!(
        "Resuming execution of '{}' from checkpoint...",
        original.workflow_name
    );
    println!("  Original execution: {}", execution_id);
    println!("  Completed nodes: {}", completed_count);
    println!();

    let execution = executor.resume(execution_id).await?;

    println!("New execution ID: {}", execution.id);
    println!("Status: {}", execution.status);

    if let Some(finished) = execution.finished_at {
        let duration = finished - execution.started_at;
        println!("Duration: {}ms", duration.num_milliseconds());
    }

    if let Some(error) = &execution.error {
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

async fn cmd_workflows_dag(name: &str, show_order: bool) -> anyhow::Result<()> {
    use r8r::workflow::{parse_workflow, WorkflowDag};

    let storage = get_storage()?;

    // Get the target workflow
    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", name))?;

    let workflow = parse_workflow(&stored.definition)?;

    if workflow.depends_on_workflows.is_empty() {
        println!("Workflow '{}' has no workflow dependencies.", name);
        return Ok(());
    }

    // Build the DAG
    let mut dag = WorkflowDag::new();

    // Add the target workflow and recursively build dependencies
    async fn add_workflow_to_dag(
        dag: &mut WorkflowDag,
        storage: &r8r::storage::SqliteStorage,
        workflow_name: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> anyhow::Result<()> {
        if visited.contains(workflow_name) {
            return Ok(());
        }
        visited.insert(workflow_name.to_string());

        if let Some(stored) = storage.get_workflow(workflow_name).await? {
            let workflow = r8r::workflow::parse_workflow(&stored.definition)?;
            dag.add_workflow(workflow_name, workflow.depends_on_workflows.clone());

            // Recursively add dependencies
            for dep in &workflow.depends_on_workflows {
                Box::pin(add_workflow_to_dag(dag, storage, &dep.workflow, visited)).await?;
            }
        }
        Ok(())
    }

    let mut visited = std::collections::HashSet::new();
    add_workflow_to_dag(&mut dag, &storage, name, &mut visited).await?;

    if show_order {
        // Show execution order (topological sort)
        match dag.execution_order(name) {
            Ok(order) => {
                println!("Execution order for '{}':", name);
                println!();
                for (i, workflow_name) in order.iter().enumerate() {
                    let marker = if *workflow_name == name {
                        " ← target"
                    } else {
                        ""
                    };
                    println!("  {}. {}{}", i + 1, workflow_name, marker);
                }
            }
            Err(e) => {
                println!("Error determining execution order: {}", e);
            }
        }
    } else {
        // Show dependency tree
        println!("Dependency graph for '{}':", name);
        println!();
        println!("{}", dag.to_text(name));
    }

    Ok(())
}

// ============================================================================
// Server Commands
// ============================================================================

async fn cmd_server(port: u16, _no_ui: bool) -> anyhow::Result<()> {
    use r8r::api::{
        api_auth_middleware, create_api_routes, create_concurrency_limit, create_cors_layer,
        create_monitored_routes, create_request_body_limit, ApiAuthConfig, AppState, Monitor,
        MonitoredAppState,
    };
    use r8r::nodes::NodeRegistry;
    use r8r::triggers::{create_webhook_routes, EventSubscriber, Scheduler};
    use std::sync::Arc;
    use tower_http::trace::TraceLayer;

    let storage = get_storage()?;
    let registry = Arc::new(NodeRegistry::default());

    // Create monitor for live WebSocket updates
    let monitor = Arc::new(Monitor::new());

    // Create and start the scheduler for cron triggers
    let scheduler = Scheduler::new(storage.clone(), registry.clone())
        .await?
        .with_monitor(monitor.clone());
    scheduler.start().await?;

    let job_count = scheduler.job_count().await;

    // Create and start the event subscriber for Redis pub/sub triggers
    let mut event_subscriber =
        EventSubscriber::new(storage.clone(), registry.clone()).with_monitor(monitor.clone());
    let event_status = match event_subscriber.start().await {
        Ok(()) => {
            if event_subscriber.is_running() {
                "active"
            } else {
                "no event triggers configured"
            }
        }
        Err(e) => {
            tracing::warn!("Failed to start event subscriber: {}", e);
            "unavailable (Redis not connected)"
        }
    };

    // Create API routes (without state)
    let api_routes = create_api_routes();

    // Create webhook routes (without state)
    let webhook_routes = create_webhook_routes(storage.clone(), registry.clone()).await;

    // Create monitored routes for WebSocket
    let monitored_routes = create_monitored_routes();

    // Merge routers and apply state
    let state = AppState {
        storage: storage.clone(),
        registry: registry.clone(),
        monitor: Some(monitor.clone()),
    };

    let monitored_state = MonitoredAppState {
        inner: state.clone(),
        monitor: monitor.clone(),
    };

    // Build app with both regular and monitored routes
    // Each router needs its state consumed to become Router<()>
    let mut app = api_routes
        .with_state(state.clone())
        .merge(monitored_routes.with_state(monitored_state))
        .merge(webhook_routes.with_state(state))
        .layer(create_request_body_limit())
        .layer(create_concurrency_limit())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn_with_state(
            ApiAuthConfig::default(),
            api_auth_middleware,
        ))
        .layer(create_cors_layer());

    // Add dashboard routes if UI is enabled
    if !_no_ui {
        use r8r::dashboard::create_dashboard_routes;
        app = app.merge(create_dashboard_routes());
    }

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("r8r server running on http://0.0.0.0:{}", port);
    println!();
    println!("Scheduler: {} cron job(s) active", job_count);
    println!("Event subscriber: {}", event_status);
    println!();
    println!("API endpoints:");
    println!("  GET  /api/health");
    println!("  GET  /api/workflows");
    println!("  GET  /api/workflows/:name");
    println!("  POST /api/workflows/:name/execute");
    println!("  GET  /api/executions/:id");
    println!("  GET  /api/executions/:id/trace");
    println!("  WS   /api/monitor (live execution monitoring)");
    println!();
    println!("Webhooks: /webhooks/:workflow_name (see workflow definitions)");
    if !_no_ui {
        println!("Dashboard: http://0.0.0.0:{}/ (Web UI)", port);
    }
    println!();
    println!("Press Ctrl+C to stop");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Gracefully stop the event subscriber and scheduler
    let _ = event_subscriber.stop().await;
    scheduler.stop().await?;

    println!("Server stopped.");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\nShutting down gracefully...");
}

async fn cmd_monitor(url: &str, token: Option<&str>) -> anyhow::Result<()> {
    r8r::tui::run_monitor(url, token).await
}

async fn cmd_dev(file: &str) -> anyhow::Result<()> {
    use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
    use std::path::Path;
    use std::time::Duration;

    let path = Path::new(file);
    if !path.exists() {
        anyhow::bail!("File not found: {}", file);
    }

    let canonical_path = path.canonicalize()?;
    let watch_dir = canonical_path.parent().unwrap_or(Path::new("."));

    println!("r8r dev mode");
    println!("Watching: {}", canonical_path.display());
    println!("Press Ctrl+C to stop\n");

    // Initial validation
    validate_workflow_file(&canonical_path);

    // Set up file watcher with tokio channel
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let target_path = canonical_path.clone();

    // Wrap std channel for notify
    let (notify_tx, notify_rx) = std::sync::mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(500), notify_tx)?;
    debouncer
        .watcher()
        .watch(watch_dir, RecursiveMode::NonRecursive)?;

    // Spawn blocking task to bridge std channel to tokio channel
    let bridge_target = target_path.clone();
    let bridge_tx = tx.clone();
    std::thread::spawn(move || {
        while let Ok(result) = notify_rx.recv() {
            if let Ok(events) = result {
                for event in events {
                    if event.path == bridge_target {
                        let _ = bridge_tx.blocking_send(());
                    }
                }
            }
        }
    });

    // Watch for changes
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nStopping dev mode...");
                break;
            }
            Some(_) = rx.recv() => {
                println!("\n--- File changed: {} ---", chrono::Local::now().format("%H:%M:%S"));
                validate_workflow_file(&target_path);
            }
        }
    }

    Ok(())
}

fn validate_workflow_file(path: &std::path::Path) {
    use r8r::workflow::{parse_workflow_file, validate_workflow};

    match parse_workflow_file(path) {
        Ok(workflow) => match validate_workflow(&workflow) {
            Ok(()) => {
                println!("✓ Workflow '{}' is valid", workflow.name);
                println!("  Nodes: {}", workflow.nodes.len());
                println!("  Triggers: {}", workflow.triggers.len());
            }
            Err(e) => {
                eprintln!("✗ Validation error in '{}': {}", workflow.name, e);
            }
        },
        Err(e) => {
            eprintln!("✗ Parse error: {}", e);
        }
    }
}

// ============================================================================
// Credentials Commands
// ============================================================================

async fn cmd_credentials_set(
    service: &str,
    key: Option<&str>,
    value: Option<&str>,
) -> anyhow::Result<()> {
    use r8r::credentials::CredentialStore;
    use std::io::{self, BufRead};

    // Get value from argument or stdin
    let credential_value = match value {
        Some(v) => v.to_string(),
        None => {
            eprintln!("Enter credential value (or pipe from stdin):");
            let stdin = io::stdin();
            let mut line = String::new();
            stdin.lock().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    if credential_value.is_empty() {
        anyhow::bail!("Credential value cannot be empty");
    }

    let mut store = CredentialStore::load().await?;
    store.set(service, key, &credential_value).await?;

    println!("✓ Credential '{}' saved", service);
    if let Some(k) = key {
        println!("  Key: {}", k);
    }

    Ok(())
}

async fn cmd_credentials_list() -> anyhow::Result<()> {
    use r8r::credentials::CredentialStore;

    let store = CredentialStore::load().await?;
    let credentials = store.list();

    if credentials.is_empty() {
        println!("No credentials stored.");
        println!();
        println!("Add one with: r8r credentials set <service> -v <value>");
        return Ok(());
    }

    println!("{:<20} {:<15} {:<20}", "SERVICE", "KEY", "UPDATED");
    println!("{}", "-".repeat(55));

    for cred in credentials {
        let key_display = cred.key.as_deref().unwrap_or("-");
        let updated = cred.updated_at.format("%Y-%m-%d %H:%M");
        println!("{:<20} {:<15} {:<20}", cred.service, key_display, updated);
    }

    Ok(())
}

async fn cmd_credentials_delete(service: &str) -> anyhow::Result<()> {
    use r8r::credentials::CredentialStore;

    let mut store = CredentialStore::load().await?;
    let deleted = store.delete(service).await?;

    if deleted {
        println!("✓ Credential '{}' deleted", service);
    } else {
        println!("Credential '{}' not found", service);
    }

    Ok(())
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
// Template Commands
// ============================================================================

async fn cmd_templates_list(by_category: bool) -> anyhow::Result<()> {
    use r8r::templates::TemplateRegistry;

    let registry = TemplateRegistry::new();

    if by_category {
        let by_category = registry.list_by_category();
        let mut categories: Vec<_> = by_category.keys().collect();
        categories.sort();

        for category in categories {
            println!("{}:", category.to_uppercase());
            for template in &by_category[category] {
                println!("  {:<25} {}", template.name, template.description);
            }
            println!();
        }
    } else {
        let templates = registry.list();

        if templates.is_empty() {
            println!("No templates available.");
            return Ok(());
        }

        println!("{:<25} {:<15} DESCRIPTION", "NAME", "CATEGORY");
        println!("{}", "-".repeat(70));

        for template in templates {
            println!(
                "{:<25} {:<15} {}",
                template.name, template.category, template.description
            );
        }
    }

    println!();
    println!("Use a template with: r8r templates use <name> --var key=value");

    Ok(())
}

async fn cmd_templates_show(name: &str) -> anyhow::Result<()> {
    use r8r::templates::TemplateRegistry;

    let registry = TemplateRegistry::new();
    let template = registry
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Template not found: {}", name))?;

    println!("Template: {}", template.name);
    println!("Category: {}", template.category);
    println!("Description: {}", template.description);
    println!();

    println!("Variables:");
    for var in &template.variables {
        let required = if var.required && var.default.is_none() {
            " (required)"
        } else {
            ""
        };
        let default = var
            .default
            .as_ref()
            .map(|d| format!(" [default: {}]", d))
            .unwrap_or_default();
        let example = var
            .example
            .as_ref()
            .map(|e| format!(" (e.g., {})", e))
            .unwrap_or_default();

        println!("  {}{}{}", var.name, required, default);
        if !var.description.is_empty() {
            println!("    {}{}", var.description, example);
        }
    }

    println!();
    println!("Template content:");
    println!("---");
    println!("{}", template.content);
    println!("---");

    Ok(())
}

async fn cmd_templates_use(
    name: &str,
    output: Option<&str>,
    vars: &[(String, String)],
    create: bool,
) -> anyhow::Result<()> {
    use r8r::templates::TemplateRegistry;
    use std::collections::HashMap;

    let registry = TemplateRegistry::new();

    // Convert vars to HashMap
    let vars_map: HashMap<String, String> = vars.iter().cloned().collect();

    // Check for missing variables
    let missing = registry.validate_variables(name, &vars_map)?;
    if !missing.is_empty() {
        println!("Missing required variables:");
        for var in &missing {
            println!("  - {}", var);
        }
        println!();

        // Show what's needed
        if let Some(template) = registry.get(name) {
            println!("Required variables for '{}':", name);
            for var in &template.variables {
                if var.required && var.default.is_none() {
                    let example = var
                        .example
                        .as_ref()
                        .map(|e| format!(" (e.g., {})", e))
                        .unwrap_or_default();
                    println!("  --var {}=<value>{}", var.name, example);
                }
            }
        }
        anyhow::bail!("Cannot instantiate template without required variables");
    }

    // Instantiate the template
    let workflow_yaml = registry.instantiate(name, &vars_map)?;

    // Output or create
    if create {
        // Parse and validate the generated workflow
        use r8r::storage::StoredWorkflow;
        use r8r::workflow::{parse_workflow, validate_workflow};

        let workflow = parse_workflow(&workflow_yaml)?;
        validate_workflow(&workflow)?;

        let storage = get_storage()?;
        let now = chrono::Utc::now();

        let stored = StoredWorkflow {
            id: uuid::Uuid::new_v4().to_string(),
            name: workflow.name.clone(),
            definition: workflow_yaml.clone(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        storage.save_workflow(&stored).await?;

        println!(
            "✓ Created workflow '{}' from template '{}'",
            workflow.name, name
        );
        println!();
        println!("Run with: r8r workflows run {}", workflow.name);

        // Also write to file if output specified
        if let Some(path) = output {
            std::fs::write(path, &workflow_yaml)?;
            println!("Also saved to: {}", path);
        }
    } else {
        match output {
            Some(path) => {
                std::fs::write(path, &workflow_yaml)?;
                println!("✓ Generated workflow from template '{}' to {}", name, path);
                println!();
                println!("Create in database with: r8r workflows create {}", path);
            }
            None => {
                // Output to stdout
                print!("{}", workflow_yaml);
            }
        }
    }

    Ok(())
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
