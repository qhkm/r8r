//! Input parsing for the REPL.

/// Parsed user input.
#[derive(Debug, Clone, PartialEq)]
pub enum InputCommand {
    /// Execute a workflow: /run <name>
    Run { workflow: String },
    /// List workflows: /list
    List,
    /// Show workflow YAML: /show <name>
    Show { workflow: String },
    /// Show workflow DAG: /dag <name>
    Dag { workflow: String },
    /// Show execution logs: /logs <name>
    Logs { workflow: String },
    /// Show execution trace: /trace <id>
    Trace { execution_id: String },
    /// List sessions: /sessions
    Sessions,
    /// Resume a session: /resume [id]
    Resume { session_id: Option<String> },
    /// Clear conversation: /clear
    Clear,
    /// Reconnect to LLM: /reconnect
    Reconnect,
    /// Set model/provider: /model <model> or /model <provider> <model>
    SetModel {
        provider: Option<String>,
        model: String,
    },
    /// Set API key for the current session: /apikey <key>
    SetApiKey { api_key: String },
    /// Set endpoint for the current session: /endpoint <url>
    SetEndpoint { endpoint: String },
    /// Show current LLM config: /llm
    ShowLlm,
    /// Directly switch inspector/context view: /view <log|tools|yaml|dag|trace|context>
    View { target: String },
    /// Export currently loaded/generated YAML to a file: /export yaml <path>
    ExportYaml { path: String },
    /// Show help: /help
    Help,
    /// Exit REPL: /exit
    Exit,
    /// Natural language input.
    NaturalLanguage(String),
    /// Plan a workflow before execution: /plan [description]
    Plan { description: Option<String> },
    /// Arm write operations for the session: /arm [writes]
    ArmWrites,
    /// Disarm write operations (revert to safe mode): /disarm
    Disarm,
    /// Toggle operator mode: /operator [on|off]
    Operator { enable: Option<bool> },
    /// Unknown slash command.
    Unknown(String),
    /// Empty input.
    Empty,
}

/// Parse raw user input into an InputCommand.
pub fn parse_input(input: &str) -> InputCommand {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return InputCommand::Empty;
    }

    if !trimmed.starts_with('/') {
        return InputCommand::NaturalLanguage(trimmed.to_string());
    }

    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
    let command = parts[0].to_lowercase();
    let arg = parts.get(1).map(|s| s.trim().to_string());

    match command.as_str() {
        "/run" => match arg {
            Some(workflow) if !workflow.is_empty() => InputCommand::Run { workflow },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/list" => InputCommand::List,
        "/show" => match arg {
            Some(workflow) if !workflow.is_empty() => InputCommand::Show { workflow },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/dag" => match arg {
            Some(workflow) if !workflow.is_empty() => InputCommand::Dag { workflow },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/logs" => match arg {
            Some(workflow) if !workflow.is_empty() => InputCommand::Logs { workflow },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/trace" => match arg {
            Some(execution_id) if !execution_id.is_empty() => InputCommand::Trace { execution_id },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/sessions" => InputCommand::Sessions,
        "/resume" => InputCommand::Resume { session_id: arg },
        "/clear" => InputCommand::Clear,
        "/reconnect" => InputCommand::Reconnect,
        "/model" => {
            let args: Vec<&str> = trimmed.split_whitespace().skip(1).collect();
            match args.as_slice() {
                [model] => InputCommand::SetModel {
                    provider: None,
                    model: (*model).to_string(),
                },
                [provider, model, ..] => InputCommand::SetModel {
                    provider: Some((*provider).to_string()),
                    model: (*model).to_string(),
                },
                _ => InputCommand::Unknown(trimmed.to_string()),
            }
        }
        "/apikey" => match arg {
            Some(api_key) if !api_key.is_empty() => InputCommand::SetApiKey { api_key },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/endpoint" => match arg {
            Some(endpoint) if !endpoint.is_empty() => InputCommand::SetEndpoint { endpoint },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/llm" => InputCommand::ShowLlm,
        "/view" => match arg {
            Some(target) if !target.is_empty() => InputCommand::View {
                target: target.to_lowercase(),
            },
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/export" => match arg {
            Some(rest) if !rest.is_empty() => {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                match parts.as_slice() {
                    [kind, path]
                        if kind.eq_ignore_ascii_case("yaml") && !path.trim().is_empty() =>
                    {
                        InputCommand::ExportYaml {
                            path: path.trim().to_string(),
                        }
                    }
                    _ => InputCommand::Unknown(trimmed.to_string()),
                }
            }
            _ => InputCommand::Unknown(trimmed.to_string()),
        },
        "/help" => InputCommand::Help,
        "/exit" | "/quit" | "/q" => InputCommand::Exit,
        "/plan" => InputCommand::Plan {
            description: arg.filter(|s| !s.is_empty()),
        },
        "/arm" => InputCommand::ArmWrites,
        "/disarm" => InputCommand::Disarm,
        "/operator" => {
            let enable = arg
                .as_deref()
                .map(|s| matches!(s.to_lowercase().as_str(), "on" | "true" | "1"));
            InputCommand::Operator { enable }
        }
        _ => InputCommand::Unknown(trimmed.to_string()),
    }
}

/// Get available slash commands and descriptions.
pub fn slash_commands() -> Vec<(&'static str, &'static str)> {
    vec![
        ("/run <workflow>", "Execute a workflow"),
        ("/list", "List all workflows"),
        ("/show <workflow>", "Show workflow YAML"),
        ("/dag <workflow>", "Show workflow DAG diagram"),
        ("/logs <workflow>", "Show execution history"),
        ("/trace <exec-id>", "Show execution trace"),
        ("/sessions", "List recent sessions"),
        ("/resume [id]", "Resume a previous session"),
        ("/clear", "Clear conversation context"),
        ("/reconnect", "Reconnect to LLM"),
        (
            "/model <provider> <model>",
            "Set provider/model for this session",
        ),
        ("/apikey <key>", "Set API key for this session"),
        ("/endpoint <url>", "Set LLM endpoint for this session"),
        ("/llm", "Show current LLM config"),
        ("/view <target>", "Switch panel: log|tools|yaml|dag|trace"),
        ("/export yaml <path>", "Export current YAML to a file"),
        ("/plan [description]", "Draft a plan before executing"),
        ("/arm", "Arm write operations for this session"),
        ("/disarm", "Disarm write operations (safe mode)"),
        (
            "/operator [on|off]",
            "Toggle operator mode (policy/gate status visible)",
        ),
        ("/help", "Show this help"),
        ("/exit", "Exit the REPL"),
    ]
}
