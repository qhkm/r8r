# R8r MCP Server Implementation Plan

> **For Claude:** Use this plan to implement the r8r MCP server task-by-task.

**Goal:** Create an MCP server that exposes r8r workflow capabilities to any MCP-compatible AI client (Claude Desktop, Claude Code, ZeptoClaw, etc.)

**Architecture:** Standalone MCP server binary that wraps r8r core library, supporting both stdio and Streamable HTTP transports.

**Tech Stack:** Rust, rmcp (Rust MCP SDK), tokio, serde, r8r core

---

## Overview

```
┌─────────────────┐     stdio/HTTP     ┌─────────────────┐
│  Claude Desktop │◄──────────────────►│   r8r-mcp       │
│  Claude Code    │                    │   (MCP Server)  │
│  ZeptoClaw      │                    │                 │
│  Any MCP Client │                    │  ┌───────────┐  │
└─────────────────┘                    │  │ r8r core  │  │
                                       │  │ (library) │  │
                                       │  └───────────┘  │
                                       │        │        │
                                       │        ▼        │
                                       │  ┌───────────┐  │
                                       │  │  SQLite   │  │
                                       │  └───────────┘  │
                                       └─────────────────┘
```

---

## Task 1: Project Setup

**Files:**
- Create: `src/mcp/mod.rs`
- Create: `src/mcp/server.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`

**Step 1: Add MCP dependencies to Cargo.toml**

```toml
[dependencies]
# ... existing deps ...

# MCP Server
rmcp = { version = "0.1", features = ["server", "transport-stdio", "transport-sse"] }
```

**Step 2: Create MCP module structure**

```rust
// src/mcp/mod.rs
//! R8r MCP Server
//!
//! Exposes r8r workflow capabilities via Model Context Protocol.

mod server;

pub use server::R8rMcpServer;
```

**Step 3: Export from lib.rs**

```rust
// src/lib.rs
pub mod mcp;
pub use mcp::R8rMcpServer;
```

---

## Task 2: Implement MCP Server Core

**Files:**
- Create: `src/mcp/server.rs`

**Implementation:**

```rust
//! R8r MCP Server implementation

use std::sync::Arc;
use rmcp::{
    Server, ServerBuilder, Tool, ToolHandler, Resource, ResourceHandler,
    Prompt, PromptHandler, ServerCapabilities,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, validate_workflow};
use crate::error::Result;

/// R8r MCP Server
pub struct R8rMcpServer {
    storage: SqliteStorage,
    executor: Arc<Executor>,
}

impl R8rMcpServer {
    /// Create a new MCP server with the given storage path
    pub fn new(db_path: &std::path::Path) -> Result<Self> {
        let storage = SqliteStorage::open(db_path)?;
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, storage.clone()));

        Ok(Self { storage, executor })
    }

    /// Create with in-memory storage (for testing)
    pub fn in_memory() -> Result<Self> {
        let storage = SqliteStorage::open_in_memory()?;
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, storage.clone()));

        Ok(Self { storage, executor })
    }

    /// Build the MCP server with all capabilities
    pub fn build(&self) -> ServerBuilder {
        ServerBuilder::new("r8r", env!("CARGO_PKG_VERSION"))
            .capabilities(ServerCapabilities {
                tools: true,
                resources: true,
                prompts: true,
                ..Default::default()
            })
            // Tools
            .tool(self.tool_execute())
            .tool(self.tool_list_workflows())
            .tool(self.tool_get_workflow())
            .tool(self.tool_get_execution())
            .tool(self.tool_get_trace())
            .tool(self.tool_resume())
            .tool(self.tool_validate())
            .tool(self.tool_create_workflow())
            // Resources
            .resource(self.resource_workflows())
            .resource(self.resource_workflow_template())
            .resource(self.resource_execution_template())
            // Prompts
            .prompt(self.prompt_debug_workflow())
            .prompt(self.prompt_create_workflow())
    }

    /// Run the server with stdio transport
    pub async fn run_stdio(&self) -> Result<()> {
        let server = self.build().build();
        server.run_stdio().await?;
        Ok(())
    }

    /// Run the server with HTTP transport
    pub async fn run_http(&self, port: u16) -> Result<()> {
        let server = self.build().build();
        server.run_http(([0, 0, 0, 0], port)).await?;
        Ok(())
    }
}
```

---

## Task 3: Implement Tools

**Files:**
- Modify: `src/mcp/server.rs`

### Tool: r8r_execute

```rust
impl R8rMcpServer {
    fn tool_execute(&self) -> Tool {
        Tool::new(
            "r8r_execute",
            "Execute an r8r workflow. Returns execution result with status, output, and execution ID.",
            json!({
                "type": "object",
                "properties": {
                    "workflow": {
                        "type": "string",
                        "description": "Name of the workflow to execute"
                    },
                    "input": {
                        "type": "object",
                        "description": "Input data for the workflow",
                        "additionalProperties": true
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "Wait for completion (default: true)",
                        "default": true
                    }
                },
                "required": ["workflow"]
            }),
        )
        .handler(ExecuteHandler {
            storage: self.storage.clone(),
            executor: self.executor.clone(),
        })
    }
}

struct ExecuteHandler {
    storage: SqliteStorage,
    executor: Arc<Executor>,
}

#[async_trait]
impl ToolHandler for ExecuteHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let workflow_name = args["workflow"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'workflow'".into()))?;

        let input = args.get("input").cloned().unwrap_or(Value::Null);

        // Get workflow from storage
        let stored = self.storage.get_workflow(workflow_name).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::InvalidParams(format!("Workflow not found: {}", workflow_name)))?;

        let workflow = parse_workflow(&stored.definition)
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        // Execute
        let execution = self.executor
            .execute(&workflow, &stored.id, "mcp", input)
            .await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        Ok(json!({
            "execution_id": execution.id,
            "status": execution.status.to_string(),
            "output": execution.output,
            "error": execution.error,
            "duration_ms": execution.finished_at.map(|f| (f - execution.started_at).num_milliseconds())
        }))
    }
}
```

### Tool: r8r_list_workflows

```rust
fn tool_list_workflows(&self) -> Tool {
    Tool::new(
        "r8r_list_workflows",
        "List all available r8r workflows with their status and description.",
        json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    )
    .handler(ListWorkflowsHandler {
        storage: self.storage.clone(),
    })
}

struct ListWorkflowsHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ToolHandler for ListWorkflowsHandler {
    async fn handle(&self, _args: Value) -> rmcp::Result<Value> {
        let workflows = self.storage.list_workflows().await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        let list: Vec<Value> = workflows.iter().map(|wf| {
            // Parse to get description
            let desc = parse_workflow(&wf.definition)
                .map(|w| w.description)
                .unwrap_or_default();

            json!({
                "name": wf.name,
                "enabled": wf.enabled,
                "description": desc,
                "updated_at": wf.updated_at.to_rfc3339()
            })
        }).collect();

        Ok(json!({ "workflows": list, "count": list.len() }))
    }
}
```

### Tool: r8r_get_workflow

```rust
fn tool_get_workflow(&self) -> Tool {
    Tool::new(
        "r8r_get_workflow",
        "Get the full definition of a workflow including all nodes and configuration.",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Workflow name"
                }
            },
            "required": ["name"]
        }),
    )
    .handler(GetWorkflowHandler {
        storage: self.storage.clone(),
    })
}

struct GetWorkflowHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ToolHandler for GetWorkflowHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let name = args["name"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'name'".into()))?;

        let stored = self.storage.get_workflow(name).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::InvalidParams(format!("Workflow not found: {}", name)))?;

        let workflow = parse_workflow(&stored.definition)
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        Ok(json!({
            "name": workflow.name,
            "description": workflow.description,
            "version": workflow.version,
            "enabled": stored.enabled,
            "triggers": workflow.triggers,
            "nodes": workflow.nodes,
            "definition_yaml": stored.definition
        }))
    }
}
```

### Tool: r8r_get_execution

```rust
fn tool_get_execution(&self) -> Tool {
    Tool::new(
        "r8r_get_execution",
        "Get the status and result of a workflow execution.",
        json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "Execution ID"
                }
            },
            "required": ["execution_id"]
        }),
    )
    .handler(GetExecutionHandler {
        storage: self.storage.clone(),
    })
}

struct GetExecutionHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ToolHandler for GetExecutionHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let exec_id = args["execution_id"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'execution_id'".into()))?;

        let execution = self.storage.get_execution(exec_id).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::InvalidParams(format!("Execution not found: {}", exec_id)))?;

        Ok(json!({
            "execution_id": execution.id,
            "workflow_name": execution.workflow_name,
            "status": execution.status.to_string(),
            "trigger_type": execution.trigger_type,
            "input": execution.input,
            "output": execution.output,
            "error": execution.error,
            "started_at": execution.started_at.to_rfc3339(),
            "finished_at": execution.finished_at.map(|t| t.to_rfc3339()),
            "duration_ms": execution.finished_at.map(|f| (f - execution.started_at).num_milliseconds())
        }))
    }
}
```

### Tool: r8r_get_trace

```rust
fn tool_get_trace(&self) -> Tool {
    Tool::new(
        "r8r_get_trace",
        "Get detailed execution trace for debugging. Shows input/output of each node.",
        json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "Execution ID"
                }
            },
            "required": ["execution_id"]
        }),
    )
    .handler(GetTraceHandler {
        storage: self.storage.clone(),
    })
}

struct GetTraceHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ToolHandler for GetTraceHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let exec_id = args["execution_id"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'execution_id'".into()))?;

        let execution = self.storage.get_execution(exec_id).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::InvalidParams(format!("Execution not found: {}", exec_id)))?;

        let node_execs = self.storage.get_node_executions(exec_id).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        let trace: Vec<Value> = node_execs.iter().map(|ne| {
            json!({
                "node_id": ne.node_id,
                "status": ne.status.to_string(),
                "input": ne.input,
                "output": ne.output,
                "error": ne.error,
                "started_at": ne.started_at.to_rfc3339(),
                "finished_at": ne.finished_at.map(|t| t.to_rfc3339()),
                "duration_ms": ne.finished_at.map(|f| (f - ne.started_at).num_milliseconds())
            })
        }).collect();

        Ok(json!({
            "execution_id": execution.id,
            "workflow_name": execution.workflow_name,
            "status": execution.status.to_string(),
            "error": execution.error,
            "nodes": trace,
            "failed_node": node_execs.iter()
                .find(|n| n.error.is_some())
                .map(|n| n.node_id.clone())
        }))
    }
}
```

### Tool: r8r_validate

```rust
fn tool_validate(&self) -> Tool {
    Tool::new(
        "r8r_validate",
        "Validate a workflow YAML definition without saving it.",
        json!({
            "type": "object",
            "properties": {
                "yaml": {
                    "type": "string",
                    "description": "Workflow YAML definition"
                }
            },
            "required": ["yaml"]
        }),
    )
    .handler(ValidateHandler)
}

struct ValidateHandler;

#[async_trait]
impl ToolHandler for ValidateHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let yaml = args["yaml"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'yaml'".into()))?;

        match parse_workflow(yaml) {
            Ok(workflow) => {
                match validate_workflow(&workflow) {
                    Ok(()) => Ok(json!({
                        "valid": true,
                        "name": workflow.name,
                        "nodes_count": workflow.nodes.len(),
                        "triggers_count": workflow.triggers.len()
                    })),
                    Err(e) => Ok(json!({
                        "valid": false,
                        "error": e.to_string(),
                        "error_type": "validation"
                    }))
                }
            }
            Err(e) => Ok(json!({
                "valid": false,
                "error": e.to_string(),
                "error_type": "parse"
            }))
        }
    }
}
```

### Tool: r8r_create_workflow

```rust
fn tool_create_workflow(&self) -> Tool {
    Tool::new(
        "r8r_create_workflow",
        "Create or update a workflow from YAML definition.",
        json!({
            "type": "object",
            "properties": {
                "yaml": {
                    "type": "string",
                    "description": "Workflow YAML definition"
                }
            },
            "required": ["yaml"]
        }),
    )
    .handler(CreateWorkflowHandler {
        storage: self.storage.clone(),
    })
}

struct CreateWorkflowHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ToolHandler for CreateWorkflowHandler {
    async fn handle(&self, args: Value) -> rmcp::Result<Value> {
        let yaml = args["yaml"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'yaml'".into()))?;

        let workflow = parse_workflow(yaml)
            .map_err(|e| rmcp::Error::InvalidParams(format!("Invalid YAML: {}", e)))?;

        validate_workflow(&workflow)
            .map_err(|e| rmcp::Error::InvalidParams(format!("Validation failed: {}", e)))?;

        let now = chrono::Utc::now();
        let stored = crate::storage::StoredWorkflow {
            id: uuid::Uuid::new_v4().to_string(),
            name: workflow.name.clone(),
            definition: yaml.to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        self.storage.save_workflow(&stored).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        Ok(json!({
            "created": true,
            "name": workflow.name,
            "id": stored.id
        }))
    }
}
```

### Tool: r8r_resume

```rust
fn tool_resume(&self) -> Tool {
    Tool::new(
        "r8r_resume",
        "Resume a failed execution from the point of failure.",
        json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "ID of the failed execution to resume"
                },
                "modified_input": {
                    "type": "object",
                    "description": "Optional: modified input for the failed node",
                    "additionalProperties": true
                }
            },
            "required": ["execution_id"]
        }),
    )
    .handler(ResumeHandler {
        storage: self.storage.clone(),
        executor: self.executor.clone(),
    })
}

// Implementation depends on resume capability in executor
```

---

## Task 4: Implement Resources

**Files:**
- Modify: `src/mcp/server.rs`

```rust
impl R8rMcpServer {
    fn resource_workflows(&self) -> Resource {
        Resource::new(
            "r8r://workflows",
            "List of all r8r workflows",
            "application/json",
        )
        .handler(WorkflowsResourceHandler {
            storage: self.storage.clone(),
        })
    }

    fn resource_workflow_template(&self) -> Resource {
        Resource::template(
            "r8r://workflows/{name}",
            "Workflow definition by name",
            "text/yaml",
        )
        .handler(WorkflowResourceHandler {
            storage: self.storage.clone(),
        })
    }

    fn resource_execution_template(&self) -> Resource {
        Resource::template(
            "r8r://executions/{id}",
            "Execution details by ID",
            "application/json",
        )
        .handler(ExecutionResourceHandler {
            storage: self.storage.clone(),
        })
    }
}

struct WorkflowsResourceHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ResourceHandler for WorkflowsResourceHandler {
    async fn read(&self, _uri: &str) -> rmcp::Result<String> {
        let workflows = self.storage.list_workflows().await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        let list: Vec<Value> = workflows.iter().map(|wf| {
            json!({
                "name": wf.name,
                "enabled": wf.enabled,
                "uri": format!("r8r://workflows/{}", wf.name)
            })
        }).collect();

        Ok(serde_json::to_string_pretty(&list).unwrap())
    }
}

struct WorkflowResourceHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl ResourceHandler for WorkflowResourceHandler {
    async fn read(&self, uri: &str) -> rmcp::Result<String> {
        // Parse name from URI: r8r://workflows/{name}
        let name = uri.strip_prefix("r8r://workflows/")
            .ok_or_else(|| rmcp::Error::InvalidParams("Invalid URI".into()))?;

        let stored = self.storage.get_workflow(name).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::NotFound(format!("Workflow not found: {}", name)))?;

        Ok(stored.definition)
    }
}
```

---

## Task 5: Implement Prompts

**Files:**
- Modify: `src/mcp/server.rs`

```rust
impl R8rMcpServer {
    fn prompt_debug_workflow(&self) -> Prompt {
        Prompt::new(
            "debug_workflow",
            "Help debug a failed r8r workflow execution",
            vec![
                PromptArgument::new("execution_id", "ID of the failed execution", true),
            ],
        )
        .handler(DebugPromptHandler {
            storage: self.storage.clone(),
        })
    }

    fn prompt_create_workflow(&self) -> Prompt {
        Prompt::new(
            "create_workflow",
            "Help create a new r8r workflow",
            vec![
                PromptArgument::new("description", "What should the workflow do?", true),
            ],
        )
        .handler(CreateWorkflowPromptHandler)
    }
}

struct DebugPromptHandler {
    storage: SqliteStorage,
}

#[async_trait]
impl PromptHandler for DebugPromptHandler {
    async fn get_messages(&self, args: Value) -> rmcp::Result<Vec<PromptMessage>> {
        let exec_id = args["execution_id"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'execution_id'".into()))?;

        let execution = self.storage.get_execution(exec_id).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?
            .ok_or_else(|| rmcp::Error::InvalidParams("Execution not found".into()))?;

        let node_execs = self.storage.get_node_executions(exec_id).await
            .map_err(|e| rmcp::Error::Internal(e.to_string()))?;

        let failed_node = node_execs.iter().find(|n| n.error.is_some());

        let context = format!(
            r#"# R8r Workflow Debugging

## Execution Summary
- **Execution ID:** {}
- **Workflow:** {}
- **Status:** {}
- **Trigger:** {}

## Input
```json
{}
```

## Error
{}

## Node Execution Trace
{}

## Failed Node Details
{}

Please analyze this execution trace and help identify:
1. What caused the failure
2. Which node failed and why
3. How to fix the issue
4. Whether the workflow can be resumed
"#,
            execution.id,
            execution.workflow_name,
            execution.status,
            execution.trigger_type,
            serde_json::to_string_pretty(&execution.input).unwrap_or_default(),
            execution.error.as_deref().unwrap_or("No error message"),
            node_execs.iter().map(|n| {
                format!("- {} [{}]: {}",
                    n.node_id,
                    n.status,
                    n.error.as_deref().unwrap_or("OK"))
            }).collect::<Vec<_>>().join("\n"),
            failed_node.map(|n| {
                format!("Node: {}\nInput: {}\nError: {}",
                    n.node_id,
                    serde_json::to_string_pretty(&n.input).unwrap_or_default(),
                    n.error.as_deref().unwrap_or("Unknown"))
            }).unwrap_or_else(|| "No failed node found".to_string())
        );

        Ok(vec![PromptMessage::user(context)])
    }
}

struct CreateWorkflowPromptHandler;

#[async_trait]
impl PromptHandler for CreateWorkflowPromptHandler {
    async fn get_messages(&self, args: Value) -> rmcp::Result<Vec<PromptMessage>> {
        let description = args["description"].as_str()
            .ok_or_else(|| rmcp::Error::InvalidParams("Missing 'description'".into()))?;

        let prompt = format!(
            r#"# Create R8r Workflow

## User Request
{}

## R8r Workflow Format

R8r workflows are YAML files with this structure:

```yaml
name: workflow-name
description: What this workflow does
version: 1

triggers:
  - type: manual  # or: webhook, schedule

nodes:
  - id: step1
    type: transform  # or: http, agent, if, switch
    config:
      expression: '"Hello, world!"'

  - id: step2
    type: http
    config:
      url: "https://api.example.com/endpoint"
      method: POST
      headers:
        Authorization: "Bearer {{{{ env.API_KEY }}}}"
      body:
        data: "{{{{ nodes.step1.output }}}}"
    depends_on: [step1]

  - id: step3
    type: agent
    config:
      prompt: "Analyze this data: {{{{ nodes.step2.output.body }}}}"
    depends_on: [step2]
```

## Available Node Types

1. **transform** - Data transformation using Rhai expressions
2. **http** - Make HTTP requests
3. **agent** - Call AI agent for reasoning (connects to ZeptoClaw)
4. **if** - Conditional branching
5. **switch** - Multi-way routing

## Template Variables
- `{{{{ input.field }}}}` - Access workflow input
- `{{{{ nodes.node_id.output }}}}` - Access previous node output
- `{{{{ env.VAR_NAME }}}}` - Access environment variable

Please create a workflow YAML that accomplishes the user's request.
"#,
            description
        );

        Ok(vec![PromptMessage::user(prompt)])
    }
}
```

---

## Task 6: Create MCP Binary

**Files:**
- Create: `src/bin/r8r-mcp.rs`

```rust
//! R8r MCP Server binary
//!
//! Run with: r8r-mcp [--http PORT]

use clap::Parser;
use r8r::mcp::R8rMcpServer;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "r8r-mcp")]
#[command(about = "R8r MCP Server - Expose r8r workflows via Model Context Protocol")]
struct Cli {
    /// Run HTTP transport instead of stdio
    #[arg(long)]
    http: Option<u16>,

    /// Database path (default: ~/.r8r/r8r.db)
    #[arg(long, short)]
    db: Option<PathBuf>,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging (stderr to not interfere with stdio transport)
    if cli.debug {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("r8r=debug"))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    }

    // Get database path
    let db_path = cli.db.unwrap_or_else(|| {
        dirs::data_dir()
            .map(|d| d.join("r8r").join("r8r.db"))
            .unwrap_or_else(|| PathBuf::from(".r8r/r8r.db"))
    });

    // Ensure directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Create server
    let server = R8rMcpServer::new(&db_path)?;

    // Run with appropriate transport
    if let Some(port) = cli.http {
        eprintln!("Starting r8r MCP server on http://0.0.0.0:{}", port);
        server.run_http(port).await?;
    } else {
        // stdio transport (default for MCP)
        server.run_stdio().await?;
    }

    Ok(())
}
```

**Update Cargo.toml:**

```toml
[[bin]]
name = "r8r-mcp"
path = "src/bin/r8r-mcp.rs"
```

---

## Task 7: Add MCP Configuration Examples

**Files:**
- Create: `examples/mcp-config-claude-desktop.json`
- Create: `examples/mcp-config-claude-code.json`

**Claude Desktop configuration:**

```json
{
  "mcpServers": {
    "r8r": {
      "command": "r8r-mcp",
      "args": [],
      "env": {
        "R8R_DB": "~/.r8r/r8r.db"
      }
    }
  }
}
```

**Claude Code configuration (.claude/settings.json):**

```json
{
  "mcpServers": {
    "r8r": {
      "command": "r8r-mcp",
      "args": ["--db", "/path/to/r8r.db"]
    }
  }
}
```

---

## Task 8: Write Tests

**Files:**
- Create: `tests/mcp_integration.rs`

```rust
use r8r::mcp::R8rMcpServer;
use serde_json::json;

#[tokio::test]
async fn test_mcp_server_creation() {
    let server = R8rMcpServer::in_memory().unwrap();
    // Server should be created successfully
}

#[tokio::test]
async fn test_list_workflows_empty() {
    let server = R8rMcpServer::in_memory().unwrap();
    // Test listing when no workflows exist
}

#[tokio::test]
async fn test_create_and_execute_workflow() {
    let server = R8rMcpServer::in_memory().unwrap();

    // Create workflow
    let yaml = r#"
name: test-workflow
description: Test workflow
version: 1
triggers:
  - type: manual
nodes:
  - id: step1
    type: transform
    config:
      expression: '"Hello, MCP!"'
"#;

    // Create via tool
    // Execute via tool
    // Get trace via tool
}

#[tokio::test]
async fn test_debug_prompt() {
    let server = R8rMcpServer::in_memory().unwrap();
    // Test debug prompt generation
}
```

---

## Verification

After implementation, run:

```bash
# Build
cargo build --release

# Test MCP server locally
echo '{"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{}},"id":1}' | ./target/release/r8r-mcp

# Run tests
cargo test mcp

# Test with Claude Desktop
# Add to ~/Library/Application Support/Claude/claude_desktop_config.json
```

---

## MCP Server Capabilities Summary

| Capability | Tools | Resources | Prompts |
|------------|-------|-----------|---------|
| Execute workflow | ✅ `r8r_execute` | | |
| List workflows | ✅ `r8r_list_workflows` | ✅ `r8r://workflows` | |
| Get workflow | ✅ `r8r_get_workflow` | ✅ `r8r://workflows/{name}` | |
| Get execution | ✅ `r8r_get_execution` | ✅ `r8r://executions/{id}` | |
| Debug trace | ✅ `r8r_get_trace` | | ✅ `debug_workflow` |
| Resume failed | ✅ `r8r_resume` | | |
| Validate YAML | ✅ `r8r_validate` | | |
| Create workflow | ✅ `r8r_create_workflow` | | ✅ `create_workflow` |
