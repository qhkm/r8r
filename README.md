<p align="center">
  <h1 align="center">r8r</h1>
  <p align="center">
    <em>Pronounced "rater" • The workflow engine that AI agents actually want to use</em>
  </p>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://github.com/qhkm/r8r"><img src="https://img.shields.io/badge/rust-1.70+-orange.svg" alt="Rust"></a>
</p>

---

**r8r** (r-eight-r → rater) is an agent-first workflow automation engine written in Rust.

While tools like n8n and Zapier were built for humans clicking through visual editors, r8r was built for the AI age—where agents need to create, execute, and orchestrate workflows programmatically.

```
Human clicks "Add Node" → n8n
Agent writes YAML → r8r
```

## ✨ What Makes r8r Different

| Traditional Tools | r8r |
|------------------|-----|
| 🖱️ Visual drag-and-drop | 📝 LLM-friendly YAML |
| 🐘 Heavy (500MB+ RAM) | 🦀 Lightweight (~30MB) |
| 🐌 Slow startup | ⚡ <100ms cold start |
| 🔒 Locked in database | 📂 Git-friendly files |
| 🧑 Built for humans | 🤖 Built for agents |

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

## 🤖 Built for AI Agents

r8r includes an **MCP server** (Model Context Protocol) so AI agents can directly invoke workflows:

```bash
# Start the MCP server
r8r-mcp
```

Your AI agent can now:
- `list_workflows` - Discover available workflows
- `execute_workflow` - Run workflows with parameters
- `get_execution` - Check execution status

## 📊 r8r vs n8n

| Feature | r8r | n8n |
|---------|-----|-----|
| **Primary User** | AI agents & developers | Human operators |
| **Interface** | CLI, API, MCP | Visual drag-and-drop |
| **Language** | Rust | TypeScript |
| **Memory** | ~30 MB | ~500 MB+ |
| **Startup** | <100ms | Seconds |
| **Storage** | SQLite (portable) | PostgreSQL/MySQL |
| **Workflows** | YAML files (git-friendly) | Database blobs |
| **MCP Support** | ✅ Built-in | ❌ None |
| **Price** | Free forever | Free (limited) |

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
| `agent` | LLM/AI agent invocation |
| `subworkflow` | Nested workflow execution |
| `variables` | Workflow state management |
| `template` | Text templating |
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
r8r workflows validate <file>         # Lint YAML
r8r workflows history <name>          # Version history
r8r workflows rollback <name> <ver>   # Rollback

# Executions
r8r workflows trace <id>              # Execution trace
r8r workflows resume <id>             # Resume failed
r8r workflows replay <id>             # Replay execution

# Security
r8r credentials set <name>            # Store secret
r8r credentials list                  # List (masked)
```

## 🔒 Security

- **SSRF Protection** — Blocks internal IP requests
- **AES-256-GCM** — Encrypted credential storage
- **Webhook Signatures** — GitHub, Stripe, Slack verification
- **Rate Limiting** — Per-workflow throttling
- **Schema Validation** — JSON Schema input validation

See [Security Audit](SECURITY_AUDIT_REPORT.md) for details.

## 📚 Documentation

| Doc | Description |
|-----|-------------|
| [API Reference](docs/API.md) | REST endpoints |
| [Node Types](docs/NODE_TYPES.md) | Node configuration |
| [Environment Variables](docs/ENVIRONMENT_VARIABLES.md) | Configuration |
| [Architecture](docs/ARCHITECTURE.md) | System design |
| [Roadmap](docs/TODO.md) | Planned features |

## 🤝 Contributing

```bash
cargo test              # Run tests (300+)
cargo clippy            # Lint
cargo fmt               # Format
```

See [Roadmap](docs/TODO.md) for contribution ideas.

## 📄 License

MIT — Use it however you want.

---

<p align="center">
  <em>r8r — Because AI agents deserve better than drag-and-drop.</em>
</p>
