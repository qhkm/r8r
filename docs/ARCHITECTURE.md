# r8r Architecture

> Rust workflow automation engine - **agent-first**, lightweight, edge-ready.

## Vision

r8r is an **agent-first** workflow automation engine designed to complement [ZeptoClaw](https://github.com/kitakod/zeptoclaw) (AI agent framework). While ZeptoClaw handles intelligent, adaptive tasks through LLM reasoning, r8r provides **deterministic, repeatable workflows** that agents can invoke reliably.

### Why Agent-First?

Traditional workflow tools (n8n, Zapier) are **human-first**: visual editors, drag-drop, manual configuration. r8r flips this:

| Aspect | Human-First (n8n) | Agent-First (r8r) |
|--------|-------------------|-------------------|
| Primary User | Humans via browser | AI agents via API |
| Workflow Creation | Drag-drop visual editor | LLM generates YAML |
| Configuration | Click through forms | Structured YAML/JSON |
| Debugging | Human inspects UI | Structured logs + traces |
| Error Handling | Human intervenes | Agent parses + retries |
| Integration | Human sets up webhooks | Direct API calls |

### Design Philosophy

1. **LLM-Friendly YAML** - Consistent patterns, no ambiguity, easy for agents to generate
2. **API-First** - REST API is primary interface; Web UI is optional debugging tool
3. **Structured Everything** - Errors, logs, traces all JSON for agent consumption
4. **ZeptoClaw Symbiosis** - Bidirectional integration (ZeptoClaw → r8r, r8r → ZeptoClaw)
5. **Human-Debuggable** - Clear logs, readable YAML, good error messages

### The ZeptoClaw + r8r Stack

```
┌─────────────────────────────────────────────────────────────┐
│                    User Request                              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     ZeptoClaw (AI Agent)                     │
│  • Understands intent         • Makes decisions              │
│  • Handles ambiguity          • Adapts to context            │
│  • Uses tools                 • Manages conversation         │
└─────────────────────────────────────────────────────────────┘
              │                               │
              │ "I need to do                 │ "Run the order
              │  something complex"           │  notification workflow"
              ▼                               ▼
┌──────────────────────┐         ┌──────────────────────────────┐
│   ZeptoClaw Tools    │         │        r8r (Workflows)        │
│  • shell             │         │  • Deterministic execution    │
│  • filesystem        │         │  • Retry logic built-in       │
│  • web_search        │         │  • Scheduled/triggered        │
│  • (custom tools)    │         │  • Multi-step orchestration   │
└──────────────────────┘         └──────────────────────────────┘
                                              │
                                              │ "Need AI decision
                                              │  at step 3"
                                              ▼
                                 ┌──────────────────────────────┐
                                 │   ZeptoClaw (via agent node)  │
                                 │  • Classify this email        │
                                 │  • Generate response          │
                                 │  • Decide next action         │
                                 └──────────────────────────────┘
```

r8r combines:
- **XERV's** performance and minimal footprint
- **Trigger.dev's** developer experience
- **Agent-native** design patterns

## Target Use Cases

| Use Case | r8r Approach |
|----------|--------------|
| **Agent Orchestration** | ZeptoClaw invokes r8r for deterministic sub-tasks |
| **Hybrid AI Workflows** | r8r workflow calls ZeptoClaw for AI decisions |
| SaaS Integrations | Pre-built nodes + HTTP node for custom |
| Data Pipelines (ETL) | Arena allocator, streaming, 100K+ rows/min |
| Background Jobs | Cron triggers, webhook triggers, queue-based |
| Edge/On-Prem | Single ~5MB binary, SQLite (no Postgres needed) |
| Malaysian E-Commerce | WhatsApp, Google Sheets, Shopee nodes built-in |

### When to Use What

| Task Type | Use | Why |
|-----------|-----|-----|
| "Send order confirmation to new customers" | **r8r** | Deterministic, same every time |
| "Analyze this customer complaint and respond appropriately" | **ZeptoClaw** | Requires reasoning, context |
| "Every hour, check orders and notify if > RM500" | **r8r** | Scheduled, rule-based |
| "Help me draft a response to this email" | **ZeptoClaw** | Creative, conversational |
| "Process all orders: notify customer, update sheet, log" | **r8r** | Multi-step orchestration |
| "Classify these support tickets by urgency" | **r8r + agent node** | Workflow with AI step |

## Core Principles

1. **Single Binary** - No runtime dependencies, just `./r8r`
2. **Minimal Memory** - Target <50MB for 1000 concurrent workflows
3. **SQLite First** - No Postgres required (optional for scale)
4. **Async Everything** - Tokio runtime, non-blocking I/O
5. **Type-Safe Workflows** - YAML/JSON schema validation
6. **Hot Reload** - Change workflows without restart

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                          r8r Engine                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │   Triggers  │  │   Nodes     │  │  Executor   │              │
│  │  ─────────  │  │  ─────────  │  │  ─────────  │              │
│  │  • Cron     │  │  • HTTP     │  │  • Queue    │              │
│  │  • Webhook  │  │  • WhatsApp │  │  • Workers  │              │
│  │  • Manual   │  │  • GSheets  │  │  • Retry    │              │
│  │  • Event    │  │  • Transform│  │  • Timeout  │              │
│  └─────────────┘  └─────────────┘  └─────────────┘              │
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │   Storage   │  │   API       │  │   Web UI    │              │
│  │  ─────────  │  │  ─────────  │  │  ─────────  │              │
│  │  • SQLite   │  │  • REST     │  │  • React    │              │
│  │  • (Postgres)│  │  • WebSocket│  │  • Editor   │              │
│  │  • Files    │  │  • GraphQL? │  │  • Monitor  │              │
│  └─────────────┘  └─────────────┘  └─────────────┘              │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Model

### Workflow Definition (YAML)

```yaml
name: order-notification
description: Send WhatsApp when new order in Google Sheet
version: 1

triggers:
  - type: cron
    schedule: "*/5 * * * *"  # Every 5 minutes

nodes:
  - id: fetch-orders
    type: google-sheets
    config:
      spreadsheet_id: "1abc..."
      range: "Orders!A:H"
      action: read

  - id: filter-new
    type: transform
    config:
      expression: |
        items.filter(row => row.status === "New")
    depends_on: [fetch-orders]

  - id: send-whatsapp
    type: whatsapp
    config:
      to: "{{ item.phone }}"
      template: order_confirmation
      params:
        - "{{ item.order_id }}"
        - "{{ item.amount }}"
    depends_on: [filter-new]
    for_each: true  # Run for each filtered item

  - id: update-status
    type: google-sheets
    config:
      spreadsheet_id: "1abc..."
      range: "Orders!G{{ item.row }}"
      action: update
      values: [["Notified"]]
    depends_on: [send-whatsapp]
```

### Execution Record (SQLite)

```sql
CREATE TABLE workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    definition TEXT NOT NULL,  -- YAML/JSON
    enabled BOOLEAN DEFAULT true,
    created_at DATETIME,
    updated_at DATETIME
);

CREATE TABLE executions (
    id TEXT PRIMARY KEY,
    workflow_id TEXT REFERENCES workflows(id),
    status TEXT,  -- pending, running, completed, failed
    trigger_type TEXT,
    started_at DATETIME,
    finished_at DATETIME,
    error TEXT
);

CREATE TABLE node_executions (
    id TEXT PRIMARY KEY,
    execution_id TEXT REFERENCES executions(id),
    node_id TEXT,
    status TEXT,
    input TEXT,   -- JSON
    output TEXT,  -- JSON
    started_at DATETIME,
    finished_at DATETIME,
    error TEXT
);
```

---

## Node Types

### Core Nodes (Built-in)

| Node | Description |
|------|-------------|
| `http` | Make HTTP requests (GET, POST, etc.) |
| `transform` | JavaScript/Rhai expressions for data transformation |
| `filter` | Filter items based on condition |
| `split` | Split array into individual items |
| `merge` | Merge multiple inputs |
| `switch` | Conditional branching |
| `wait` | Delay execution |
| `set` | Set variables |
| **`agent`** | **Call ZeptoClaw for AI-powered decisions** |

### Agent Node (ZeptoClaw Integration)

The `agent` node allows workflows to call ZeptoClaw for tasks requiring AI reasoning:

```yaml
nodes:
  - id: classify-ticket
    type: agent
    config:
      # ZeptoClaw connection
      endpoint: "http://localhost:3000/api/chat"  # or use shared config

      # The prompt - can use template variables
      prompt: |
        Classify this support ticket by urgency (low/medium/high/critical):

        Subject: {{ item.subject }}
        Body: {{ item.body }}
        Customer: {{ item.customer_name }} ({{ item.plan }})

        Respond with JSON: {"urgency": "...", "reason": "..."}

      # Optional: specific model
      model: "claude-sonnet-4-20250514"

      # Parse response as JSON
      response_format: json

    depends_on: [fetch-tickets]

  - id: route-by-urgency
    type: switch
    config:
      field: "{{ nodes.classify-ticket.output.urgency }}"
      cases:
        critical: escalate-to-human
        high: notify-senior-support
        medium: auto-respond
        low: queue-for-later
    depends_on: [classify-ticket]
```

**When to use `agent` node:**
- Classification tasks (urgency, category, sentiment)
- Content generation (responses, summaries)
- Decision making with context
- Natural language processing
- Anything requiring "reasoning"

### Integration Nodes (Phase 1 - Malaysian Focus)

| Node | Description |
|------|-------------|
| `whatsapp` | WhatsApp Cloud API |
| `google-sheets` | Google Sheets read/write |
| `telegram` | Telegram Bot API |
| `email` | SMTP/Resend/SendGrid |
| `webhook` | Receive webhooks |

### Integration Nodes (Phase 2)

| Node | Description |
|------|-------------|
| `slack` | Slack API |
| `discord` | Discord webhooks/bot |
| `notion` | Notion API |
| `airtable` | Airtable API |
| `stripe` | Stripe payments |
| `shopify` | Shopify API |

---

## ZeptoClaw Integration

### ZeptoClaw → r8r (Agent Invokes Workflow)

ZeptoClaw has an `r8r` tool that can trigger workflows:

```rust
// In ZeptoClaw's tool registry
Tool::new("r8r")
    .description("Run a deterministic workflow for repeatable tasks")
    .parameters(json!({
        "type": "object",
        "properties": {
            "workflow": {
                "type": "string",
                "description": "Workflow name or ID"
            },
            "inputs": {
                "type": "object",
                "description": "Input data for the workflow"
            },
            "wait": {
                "type": "boolean",
                "description": "Wait for completion (default: true)"
            }
        },
        "required": ["workflow"]
    }))
```

**Example conversation:**

```
User: "Send order confirmations to all new orders today"

ZeptoClaw thinking: "This is a repeatable task with clear rules.
I should use the r8r workflow instead of doing it manually."

ZeptoClaw: [calls r8r tool]
{
  "workflow": "order-notification",
  "inputs": {
    "date_filter": "today",
    "status_filter": "new"
  }
}

r8r: {
  "execution_id": "exec_abc123",
  "status": "completed",
  "summary": {
    "orders_processed": 15,
    "notifications_sent": 15,
    "errors": 0
  },
  "duration_ms": 2340
}

ZeptoClaw: "Done! I ran the order-notification workflow and sent
confirmations to 15 new orders. All successful, no errors."
```

### r8r → ZeptoClaw (Workflow Calls Agent)

When workflows need AI reasoning, they use the `agent` node:

```yaml
name: smart-ticket-router
description: Route support tickets with AI classification

nodes:
  - id: fetch-tickets
    type: http
    config:
      url: "https://api.freshdesk.com/tickets"
      method: GET

  - id: classify
    type: agent  # ← Calls ZeptoClaw
    config:
      prompt: "Classify ticket urgency: {{ item.subject }}"
      response_format: json
    depends_on: [fetch-tickets]
    for_each: true

  - id: route
    type: switch
    config:
      field: "{{ nodes.classify.output.urgency }}"
      cases:
        critical: page-oncall
        high: slack-senior
        low: auto-respond
    depends_on: [classify]
```

### Shared Configuration

r8r and ZeptoClaw share credentials via environment or config file:

```toml
# ~/.config/kitakod/config.toml (shared)

[providers.anthropic]
api_key = "sk-ant-..."

[providers.openai]
api_key = "sk-..."

[integrations.whatsapp]
access_token = "..."
phone_number_id = "..."

[integrations.google]
service_account_path = "~/.config/kitakod/google-sa.json"
```

Both tools read from this shared config, avoiding duplication.

---

## Execution Engine

### Queue-Based Execution

```rust
pub struct Executor {
    queue: Arc<WorkQueue>,
    workers: Vec<Worker>,
    node_registry: NodeRegistry,
}

impl Executor {
    pub async fn run_workflow(&self, workflow: &Workflow, trigger: TriggerContext) {
        let execution = Execution::new(workflow, trigger);

        // Topological sort nodes by dependencies
        let order = workflow.topological_sort();

        for node_id in order {
            let node = workflow.get_node(node_id);
            let input = self.gather_inputs(node, &execution);

            // Execute node
            let result = self.execute_node(node, input).await;

            // Handle for_each
            if node.for_each && result.is_array() {
                for item in result.as_array() {
                    self.queue.push(NodeTask { node_id, item });
                }
            }

            execution.record_output(node_id, result);
        }
    }
}
```

### Retry & Error Handling

```yaml
nodes:
  - id: send-whatsapp
    type: whatsapp
    config: ...
    retry:
      max_attempts: 3
      delay_seconds: 60
      backoff: exponential
    on_error:
      continue: true  # Don't fail workflow
      fallback_node: send-email  # Try email instead
```

---

## API Design (Agent-Friendly)

The API is designed for **agent consumption** - structured JSON, predictable error codes, parseable responses.

### REST API

```
GET    /api/workflows              # List workflows
POST   /api/workflows              # Create workflow (accepts YAML or JSON)
GET    /api/workflows/:id          # Get workflow
PUT    /api/workflows/:id          # Update workflow
DELETE /api/workflows/:id          # Delete workflow

POST   /api/workflows/:id/execute  # Execute workflow (primary agent endpoint)
GET    /api/workflows/:id/executions  # Execution history

GET    /api/executions/:id         # Execution details
GET    /api/executions/:id/logs    # Execution logs (structured JSON)
GET    /api/executions/:id/trace   # Full execution trace

POST   /api/webhooks/:id           # Webhook trigger endpoint
```

### Agent-Friendly Response Format

All responses follow a consistent structure agents can parse:

```json
// Success response
{
  "success": true,
  "data": {
    "execution_id": "exec_abc123",
    "workflow_id": "order-notification",
    "status": "completed",
    "started_at": "2024-02-13T10:00:00Z",
    "finished_at": "2024-02-13T10:00:02Z",
    "duration_ms": 2340,
    "summary": {
      "nodes_executed": 4,
      "items_processed": 15,
      "errors": 0
    },
    "outputs": {
      "send-whatsapp": { "sent": 15, "failed": 0 },
      "update-status": { "updated": 15 }
    }
  }
}

// Error response (agent can parse and retry)
{
  "success": false,
  "error": {
    "code": "RATE_LIMITED",
    "message": "WhatsApp API rate limit exceeded",
    "node_id": "send-whatsapp",
    "retry_after_ms": 60000,
    "partial_results": {
      "completed": 10,
      "remaining": 5
    }
  }
}
```

### Error Codes (Agent-Parseable)

| Code | Meaning | Agent Action |
|------|---------|--------------|
| `WORKFLOW_NOT_FOUND` | Workflow doesn't exist | Check workflow name |
| `VALIDATION_ERROR` | Invalid input | Fix input data |
| `RATE_LIMITED` | API rate limit | Wait `retry_after_ms` |
| `AUTH_FAILED` | Credentials invalid | Report to user |
| `NODE_FAILED` | Node execution failed | Check `node_id`, `error_details` |
| `TIMEOUT` | Execution timed out | Retry or break into smaller batches |
| `PARTIAL_SUCCESS` | Some items failed | Check `partial_results` |

### WebSocket (Real-time Streaming)

```
WS /api/ws
  → subscribe: { execution_id: "exec_abc123" }
  ← { "event": "node_started", "node_id": "fetch-orders", "timestamp": "..." }
  ← { "event": "node_completed", "node_id": "fetch-orders", "output": {...} }
  ← { "event": "node_started", "node_id": "send-whatsapp", "item_index": 0 }
  ← { "event": "execution_completed", "summary": {...} }
```

### Execution Trace (for Debugging)

The `/executions/:id/trace` endpoint returns a complete execution trace:

```json
{
  "execution_id": "exec_abc123",
  "workflow": "order-notification",
  "trace": [
    {
      "timestamp": "2024-02-13T10:00:00.000Z",
      "node_id": "fetch-orders",
      "event": "started",
      "input": { "spreadsheet_id": "1abc..." }
    },
    {
      "timestamp": "2024-02-13T10:00:00.500Z",
      "node_id": "fetch-orders",
      "event": "completed",
      "output": { "rows": 15 },
      "duration_ms": 500
    },
    {
      "timestamp": "2024-02-13T10:00:00.501Z",
      "node_id": "filter-new",
      "event": "started",
      "input": { "items_count": 15 }
    }
    // ... complete trace
  ]
}
```

This allows agents (and humans) to understand exactly what happened during execution.

---

## CLI Design

```bash
# Workflow management
r8r workflows list
r8r workflows create order-notification.yaml
r8r workflows run order-notification
r8r workflows logs order-notification

# Server mode
r8r server                    # Start API + UI
r8r server --port 8080
r8r server --no-ui            # API only

# Development
r8r dev order-notification.yaml  # Watch mode, hot reload

# Credentials
r8r credentials set whatsapp --token xxx
r8r credentials list
```

---

## Project Structure

```
r8r/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry
│   ├── lib.rs               # Library exports
│   ├── api/                 # REST API
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   └── handlers.rs
│   ├── engine/              # Execution engine
│   │   ├── mod.rs
│   │   ├── executor.rs
│   │   ├── queue.rs
│   │   └── scheduler.rs
│   ├── nodes/               # Node implementations
│   │   ├── mod.rs
│   │   ├── registry.rs
│   │   ├── http.rs
│   │   ├── transform.rs
│   │   ├── whatsapp.rs
│   │   └── gsheets.rs
│   ├── triggers/            # Trigger implementations
│   │   ├── mod.rs
│   │   ├── cron.rs
│   │   ├── webhook.rs
│   │   └── manual.rs
│   ├── storage/             # Database layer
│   │   ├── mod.rs
│   │   ├── sqlite.rs
│   │   └── models.rs
│   ├── workflow/            # Workflow parsing/validation
│   │   ├── mod.rs
│   │   ├── parser.rs
│   │   └── validator.rs
│   └── config/              # Configuration
│       ├── mod.rs
│       └── types.rs
├── ui/                      # React frontend (optional)
│   ├── package.json
│   └── src/
├── examples/                # Example workflows
│   ├── order-notification.yaml
│   └── inventory-alert.yaml
└── tests/
    ├── integration.rs
    └── workflows/
```

---

## Technology Choices

| Component | Choice | Reason |
|-----------|--------|--------|
| Runtime | Tokio | Async, battle-tested |
| HTTP Server | Axum | Fast, ergonomic, tower ecosystem |
| Database | SQLite (rusqlite) | Zero config, embedded |
| Serialization | serde + serde_yaml | Standard |
| Expression Engine | Rhai or mini-js | Sandboxed, fast |
| Cron | tokio-cron-scheduler | Native async |
| HTTP Client | reqwest | Full-featured |
| CLI | clap | Standard |
| Logging | tracing | Structured, async-aware |

---

## Development Phases

### Phase 1: Core Engine (MVP)
- [ ] Workflow YAML parser
- [ ] Execution engine with queue
- [ ] Core nodes: HTTP, Transform, Filter
- [ ] SQLite storage
- [ ] CLI: create, run, list
- [ ] Cron trigger
- [ ] Manual trigger

### Phase 2: Malaysian Integrations
- [ ] WhatsApp Cloud API node
- [ ] Google Sheets node
- [ ] Telegram node
- [ ] Webhook trigger
- [ ] REST API

### Phase 3: Web UI
- [ ] React workflow editor
- [ ] Execution monitoring
- [ ] Credentials management
- [ ] Real-time WebSocket updates

### Phase 4: Scale & Polish
- [ ] Postgres support
- [ ] Distributed execution
- [ ] More integrations (Slack, Discord, etc.)
- [ ] Plugin system for custom nodes
- [ ] Cloud hosted version

---

## Competitive Positioning

| Feature | n8n | Trigger.dev | XERV | r8r |
|---------|-----|-------------|------|-----|
| **Primary User** | **Humans** | **Developers** | **Developers** | **AI Agents** |
| Language | TypeScript | TypeScript | Rust | Rust |
| Binary Size | ~500MB | N/A (hosted) | ~5MB | ~10MB target |
| Memory | High | N/A | Low | Low |
| Self-hosted | ✅ | ✅ | ✅ | ✅ |
| Edge-ready | ❌ | ❌ | ✅ | ✅ |
| Visual Editor | ✅ | ❌ | ❌ | Debug UI (Phase 3) |
| Pre-built Nodes | 400+ | 50+ | Few | 20+ (growing) |
| **Agent Integration** | ❌ | ❌ | ❌ | **Native (ZeptoClaw)** |
| **LLM-Friendly YAML** | ❌ | ❌ | ❌ | **✅** |
| **Structured Traces** | Partial | ✅ | ❌ | **✅** |
| Malaysian Focus | ❌ | ❌ | ❌ | ✅ |

### The Agent-First Advantage

Why r8r wins in the AI era:

1. **LLMs can generate r8r workflows** - Consistent YAML patterns are easy for Claude/GPT to produce
2. **ZeptoClaw native integration** - Bidirectional, not bolted-on
3. **Structured everything** - Agents can parse errors, retry intelligently
4. **Hybrid workflows** - Deterministic steps + AI decisions where needed
5. **Debuggable by humans** - When things go wrong, humans can read the traces

---

## Success Metrics

### Performance
1. **Binary size** < 15MB
2. **Memory usage** < 50MB for 100 concurrent workflows
3. **Throughput** > 10,000 executions/minute
4. **Startup time** < 1 second
5. **Node latency** < 10ms overhead per node

### Agent Experience
6. **YAML generation success rate** > 95% (LLM-generated workflows that validate)
7. **Error parseability** 100% (all errors return structured JSON)
8. **API response consistency** 100% (predictable response shapes)
9. **Trace completeness** 100% (every execution has full trace)

### Integration
10. **ZeptoClaw round-trip** < 100ms (agent calls r8r, gets response)
11. **Agent node latency** < 2s (r8r calls ZeptoClaw for AI decision)

---

## Example: Agent-Generated Workflow

When a user tells ZeptoClaw "Create a workflow that sends Telegram alerts when inventory is low", the agent generates:

```yaml
# Generated by ZeptoClaw via Claude
name: inventory-alert
description: Send Telegram alert when product stock falls below threshold
version: 1

triggers:
  - type: cron
    schedule: "0 9 * * *"  # Daily at 9 AM

inputs:
  threshold:
    type: number
    default: 10
    description: Stock level that triggers alert

nodes:
  - id: fetch-inventory
    type: google-sheets
    config:
      spreadsheet_id: "{{ env.INVENTORY_SHEET_ID }}"
      range: "Products!A:D"
      action: read

  - id: filter-low-stock
    type: filter
    config:
      condition: "item.stock < inputs.threshold"
    depends_on: [fetch-inventory]

  - id: format-message
    type: transform
    config:
      expression: |
        items.map(p => `⚠️ Low stock: ${p.name} (${p.stock} left)`).join('\n')
    depends_on: [filter-low-stock]

  - id: send-alert
    type: telegram
    config:
      chat_id: "{{ env.TELEGRAM_ALERT_CHAT }}"
      message: |
        📦 Inventory Alert

        {{ nodes.format-message.output }}

        Check: {{ env.INVENTORY_SHEET_URL }}
    depends_on: [format-message]
    # Only send if there are low-stock items
    condition: "nodes.filter-low-stock.output.length > 0"
```

This YAML:
- Is **valid** (passes schema validation)
- Uses **consistent patterns** (LLMs learn from examples)
- Is **human-readable** (for debugging)
- Has **clear dependencies** (execution order is obvious)

---

*Last updated: 2026-02-13*
