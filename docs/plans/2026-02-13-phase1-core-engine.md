# r8r Phase 1: Core Engine Implementation Plan

> **For Claude:** Use this plan to implement the r8r core engine task-by-task.

**Goal:** Build the MVP of r8r - a Rust workflow automation engine with YAML workflows, execution engine, core nodes, and CLI.

**Architecture:** Tokio async runtime, Axum for API, SQLite for storage, YAML workflow definitions.

**Tech Stack:** Rust, Tokio, Axum, SQLite (rusqlite), serde, clap, tracing

---

## Phase 1 Scope

- ✅ Workflow YAML parser with validation
- ✅ Execution engine with topological sort
- ✅ Core nodes: HTTP, Transform, Filter, Set
- ✅ **Agent node** (calls ZeptoClaw for AI decisions)
- ✅ SQLite storage for workflows and executions
- ✅ CLI: workflows list/create/run/logs
- ✅ Cron trigger
- ✅ Manual trigger
- ✅ **Agent-friendly API responses** (structured JSON, error codes)

**NOT in Phase 1:** Web UI, WhatsApp/GSheets nodes, Webhook trigger

## Agent-First Design Principles

This is an **agent-first** workflow engine - designed for AI agents to invoke, not humans to click through:

1. **LLM-Friendly YAML** - Consistent patterns, predictable structure
2. **Structured Responses** - JSON with predictable shapes, parseable error codes
3. **ZeptoClaw Integration** - Bidirectional (r8r ↔ ZeptoClaw)
4. **Execution Traces** - Complete logs for debugging

---

## Task 1: Project Setup

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`

**Cargo.toml:**

```toml
[package]
name = "r8r"
version = "0.1.0"
edition = "2021"
description = "Rust workflow automation engine"
license = "MIT"
repository = "https://github.com/user/r8r"

[[bin]]
name = "r8r"
path = "src/main.rs"

[lib]
name = "r8r"
path = "src/lib.rs"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# HTTP client
reqwest = { version = "0.11", features = ["json", "rustls-tls"], default-features = false }

# Database
rusqlite = { version = "0.31", features = ["bundled"] }

# Cron scheduling
tokio-cron-scheduler = "0.10"

# Expression evaluation
rhai = "1"

# Utilities
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
async-trait = "0.1"
dirs = "5"

[dev-dependencies]
tokio-test = "0.4"
tempfile = "3"
```

**src/lib.rs:**

```rust
//! r8r - Rust workflow automation engine
//!
//! A lightweight, fast workflow automation tool inspired by n8n,
//! built for edge deployment and high throughput.

pub mod config;
pub mod engine;
pub mod error;
pub mod nodes;
pub mod storage;
pub mod triggers;
pub mod workflow;

pub use error::{Error, Result};
```

**src/main.rs:**

```rust
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "r8r")]
#[command(about = "Rust workflow automation engine", long_about = None)]
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
    },
    /// Show workflow execution logs
    Logs {
        /// Workflow name or ID
        name: String,
        /// Number of recent executions to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
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
            WorkflowActions::Run { name } => cmd_workflows_run(&name).await?,
            WorkflowActions::Logs { name, limit } => cmd_workflows_logs(&name, limit).await?,
        },
        Commands::Server { port } => cmd_server(port).await?,
    }

    Ok(())
}

async fn cmd_workflows_list() -> anyhow::Result<()> {
    todo!("Implement workflows list")
}

async fn cmd_workflows_create(file: &str) -> anyhow::Result<()> {
    todo!("Implement workflows create")
}

async fn cmd_workflows_run(name: &str) -> anyhow::Result<()> {
    todo!("Implement workflows run")
}

async fn cmd_workflows_logs(name: &str, limit: usize) -> anyhow::Result<()> {
    todo!("Implement workflows logs")
}

async fn cmd_server(port: u16) -> anyhow::Result<()> {
    todo!("Implement server")
}
```

**Verification:**
```bash
cargo check
cargo build
./target/debug/r8r --help
```

---

## Task 2: Error Types

**Files:**
- Create: `src/error.rs`

**Implementation:**

```rust
//! Error types for r8r.

use thiserror::Error;

/// Result type alias for r8r operations.
pub type Result<T> = std::result::Result<T, Error>;

/// r8r error types.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Workflow error: {0}")]
    Workflow(String),

    #[error("Node error: {0}")]
    Node(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

---

## Task 3: Workflow Types and Parser

**Files:**
- Create: `src/workflow/mod.rs`
- Create: `src/workflow/types.rs`
- Create: `src/workflow/parser.rs`
- Create: `src/workflow/validator.rs`

**src/workflow/mod.rs:**

```rust
//! Workflow definition and parsing.

mod parser;
mod types;
mod validator;

pub use parser::parse_workflow;
pub use types::*;
pub use validator::validate_workflow;
```

**src/workflow/types.rs:**

```rust
//! Workflow type definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique workflow name
    pub name: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Version number
    #[serde(default = "default_version")]
    pub version: u32,

    /// Triggers that start this workflow
    #[serde(default)]
    pub triggers: Vec<Trigger>,

    /// Nodes in the workflow
    pub nodes: Vec<Node>,

    /// Global settings
    #[serde(default)]
    pub settings: WorkflowSettings,
}

fn default_version() -> u32 {
    1
}

/// Workflow trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Cron-based schedule
    Cron {
        schedule: String,
        #[serde(default)]
        timezone: Option<String>,
    },
    /// Manual trigger via CLI/API
    Manual,
    /// Webhook trigger
    Webhook {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        method: Option<String>,
    },
}

/// A node in the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique node ID within workflow
    pub id: String,

    /// Node type (http, transform, filter, etc.)
    #[serde(rename = "type")]
    pub node_type: String,

    /// Node-specific configuration
    #[serde(default)]
    pub config: serde_json::Value,

    /// Node dependencies (runs after these complete)
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Run once per item in input array
    #[serde(default)]
    pub for_each: bool,

    /// Retry configuration
    #[serde(default)]
    pub retry: Option<RetryConfig>,

    /// Error handling
    #[serde(default)]
    pub on_error: Option<ErrorConfig>,
}

/// Retry configuration for a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    #[serde(default = "default_delay_seconds")]
    pub delay_seconds: u64,

    #[serde(default)]
    pub backoff: BackoffType,
}

fn default_max_attempts() -> u32 {
    3
}

fn default_delay_seconds() -> u64 {
    60
}

/// Backoff strategy for retries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffType {
    #[default]
    Fixed,
    Linear,
    Exponential,
}

/// Error handling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorConfig {
    /// Continue workflow even if this node fails
    #[serde(default)]
    pub continue_on_error: bool,

    /// Fallback node to run on error
    #[serde(default)]
    pub fallback_node: Option<String>,
}

/// Global workflow settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowSettings {
    /// Maximum execution time in seconds
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,

    /// Maximum concurrent node executions
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
}

fn default_timeout() -> u64 {
    3600 // 1 hour
}

fn default_concurrency() -> usize {
    10
}

impl Workflow {
    /// Get node by ID.
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Get nodes in topological order (respecting dependencies).
    pub fn topological_sort(&self) -> Vec<&str> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut temp_visited = std::collections::HashSet::new();

        for node in &self.nodes {
            if !visited.contains(&node.id) {
                self.visit_node(&node.id, &mut visited, &mut temp_visited, &mut result);
            }
        }

        result
    }

    fn visit_node<'a>(
        &'a self,
        node_id: &'a str,
        visited: &mut std::collections::HashSet<&'a str>,
        temp: &mut std::collections::HashSet<&'a str>,
        result: &mut Vec<&'a str>,
    ) {
        if temp.contains(node_id) {
            return; // Cycle detected, skip
        }
        if visited.contains(node_id) {
            return;
        }

        temp.insert(node_id);

        if let Some(node) = self.get_node(node_id) {
            for dep in &node.depends_on {
                self.visit_node(dep, visited, temp, result);
            }
        }

        temp.remove(node_id);
        visited.insert(node_id);
        result.push(node_id);
    }
}
```

**src/workflow/parser.rs:**

```rust
//! Workflow YAML parser.

use std::path::Path;

use crate::error::{Error, Result};
use super::types::Workflow;

/// Parse a workflow from YAML string.
pub fn parse_workflow(yaml: &str) -> Result<Workflow> {
    let workflow: Workflow = serde_yaml::from_str(yaml)?;
    Ok(workflow)
}

/// Parse a workflow from a file path.
pub fn parse_workflow_file(path: &Path) -> Result<Workflow> {
    let content = std::fs::read_to_string(path)?;
    parse_workflow(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_workflow() {
        let yaml = r#"
name: test-workflow
description: A test workflow

triggers:
  - type: manual

nodes:
  - id: step1
    type: http
    config:
      url: https://example.com
      method: GET

  - id: step2
    type: transform
    config:
      expression: "data.items"
    depends_on: [step1]
"#;

        let workflow = parse_workflow(yaml).unwrap();
        assert_eq!(workflow.name, "test-workflow");
        assert_eq!(workflow.nodes.len(), 2);
        assert_eq!(workflow.nodes[1].depends_on, vec!["step1"]);
    }

    #[test]
    fn test_topological_sort() {
        let yaml = r#"
name: test
nodes:
  - id: c
    type: http
    depends_on: [a, b]
  - id: a
    type: http
  - id: b
    type: http
    depends_on: [a]
"#;

        let workflow = parse_workflow(yaml).unwrap();
        let order = workflow.topological_sort();

        // a must come before b and c
        // b must come before c
        let a_pos = order.iter().position(|&x| x == "a").unwrap();
        let b_pos = order.iter().position(|&x| x == "b").unwrap();
        let c_pos = order.iter().position(|&x| x == "c").unwrap();

        assert!(a_pos < b_pos);
        assert!(a_pos < c_pos);
        assert!(b_pos < c_pos);
    }
}
```

**src/workflow/validator.rs:**

```rust
//! Workflow validation.

use crate::error::{Error, Result};
use super::types::Workflow;

/// Validate a workflow definition.
pub fn validate_workflow(workflow: &Workflow) -> Result<()> {
    // Check workflow has a name
    if workflow.name.is_empty() {
        return Err(Error::Validation("Workflow name is required".into()));
    }

    // Check workflow has at least one node
    if workflow.nodes.is_empty() {
        return Err(Error::Validation("Workflow must have at least one node".into()));
    }

    // Check all node IDs are unique
    let mut ids = std::collections::HashSet::new();
    for node in &workflow.nodes {
        if !ids.insert(&node.id) {
            return Err(Error::Validation(format!(
                "Duplicate node ID: {}",
                node.id
            )));
        }
    }

    // Check all dependencies exist
    for node in &workflow.nodes {
        for dep in &node.depends_on {
            if !ids.contains(dep) {
                return Err(Error::Validation(format!(
                    "Node '{}' depends on non-existent node '{}'",
                    node.id, dep
                )));
            }
        }
    }

    // Check for cycles (topological sort will handle this, but explicit check is clearer)
    if has_cycle(workflow) {
        return Err(Error::Validation("Workflow has circular dependencies".into()));
    }

    Ok(())
}

fn has_cycle(workflow: &Workflow) -> bool {
    use std::collections::{HashMap, HashSet};

    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    fn dfs(
        node_id: &str,
        deps: &HashMap<&str, Vec<&str>>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(node_id.to_string());
        rec_stack.insert(node_id.to_string());

        if let Some(neighbors) = deps.get(node_id) {
            for neighbor in neighbors {
                if !visited.contains(*neighbor) {
                    if dfs(neighbor, deps, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(*neighbor) {
                    return true;
                }
            }
        }

        rec_stack.remove(node_id);
        false
    }

    let deps: HashMap<&str, Vec<&str>> = workflow
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.depends_on.iter().map(|s| s.as_str()).collect()))
        .collect();

    for node in &workflow.nodes {
        if !visited.contains(&node.id) {
            if dfs(&node.id, &deps, &mut visited, &mut rec_stack) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parse_workflow;

    #[test]
    fn test_validate_empty_name() {
        let yaml = r#"
name: ""
nodes:
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_duplicate_ids() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_missing_dependency() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
    depends_on: [nonexistent]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_cycle() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
    depends_on: [b]
  - id: b
    type: http
    depends_on: [a]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }
}
```

---

## Task 4: Storage Layer (SQLite)

**Files:**
- Create: `src/storage/mod.rs`
- Create: `src/storage/sqlite.rs`
- Create: `src/storage/models.rs`

**src/storage/mod.rs:**

```rust
//! Storage layer for workflows and executions.

mod models;
mod sqlite;

pub use models::*;
pub use sqlite::SqliteStorage;
```

**src/storage/models.rs:**

```rust
//! Storage models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stored workflow record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWorkflow {
    pub id: String,
    pub name: String,
    pub definition: String,  // YAML
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub trigger_type: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// Node execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExecution {
    pub id: String,
    pub execution_id: String,
    pub node_id: String,
    pub status: ExecutionStatus,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}
```

**src/storage/sqlite.rs:**

```rust
//! SQLite storage implementation.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use super::models::*;

/// SQLite-based storage.
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema_blocking()?;
        Ok(storage)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema_blocking()?;
        Ok(storage)
    }

    fn init_schema_blocking(&self) -> Result<()> {
        let conn = self.conn.blocking_lock();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                definition TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS executions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                status TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                error TEXT,
                FOREIGN KEY (workflow_id) REFERENCES workflows(id)
            );

            CREATE TABLE IF NOT EXISTS node_executions (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                status TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                error TEXT,
                FOREIGN KEY (execution_id) REFERENCES executions(id)
            );

            CREATE INDEX IF NOT EXISTS idx_executions_workflow ON executions(workflow_id);
            CREATE INDEX IF NOT EXISTS idx_node_executions_execution ON node_executions(execution_id);
            "#,
        )?;
        Ok(())
    }

    // Workflow operations

    pub async fn save_workflow(&self, workflow: &StoredWorkflow) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO workflows (id, name, definition, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                workflow.id,
                workflow.name,
                workflow.definition,
                workflow.enabled,
                workflow.created_at.to_rfc3339(),
                workflow.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub async fn get_workflow(&self, name: &str) -> Result<Option<StoredWorkflow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at FROM workflows WHERE name = ?1"
        )?;

        let workflow = stmt.query_row([name], |row| {
            Ok(StoredWorkflow {
                id: row.get(0)?,
                name: row.get(1)?,
                definition: row.get(2)?,
                enabled: row.get(3)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        }).optional()?;

        Ok(workflow)
    }

    pub async fn list_workflows(&self) -> Result<Vec<StoredWorkflow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at FROM workflows ORDER BY name"
        )?;

        let workflows = stmt.query_map([], |row| {
            Ok(StoredWorkflow {
                id: row.get(0)?,
                name: row.get(1)?,
                definition: row.get(2)?,
                enabled: row.get(3)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(workflows)
    }

    // Execution operations

    pub async fn save_execution(&self, execution: &Execution) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO executions
             (id, workflow_id, workflow_name, status, trigger_type, started_at, finished_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                execution.id,
                execution.workflow_id,
                execution.workflow_name,
                execution.status.to_string(),
                execution.trigger_type,
                execution.started_at.to_rfc3339(),
                execution.finished_at.map(|t| t.to_rfc3339()),
                execution.error,
            ],
        )?;
        Ok(())
    }

    pub async fn list_executions(&self, workflow_name: &str, limit: usize) -> Result<Vec<Execution>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, status, trigger_type, started_at, finished_at, error
             FROM executions WHERE workflow_name = ?1 ORDER BY started_at DESC LIMIT ?2"
        )?;

        let executions = stmt.query_map(params![workflow_name, limit], |row| {
            let status_str: String = row.get(3)?;
            let status = match status_str.as_str() {
                "pending" => ExecutionStatus::Pending,
                "running" => ExecutionStatus::Running,
                "completed" => ExecutionStatus::Completed,
                "failed" => ExecutionStatus::Failed,
                "cancelled" => ExecutionStatus::Cancelled,
                _ => ExecutionStatus::Failed,
            };

            Ok(Execution {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                workflow_name: row.get(2)?,
                status,
                trigger_type: row.get(4)?,
                started_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&Utc),
                finished_at: row.get::<_, Option<String>>(6)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|t| t.with_timezone(&Utc)),
                error: row.get(7)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(executions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workflow_crud() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let workflow = StoredWorkflow {
            id: "wf-123".to_string(),
            name: "test-workflow".to_string(),
            definition: "name: test".to_string(),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage.save_workflow(&workflow).await.unwrap();

        let loaded = storage.get_workflow("test-workflow").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "test-workflow");
    }
}
```

---

## Task 5: Node Trait and Registry

**Files:**
- Create: `src/nodes/mod.rs`
- Create: `src/nodes/types.rs`
- Create: `src/nodes/registry.rs`

**src/nodes/mod.rs:**

```rust
//! Node implementations.

mod http;
mod registry;
mod transform;
mod types;

pub use http::HttpNode;
pub use registry::NodeRegistry;
pub use transform::TransformNode;
pub use types::{Node, NodeContext, NodeResult};
```

**src/nodes/types.rs:**

```rust
//! Node trait and context.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::Result;

/// Result of node execution.
#[derive(Debug, Clone)]
pub struct NodeResult {
    /// Output data
    pub data: Value,
    /// Metadata (timing, etc.)
    pub metadata: Value,
}

impl NodeResult {
    pub fn new(data: Value) -> Self {
        Self {
            data,
            metadata: serde_json::json!({}),
        }
    }

    pub fn with_metadata(data: Value, metadata: Value) -> Self {
        Self { data, metadata }
    }
}

/// Context passed to node during execution.
#[derive(Debug, Clone)]
pub struct NodeContext {
    /// Input data from previous nodes
    pub input: Value,
    /// Workflow variables
    pub variables: Value,
    /// Execution ID
    pub execution_id: String,
    /// Workflow name
    pub workflow_name: String,
}

impl NodeContext {
    pub fn new(execution_id: &str, workflow_name: &str) -> Self {
        Self {
            input: Value::Null,
            variables: serde_json::json!({}),
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
        }
    }

    pub fn with_input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }
}

/// Trait that all nodes must implement.
#[async_trait]
pub trait Node: Send + Sync {
    /// Node type name (e.g., "http", "transform").
    fn node_type(&self) -> &str;

    /// Execute the node with given config and context.
    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult>;
}
```

**src/nodes/registry.rs:**

```rust
//! Node registry.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::error::{Error, Result};
use super::types::{Node, NodeContext, NodeResult};
use super::{HttpNode, TransformNode};

/// Registry of available node types.
pub struct NodeRegistry {
    nodes: HashMap<String, Arc<dyn Node>>,
}

impl NodeRegistry {
    /// Create a new registry with default nodes.
    pub fn new() -> Self {
        let mut registry = Self {
            nodes: HashMap::new(),
        };

        // Register built-in nodes
        registry.register(Arc::new(HttpNode));
        registry.register(Arc::new(TransformNode));

        registry
    }

    /// Register a node type.
    pub fn register(&mut self, node: Arc<dyn Node>) {
        self.nodes.insert(node.node_type().to_string(), node);
    }

    /// Get a node by type name.
    pub fn get(&self, node_type: &str) -> Option<Arc<dyn Node>> {
        self.nodes.get(node_type).cloned()
    }

    /// Execute a node by type.
    pub async fn execute(
        &self,
        node_type: &str,
        config: &Value,
        ctx: &NodeContext,
    ) -> Result<NodeResult> {
        let node = self.get(node_type).ok_or_else(|| {
            Error::Node(format!("Unknown node type: {}", node_type))
        })?;

        node.execute(config, ctx).await
    }

    /// List all registered node types.
    pub fn list(&self) -> Vec<&str> {
        self.nodes.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

---

## Task 6: HTTP Node Implementation

**Files:**
- Create: `src/nodes/http.rs`

**Implementation:**

```rust
//! HTTP node - make HTTP requests.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use crate::error::{Error, Result};
use super::types::{Node, NodeContext, NodeResult};

/// HTTP request node.
pub struct HttpNode;

#[derive(Debug, Deserialize)]
struct HttpConfig {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: Option<Value>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

fn default_method() -> String {
    "GET".to_string()
}

#[async_trait]
impl Node for HttpNode {
    fn node_type(&self) -> &str {
        "http"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: HttpConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid HTTP config: {}", e)))?;

        debug!("HTTP {} {}", config.method, config.url);

        let client = Client::new();
        let mut request = match config.method.to_uppercase().as_str() {
            "GET" => client.get(&config.url),
            "POST" => client.post(&config.url),
            "PUT" => client.put(&config.url),
            "DELETE" => client.delete(&config.url),
            "PATCH" => client.patch(&config.url),
            _ => return Err(Error::Node(format!("Unknown HTTP method: {}", config.method))),
        };

        // Add headers
        if let Some(headers) = &config.headers {
            if let Some(headers_obj) = headers.as_object() {
                for (key, value) in headers_obj {
                    if let Some(v) = value.as_str() {
                        request = request.header(key, v);
                    }
                }
            }
        }

        // Add body
        if let Some(body) = &config.body {
            request = request.json(body);
        }

        // Set timeout
        if let Some(timeout) = config.timeout_seconds {
            request = request.timeout(std::time::Duration::from_secs(timeout));
        }

        let response = request.send().await?;
        let status = response.status().as_u16();
        let headers: Value = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect::<serde_json::Map<String, Value>>()
            .into();

        let body: Value = response.json().await.unwrap_or(Value::Null);

        Ok(NodeResult::new(json!({
            "status": status,
            "headers": headers,
            "body": body,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_get() {
        let node = HttpNode;
        let config = json!({
            "url": "https://httpbin.org/get",
            "method": "GET"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        let status = result.data["status"].as_u64().unwrap();
        assert_eq!(status, 200);
    }
}
```

---

## Task 7: Transform Node Implementation

**Files:**
- Create: `src/nodes/transform.rs`

**Implementation:**

```rust
//! Transform node - data transformation using Rhai expressions.

use async_trait::async_trait;
use rhai::{Engine, Scope};
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::error::{Error, Result};
use super::types::{Node, NodeContext, NodeResult};

/// Transform node using Rhai expressions.
pub struct TransformNode;

#[derive(Debug, Deserialize)]
struct TransformConfig {
    expression: String,
}

#[async_trait]
impl Node for TransformNode {
    fn node_type(&self) -> &str {
        "transform"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: TransformConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid transform config: {}", e)))?;

        debug!("Transform: {}", config.expression);

        let engine = Engine::new();
        let mut scope = Scope::new();

        // Add input data to scope
        let input_json = ctx.input.to_string();
        scope.push("input", input_json.clone());
        scope.push("data", input_json);

        // Evaluate expression
        let result: String = engine
            .eval_with_scope(&mut scope, &config.expression)
            .map_err(|e| Error::Node(format!("Transform error: {}", e)))?;

        // Try to parse as JSON, fall back to string
        let output: Value = serde_json::from_str(&result).unwrap_or(Value::String(result));

        Ok(NodeResult::new(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_transform_simple() {
        let node = TransformNode;
        let config = json!({
            "expression": r#""hello world""#
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, "hello world");
    }
}
```

---

## Task 7.5: Agent Node Implementation (ZeptoClaw Integration)

**Files:**
- Create: `src/nodes/agent.rs`
- Modify: `src/nodes/mod.rs` (add export)
- Modify: `src/nodes/registry.rs` (register agent node)

**src/nodes/agent.rs:**

```rust
//! Agent node - calls ZeptoClaw for AI-powered decisions.
//!
//! This node allows workflows to invoke an AI agent (ZeptoClaw) for tasks
//! that require reasoning, classification, or content generation.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info};

use crate::error::{Error, Result};
use super::types::{Node, NodeContext, NodeResult};

/// Agent node - invoke ZeptoClaw for AI decisions.
pub struct AgentNode {
    client: Client,
}

impl AgentNode {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for AgentNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct AgentConfig {
    /// The prompt to send to the agent
    prompt: String,

    /// ZeptoClaw endpoint (default: http://localhost:3000/api/chat)
    #[serde(default = "default_endpoint")]
    endpoint: String,

    /// Model to use (optional, uses ZeptoClaw default)
    #[serde(default)]
    model: Option<String>,

    /// Expected response format: "text" or "json"
    #[serde(default = "default_response_format")]
    response_format: String,

    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,

    /// System prompt (optional)
    #[serde(default)]
    system: Option<String>,
}

fn default_endpoint() -> String {
    std::env::var("R8R_AGENT_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:3000/api/chat".to_string())
}

fn default_response_format() -> String {
    "text".to_string()
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Serialize)]
struct AgentRequest {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentResponse {
    content: String,
    #[serde(default)]
    usage: Option<AgentUsage>,
}

#[derive(Debug, Deserialize)]
struct AgentUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[async_trait]
impl Node for AgentNode {
    fn node_type(&self) -> &str {
        "agent"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: AgentConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid agent config: {}", e)))?;

        // Render prompt with template variables
        let prompt = render_template(&config.prompt, ctx)?;

        debug!("Agent node calling ZeptoClaw: {}", config.endpoint);
        info!("Agent prompt: {}...", &prompt.chars().take(100).collect::<String>());

        let request = AgentRequest {
            message: prompt,
            model: config.model,
            system: config.system,
        };

        let response = self.client
            .post(&config.endpoint)
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Node(format!(
                "Agent API error ({}): {}",
                status, error_text
            )));
        }

        let agent_response: AgentResponse = response.json().await
            .map_err(|e| Error::Node(format!("Failed to parse agent response: {}", e)))?;

        // Parse response based on expected format
        let output = if config.response_format == "json" {
            serde_json::from_str(&agent_response.content)
                .unwrap_or_else(|_| Value::String(agent_response.content.clone()))
        } else {
            Value::String(agent_response.content.clone())
        };

        let metadata = json!({
            "response_format": config.response_format,
            "usage": agent_response.usage,
        });

        Ok(NodeResult::with_metadata(output, metadata))
    }
}

/// Simple template rendering: replaces {{ variable }} with values from context.
fn render_template(template: &str, ctx: &NodeContext) -> Result<String> {
    let mut result = template.to_string();

    // Replace {{ input }} or {{ input.field }}
    if let Some(captures) = regex_lite::Regex::new(r"\{\{\s*input\.(\w+)\s*\}\}").ok() {
        for cap in captures.captures_iter(template) {
            if let Some(field) = cap.get(1) {
                let field_name = field.as_str();
                if let Some(value) = ctx.input.get(field_name) {
                    let replacement = match value {
                        Value::String(s) => s.clone(),
                        _ => value.to_string(),
                    };
                    result = result.replace(&cap[0], &replacement);
                }
            }
        }
    }

    // Replace {{ input }} with full input JSON
    result = result.replace("{{ input }}", &ctx.input.to_string());
    result = result.replace("{{input}}", &ctx.input.to_string());

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_simple() {
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "John", "email": "john@example.com"}));

        let template = "Hello {{ input.name }}, your email is {{ input.email }}";
        let result = render_template(template, &ctx).unwrap();

        assert!(result.contains("John"));
        assert!(result.contains("john@example.com"));
    }

    #[test]
    fn test_agent_config_defaults() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "Hello"
        })).unwrap();

        assert_eq!(config.response_format, "text");
        assert_eq!(config.timeout_seconds, 30);
    }
}
```

**Add to Cargo.toml dependencies:**
```toml
regex-lite = "0.1"
```

**Update src/nodes/mod.rs:**
```rust
mod agent;
// ... existing mods

pub use agent::AgentNode;
```

**Update src/nodes/registry.rs:**
```rust
use super::{HttpNode, TransformNode, AgentNode};

impl NodeRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            nodes: HashMap::new(),
        };

        // Register built-in nodes
        registry.register(Arc::new(HttpNode));
        registry.register(Arc::new(TransformNode));
        registry.register(Arc::new(AgentNode::new()));  // ADD THIS

        registry
    }
}
```

---

## Remaining Tasks (Summary)

### Task 8: Execution Engine
- Create `src/engine/mod.rs`, `executor.rs`, `scheduler.rs`
- Implement workflow execution with topological order
- Handle `for_each`, retries, error handling

### Task 9: Cron Trigger
- Create `src/triggers/mod.rs`, `cron.rs`
- Use `tokio-cron-scheduler` for scheduling
- Start/stop workflows on schedule

### Task 10: CLI Implementation
- Implement `cmd_workflows_list`, `cmd_workflows_create`, etc.
- Wire up storage and executor
- Pretty print output

### Task 11: Example Workflows
- Create `examples/hello-world.yaml`
- Create `examples/order-notification.yaml`

### Task 12: Integration Tests
- End-to-end workflow tests
- Storage tests
- Node execution tests

---

## Verification

```bash
# Build
cargo build

# Test
cargo test

# Run CLI
./target/debug/r8r workflows list
./target/debug/r8r workflows create examples/hello-world.yaml
./target/debug/r8r workflows run hello-world
./target/debug/r8r workflows logs hello-world
```

---

## Success Criteria

- [ ] Parse YAML workflows with validation
- [ ] Execute workflows in topological order
- [ ] HTTP, Transform, and Agent nodes working
- [ ] SQLite storage for workflows and executions
- [ ] CLI commands functional
- [ ] Cron trigger scheduling workflows
- [ ] Agent node can call ZeptoClaw

---

## Appendix A: ZeptoClaw r8r Tool Design

This tool allows ZeptoClaw to invoke r8r workflows. Add this to ZeptoClaw's tool registry.

**File:** `zeptoclaw/src/tools/r8r.rs`

```rust
//! r8r tool - invoke r8r workflows from ZeptoClaw.
//!
//! This tool allows the AI agent to run deterministic workflows
//! for repeatable, multi-step operations.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{PicoError, Result};
use crate::tools::{Tool, ToolContext, ToolResult};

/// r8r workflow execution tool.
pub struct R8rTool {
    client: Client,
    endpoint: String,
}

impl R8rTool {
    pub fn new() -> Self {
        let endpoint = std::env::var("R8R_API_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:8080/api".to_string());

        Self {
            client: Client::new(),
            endpoint,
        }
    }

    pub fn with_endpoint(endpoint: &str) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct R8rArgs {
    /// Workflow name or ID to execute
    workflow: String,

    /// Input data for the workflow (optional)
    #[serde(default)]
    inputs: Value,

    /// Wait for completion (default: true)
    #[serde(default = "default_wait")]
    wait: bool,
}

fn default_wait() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct R8rExecutionResponse {
    success: bool,
    data: Option<R8rExecutionData>,
    error: Option<R8rError>,
}

#[derive(Debug, Deserialize)]
struct R8rExecutionData {
    execution_id: String,
    workflow_id: String,
    status: String,
    duration_ms: Option<u64>,
    summary: Option<Value>,
    outputs: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct R8rError {
    code: String,
    message: String,
    node_id: Option<String>,
    retry_after_ms: Option<u64>,
}

#[async_trait]
impl Tool for R8rTool {
    fn name(&self) -> &str {
        "r8r"
    }

    fn description(&self) -> &str {
        "Run a deterministic workflow for repeatable, multi-step operations. \
         Use this for tasks like: sending notifications to multiple recipients, \
         processing orders, updating spreadsheets, or any task that should \
         execute the same way every time."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "workflow": {
                    "type": "string",
                    "description": "Name of the workflow to run (e.g., 'order-notification', 'inventory-alert')"
                },
                "inputs": {
                    "type": "object",
                    "description": "Input data for the workflow. Keys depend on the specific workflow.",
                    "additionalProperties": true
                },
                "wait": {
                    "type": "boolean",
                    "description": "Wait for workflow to complete before returning (default: true)",
                    "default": true
                }
            },
            "required": ["workflow"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let args: R8rArgs = serde_json::from_value(args)
            .map_err(|e| PicoError::Tool(format!("Invalid r8r arguments: {}", e)))?;

        // Build request
        let url = format!("{}/workflows/{}/execute", self.endpoint, args.workflow);

        let request_body = json!({
            "inputs": args.inputs,
            "wait": args.wait,
        });

        // Execute workflow
        let response = self.client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("r8r request failed: {}", e)))?;

        let status = response.status();
        let response_body: R8rExecutionResponse = response.json().await
            .map_err(|e| PicoError::Tool(format!("Failed to parse r8r response: {}", e)))?;

        if !response_body.success {
            if let Some(error) = response_body.error {
                // Format error message for agent to understand
                let error_msg = format!(
                    "Workflow '{}' failed: [{}] {}{}",
                    args.workflow,
                    error.code,
                    error.message,
                    error.node_id.map(|n| format!(" (at node: {})", n)).unwrap_or_default()
                );

                return Ok(ToolResult::error(&error_msg));
            }
            return Ok(ToolResult::error("Workflow execution failed"));
        }

        // Format success response for agent
        if let Some(data) = response_body.data {
            let summary = if let Some(outputs) = data.outputs {
                format!(
                    "Workflow '{}' completed successfully in {}ms.\n\nOutputs:\n{}",
                    args.workflow,
                    data.duration_ms.unwrap_or(0),
                    serde_json::to_string_pretty(&outputs).unwrap_or_default()
                )
            } else {
                format!(
                    "Workflow '{}' completed successfully in {}ms.",
                    args.workflow,
                    data.duration_ms.unwrap_or(0)
                )
            };

            Ok(ToolResult::success(&summary))
        } else {
            Ok(ToolResult::success(&format!(
                "Workflow '{}' started. Status: {}",
                args.workflow,
                if args.wait { "completed" } else { "running" }
            )))
        }
    }
}

// Register in ZeptoClaw's tool registry
impl Default for R8rTool {
    fn default() -> Self {
        Self::new()
    }
}
```

**Register in ZeptoClaw:**

```rust
// In zeptoclaw/src/tools/registry.rs or wherever tools are registered

use super::r8r::R8rTool;

impl ToolRegistry {
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        // ... existing tools ...
        registry.register(Box::new(R8rTool::new()));

        registry
    }
}
```

**Usage Example (Agent Conversation):**

```
User: "Send order confirmations to all new orders from today"

Agent (thinking): This is a repeatable task with clear rules.
I should use the r8r workflow.

Agent: [calls r8r tool]
{
  "workflow": "order-notification",
  "inputs": {
    "date_filter": "today",
    "status_filter": "new"
  }
}

r8r response:
{
  "success": true,
  "data": {
    "execution_id": "exec_abc123",
    "workflow_id": "wf_order_notification",
    "status": "completed",
    "duration_ms": 2340,
    "summary": {
      "orders_processed": 15,
      "notifications_sent": 15,
      "errors": 0
    }
  }
}

Agent: "Done! I ran the order-notification workflow and sent
confirmations to all 15 new orders from today. Everything completed
successfully with no errors."
```

---

## Appendix B: Example Agent-Hybrid Workflow

This workflow demonstrates r8r calling ZeptoClaw for AI decisions:

**File:** `examples/smart-ticket-router.yaml`

```yaml
name: smart-ticket-router
description: Route support tickets using AI classification
version: 1

triggers:
  - type: cron
    schedule: "*/5 * * * *"  # Every 5 minutes

nodes:
  # 1. Fetch unprocessed tickets
  - id: fetch-tickets
    type: http
    config:
      url: "{{ env.HELPDESK_API }}/tickets?status=new"
      method: GET
      headers:
        Authorization: "Bearer {{ env.HELPDESK_TOKEN }}"

  # 2. AI classification (calls ZeptoClaw)
  - id: classify-urgency
    type: agent
    config:
      prompt: |
        Classify the urgency of this support ticket.

        Subject: {{ input.subject }}
        Body: {{ input.body }}
        Customer: {{ input.customer_name }}
        Plan: {{ input.plan }}

        Consider:
        - Is this blocking their business? (critical)
        - Is this affecting multiple users? (high)
        - Is this a feature request or minor issue? (low)

        Respond with JSON:
        {
          "urgency": "critical|high|medium|low",
          "reason": "brief explanation",
          "suggested_response": "template to use"
        }
      response_format: json
      model: "claude-sonnet-4-20250514"
    depends_on: [fetch-tickets]
    for_each: true

  # 3. Route based on AI classification
  - id: route-ticket
    type: switch
    config:
      field: "{{ nodes.classify-urgency.output.urgency }}"
      cases:
        critical:
          next: page-oncall
        high:
          next: notify-senior
        medium:
          next: auto-respond
        low:
          next: queue-standard
    depends_on: [classify-urgency]

  # 4a. Critical: Page on-call
  - id: page-oncall
    type: http
    config:
      url: "{{ env.PAGERDUTY_API }}/incidents"
      method: POST
      body:
        title: "Critical ticket: {{ input.subject }}"
        urgency: "high"
    depends_on: [route-ticket]
    condition: "route == 'page-oncall'"

  # 4b. High: Slack senior support
  - id: notify-senior
    type: http
    config:
      url: "{{ env.SLACK_WEBHOOK }}"
      method: POST
      body:
        text: "🔴 High priority ticket needs attention"
        attachments:
          - title: "{{ input.subject }}"
            text: "{{ nodes.classify-urgency.output.reason }}"
    depends_on: [route-ticket]
    condition: "route == 'notify-senior'"

  # 4c. Medium: Auto-respond
  - id: auto-respond
    type: http
    config:
      url: "{{ env.HELPDESK_API }}/tickets/{{ input.id }}/reply"
      method: POST
      body:
        message: "{{ nodes.classify-urgency.output.suggested_response }}"
        internal: false
    depends_on: [route-ticket]
    condition: "route == 'auto-respond'"

  # 5. Update ticket with classification
  - id: update-ticket
    type: http
    config:
      url: "{{ env.HELPDESK_API }}/tickets/{{ input.id }}"
      method: PATCH
      body:
        tags:
          - "urgency:{{ nodes.classify-urgency.output.urgency }}"
          - "ai-classified"
        custom_fields:
          ai_reason: "{{ nodes.classify-urgency.output.reason }}"
    depends_on: [route-ticket]
```

This shows:
- **Deterministic parts** (fetch, route, update) handled by r8r
- **AI reasoning** (classification) delegated to ZeptoClaw via agent node
- **Best of both worlds**: reliable execution + intelligent decisions

---

*Last updated: 2026-02-13*
