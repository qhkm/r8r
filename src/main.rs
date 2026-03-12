use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "r8r")]
#[command(about = "Agent-first Rust workflow automation engine", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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
    /// Generate a workflow from a natural language description
    #[command(alias = "generate")]
    Create {
        /// Natural language description of the workflow to create
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    /// Refine an existing workflow with a natural language change description
    Refine {
        /// Name of the workflow to refine
        name: String,
        /// Description of the change to make
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    /// Interactive REPL — create, manage, and debug workflows with AI
    Chat {
        /// Resume a previous session
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        resume: Option<String>,
    },
    /// Generate or refine a workflow from a natural language description (shorthand)
    Prompt {
        /// Description of the workflow to create or change
        #[arg(trailing_var_arg = true)]
        description: Vec<String>,
        /// Refine an existing workflow by name instead of creating new
        #[arg(long, short)]
        patch: Option<String>,
        /// Print generated YAML to stdout instead of saving
        #[arg(long)]
        emit: bool,
        /// Generate and validate only, do not save
        #[arg(long)]
        dry_run: bool,
        /// Skip interactive review and save immediately
        #[arg(long, short = 'y')]
        yes: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a workflow (shorthand for `workflows run`)
    Run {
        /// Workflow name or ID
        name: String,
        /// JSON input data
        #[arg(short, long)]
        input: Option<String>,
        /// Parameter values (key=value)
        #[arg(short, long = "param", value_parser = parse_var)]
        params: Vec<(String, String)>,
        /// Skip review screen and run immediately
        #[arg(long, short = 'y')]
        yes: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show and manage recent workflow executions
    Runs {
        #[command(subcommand)]
        action: Option<RunsActions>,
        /// Filter by workflow name (when no subcommand given)
        #[arg(global = false)]
        workflow: Option<String>,
        /// Number of executions to show
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
        /// Filter by status (pending|running|completed|failed|waiting_for_approval)
        #[arg(long)]
        status: Option<String>,
    },
    /// Manage secrets/credentials (shorthand for `credentials`)
    Secrets {
        #[command(subcommand)]
        action: SecretsActions,
    },
    /// Live monitor for a running r8r server (shorthand for `monitor`)
    Watch {
        /// Server URL to connect to
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,
        /// Authentication token
        #[arg(short, long)]
        token: Option<String>,
    },
    /// Health check — verify DB, API, credentials, and node registry
    Doctor,
    /// Run workflow tests from a fixture file
    Test {
        /// Path to test fixture file (.yaml). Defaults to r8r-tests.yaml
        #[arg(default_value = "r8r-tests.yaml")]
        file: String,
        /// Run only tests matching this name substring
        #[arg(long, short = 'k')]
        filter: Option<String>,
        /// Write JUnit XML report to this file
        #[arg(long)]
        junit_xml: Option<String>,
        /// Update snapshots instead of asserting (writes expected output)
        #[arg(long)]
        update_snapshots: bool,
    },
    /// Lint a workflow YAML file for errors and warnings
    Lint {
        /// Path(s) to workflow YAML file(s)
        #[arg(required = true)]
        files: Vec<String>,
        /// Fail only on errors, not warnings
        #[arg(long)]
        errors_only: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending workflow execution
    Approve {
        /// Execution ID or Approval ID to approve
        id: String,
        /// Optional comment
        #[arg(long, short)]
        comment: Option<String>,
        /// Identity of the approver
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// Deny (reject) a pending workflow execution
    Deny {
        /// Execution ID or Approval ID to reject
        id: String,
        /// Optional comment
        #[arg(long, short)]
        comment: Option<String>,
        /// Identity of the denier
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// Manage workflow execution policy
    Policy {
        #[command(subcommand)]
        action: PolicyActions,
    },
    /// Emit the JSON Schema for workflow YAML (for IDE autocomplete)
    Schema {
        /// Write schema to this file instead of stdout
        #[arg(long, short)]
        output: Option<String>,
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

#[derive(Subcommand)]
enum SecretsActions {
    /// Set a secret/credential
    Set {
        /// Service name (e.g., openai, slack)
        service: String,
        /// Credential key
        #[arg(short, long)]
        key: Option<String>,
        /// Credential value (or read from stdin)
        #[arg(short, long)]
        value: Option<String>,
    },
    /// List configured secrets
    List,
    /// Delete a secret
    Delete {
        /// Service name
        service: String,
    },
}

#[derive(Subcommand)]
enum RunsActions {
    /// Show executions waiting for human approval
    Pending {
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
    },
    /// Show full detail of a single execution
    Show {
        /// Execution ID
        run_id: String,
        /// Include node-level trace
        #[arg(long)]
        trace: bool,
    },
    /// Export an execution as JSONL (one JSON object per line)
    Export {
        /// Execution ID
        run_id: String,
        /// Output format: jsonl (default) or json
        #[arg(long, default_value = "jsonl")]
        format: String,
        /// Output file (stdout if omitted)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum PolicyActions {
    /// Show the active policy profile and all knobs
    Show,
    /// Switch to a named profile: lenient, standard, or strict
    Set {
        /// Profile name: lenient | standard | strict
        profile: String,
    },
    /// Validate a workflow file against the active policy
    Validate {
        /// Path to workflow YAML file
        file: String,
    },
}

fn parse_var(s: &str) -> std::result::Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("Invalid variable format '{}'. Expected key=value", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

fn init_logging(for_chat_tui: bool) {
    let default_filter = if for_chat_tui { "r8r=warn" } else { "r8r=info" };
    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| default_filter.into()),
    );

    if for_chat_tui {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let for_chat_tui = matches!(cli.command, None | Some(Commands::Chat { .. }));
    init_logging(for_chat_tui);

    match cli.command {
        None => cmd_chat(None).await?,
        Some(Commands::Chat { resume }) => cmd_chat(resume).await?,
        Some(Commands::Workflows { action }) => match action {
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
        Some(Commands::Server { port, no_ui }) => cmd_server(port, no_ui).await?,
        Some(Commands::Dev { file }) => cmd_dev(&file).await?,
        Some(Commands::Credentials { action }) => match action {
            CredentialActions::Set {
                service,
                key,
                value,
            } => cmd_credentials_set(&service, key.as_deref(), value.as_deref()).await?,
            CredentialActions::List => cmd_credentials_list().await?,
            CredentialActions::Delete { service } => cmd_credentials_delete(&service).await?,
        },
        Some(Commands::Db { action }) => match action {
            DbActions::Check => cmd_db_check().await?,
        },
        Some(Commands::Templates { action }) => match action {
            TemplateActions::List { by_category } => cmd_templates_list(by_category).await?,
            TemplateActions::Show { name } => cmd_templates_show(&name).await?,
            TemplateActions::Use {
                name,
                output,
                vars,
                create,
            } => cmd_templates_use(&name, output.as_deref(), &vars, create).await?,
        },
        Some(Commands::Monitor { url, token }) => cmd_monitor(&url, token.as_deref()).await?,
        Some(Commands::Completions { shell }) => {
            cmd_completions(shell)?;
        }
        Some(Commands::Create { prompt }) => cmd_create(&prompt).await?,
        Some(Commands::Refine { name, prompt }) => cmd_refine(&name, &prompt).await?,
        Some(Commands::Prompt {
            description,
            patch,
            emit,
            dry_run,
            yes,
            json,
        }) => cmd_prompt(&description, patch.as_deref(), emit, dry_run, yes, json).await?,
        Some(Commands::Run {
            name,
            input,
            params,
            yes,
            json,
        }) => cmd_run(&name, input.as_deref(), &params, yes, json).await?,
        Some(Commands::Runs {
            action,
            workflow,
            limit,
            status,
        }) => match action {
            Some(RunsActions::Pending { limit }) => cmd_runs_pending(limit).await?,
            Some(RunsActions::Show { run_id, trace }) => cmd_runs_show(&run_id, trace).await?,
            Some(RunsActions::Export {
                run_id,
                format,
                output,
            }) => cmd_runs_export(&run_id, &format, output.as_deref()).await?,
            None => cmd_runs(workflow.as_deref(), limit, status.as_deref()).await?,
        },
        Some(Commands::Secrets { action }) => match action {
            SecretsActions::Set {
                service,
                key,
                value,
            } => cmd_credentials_set(&service, key.as_deref(), value.as_deref()).await?,
            SecretsActions::List => cmd_credentials_list().await?,
            SecretsActions::Delete { service } => cmd_credentials_delete(&service).await?,
        },
        Some(Commands::Watch { url, token }) => cmd_monitor(&url, token.as_deref()).await?,
        Some(Commands::Doctor) => cmd_doctor().await?,
        Some(Commands::Approve { id, comment, by }) => {
            cmd_approve_deny(&id, "approve", comment.as_deref(), &by).await?
        }
        Some(Commands::Deny { id, comment, by }) => {
            cmd_approve_deny(&id, "reject", comment.as_deref(), &by).await?
        }
        Some(Commands::Policy { action }) => match action {
            PolicyActions::Show => cmd_policy_show().await?,
            PolicyActions::Set { profile } => cmd_policy_set(&profile).await?,
            PolicyActions::Validate { file } => cmd_policy_validate(&file).await?,
        },
        Some(Commands::Test {
            file,
            filter,
            junit_xml,
            update_snapshots,
        }) => {
            cmd_test(
                &file,
                filter.as_deref(),
                junit_xml.as_deref(),
                update_snapshots,
            )
            .await?
        }
        Some(Commands::Lint {
            files,
            errors_only,
            json,
        }) => cmd_lint(&files, errors_only, json).await?,
        Some(Commands::Schema { output }) => cmd_schema(output.as_deref()).await?,
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

async fn cmd_chat(resume: Option<String>) -> anyhow::Result<()> {
    r8r::repl::run_repl(resume).await
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
    let registry = build_registry();
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

    let storage = get_storage()?;
    let registry = build_registry();
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

    let storage = get_storage()?;
    let registry = build_registry();
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
        storage: &dyn r8r::storage::Storage,
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
    use r8r::shutdown::ShutdownCoordinator;
    use r8r::triggers::{
        create_webhook_routes, ApprovalTimeoutChecker, DelayedEventProcessor, EventSubscriber,
        NativeEventBackend, Scheduler,
    };
    use std::sync::Arc;
    use tower_http::trace::TraceLayer;

    let storage = get_storage()?;
    let registry = Arc::new(build_registry());

    // Create shutdown coordinator
    let shutdown = Arc::new(ShutdownCoordinator::new());

    // Start signal listener for graceful shutdown
    shutdown.start_cross_platform_signal_listener();

    // Create monitor for live WebSocket updates
    let monitor = Arc::new(Monitor::new());

    // Create and start the scheduler for cron triggers
    let scheduler = Scheduler::new(storage.clone(), registry.clone())
        .await?
        .with_monitor(monitor.clone());
    scheduler.start().await?;

    let job_count = scheduler.job_count().await;

    // Create event backend (native by default, Redis when feature-enabled + REDIS_URL set)
    let event_backend: Arc<dyn r8r::triggers::EventBackend> = {
        #[cfg(feature = "redis")]
        {
            if let Ok(url) = std::env::var("REDIS_URL") {
                match r8r::triggers::RedisEventBackend::new(&url) {
                    Ok(rb) => {
                        tracing::info!("Using Redis event backend");
                        Arc::new(rb)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create Redis event backend: {}. Falling back to native.",
                            e
                        );
                        Arc::new(NativeEventBackend::new())
                    }
                }
            } else {
                Arc::new(NativeEventBackend::new())
            }
        }
        #[cfg(not(feature = "redis"))]
        {
            Arc::new(NativeEventBackend::new())
        }
    };

    // Create delayed event processor (polls SQLite, re-publishes via backend)
    let mut delayed_processor =
        DelayedEventProcessor::new(storage.clone(), registry.clone(), event_backend.clone())
            .with_monitor(monitor.clone());
    delayed_processor.start().await?;

    // Create and start the event subscriber
    let mut event_subscriber =
        EventSubscriber::new(storage.clone(), registry.clone(), event_backend.clone())
            .with_monitor(monitor.clone())
            .with_delayed_processor(Arc::new(DelayedEventProcessor::new(
                storage.clone(),
                registry.clone(),
                event_backend.clone(),
            )));
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
            "unavailable"
        }
    };

    // Start approval timeout checker (processes expired approval requests)
    let timeout_executor = {
        let mut exec = r8r::engine::Executor::new((*registry).clone(), storage.clone());
        exec = exec.with_monitor(monitor.clone());
        exec = exec.with_pause_registry(r8r::engine::PauseRegistry::new());
        Arc::new(exec)
    };
    let mut approval_timeout_checker =
        ApprovalTimeoutChecker::new(storage.clone(), timeout_executor);
    approval_timeout_checker.start().await;

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
        shutdown: shutdown.clone(),
        pause_registry: r8r::engine::PauseRegistry::new(),
        event_backend: Some(event_backend.clone()),
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
    println!("  GET  /api/executions");
    println!("  GET  /api/executions/:id");
    println!("  GET  /api/executions/:id/trace");
    println!("  POST /api/executions/:id/pause");
    println!("  POST /api/executions/:id/resume");
    println!("  WS   /api/monitor (live execution monitoring)");
    println!();
    println!("Webhooks: /webhooks/:workflow_name (see workflow definitions)");
    if !_no_ui {
        println!("Dashboard: http://0.0.0.0:{}/ (Web UI)", port);
    }
    println!();
    println!("Press Ctrl+C to stop");

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Start the server with graceful shutdown
    let server = axum::serve(listener, app);

    tokio::select! {
        result = server => {
            result?;
        }
        _ = shutdown.wait_for_shutdown() => {
            println!("\nShutting down gracefully...");
        }
    }

    // Gracefully stop background tasks
    let _ = event_subscriber.stop().await;
    let _ = delayed_processor.stop().await;
    approval_timeout_checker.stop().await;
    scheduler.stop().await?;

    println!("Server stopped.");
    Ok(())
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

/// Build a NodeRegistry, optionally with sandbox backend if feature is enabled.
fn build_registry() -> r8r::nodes::NodeRegistry {
    #[allow(unused_mut)]
    let mut registry = r8r::nodes::NodeRegistry::new();

    #[cfg(feature = "sandbox")]
    {
        use r8r::config::Config;
        use r8r::sandbox::SubprocessBackend;
        use std::sync::Arc;

        let config = Config::load();
        let sandbox_backend: Arc<dyn r8r::sandbox::SandboxBackend> = match config
            .sandbox
            .backend
            .as_str()
        {
            #[cfg(feature = "sandbox-docker")]
            "docker" => {
                use r8r::sandbox::SandboxBackend as _;
                match r8r::sandbox::DockerBackend::new(&config.sandbox.docker) {
                    Ok(db) if db.available() => {
                        tracing::info!("Using Docker sandbox backend");
                        Arc::new(db)
                    }
                    Ok(_) => {
                        tracing::warn!("Docker unavailable, falling back to subprocess sandbox");
                        Arc::new(SubprocessBackend::new())
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Docker sandbox init failed: {}. Falling back to subprocess.",
                            e
                        );
                        Arc::new(SubprocessBackend::new())
                    }
                }
            }
            #[cfg(feature = "sandbox-firecracker")]
            "firecracker" => {
                use r8r::sandbox::FirecrackerBackend;
                use r8r::sandbox::SandboxBackend as _;
                let fb = FirecrackerBackend::new(&config.sandbox.firecracker);
                if fb.available() {
                    tracing::info!("Using Firecracker sandbox backend");
                    Arc::new(fb)
                } else {
                    tracing::warn!(
                            "Firecracker unavailable (no /dev/kvm?), falling back to subprocess sandbox"
                        );
                    Arc::new(SubprocessBackend::new())
                }
            }
            _ => Arc::new(SubprocessBackend::new()),
        };

        registry.register(Arc::new(r8r::nodes::SandboxNode::new(sandbox_backend)));
    }

    registry
}

fn get_storage() -> anyhow::Result<std::sync::Arc<dyn r8r::storage::Storage>> {
    use r8r::storage::SqliteStorage;
    let config = r8r::config::Config::load();

    // If DATABASE_URL is set and points to postgres, use PostgreSQL backend.
    // Requires the binary to be compiled with the `storage-postgres` feature.
    if let Ok(db_url) = std::env::var("DATABASE_URL") {
        if db_url.starts_with("postgres://") || db_url.starts_with("postgresql://") {
            #[cfg(feature = "storage-postgres")]
            {
                let storage = tokio::runtime::Handle::current()
                    .block_on(r8r::storage::PostgresStorage::connect(&db_url))?;
                return Ok(std::sync::Arc::new(storage));
            }
            #[cfg(not(feature = "storage-postgres"))]
            {
                anyhow::bail!(
                    "DATABASE_URL points to PostgreSQL but r8r was not compiled with the \
                     `storage-postgres` feature. Recompile with: cargo build --features storage-postgres"
                );
            }
        }
    }

    let db_path = config
        .storage
        .database_path
        .unwrap_or_else(|| r8r::config::Config::data_dir().join("r8r.db"));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(std::sync::Arc::new(SqliteStorage::open(&db_path)?))
}

// ============================================================================
// Generate / Refine Commands
// ============================================================================

async fn cmd_create(prompt_parts: &[String]) -> anyhow::Result<()> {
    use r8r::generator;
    use r8r::storage::StoredWorkflow;
    use std::io::{self, BufRead, Write};

    let user_prompt = prompt_parts.join(" ");
    if user_prompt.trim().is_empty() {
        anyhow::bail!(
            "Please provide a description. Example: r8r create fetch HN top stories and send to Slack"
        );
    }

    println!("Generating workflow...\n");

    let mut result = generator::generate(&user_prompt)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    loop {
        // Show the YAML
        println!("{}", result.yaml);
        println!();

        if result.valid {
            println!("✓ {}", result.summary);
        } else {
            println!("⚠ Workflow has validation errors:");
            for err in &result.errors {
                println!("  - {}", err);
            }
        }

        println!();
        print!("Save? [yes/no/or type feedback to refine] > ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;
        let input = input.trim();

        match input.to_lowercase().as_str() {
            "yes" | "y" => {
                if !result.valid {
                    println!(
                        "Cannot save — workflow has validation errors. Provide feedback to fix or type 'no' to discard."
                    );
                    continue;
                }

                let workflow = r8r::workflow::parse_workflow(&result.yaml)?;
                let storage = get_storage()?;
                let now = chrono::Utc::now();

                let stored = StoredWorkflow {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: workflow.name.clone(),
                    definition: result.yaml.clone(),
                    enabled: true,
                    created_at: now,
                    updated_at: now,
                };

                storage.save_workflow(&stored).await?;
                println!("\n✓ Workflow '{}' saved", workflow.name);
                println!("Run with: r8r workflows run {}", workflow.name);
                return Ok(());
            }
            "no" | "n" => {
                println!("Discarded.");
                return Ok(());
            }
            feedback => {
                println!("\nRefining...\n");
                result = generator::refine(&result.yaml, feedback)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }
        }
    }
}

async fn cmd_refine(name: &str, prompt_parts: &[String]) -> anyhow::Result<()> {
    use r8r::generator;
    use r8r::storage::StoredWorkflow;
    use std::io::{self, BufRead, Write};

    let user_prompt = prompt_parts.join(" ");
    if user_prompt.trim().is_empty() {
        anyhow::bail!(
            "Please describe the change. Example: r8r refine my-workflow add retry with exponential backoff"
        );
    }

    // Load existing workflow
    let storage = get_storage()?;
    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;

    let current_yaml = stored.definition.clone();
    let stored_id = stored.id.clone();
    let stored_enabled = stored.enabled;
    let stored_created_at = stored.created_at;

    println!("Refining workflow '{}'...\n", name);

    let mut result = generator::refine(&current_yaml, &user_prompt)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    loop {
        println!("{}", result.yaml);
        println!();

        if result.valid {
            println!("✓ {}", result.summary);
        } else {
            println!("⚠ Workflow has validation errors:");
            for err in &result.errors {
                println!("  - {}", err);
            }
        }

        println!();
        print!("Save? [yes/no/or type feedback to refine further] > ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;
        let input = input.trim();

        match input.to_lowercase().as_str() {
            "yes" | "y" => {
                if !result.valid {
                    println!("Cannot save — workflow has validation errors.");
                    continue;
                }

                let workflow = r8r::workflow::parse_workflow(&result.yaml)?;
                let now = chrono::Utc::now();

                // Prevent LLM from changing the workflow name
                if workflow.name != name {
                    println!(
                        "⚠ LLM changed workflow name from '{}' to '{}'. Forcing original name.",
                        name, workflow.name
                    );
                    result.yaml = result.yaml.replacen(
                        &format!("name: {}", workflow.name),
                        &format!("name: {}", name),
                        1,
                    );
                }

                let updated = StoredWorkflow {
                    id: stored_id.clone(),
                    name: name.to_string(),
                    definition: result.yaml.clone(),
                    enabled: stored_enabled,
                    created_at: stored_created_at,
                    updated_at: now,
                };

                storage.save_workflow(&updated).await?;
                println!("\n✓ Workflow '{}' updated", name);
                return Ok(());
            }
            "no" | "n" => {
                println!("Discarded.");
                return Ok(());
            }
            feedback => {
                println!("\nRefining further...\n");
                result = generator::refine(&result.yaml, feedback)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }
        }
    }
}

// ─── Sprint 1: CLI Guardrails ─────────────────────────────────────────────────

/// Returns true if stdin is connected to an interactive terminal.
fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// `r8r prompt` — smart wrapper for workflow generation/refinement.
async fn cmd_prompt(
    description: &[String],
    patch: Option<&str>,
    emit: bool,
    dry_run: bool,
    yes: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    use r8r::generator;
    use r8r::storage::StoredWorkflow;
    use std::io::{self, BufRead, Write};

    let user_prompt = description.join(" ");
    if user_prompt.trim().is_empty() {
        if let Some(name) = patch {
            anyhow::bail!(
                "Please describe the change. Example: r8r prompt --patch {} add retry with backoff",
                name
            );
        } else {
            anyhow::bail!(
                "Please provide a description. Example: r8r prompt fetch HN top stories and send to Slack"
            );
        }
    }

    // Generate or refine
    let mut result = if let Some(name) = patch {
        let storage = get_storage()?;
        let stored = storage
            .get_workflow(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;
        eprintln!("Refining workflow '{}'...\n", name);
        generator::refine(&stored.definition, &user_prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?
    } else {
        eprintln!("Generating workflow...\n");
        generator::generate(&user_prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?
    };

    loop {
        // --emit: print YAML to stdout and exit
        if emit {
            print!("{}", result.yaml);
            if !result.valid {
                eprintln!("\n⚠ Workflow has validation errors:");
                for err in &result.errors {
                    eprintln!("  - {}", err);
                }
                std::process::exit(1);
            }
            return Ok(());
        }

        // --json: machine-readable output
        if json_output {
            let out = serde_json::json!({
                "yaml": result.yaml,
                "valid": result.valid,
                "summary": result.summary,
                "errors": result.errors,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
            if dry_run || !result.valid {
                return Ok(());
            }
        }

        // Show the YAML (non-JSON mode)
        if !json_output {
            println!("{}", result.yaml);
            println!();
            if result.valid {
                eprintln!("✓ {}", result.summary);
            } else {
                eprintln!("⚠ Workflow has validation errors:");
                for err in &result.errors {
                    eprintln!("  - {}", err);
                }
            }
            println!();
        }

        // --dry-run: validate only, do not save
        if dry_run {
            if result.valid {
                eprintln!("✓ Dry-run: workflow is valid (not saved)");
            } else {
                eprintln!("✗ Dry-run: workflow has errors (not saved)");
                std::process::exit(1);
            }
            return Ok(());
        }

        // --yes or non-interactive: skip review prompt
        let save = if yes || !is_interactive() {
            if !result.valid {
                eprintln!("✗ Cannot auto-save — workflow has validation errors.");
                std::process::exit(1);
            }
            true
        } else {
            print!("Save? [yes/no/or type feedback to refine] > ");
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            let line = line.trim().to_string();
            match line.to_lowercase().as_str() {
                "yes" | "y" => {
                    if !result.valid {
                        println!("Cannot save — workflow has validation errors. Provide feedback to fix or type 'no' to discard.");
                        continue;
                    }
                    true
                }
                "no" | "n" => {
                    println!("Discarded.");
                    return Ok(());
                }
                feedback => {
                    println!("\nRefining...\n");
                    result = generator::refine(&result.yaml, feedback)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e))?;
                    continue;
                }
            }
        };

        if save {
            let workflow = r8r::workflow::parse_workflow(&result.yaml)?;
            let storage = get_storage()?;
            let now = chrono::Utc::now();

            if let Some(name) = patch {
                // Update existing workflow
                let stored = storage
                    .get_workflow(name)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;
                let updated = StoredWorkflow {
                    id: stored.id,
                    name: name.to_string(),
                    definition: result.yaml.clone(),
                    enabled: stored.enabled,
                    created_at: stored.created_at,
                    updated_at: now,
                };
                storage.save_workflow(&updated).await?;
                if json_output {
                    println!("{}", serde_json::json!({ "saved": true, "name": name }));
                } else {
                    eprintln!("\n✓ Workflow '{}' updated", name);
                }
            } else {
                let stored = StoredWorkflow {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: workflow.name.clone(),
                    definition: result.yaml.clone(),
                    enabled: true,
                    created_at: now,
                    updated_at: now,
                };
                storage.save_workflow(&stored).await?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::json!({ "saved": true, "name": workflow.name })
                    );
                } else {
                    eprintln!("\n✓ Workflow '{}' saved", workflow.name);
                    eprintln!("Run with: r8r run {}", workflow.name);
                }
            }
            return Ok(());
        }
    }
}

/// `r8r run` — shorthand for `r8r workflows run` with a TTY review screen.
async fn cmd_run(
    name: &str,
    input: Option<&str>,
    params: &[(String, String)],
    yes: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::workflow::{get_parameter_info, merge_params, parse_cli_params, parse_workflow};
    use std::io::{self, BufRead, Write};

    let storage = get_storage()?;

    let stored = storage
        .get_workflow(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Workflow '{}' not found", name))?;

    let workflow = parse_workflow(&stored.definition)?;

    // Check required parameters
    if !workflow.inputs.is_empty() && params.is_empty() && input.is_none() {
        let param_info = get_parameter_info(&workflow);
        let required: Vec<_> = param_info
            .iter()
            .filter(|p| p.required && p.default.is_none())
            .collect();
        if !required.is_empty() {
            eprintln!("Workflow '{}' requires parameters:", name);
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
                eprintln!(
                    "  --param {}=<{}>{}{}  {}",
                    p.name, p.input_type, req, def, p.description
                );
            }
            anyhow::bail!("Missing required parameters");
        }
    }

    // Show review screen on TTY unless --yes
    if !yes && is_interactive() {
        // Identify side-effect nodes (http, agent, sandbox nodes that may call external services)
        let side_effect_types = ["http", "agent", "sandbox"];
        let side_effect_nodes: Vec<_> = workflow
            .nodes
            .iter()
            .filter(|n| side_effect_types.contains(&n.node_type.as_str()))
            .collect();

        println!("  Workflow : {}", workflow.name);
        println!("  Nodes    : {}", workflow.nodes.len());
        if !side_effect_nodes.is_empty() {
            println!(
                "  Side effects: {} node(s) may make external calls ({})",
                side_effect_nodes.len(),
                side_effect_nodes
                    .iter()
                    .map(|n| format!("{} [{}]", n.id, n.node_type))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        } else {
            println!("  Side effects: none detected");
        }
        println!();
        print!("Run? [yes/no] > ");
        io::stdout().flush()?;

        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let line = line.trim().to_lowercase();
        if line != "yes" && line != "y" {
            println!("Cancelled.");
            return Ok(());
        }
        println!();
    }

    // Parse input
    let base_input: serde_json::Value = if let Some(s) = input {
        serde_json::from_str(s)?
    } else {
        serde_json::json!({})
    };
    let cli_params = parse_cli_params(params)?;
    let input_value = merge_params(&base_input, &cli_params);

    if !json_output {
        eprintln!("Running '{}'...", name);
    }

    let registry = build_registry();
    let executor = Executor::new(registry, storage.clone());
    let execution = executor
        .execute(&workflow, &stored.id, "manual", input_value)
        .await?;

    // Fail-closed: non-interactive + waiting_for_approval → exit 42
    if r8r::should_exit_42(&execution.status.to_string(), is_interactive()) {
        eprintln!(
            "Execution {} requires human approval. Exiting with code 42 (non-interactive).",
            execution.id
        );
        std::process::exit(42);
    }

    if json_output {
        let out = serde_json::json!({
            "execution_id": execution.id,
            "status": execution.status.to_string(),
            "error": execution.error,
            "output": execution.output,
            "duration_ms": execution.finished_at.map(|f| (f - execution.started_at).num_milliseconds()),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!();
        println!("Execution ID : {}", execution.id);
        println!("Status       : {}", execution.status);
        if let Some(err) = &execution.error {
            println!("Error        : {}", err);
        }
        if let Some(finished) = execution.finished_at {
            println!(
                "Duration     : {}ms",
                (finished - execution.started_at).num_milliseconds()
            );
        }
        if execution.status.to_string() == "waiting_for_approval" {
            println!();
            println!("⏸  Execution is waiting for approval.");
            println!("   Approve:  r8r workflows run-approve {}", execution.id);
            println!("   View:     r8r runs");
        }
    }

    Ok(())
}

/// `r8r runs` — show recent executions across all (or one) workflow.
async fn cmd_runs(
    workflow: Option<&str>,
    limit: usize,
    status_filter: Option<&str>,
) -> anyhow::Result<()> {
    use r8r::storage::ExecutionQuery;

    let storage = get_storage()?;

    let status = status_filter.and_then(|s| s.parse().ok());

    let query = ExecutionQuery {
        workflow_name: workflow.map(String::from),
        status,
        limit,
        ..Default::default()
    };
    let executions = storage.query_executions(&query).await?;

    if executions.is_empty() {
        println!("No executions found.");
        return Ok(());
    }

    println!(
        "{:<36} {:<24} {:<12} {:<10} {:<20}",
        "EXECUTION ID", "WORKFLOW", "STATUS", "TRIGGER", "STARTED"
    );
    println!("{}", "─".repeat(105));

    for exec in &executions {
        println!(
            "{:<36} {:<24} {:<12} {:<10} {:<20}",
            exec.id,
            truncate(&exec.workflow_name, 23),
            exec.status.to_string(),
            exec.trigger_type,
            exec.started_at.format("%Y-%m-%d %H:%M:%S"),
        );
    }

    println!();
    println!("{} execution(s) shown", executions.len());

    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Find the largest char boundary <= max to avoid splitting multi-byte chars
    let mut boundary = max;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

/// `r8r doctor` — verify DB, config, credentials, and node registry.
async fn cmd_doctor() -> anyhow::Result<()> {
    let mut pass = 0usize;
    let mut fail = 0usize;

    println!("r8r doctor\n");

    // Helper closure to print result
    let mut report = |label: &str, result: Result<String, String>| match result {
        Ok(msg) => {
            println!("  ✓ {:<30} {}", label, msg);
            pass += 1;
        }
        Err(e) => {
            println!("  ✗ {:<30} {}", label, e);
            fail += 1;
        }
    };

    // 1. Database connectivity
    let db_result = async {
        let storage = get_storage().map_err(|e| e.to_string())?;
        let workflows = storage.list_workflows().await.map_err(|e| e.to_string())?;
        Ok::<String, String>(format!("{} workflow(s) stored", workflows.len()))
    }
    .await;
    report("Database", db_result);

    // 2. LLM config
    let llm_result = {
        let base_url = std::env::var("R8R_LLM_BASE_URL")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("R8R_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        let api_key = std::env::var("R8R_LLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_default();
        if api_key.is_empty() {
            Err("R8R_LLM_API_KEY / OPENAI_API_KEY not set".to_string())
        } else {
            Ok(format!("model={} endpoint={}", model, base_url))
        }
    };
    report("LLM config", llm_result);

    // 3. Credentials
    let cred_result = async {
        use r8r::credentials::CredentialStore;
        let store = CredentialStore::load().await.map_err(|e| e.to_string())?;
        let creds = store.list();
        Ok::<String, String>(format!("{} configured", creds.len()))
    }
    .await;
    report("Credentials", cred_result);

    // 4. Node registry
    let reg_result = {
        let registry = build_registry();
        // Count registered node types by probing known types
        let known = [
            "http",
            "transform",
            "agent",
            "debug",
            "wait",
            "switch",
            "filter",
            "sort",
            "limit",
            "set",
            "aggregate",
            "split",
            "crypto",
            "datetime",
            "dedupe",
            "summarize",
            "if",
            "subworkflow",
            "variables",
            "template",
            "approval",
            "sandbox",
        ];
        let count = known.iter().filter(|t| registry.has(t)).count();
        Ok::<String, String>(format!("{} node type(s) registered", count))
    };
    report("Node registry", reg_result);

    println!();
    if fail == 0 {
        println!("All {} checks passed.", pass);
    } else {
        println!("{} passed, {} failed.", pass, fail);
        std::process::exit(1);
    }

    Ok(())
}

// ─── Task #2: r8r test ────────────────────────────────────────────────────────

/// Fixture file format: list of test cases.
#[derive(Debug, serde::Deserialize)]
struct TestFixture {
    /// Path to workflow YAML file (relative to fixture). Overridden per-test.
    workflow: Option<String>,
    /// Inline workflow YAML. Overridden per-test.
    workflow_yaml: Option<String>,
    /// Test cases.
    tests: Vec<TestCase>,
}

#[derive(Debug, serde::Deserialize)]
struct TestCase {
    /// Human-readable test name.
    name: String,
    /// Workflow YAML file path (overrides top-level).
    workflow: Option<String>,
    /// Inline workflow YAML (overrides top-level).
    workflow_yaml: Option<String>,
    /// Input data passed to the workflow.
    #[serde(default)]
    input: serde_json::Value,
    /// Node ID → pinned output (mock). Injected as pinned_data on matching nodes.
    #[serde(default)]
    mocks: std::collections::HashMap<String, serde_json::Value>,
    /// Expected output for assertion. If absent, test only checks for no failure.
    expected: Option<serde_json::Value>,
    /// Expected error substring. If set, test asserts the workflow fails with this message.
    expected_error: Option<String>,
    /// Comparison mode: "exact" (default) or "contains".
    #[serde(default)]
    mode: Option<String>,
}

struct TestResult {
    name: String,
    passed: bool,
    duration_ms: u64,
    message: Option<String>,
}

async fn cmd_test(
    file: &str,
    filter: Option<&str>,
    junit_xml: Option<&str>,
    update_snapshots: bool,
) -> anyhow::Result<()> {
    use r8r::engine::Executor;
    use r8r::storage::SqliteStorage;
    use r8r::workflow::parse_workflow;
    use std::path::Path;

    let fixture_path = Path::new(file);
    if !fixture_path.exists() {
        anyhow::bail!("Test fixture not found: {}\n\nCreate one with:\n  tests:\n    - name: \"my test\"\n      workflow: ./workflow.yaml\n      input: {{}}\n      expected: {{}}", file);
    }

    let fixture_dir = fixture_path.parent().unwrap_or(Path::new("."));
    let raw = std::fs::read_to_string(fixture_path)?;
    let fixture: TestFixture = serde_yaml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse test fixture: {}", e))?;

    let cases: Vec<&TestCase> = fixture
        .tests
        .iter()
        .filter(|t| filter.map(|f| t.name.contains(f)).unwrap_or(true))
        .collect();

    if cases.is_empty() {
        if let Some(f) = filter {
            println!("No tests matched filter '{}'", f);
        } else {
            println!("No tests found in {}", file);
        }
        return Ok(());
    }

    println!("Running {} test(s) from {}\n", cases.len(), file);

    let mut results: Vec<TestResult> = Vec::new();

    for case in &cases {
        let start = std::time::Instant::now();

        // Resolve workflow YAML
        let yaml = match (
            case.workflow_yaml.as_ref(),
            case.workflow.as_ref(),
            fixture.workflow_yaml.as_ref(),
            fixture.workflow.as_ref(),
        ) {
            (Some(y), _, _, _) => y.clone(),
            (_, Some(p), _, _) => {
                let path = fixture_dir.join(p);
                std::fs::read_to_string(&path).map_err(|e| {
                    anyhow::anyhow!("Cannot read workflow '{}': {}", path.display(), e)
                })?
            }
            (_, _, Some(y), _) => y.clone(),
            (_, _, _, Some(p)) => {
                let path = fixture_dir.join(p);
                std::fs::read_to_string(&path).map_err(|e| {
                    anyhow::anyhow!("Cannot read workflow '{}': {}", path.display(), e)
                })?
            }
            _ => anyhow::bail!("Test '{}' has no workflow source", case.name),
        };

        // Parse and inject mocks as pinned_data
        let mut workflow = match parse_workflow(&yaml) {
            Ok(w) => w,
            Err(e) => {
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    message: Some(format!("Workflow parse error: {}", e)),
                });
                println!("  FAIL {}", case.name);
                println!("       Workflow parse error: {}", e);
                continue;
            }
        };

        for node in &mut workflow.nodes {
            if let Some(mock_output) = case.mocks.get(&node.id) {
                node.pinned_data = Some(mock_output.clone());
            }
        }

        // Run in isolated in-memory storage
        let test_storage: std::sync::Arc<dyn r8r::storage::Storage> =
            match SqliteStorage::open_in_memory() {
                Ok(s) => std::sync::Arc::new(s),
                Err(e) => anyhow::bail!("Cannot create test storage: {}", e),
            };
        let registry = build_registry();
        let executor = Executor::new(registry, std::sync::Arc::clone(&test_storage));

        let stored_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        test_storage
            .save_workflow(&r8r::storage::StoredWorkflow {
                id: stored_id.clone(),
                name: workflow.name.clone(),
                definition: yaml.clone(),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await?;

        let execution = match executor
            .execute(&workflow, &stored_id, "test", case.input.clone())
            .await
        {
            Ok(e) => e,
            Err(e) => {
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    message: Some(format!("Executor error: {}", e)),
                });
                println!("  FAIL {} ({}ms)", case.name, start.elapsed().as_millis());
                println!("       {}", e);
                continue;
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let failed = execution.status.to_string() == "failed";

        // Handle expected_error cases
        if let Some(expected_err) = &case.expected_error {
            let actual_err = execution.error.as_deref().unwrap_or("");
            if failed && actual_err.contains(expected_err.as_str()) {
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: true,
                    duration_ms,
                    message: None,
                });
                println!("  PASS {} ({}ms)", case.name, duration_ms);
            } else if failed {
                let msg = format!("Expected error '{}', got '{}'", expected_err, actual_err);
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: false,
                    duration_ms,
                    message: Some(msg.clone()),
                });
                println!("  FAIL {} ({}ms)", case.name, duration_ms);
                println!("       {}", msg);
            } else {
                let msg = format!(
                    "Expected workflow to fail with '{}', but it succeeded",
                    expected_err
                );
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: false,
                    duration_ms,
                    message: Some(msg.clone()),
                });
                println!("  FAIL {} ({}ms)", case.name, duration_ms);
                println!("       {}", msg);
            }
            continue;
        }

        // Handle normal failure
        if failed {
            let msg = format!(
                "Workflow failed: {}",
                execution.error.as_deref().unwrap_or("unknown")
            );
            results.push(TestResult {
                name: case.name.clone(),
                passed: false,
                duration_ms,
                message: Some(msg.clone()),
            });
            println!("  FAIL {} ({}ms)", case.name, duration_ms);
            println!("       {}", msg);
            continue;
        }

        let actual = execution.output.clone().unwrap_or(serde_json::json!(null));

        // --update-snapshots: write expected to fixture (in-memory update only, inform user)
        if update_snapshots {
            println!(
                "  SNAP {} — output: {}",
                case.name,
                serde_json::to_string_pretty(&actual).unwrap_or_default()
            );
            results.push(TestResult {
                name: case.name.clone(),
                passed: true,
                duration_ms,
                message: Some("snapshot updated (add to fixture manually)".to_string()),
            });
            continue;
        }

        // Assert output if expected is set
        if let Some(expected) = &case.expected {
            let mode = case.mode.as_deref().unwrap_or("exact");
            let diffs = cli_json_diff(&actual, expected, mode);
            if diffs.is_empty() {
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: true,
                    duration_ms,
                    message: None,
                });
                println!("  PASS {} ({}ms)", case.name, duration_ms);
            } else {
                let msg = diffs.join("\n       ");
                results.push(TestResult {
                    name: case.name.clone(),
                    passed: false,
                    duration_ms,
                    message: Some(diffs.join("; ")),
                });
                println!("  FAIL {} ({}ms)", case.name, duration_ms);
                println!("       {}", msg);
            }
        } else {
            // No expected — just check no failure
            results.push(TestResult {
                name: case.name.clone(),
                passed: true,
                duration_ms,
                message: None,
            });
            println!("  PASS {} ({}ms)", case.name, duration_ms);
        }
    }

    let passed = results.iter().filter(|r| r.passed).count();
    let failed_count = results.len() - passed;
    let total_ms: u64 = results.iter().map(|r| r.duration_ms).sum();

    println!();
    println!(
        "{} passed, {} failed, {} total ({:.2}s)",
        passed,
        failed_count,
        results.len(),
        total_ms as f64 / 1000.0
    );

    // JUnit XML output
    if let Some(xml_path) = junit_xml {
        let xml = build_junit_xml(&results, file);
        std::fs::write(xml_path, xml)?;
        println!("JUnit XML written to {}", xml_path);
    }

    if failed_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

/// Minimal JSON diff for CLI test runner (mirrors MCP json_diff).
fn cli_json_diff(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    mode: &str,
) -> Vec<String> {
    let mut diffs = Vec::new();
    cli_json_diff_inner(actual, expected, mode, "", &mut diffs);
    diffs
}

fn cli_json_diff_inner(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    mode: &str,
    path: &str,
    diffs: &mut Vec<String>,
) {
    match (actual, expected) {
        (serde_json::Value::Object(a_map), serde_json::Value::Object(e_map)) => {
            for (key, e_val) in e_map {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                if let Some(a_val) = a_map.get(key) {
                    cli_json_diff_inner(a_val, e_val, mode, &child, diffs);
                } else {
                    diffs.push(format!("missing key '{}': expected {}", child, e_val));
                }
            }
            if mode == "exact" {
                for key in a_map.keys() {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };
                    if !e_map.contains_key(key) {
                        diffs.push(format!("unexpected key '{}' in output", child));
                    }
                }
            }
        }
        (a, e) => {
            if a != e {
                let p = if path.is_empty() {
                    "(root)".to_string()
                } else {
                    path.to_string()
                };
                diffs.push(format!("at '{}': expected {} got {}", p, e, a));
            }
        }
    }
}

fn build_junit_xml(results: &[TestResult], suite_name: &str) -> String {
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    let total_ms: u64 = results.iter().map(|r| r.duration_ms).sum();

    // Escape XML attribute values: & < > " '
    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="{}" tests="{}" failures="{}" time="{:.3}">
"#,
        xml_escape(suite_name),
        results.len(),
        failed,
        total_ms as f64 / 1000.0
    );

    for r in results {
        let name_escaped = xml_escape(&r.name);
        if r.passed {
            xml.push_str(&format!(
                "  <testcase name=\"{}\" time=\"{:.3}\"/>\n",
                name_escaped,
                r.duration_ms as f64 / 1000.0
            ));
        } else {
            let msg = r.message.as_deref().unwrap_or("assertion failed");
            xml.push_str(&format!(
                "  <testcase name=\"{}\" time=\"{:.3}\">\n    <failure message=\"{}\"/>\n  </testcase>\n",
                name_escaped,
                r.duration_ms as f64 / 1000.0,
                xml_escape(msg)
            ));
        }
    }

    xml.push_str("</testsuite>\n");
    xml
}

// ─── Task #3: r8r lint ────────────────────────────────────────────────────────

async fn cmd_lint(files: &[String], errors_only: bool, json_output: bool) -> anyhow::Result<()> {
    use r8r::workflow::{parse_workflow, validate_workflow};
    use std::path::Path;

    let mut any_error = false;
    let mut all_results: Vec<serde_json::Value> = Vec::new();

    for file in files {
        let path = Path::new(file);
        if !path.exists() {
            if json_output {
                all_results.push(serde_json::json!({ "file": file, "valid": false, "issues": [{"level": "error", "type": "io", "message": "File not found"}] }));
            } else {
                eprintln!("{}: file not found", file);
            }
            any_error = true;
            continue;
        }

        let yaml = std::fs::read_to_string(path)?;
        let mut issues: Vec<serde_json::Value> = Vec::new();

        let workflow = match parse_workflow(&yaml) {
            Ok(w) => w,
            Err(e) => {
                let err_str = e.to_string();
                let mut issue = serde_json::json!({
                    "level": "error",
                    "type": "parse",
                    "message": err_str,
                });
                // Common mistake suggestions
                if err_str.contains("dependsOn") {
                    issue["suggestion"] = serde_json::json!("Use 'depends_on' (snake_case)");
                } else if err_str.contains("nodeType") {
                    issue["suggestion"] =
                        serde_json::json!("Use 'type' for node type, not 'nodeType'");
                }
                issues.push(issue);
                let result = serde_json::json!({ "file": file, "valid": false, "issues": issues });
                if json_output {
                    all_results.push(result);
                } else {
                    println!("{}: ✗ parse error", file);
                    for iss in &issues {
                        println!("  error  {}", iss["message"].as_str().unwrap_or(""));
                        if let Some(s) = iss.get("suggestion") {
                            println!("  hint   {}", s.as_str().unwrap_or(""));
                        }
                    }
                }
                any_error = true;
                continue;
            }
        };

        // Structural validation
        if let Err(e) = validate_workflow(&workflow) {
            issues.push(serde_json::json!({ "level": "error", "type": "validation", "message": e.to_string() }));
        }

        // Lint warnings
        if !errors_only {
            if workflow.description.is_empty() {
                issues.push(serde_json::json!({
                    "level": "warning", "type": "lint",
                    "message": "No 'description' field — add one for better discoverability"
                }));
            }
            if workflow.triggers.is_empty() {
                issues.push(serde_json::json!({
                    "level": "warning", "type": "lint",
                    "message": "No triggers defined — workflow can only be run via API or MCP"
                }));
            }
            let has_deps = workflow.nodes.iter().any(|n| !n.depends_on.is_empty());
            if workflow.nodes.len() > 1 && !has_deps {
                issues.push(serde_json::json!({
                    "level": "warning", "type": "lint",
                    "message": "Multiple nodes but none have 'depends_on' — add explicit dependencies for clarity"
                }));
            }
            // Warn on nodes with no id-referenced dependencies that look like chains
            for node in &workflow.nodes {
                if node.node_type == "agent" && node.config.get("prompt").is_none() {
                    issues.push(serde_json::json!({
                        "level": "warning", "type": "lint",
                        "message": format!("Agent node '{}' has no 'prompt' field", node.id)
                    }));
                }
            }
        }

        let has_errors = issues.iter().any(|i| i["level"] == "error");
        if has_errors {
            any_error = true;
        }

        let result = serde_json::json!({
            "file": file,
            "valid": !has_errors,
            "name": workflow.name,
            "nodes": workflow.nodes.len(),
            "issues": issues,
        });

        if json_output {
            all_results.push(result);
        } else {
            let status = if has_errors {
                "✗"
            } else if issues.is_empty() {
                "✓"
            } else {
                "⚠"
            };
            println!(
                "{}: {} {} ({} nodes)",
                status,
                file,
                workflow.name,
                workflow.nodes.len()
            );
            for iss in &issues {
                let level = iss["level"].as_str().unwrap_or("info");
                let msg = iss["message"].as_str().unwrap_or("");
                println!("  {:<8} {}", level, msg);
                if let Some(s) = iss.get("suggestion") {
                    println!("  {:<8} hint: {}", "", s.as_str().unwrap_or(""));
                }
            }
        }
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(all_results))?
        );
    }

    if any_error {
        std::process::exit(1);
    }

    Ok(())
}

// ─── r8r schema ───────────────────────────────────────────────────────────────

async fn cmd_schema(output: Option<&str>) -> anyhow::Result<()> {
    let schema = r8r::schema::workflow_schema_json();
    match output {
        Some(path) => {
            std::fs::write(path, schema)?;
            println!("✓ Schema written to {}", path);
            println!();
            println!("VSCode: add to .vscode/settings.json:");
            println!("  \"yaml.schemas\": {{");
            println!("    \"{}\": \"workflows/**/*.yaml\"", path);
            println!("  }}");
            println!();
            println!("Or reference inline at the top of a workflow file:");
            println!("  # yaml-language-server: $schema={}", path);
        }
        None => {
            println!("{}", schema);
        }
    }
    Ok(())
}

// ─── Sprint 2: r8r runs subcommands ──────────────────────────────────────────

async fn cmd_runs_pending(limit: usize) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let approvals = storage
        .list_approval_requests(Some("pending"), limit, 0)
        .await?;

    if approvals.is_empty() {
        println!("No pending approval requests.");
        return Ok(());
    }

    println!(
        "{:<36} {:<24} {:<22} {:<20}",
        "APPROVAL ID", "WORKFLOW", "ASSIGNEE", "CREATED"
    );
    println!("{}", "─".repeat(104));
    for a in &approvals {
        let assignee_display = match (&a.assignee, a.groups.as_slice()) {
            (Some(user), _) => truncate(user, 21).to_string(),
            (None, groups) if !groups.is_empty() => {
                format!("[{}]", groups.join(", "))
            }
            _ => "(any)".to_string(),
        };
        println!(
            "{:<36} {:<24} {:<22} {:<20}",
            a.id,
            truncate(&a.workflow_name, 23),
            truncate(&assignee_display, 21),
            a.created_at.format("%Y-%m-%d %H:%M:%S"),
        );
    }
    println!();
    println!("{} pending approval(s)", approvals.len());
    println!("Approve with: r8r approve <approval-id-or-execution-id>");
    Ok(())
}

async fn cmd_runs_show(run_id: &str, include_trace: bool) -> anyhow::Result<()> {
    let storage = get_storage()?;

    let exec = storage
        .get_execution(run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Execution not found: {}", run_id))?;

    println!("Execution  : {}", exec.id);
    println!("Workflow   : {}", exec.workflow_name);
    println!("Status     : {}", exec.status);
    println!("Trigger    : {}", exec.trigger_type);
    println!(
        "Started    : {}",
        exec.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    if let Some(fin) = exec.finished_at {
        let dur = (fin - exec.started_at).num_milliseconds();
        println!(
            "Finished   : {} ({}ms)",
            fin.format("%Y-%m-%d %H:%M:%S UTC"),
            dur
        );
    }
    if let Some(err) = &exec.error {
        println!("Error      : {}", err);
    }
    if let Some(origin) = &exec.origin {
        println!("Origin     : {}", origin);
    }

    if exec.input != serde_json::json!({}) {
        println!();
        println!("Input:");
        println!("{}", serde_json::to_string_pretty(&exec.input)?);
    }

    if let Some(output) = &exec.output {
        println!();
        println!("Output:");
        println!("{}", serde_json::to_string_pretty(output)?);
    }

    // Show pending approvals for this execution (filter by status at storage level)
    let approvals = storage
        .list_approval_requests(Some("pending"), 20, 0)
        .await?;
    let pending: Vec<_> = approvals
        .iter()
        .filter(|a| a.execution_id == run_id)
        .collect();
    if !pending.is_empty() {
        println!();
        println!("Pending approvals:");
        for a in &pending {
            println!("  {} — {} (approval ID: {})", a.title, a.status, a.id);
            if let Some(desc) = &a.description {
                println!("    {}", desc);
            }
        }
        println!("  Approve:  r8r approve {}", run_id);
        println!("  Deny:     r8r deny {}", run_id);
    }

    if include_trace {
        let nodes = storage.get_node_executions(run_id).await?;
        if nodes.is_empty() {
            println!("\nNo node trace recorded.");
        } else {
            println!();
            println!("{:<24} {:<12} {:<8} ERROR", "NODE", "STATUS", "MS");
            println!("{}", "─".repeat(72));
            for n in &nodes {
                let ms = n
                    .finished_at
                    .map(|f| (f - n.started_at).num_milliseconds())
                    .unwrap_or(0);
                let err = n.error.as_deref().unwrap_or("");
                println!(
                    "{:<24} {:<12} {:<8} {}",
                    truncate(&n.node_id, 23),
                    n.status.to_string(),
                    ms,
                    err
                );
            }
        }
    }

    Ok(())
}

async fn cmd_runs_export(
    run_id: &str,
    format: &str,
    output_path: Option<&str>,
) -> anyhow::Result<()> {
    use std::io::Write;

    let storage = get_storage()?;

    let exec = storage
        .get_execution(run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Execution not found: {}", run_id))?;

    let nodes = storage.get_node_executions(run_id).await?;

    let lines: Vec<String> = if format == "json" {
        // Single JSON object
        let obj = serde_json::json!({
            "execution": exec,
            "nodes": nodes,
        });
        vec![serde_json::to_string_pretty(&obj)?]
    } else {
        // JSONL: one object per line
        let mut out = vec![serde_json::to_string(&serde_json::json!({
            "type": "execution",
            "id": exec.id,
            "workflow_name": exec.workflow_name,
            "status": exec.status.to_string(),
            "trigger_type": exec.trigger_type,
            "started_at": exec.started_at.to_rfc3339(),
            "finished_at": exec.finished_at.map(|t| t.to_rfc3339()),
            "error": exec.error,
            "input": exec.input,
            "output": exec.output,
        }))?];
        for n in &nodes {
            out.push(serde_json::to_string(&serde_json::json!({
                "type": "node_execution",
                "execution_id": n.execution_id,
                "node_id": n.node_id,
                "status": n.status.to_string(),
                "started_at": n.started_at.to_rfc3339(),
                "finished_at": n.finished_at.map(|t| t.to_rfc3339()),
                "error": n.error,
                "output": n.output,
            }))?);
        }
        out
    };

    let content = lines.join("\n") + "\n";

    match output_path {
        Some(path) => {
            std::fs::write(path, &content)?;
            println!("Exported {} to {}", run_id, path);
        }
        None => {
            std::io::stdout().write_all(content.as_bytes())?;
        }
    }

    Ok(())
}

// ─── Sprint 2: r8r approve / r8r deny ────────────────────────────────────────

async fn cmd_approve_deny(
    id: &str,
    decision: &str,
    comment: Option<&str>,
    decided_by: &str,
) -> anyhow::Result<()> {
    use r8r::engine::Executor;

    let storage = get_storage()?;

    // Look up approval by approval_id directly, or by execution_id
    let approval = {
        let by_id = storage.get_approval_request(id).await?;
        if let Some(a) = by_id {
            a
        } else {
            // Try to find pending approval for this execution_id
            let all = storage
                .list_approval_requests(Some("pending"), 100, 0)
                .await?;
            all.into_iter()
                .find(|a| a.execution_id == id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No pending approval found for '{}'. Use `r8r runs pending` to list them.",
                        id
                    )
                })?
        }
    };

    if approval.status != "pending" {
        anyhow::bail!(
            "Approval {} is already '{}', cannot {}",
            approval.id,
            approval.status,
            decision
        );
    }

    // Inject decision into checkpoint
    let mut checkpoint = storage
        .get_latest_checkpoint(&approval.execution_id)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No checkpoint found for execution {}",
                approval.execution_id
            )
        })?;

    let mut outputs = checkpoint
        .node_outputs
        .as_object()
        .cloned()
        .unwrap_or_default();

    // Inject the approval decision into the checkpoint node output
    let node_id = {
        let approved_status = if decision == "approve" {
            "approved"
        } else {
            "rejected"
        };
        let hint = if approval.node_id.is_empty() {
            None
        } else {
            Some(approval.node_id.as_str())
        };
        // Find the node by hint or by scanning for matching approval_id
        let found = hint
            .filter(|id| outputs.contains_key(*id))
            .map(ToOwned::to_owned)
            .or_else(|| {
                outputs.iter().find_map(|(nid, out)| {
                    out.get("approval_id")
                        .and_then(|v| v.as_str())
                        .filter(|&aid| aid == approval.id.as_str())
                        .map(|_| nid.clone())
                })
            });
        let nid = found.ok_or_else(|| {
            anyhow::anyhow!("Approval {} not found in checkpoint outputs", approval.id)
        })?;
        let existing = outputs.get(&nid).cloned().unwrap_or(serde_json::json!({}));
        let mut obj = existing.as_object().cloned().unwrap_or_default();
        obj.insert("approval_id".into(), serde_json::json!(approval.id));
        obj.insert("status".into(), serde_json::json!(approved_status));
        obj.insert("decision".into(), serde_json::json!(decision));
        obj.insert(
            "comment".into(),
            comment
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        );
        outputs.insert(nid.clone(), serde_json::Value::Object(obj));
        nid
    };

    checkpoint.node_outputs = serde_json::Value::Object(outputs);
    storage.save_checkpoint(&checkpoint).await?;

    let decided_status = if decision == "approve" {
        "approved"
    } else {
        "rejected"
    };
    let mut updated = approval.clone();
    updated.status = decided_status.to_string();
    updated.node_id = node_id;
    updated.decision_comment = comment.map(String::from);
    updated.decided_by = Some(decided_by.to_string());
    updated.decided_at = Some(chrono::Utc::now());
    storage.save_approval_request(&updated).await?;

    println!(
        "{} approval {} ({})",
        if decision == "approve" {
            "✓ Approved"
        } else {
            "✗ Denied"
        },
        updated.id,
        updated.workflow_name
    );
    if let Some(c) = comment {
        println!("  Comment: {}", c);
    }

    // Resume execution
    let registry = build_registry();
    let executor = Executor::new(registry, storage);
    match executor.resume_from_checkpoint(&updated.execution_id).await {
        Ok(execution) => {
            println!();
            println!("Execution  : {}", execution.id);
            println!("Status     : {}", execution.status);
            if let Some(err) = &execution.error {
                println!("Error      : {}", err);
            }
            if let Some(output) = &execution.output {
                println!("Output     : {}", serde_json::to_string_pretty(output)?);
            }
        }
        Err(e) => {
            eprintln!("⚠ Decision recorded but failed to resume execution: {}", e);
            eprintln!("  The execution may need manual intervention.");
        }
    }

    Ok(())
}

// ─── Sprint 2: r8r policy ─────────────────────────────────────────────────────

/// Policy knobs that can be enforced at run/prompt time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PolicyConfig {
    /// Active profile name: lenient, standard, or strict.
    profile: String,
    /// Require review before saving a patch (--patch) even with --yes.
    require_review_on_patch: bool,
    /// Require an idempotency key for every `r8r run` invocation.
    require_idempotency_key_for_run: bool,
    /// Maximum allowed workflow runtime in seconds (0 = unlimited).
    max_runtime_seconds: u64,
    /// Reject workflows that contain unknown node types.
    deny_unknown_node_types: bool,
}

impl PolicyConfig {
    fn lenient() -> Self {
        Self {
            profile: "lenient".into(),
            require_review_on_patch: false,
            require_idempotency_key_for_run: false,
            max_runtime_seconds: 0,
            deny_unknown_node_types: false,
        }
    }

    fn standard() -> Self {
        Self {
            profile: "standard".into(),
            require_review_on_patch: true,
            require_idempotency_key_for_run: false,
            max_runtime_seconds: 3600,
            deny_unknown_node_types: true,
        }
    }

    fn strict() -> Self {
        Self {
            profile: "strict".into(),
            require_review_on_patch: true,
            require_idempotency_key_for_run: true,
            max_runtime_seconds: 600,
            deny_unknown_node_types: true,
        }
    }
}

fn policy_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("R8R_POLICY_FILE") {
        return std::path::PathBuf::from(p);
    }
    let base = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("r8r").join("policy.toml")
}

fn load_policy() -> PolicyConfig {
    let path = policy_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        match toml::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Warning: policy file {} is malformed ({}); falling back to lenient profile.",
                    path.display(),
                    e
                );
                PolicyConfig::lenient()
            }
        }
    } else {
        PolicyConfig::lenient()
    }
}

fn save_policy(policy: &PolicyConfig) -> anyhow::Result<()> {
    let path = policy_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(policy)?;
    std::fs::write(&path, content)?;
    Ok(())
}

async fn cmd_policy_show() -> anyhow::Result<()> {
    let policy = load_policy();
    let path = policy_path();

    println!("Active policy: {}", policy.profile.to_uppercase());
    println!("Config file  : {}", path.display());
    println!();
    println!(
        "  {:<40} {}",
        "require_review_on_patch", policy.require_review_on_patch
    );
    println!(
        "  {:<40} {}",
        "require_idempotency_key_for_run", policy.require_idempotency_key_for_run
    );
    println!(
        "  {:<40} {}",
        "max_runtime_seconds (0=unlimited)", policy.max_runtime_seconds
    );
    println!(
        "  {:<40} {}",
        "deny_unknown_node_types", policy.deny_unknown_node_types
    );
    println!();
    println!("Available profiles: lenient  standard  strict");
    println!("Switch with: r8r policy set <profile>");
    Ok(())
}

async fn cmd_policy_set(profile: &str) -> anyhow::Result<()> {
    let policy = match profile {
        "lenient" => PolicyConfig::lenient(),
        "standard" => PolicyConfig::standard(),
        "strict" => PolicyConfig::strict(),
        other => anyhow::bail!(
            "Unknown profile '{}'. Valid options: lenient, standard, strict",
            other
        ),
    };
    save_policy(&policy)?;
    println!("✓ Policy set to '{}'", profile);
    println!(
        "  require_review_on_patch          = {}",
        policy.require_review_on_patch
    );
    println!(
        "  require_idempotency_key_for_run  = {}",
        policy.require_idempotency_key_for_run
    );
    println!(
        "  max_runtime_seconds              = {}",
        policy.max_runtime_seconds
    );
    println!(
        "  deny_unknown_node_types          = {}",
        policy.deny_unknown_node_types
    );
    Ok(())
}

async fn cmd_policy_validate(file: &str) -> anyhow::Result<()> {
    use r8r::workflow::parse_workflow;
    use std::path::Path;

    let policy = load_policy();
    let path = Path::new(file);
    if !path.exists() {
        anyhow::bail!("File not found: {}", file);
    }

    let yaml = std::fs::read_to_string(path)?;
    let workflow = parse_workflow(&yaml).map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;

    let registry = build_registry();
    let mut violations: Vec<String> = Vec::new();

    // max_runtime check
    if policy.max_runtime_seconds > 0 {
        let timeout = workflow.settings.timeout_seconds;
        if timeout > policy.max_runtime_seconds {
            violations.push(format!(
                "timeout_seconds {} exceeds policy max_runtime_seconds {}",
                timeout, policy.max_runtime_seconds
            ));
        }
    }

    // deny_unknown_node_types check
    if policy.deny_unknown_node_types {
        for node in &workflow.nodes {
            if !registry.has(&node.node_type) {
                violations.push(format!(
                    "node '{}' has unknown type '{}'",
                    node.id, node.node_type
                ));
            }
        }
    }

    if violations.is_empty() {
        println!("✓ '{}' passes {} policy", workflow.name, policy.profile);
    } else {
        println!("✗ '{}' violates {} policy:", workflow.name, policy.profile);
        for v in &violations {
            println!("  - {}", v);
        }
        std::process::exit(1);
    }

    Ok(())
}
