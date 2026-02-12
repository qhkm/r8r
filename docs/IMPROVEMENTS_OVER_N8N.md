# R8r: Improvements Over n8n

> Research-based analysis of n8n pain points and how r8r can do better.

Based on research from [n8n Community Forums](https://community.n8n.io/), [Reddit discussions](https://www.toksta.com/products/n8n), and [user feedback](https://community.n8n.io/t/great-idea-terrible-software/29304).

---

## 1. Performance & Memory

### n8n Problems
- **Memory bloat**: [Workflows cause exponential slowdown](https://community.n8n.io/t/n8n-workflow-memory-bloat-processing-daily-sales-data-causes-exponential-slowdown-and-stalls/114385) when processing large datasets
- **Out of Memory crashes**: [JavaScript heap out of memory](https://community.n8n.io/t/n8n-error-out-of-memory-slow-ui-502-bad-gateway/234534) errors are common
- **Slow AI Agent**: [2+ minutes to process 5-10 entries](https://community.n8n.io/t/why-is-the-ai-agent-in-n8n-is-extremely-slow-when-processing-data/73442)
- **Memory leaks**: [Updates introduce memory issues](https://community.n8n.io/t/memory-leaks-with-the-new-n8n-updates/128842) requiring RAM upgrades
- **Large dataset limit**: Performance degrades beyond 100K rows

### R8r Solution
```rust
// Rust advantages:
// - No garbage collection pauses
// - Predictable memory usage
// - Zero-cost abstractions
// - Streaming data processing

pub struct StreamingExecutor {
    chunk_size: usize,  // Process in chunks, not all at once
    max_memory_mb: usize,
}

impl StreamingExecutor {
    /// Process large datasets without loading everything into memory
    pub async fn execute_streaming<S: Stream<Item = Value>>(
        &self,
        workflow: &Workflow,
        input_stream: S,
    ) -> Result<impl Stream<Item = Value>> {
        // Process chunk by chunk
        input_stream
            .chunks(self.chunk_size)
            .map(|chunk| self.process_chunk(workflow, chunk))
    }
}
```

**Key improvements:**
- **Rust native**: No JavaScript heap, no GC pauses
- **Streaming execution**: Process millions of rows without loading all into memory
- **Predictable memory**: Set hard limits, graceful degradation
- **Binary size**: 12MB vs n8n's 500MB+ Docker image

---

## 2. Debugging Experience

### n8n Problems
- **[Subworkflow debugging is hard](https://community.n8n.io/t/how-to-debug-subworkflows/97720)**: Missing parent data when testing
- **[Silent failures](https://medium.com/@echosilo/why-i-built-n8n-debug-mcp-for-real-world-n8n-workflow-debugging-2ef8e7ee803d)**: Workflows fail quietly, hard to trace
- **[Hanging workflows](https://community.n8n.io/t/how-do-you-debug-forever-executing-workflows-with-no-progress/26252)**: Spinner with no progress, no way to see what's stuck
- **Limited logging**: Must add Code nodes with console.log manually

### R8r Solution
```yaml
# Built-in debug mode with full tracing
name: my-workflow
debug:
  enabled: true
  trace_level: detailed  # minimal | normal | detailed | verbose
  capture_inputs: true
  capture_outputs: true
  timing: true
```

```rust
// Execution trace with full context
pub struct ExecutionTrace {
    pub execution_id: String,
    pub nodes: Vec<NodeTrace>,
    pub timeline: Vec<TraceEvent>,
}

pub struct NodeTrace {
    pub node_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: NodeStatus,
    pub input: Value,           // What went in
    pub output: Option<Value>,  // What came out
    pub error: Option<String>,
    pub logs: Vec<LogEntry>,    // Node-level logs
    pub memory_used_bytes: u64,
    pub parent_context: Value,  // Full context from parent workflow
}

// Live execution monitoring
pub struct LiveMonitor {
    pub async fn watch(&self, execution_id: &str) -> impl Stream<Item = TraceEvent> {
        // Real-time updates as nodes execute
    }
}
```

**Key improvements:**
- **Full execution trace**: Every input/output captured automatically
- **Live monitoring**: Watch execution in real-time, see exactly where it's stuck
- **Parent context preservation**: Subworkflows get full parent state
- **Structured logs**: Not just console.log, but structured log entries per node
- **Memory tracking**: See memory usage per node

---

## 3. Error Handling

### n8n Problems
- **[Confusing error workflows](https://community.n8n.io/t/best-practice-for-workflow-error-handling/136458)**: Users struggle to set up proper error handling
- **Separate error workflow required**: Can't handle errors inline
- **Limited retry control**: Basic retry, no exponential backoff patterns
- **No partial failure recovery**: If node 5 fails, can't resume from node 5

### R8r Solution
```yaml
# Inline error handling per node
nodes:
  - id: risky-api-call
    type: http
    config:
      url: "https://api.example.com/data"

    # Built-in retry with exponential backoff
    retry:
      max_attempts: 5
      initial_delay_ms: 1000
      max_delay_ms: 30000
      backoff: exponential  # linear | exponential | fibonacci
      retry_on: [500, 502, 503, 504, 429]

    # Inline error handling
    on_error:
      action: fallback      # fail | skip | fallback | retry
      fallback_value:
        data: []
        source: "cache"

    # Or route to error branch
    on_error:
      action: branch
      branch: error-handler

  - id: error-handler
    type: transform
    config:
      expression: |
        #{
          error: error.message,
          node: error.node_id,
          retries: error.retry_count,
          timestamp: now()
        }
```

```rust
// Resume from failure
impl Executor {
    /// Resume a failed execution from the failed node
    pub async fn resume(
        &self,
        execution_id: &str,
        from_node: Option<&str>,  // None = resume from failed node
        modified_input: Option<Value>,
    ) -> Result<Execution> {
        let failed = self.storage.get_execution(execution_id).await?;
        let checkpoint = self.get_checkpoint(&failed, from_node)?;

        // Resume with all previous node outputs preserved
        self.execute_from_checkpoint(checkpoint, modified_input).await
    }
}
```

**Key improvements:**
- **Inline error handling**: No separate error workflow needed
- **Smart retries**: Exponential backoff, jitter, circuit breaker patterns
- **Resume from failure**: Don't re-run successful nodes
- **Partial results**: Access results from nodes that succeeded before failure
- **Error codes**: Structured error codes for programmatic handling

---

## 4. Version Control

### n8n Problems
- **[Enterprise-only](https://medium.com/@Quaxel/n8n-workflow-versioning-without-who-changed-this-f6007b5db1be)**: Version control costs €667+/month
- **[No PR workflow](https://docs.n8n.io/source-control-environments/)**: No review process in n8n itself
- **[Tag sync bugs](https://github.com/n8n-io/n8n/issues/24999)**: Tags don't sync correctly
- **Conflict hell**: Multiple people editing = overwrites

### R8r Solution
```yaml
# Workflows are just YAML files - use any VCS!

# .r8r/config.yaml
versioning:
  enabled: true
  auto_commit: false  # Manual commits preferred

# Built-in diff support
```

```bash
# Native git workflow - no special tooling needed
$ git diff workflows/order-processor.yaml
$ git log --oneline workflows/
$ git blame workflows/order-processor.yaml

# R8r CLI helpers
$ r8r workflows diff order-processor --from v1.0 --to v1.1
$ r8r workflows history order-processor
$ r8r workflows rollback order-processor --to v1.0
```

```rust
// Built-in versioning (free, not enterprise)
pub struct WorkflowVersion {
    pub version: u32,
    pub workflow_id: String,
    pub definition: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub changelog: Option<String>,
    pub checksum: String,  // Detect changes
}

impl Storage {
    pub async fn save_workflow_version(&self, workflow: &Workflow, changelog: &str) -> Result<u32>;
    pub async fn get_workflow_versions(&self, name: &str) -> Result<Vec<WorkflowVersion>>;
    pub async fn rollback_workflow(&self, name: &str, version: u32) -> Result<()>;
    pub async fn diff_versions(&self, name: &str, v1: u32, v2: u32) -> Result<WorkflowDiff>;
}
```

**Key improvements:**
- **YAML files**: Standard git workflow works perfectly
- **Free versioning**: Built into core, not enterprise-only
- **Diff & blame**: Standard git tools work
- **Rollback**: One command to revert
- **Conflict resolution**: Git handles it, not custom tooling

---

## 5. Agent-First Design

### n8n Problems
- **UI-first**: Designed for humans clicking through UI
- **Verbose JSON**: Hard for AI to generate complex workflows
- **No programmatic creation**: API exists but awkward

### R8r Solution
```yaml
# LLM-friendly YAML patterns
name: process-order
description: |  # Rich description for AI understanding
  Process incoming orders:
  1. Validate order data
  2. Check inventory
  3. Calculate pricing
  4. Send confirmation

# Consistent, predictable structure
nodes:
  - id: validate
    type: transform
    config:
      expression: |
        # AI can easily understand and generate
        validate_order(from_json(input))
```

```rust
// Tool interface designed for AI agents
pub struct R8rTool {
    // Simple, predictable inputs
    // Structured, parseable outputs
}

// Response format optimized for LLM parsing
pub struct WorkflowResult {
    pub status: String,           // "completed" | "failed" | "running"
    pub output: Value,            // Structured output
    pub error: Option<ErrorInfo>, // Parseable error info
    pub execution_id: String,     // For follow-up queries
}

pub struct ErrorInfo {
    pub code: String,        // "HTTP_TIMEOUT", "VALIDATION_FAILED"
    pub message: String,     // Human-readable
    pub node_id: String,     // Where it failed
    pub retryable: bool,     // Can agent retry?
    pub suggestion: String,  // What to do next
}
```

**Key improvements:**
- **YAML over JSON**: More readable, less verbose
- **Consistent patterns**: AI can learn and generate reliably
- **Structured errors**: Agent can parse and decide next action
- **Tool-native**: Designed to be called by AI agents

---

## 6. Self-Hosting Experience

### n8n Problems
- **[Complex setup](https://www.toksta.com/products/n8n)**: Docker, PostgreSQL, Redis, reverse proxy
- **[500MB+ image](https://hub.docker.com/r/n8nio/n8n)**: Large footprint
- **Node.js dependencies**: npm ecosystem complexity
- **Resource hungry**: Needs 2GB+ RAM

### R8r Solution
```bash
# Single binary, zero dependencies
$ curl -L https://github.com/r8r/releases/latest/r8r-darwin-arm64 -o r8r
$ chmod +x r8r
$ ./r8r server

# That's it. Running on port 8080.
```

```
# Resource comparison
┌─────────────┬─────────────┬────────────┐
│             │ n8n         │ r8r        │
├─────────────┼─────────────┼────────────┤
│ Binary size │ 500MB+      │ 12MB       │
│ RAM minimum │ 2GB         │ 128MB      │
│ Dependencies│ Node, npm   │ None       │
│ Database    │ PostgreSQL  │ SQLite     │
│ Startup     │ 10-30s      │ <1s        │
│ Docker      │ Required    │ Optional   │
└─────────────┴─────────────┴────────────┘
```

**Key improvements:**
- **Single binary**: Download and run
- **Embedded SQLite**: No external database needed
- **Low resources**: Runs on Raspberry Pi
- **Fast startup**: Sub-second cold start
- **No npm**: No node_modules hell

---

## 7. Pricing & Features

### n8n Problems
- **[SSO locked to Enterprise](https://www.toksta.com/products/n8n)**: €667+/month
- **Folders locked**: Organization features paywalled
- **Version control locked**: Basic feature costs enterprise pricing
- **Collaboration limited**: Sharing restricted on cloud

### R8r Solution
```
# Everything is free and open source

✅ Version control - Free
✅ Workflow folders/organization - Free
✅ Full API access - Free
✅ Unlimited executions - Free
✅ All node types - Free
✅ Error workflows - Free
✅ Subworkflows - Free
✅ Webhooks - Free
✅ Scheduling - Free
```

---

## 8. Expression Language

### n8n Problems
- **JavaScript expressions**: Complex for non-programmers
- **Inconsistent syntax**: `{{ }}` vs `$json` vs `$node`
- **Limited sandboxing**: Security concerns

### R8r Solution
```yaml
# Rhai: Safe, sandboxed, simple
nodes:
  - id: transform
    type: transform
    config:
      expression: |
        // Rhai - safe by default
        let data = from_json(input);

        // Simple syntax
        let total = data.items
          .filter(|x| x.active)
          .map(|x| x.price)
          .sum();

        // Return JSON
        to_json(#{
          total: total,
          count: data.items.len()
        })
```

**Rhai advantages:**
- **Sandboxed**: No file system, no network access
- **Memory limited**: Can't allocate unbounded memory
- **Operation limited**: Built-in loop protection
- **Simple syntax**: Rust-like, easy to learn
- **Fast**: Compiled to bytecode

---

## Summary: R8r vs n8n

| Aspect | n8n | r8r |
|--------|-----|-----|
| **Performance** | JS/Memory issues | Rust/Streaming |
| **Binary size** | 500MB+ | 12MB |
| **RAM requirement** | 2GB+ | 128MB |
| **Startup time** | 10-30s | <1s |
| **Debugging** | Limited, UI-focused | Full tracing, CLI |
| **Error handling** | Separate workflow | Inline + resume |
| **Version control** | Enterprise only | Git-native, free |
| **Self-hosting** | Complex | Single binary |
| **Target user** | Human in UI | AI agents |
| **Expression lang** | JavaScript | Rhai (sandboxed) |
| **Large datasets** | 100K limit | Streaming millions |

---

## Sources

- [n8n Community: Great idea, terrible software](https://community.n8n.io/t/great-idea-terrible-software/29304)
- [n8n Community: I'm really disappointed](https://community.n8n.io/t/im-really-disappointed/261856)
- [n8n Memory Issues](https://community.n8n.io/t/memory-performance-issues/80087)
- [n8n Workflow Memory Bloat](https://community.n8n.io/t/n8n-workflow-memory-bloat-processing-daily-sales-data-causes-exponential-slowdown-and-stalls/114385)
- [n8n Slow AI Agent](https://community.n8n.io/t/why-is-the-ai-agent-in-n8n-is-extremely-slow-when-processing-data/73442)
- [n8n Debugging Subworkflows](https://community.n8n.io/t/how-to-debug-subworkflows/97720)
- [n8n Debug MCP](https://medium.com/@echosilo/why-i-built-n8n-debug-mcp-for-real-world-n8n-workflow-debugging-2ef8e7ee803d)
- [n8n Version Control (Enterprise)](https://medium.com/@Quaxel/n8n-workflow-versioning-without-who-changed-this-f6007b5db1be)
- [n8n Git Integration](https://docs.n8n.io/source-control-environments/)
- [n8n Review - Toksta](https://www.toksta.com/products/n8n)
- [n8n Error Handling](https://docs.n8n.io/flow-logic/error-handling/)
