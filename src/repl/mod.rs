pub mod conversation;
pub mod engine;
pub mod input;
pub mod prompt;
pub mod tools;
pub mod tui;

pub use engine::{run_turn, StreamUpdate, ToolCallRecord, TurnResult};

use crate::credentials::CredentialStore;
use crate::engine::Executor;
use crate::generator;
use crate::llm;
use crate::nodes::NodeRegistry;
use crate::storage::{SqliteStorage, Storage};
use std::io::{self, Write};
use std::sync::Arc;

/// Entry point for `r8r chat`.
pub async fn run_repl(resume_session: Option<String>) -> anyhow::Result<()> {
    let config = crate::config::Config::load();
    let db_path = config
        .storage
        .database_path
        .unwrap_or_else(|| crate::config::Config::data_dir().join("r8r.db"));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&db_path)?);
    let persisted_llm_config = storage.get_repl_llm_config().await?;
    let env_llm_config = generator::resolve_llm_config().await;
    let llm_config = persisted_llm_config
        .clone()
        .or_else(|| env_llm_config.as_ref().ok().cloned());
    if llm_config.is_none() {
        if let Err(e) = env_llm_config {
            eprintln!("Warning: {}. REPL will run in slash-only mode.", e);
        } else {
            eprintln!("Warning: LLM is not configured. REPL will run in slash-only mode.");
        }
    } else if persisted_llm_config.is_none() {
        if let Some(cfg) = &llm_config {
            let _ = storage.save_repl_llm_config(cfg).await;
        }
    }

    let executor = Arc::new(Executor::new(NodeRegistry::new(), Arc::clone(&storage)));

    let model_name = llm_config
        .as_ref()
        .and_then(|c| c.model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let session_id = match resume_session {
        Some(id) if id.is_empty() => pick_resume_session(&storage, &model_name).await?,
        Some(id) => id,
        None => storage.create_repl_session(&model_name).await?,
    };

    let tool_defs = tools::build_tool_definitions();
    let tool_names: Vec<&str> = tool_defs
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();

    let node_types: Vec<String> = NodeRegistry::new()
        .list()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let node_type_refs: Vec<&str> = node_types.iter().map(|s| s.as_str()).collect();

    let credential_names: Vec<String> = match CredentialStore::load().await {
        Ok(store) => store
            .list()
            .iter()
            .map(|c| c.service.clone())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let cred_refs: Vec<&str> = credential_names.iter().map(|s| s.as_str()).collect();
    let workflow_names: Vec<String> = storage
        .list_workflows()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|w| w.name)
        .collect();
    let workflow_refs: Vec<&str> = workflow_names.iter().map(|s| s.as_str()).collect();

    let provider = llm_config
        .as_ref()
        .map(|c| c.provider.clone())
        .unwrap_or_default();
    let system_prompt = prompt::build_repl_system_prompt(
        &tool_names,
        &node_type_refs,
        &cred_refs,
        &workflow_refs,
        provider,
    );

    tui::run_repl_tui(
        storage,
        executor,
        llm_config,
        llm::create_llm_client(),
        session_id,
        system_prompt,
        tool_defs,
    )
    .await
}

async fn pick_resume_session(storage: &dyn Storage, model_name: &str) -> anyhow::Result<String> {
    let sessions = storage.list_repl_sessions(20).await?;
    if sessions.is_empty() {
        eprintln!("No previous sessions found. Creating a new one.");
        return Ok(storage.create_repl_session(model_name).await?);
    }

    eprintln!("Select a session to resume:");
    for (idx, s) in sessions.iter().enumerate() {
        let summary = s.summary.as_deref().unwrap_or("");
        eprintln!("  {}. {}  {}  {}", idx + 1, s.id, s.model, summary);
    }
    eprint!("Enter number (or press Enter for newest): ");
    io::stderr().flush()?;

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(sessions[0].id.clone());
    }

    if let Ok(idx) = trimmed.parse::<usize>() {
        if idx >= 1 && idx <= sessions.len() {
            return Ok(sessions[idx - 1].id.clone());
        }
    }

    if sessions.iter().any(|s| s.id == trimmed) {
        return Ok(trimmed.to_string());
    }

    Err(anyhow::anyhow!(
        "Invalid session selection '{}'. Use an index or full session id.",
        trimmed
    ))
}
