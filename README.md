<p align="center">
  <h1 align="center">r8r</h1>
  <p align="center">
    <em>Pronounced "rater" • The agent-native workflow automation engine</em>
  </p>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL%20v3-blue.svg" alt="License: AGPL v3"></a>
  <a href="https://github.com/qhkm/r8r"><img src="https://img.shields.io/badge/rust-1.70+-orange.svg" alt="Rust"></a>
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Status: Beta">
</p>

> ⚠️ **Beta Software**: r8r is under active development. APIs may change between versions. Not recommended for production workloads yet. We welcome feedback and contributions!

---

**r8r** (r-eight-r → rater) is an agent-native workflow automation engine written in Rust.

While tools like n8n and Zapier were built for humans clicking through visual editors, r8r was built for the AI age — where agents need to create, execute, and orchestrate workflows programmatically.

**Why not just let AI do everything?** Tools that route every step through an LLM burn tokens on tasks that don't need intelligence — HTTP calls, JSON parsing, conditional routing. r8r uses deterministic nodes for deterministic work and only calls the LLM when you actually need reasoning. The result: same automation, a fraction of the token cost.

```
Fully agentic:  LLM token on every step  →  $$$
r8r:            LLM only where needed    →  $
```

## ✨ What Makes r8r Different

| Traditional Tools | r8r |
|------------------|-----|
| 🖱️ Visual drag-and-drop | 📝 LLM-friendly YAML |
| 🐘 Heavy (500MB+ RAM) | 🦀 Lightweight (~15MB RAM) |
| 🐌 Slow startup | ⚡ ~50ms cold start |
| 🔒 Locked in database | 📂 Git-friendly files |
| 🧑 Built for humans | 🤖 Agent-native |

## 🚀 Quick Start

```bash
# Install
git clone https://github.com/qhkm/r8r.git && cd r8r
cargo build --release

# Create a workflow
cat > hello.yaml << 'EOF'
name: hello-world
nodes:
  - id: greet
    type: transform
    config:
      expression: '"Hello, " + (input.name ?? "World") + "!"'
EOF

# Run it
./target/release/r8r server --workflows . &
curl -X POST localhost:3000/api/workflows/hello-world/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"name": "Agent"}}'
```

## 🤖 Agent-Native

### Agent as a Node Type

The `agent` node lets you drop AI reasoning into any workflow — call OpenAI, Anthropic, Ollama, or any OpenAI-compatible endpoint directly. Use AI where you need it, skip it where you don't.

```yaml
name: order-processor

nodes:
  - id: fetch-order
    type: http
    config:
      url: "https://api.store.com/orders/{{ input.order_id }}"

  - id: check-fraud
    type: agent
    config:
      provider: openai                     # or anthropic, ollama, custom
      model: gpt-4o
      prompt: "Is this order fraudulent? {{ nodes.fetch-order.output }}"
      response_format: json
      json_schema:                         # validate AI output structure
        type: object
        required: [verdict, confidence]
        properties:
          verdict: { type: string, enum: [fraud, legit] }
          confidence: { type: number }
    depends_on: [fetch-order]
    retry:
      max_attempts: 3
      backoff: exponential

  - id: route
    type: switch
    depends_on: [check-fraud]
    config:
      expression: "nodes.check_fraud.output.verdict"
      cases:
        fraud: [flag-order]
        legit: [process-order]
```

The agent node gets the same durability as every other node — retries, checkpoints, fallback values. If the AI call fails at 3am, r8r retries with backoff, not the entire pipeline.

**Supported providers:**

| Provider | Config | Credential |
|----------|--------|------------|
| OpenAI | `provider: openai` | `credential: openai` |
| Anthropic | `provider: anthropic` | `credential: anthropic` |
| Ollama | `provider: ollama` | None (local) |
| Custom | `provider: custom` + `endpoint: ...` | `credential: ...` |

### MCP Server

r8r includes an **MCP server** (Model Context Protocol) so AI agents can directly invoke workflows:

```bash
# Start the MCP server
r8r-mcp
```

**Available tools (13):**

| Tool | Description |
|------|-------------|
| `r8r_execute` | Execute a workflow, returns execution metadata |
| `r8r_run_and_wait` | Execute a workflow, returns just the output |
| `r8r_discover` | Discover workflow parameters, nodes, triggers |
| `r8r_lint` | Lint workflow YAML with detailed error messages |
| `r8r_list_workflows` | List all available workflows |
| `r8r_get_workflow` | Get full workflow definition |
| `r8r_get_execution` | Get execution status and result |
| `r8r_get_trace` | Get detailed execution trace |
| `r8r_list_executions` | List recent executions |
| `r8r_validate` | Validate workflow YAML |
| `r8r_create_workflow` | Create or update a workflow |
| `r8r_test` | Test a workflow with input and assert expected output. Mocks via `pinned_data`. Fully isolated. |
| `r8r_list_approvals` | List pending approval requests that need action |
| `r8r_approve` | Approve or reject a pending approval, then resume the paused execution |

## 📊 r8r vs n8n

| Feature | r8r | n8n |
|---------|-----|-----|
| **Primary User** | AI agents & developers | Human operators |
| **Interface** | CLI, API, MCP | Visual drag-and-drop |
| **Language** | Rust | TypeScript |
| **Binary Size** | 24 MB | ~200 MB+ |
| **Memory (idle)** | ~15 MB | ~500 MB+ |
| **Startup** | ~50ms | Seconds |
| **Storage** | SQLite (embedded) | PostgreSQL/MySQL |
| **Workflows** | YAML files (git-friendly) | Database blobs |
| **AI Agent Nodes** | ✅ Multi-provider (OpenAI, Anthropic, Ollama) | ❌ None |
| **Code Sandbox** | ✅ Pluggable (subprocess, Docker, Firecracker) | ❌ None |
| **MCP Support** | ✅ Built-in | ❌ None |
| **Durable Execution** | Checkpoint, resume, replay | Basic retry |
| **Circuit Breakers** | ✅ Built-in | ❌ None |

### Use r8r when:
- 🤖 AI agents trigger your workflows
- ⚡ You need fast, lightweight automation
- 📂 You want workflows in version control
- 🛠️ You prefer code over clicking

### Use n8n when:
- 🖱️ You prefer visual workflow building
- 🔌 You need 400+ pre-built integrations
- 👥 Your team is non-technical

## 📖 Workflow Anatomy

```yaml
name: order-processor
description: Process incoming orders

triggers:
  - type: webhook
    path: /orders
  - type: cron
    schedule: "0 * * * *"  # Every hour

nodes:
  - id: validate
    type: transform
    config:
      expression: |
        if input.amount > 0 { input } else { throw "Invalid amount" }

  - id: notify
    type: http
    config:
      url: https://slack.com/api/chat.postMessage
      method: POST
      body: '{"text": "New order: ${{ input.amount }}"}'
    depends_on: [validate]
    retry:
      max_attempts: 3
      backoff: exponential
    on_error:
      action: continue  # Don't fail the whole workflow
```

## 🧩 Node Types

| Node | Purpose |
|------|---------|
| `http` | REST API calls |
| `transform` | Data transformation (Rhai expressions) |
| **`agent`** | **AI reasoning — call OpenAI, Anthropic, Ollama, or any LLM with structured output validation** |
| **`sandbox`** | **Execute Python, Node, or Bash in isolated environments (subprocess, Docker, or Firecracker)** |
| `subworkflow` | Nested workflow execution |
| `email` | Send emails (SMTP, SendGrid, Resend, Mailgun) |
| `slack` | Slack messaging |
| `database` | SQL query execution (SQLite, PostgreSQL, MySQL) |
| `s3` | S3 object storage operations |
| `switch` | Multi-branch conditional routing |
| `if` | Binary conditional |
| `filter` | Filter data by conditions |
| `sort` | Sort data by fields |
| `split` | Split data into chunks |
| `aggregate` | Aggregate/reduce data |
| `merge` | Merge multiple inputs |
| `dedupe` | Remove duplicates |
| `set` | Set values |
| `variables` | Workflow state management |
| `crypto` | Hash, encrypt, sign |
| `datetime` | Date/time operations |
| `wait` | Delay/sleep node |
| `approval` | Pause execution for human/agent approval |
| `circuit_breaker` | Fault tolerance |
| `debug` | Development logging |

See [Node Types](docs/NODE_TYPES.md) for full documentation.

## 🔧 CLI Reference

```bash
# Server
r8r server --workflows ./workflows    # Start server
r8r dev --workflows ./workflows       # Hot-reload mode

# Workflows
r8r workflows list                    # List workflows
r8r workflows run <name>              # Execute
r8r workflows run <name> -p key=val   # Execute with parameters
r8r workflows validate <file>         # Lint YAML
r8r workflows history <name>          # Version history
r8r workflows rollback <name> <ver>   # Rollback
r8r workflows dag <name>              # Show dependency graph
r8r workflows dag <name> --order      # Show execution order

# Executions
r8r workflows trace <id>              # Execution trace
r8r workflows resume <id>             # Resume failed
r8r workflows replay <id>             # Replay execution

# Templates
r8r templates list                    # List available templates
r8r templates show <name>             # Show template details
r8r templates use <name> -o out.yaml  # Create workflow from template

# Generate from natural language
r8r create <description>              # Generate workflow from prompt
r8r refine <name> <description>       # Refine existing workflow

# Security
r8r credentials set <name>            # Store secret
r8r credentials list                  # List (masked)
```

### Generate Workflows from Natural Language

```bash
# Create a new workflow from a description
r8r create fetch HN top stories every morning and send digest to Slack

# Refine an existing workflow
r8r refine hn-digest add error handling that notifies via email on failure
```

The generator uses your configured LLM provider (set `R8R_AGENT_PROVIDER` and `R8R_AGENT_API_KEY`) to produce valid workflow YAML, validates it, and lets you review before saving.

## 🔒 Security

- **SSRF Protection** — Blocks internal IP requests
- **AES-256-GCM** — Encrypted credential storage
- **Webhook Signatures** — GitHub, Stripe, Slack verification
- **Rate Limiting** — Per-workflow throttling
- **Schema Validation** — JSON Schema input validation
- **Sandbox Isolation** — Pluggable backends: Docker containers or Firecracker microVMs with memory, network, and filesystem restrictions

See [Security Audit](SECURITY_AUDIT_REPORT.md) for details.

## 📚 Documentation

| Doc | Description |
|-----|-------------|
| [API Reference](docs/API.md) | REST endpoints |
| [Node Types](docs/NODE_TYPES.md) | Node configuration |
| [Environment Variables](docs/ENVIRONMENT_VARIABLES.md) | Configuration |
| [Architecture](docs/ARCHITECTURE.md) | System design |
| [Roadmap](docs/TODO.md) | Planned features |
| [Runner Plan](docs/RUNNER_PLAN.md) | Edge/fleet runner roadmap |

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines and our [CLA](CLA.md) (required for first PR).

```bash
cargo test              # Run tests (400+)
cargo clippy            # Lint
cargo fmt               # Format
```

See [Roadmap](docs/TODO.md) for contribution ideas.

## 📄 License

AGPL-3.0 — Free to use, modify, and distribute. If you run a modified version as a network service, you must make the source available.

**Commercial licensing available** for organizations that cannot use AGPL. [Contact us](mailto:hello@r8r.dev) for details.

---

<p align="center">
  <em>r8r — Because AI agents deserve better than drag-and-drop.</em>
</p>
