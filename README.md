# r8r

**Agent-first workflow automation engine in Rust.**

r8r is a lightweight, fast workflow automation tool designed for AI agents. It provides deterministic, repeatable workflows that agents can invoke reliably.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Features

- **Agent-First Design** - Built for AI agents to invoke, not humans to click
- **LLM-Friendly YAML** - Consistent patterns, easy for agents to generate
- **Fast & Lightweight** - Single binary, minimal dependencies, sub-millisecond overhead
- **SQLite Storage** - Zero external dependencies, portable data
- **MCP Server** - Model Context Protocol support for AI tool integration
- **Distributed Tracing** - OpenTelemetry integration for observability

## Quick Start

### Installation

```bash
# From source
git clone https://github.com/qhkm/r8r.git
cd r8r
cargo build --release

# Binary will be at ./target/release/r8r
```

### Create Your First Workflow

```yaml
# workflows/hello.yaml
name: hello-world
description: A simple greeting workflow

triggers:
  - type: manual

nodes:
  - id: greet
    type: transform
    config:
      expression: |
        "Hello, " + (input.name ?? "World") + "!"
```

### Run It

```bash
# Start the server
r8r server --workflows ./workflows

# Execute via API
curl -X POST http://localhost:3000/api/workflows/hello-world/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"name": "Agent"}}'
```

## Documentation

| Document | Description |
|----------|-------------|
| [API Reference](docs/API.md) | REST API endpoints |
| [Node Types](docs/NODE_TYPES.md) | Available node types and configuration |
| [Environment Variables](docs/ENVIRONMENT_VARIABLES.md) | Configuration options |
| [Architecture](docs/ARCHITECTURE.md) | System design and internals |

## Workflow Structure

```yaml
name: workflow-name          # Required: unique identifier
description: What it does    # Optional: human-readable description
version: "1.0.0"             # Optional: semantic version

settings:                    # Optional: workflow-level settings
  timeout_seconds: 3600      # Max execution time
  max_concurrency: 10        # Concurrent node limit
  debug: false               # Enable debug logging

triggers:                    # What starts the workflow
  - type: cron
    schedule: "*/5 * * * *"  # Every 5 minutes
  - type: webhook
    path: /hook/custom       # Custom webhook path
  - type: manual             # API-triggered only

nodes:                       # The steps to execute
  - id: step-1
    type: http
    config:
      url: https://api.example.com/data
      method: GET

  - id: step-2
    type: transform
    config:
      expression: "input.data"
    depends_on: [step-1]     # Execute after step-1
    condition: "input.status == 200"  # Only if condition is true
    retry:
      max_attempts: 3
      backoff: exponential
    on_error:
      action: fallback
      fallback_value: []
```

## Node Types

| Type | Description |
|------|-------------|
| `http` | Make HTTP requests |
| `transform` | Transform data with Rhai expressions |
| `agent` | Invoke AI agents (LLM calls) |
| `subworkflow` | Call another workflow |
| `debug` | Log values for debugging |
| `variables` | Set/get workflow variables |
| `template` | Render templates |
| `circuit_breaker` | Fault tolerance wrapper |

See [Node Types](docs/NODE_TYPES.md) for detailed configuration.

## CLI Commands

```bash
# Server
r8r server --workflows ./workflows    # Start API server
r8r dev --workflows ./workflows       # Dev mode with hot reload

# Workflows
r8r workflows list                    # List all workflows
r8r workflows run <name>              # Execute a workflow
r8r workflows validate <file>         # Validate workflow YAML
r8r workflows history <name>          # Version history
r8r workflows rollback <name> <ver>   # Rollback to version

# Executions
r8r workflows trace <execution_id>    # View execution trace
r8r workflows resume <execution_id>   # Resume failed execution
r8r workflows replay <execution_id>   # Replay execution

# Credentials
r8r credentials set <name>            # Store credential
r8r credentials list                  # List credentials (masked)
r8r credentials delete <name>         # Remove credential

# Database
r8r db check                          # Health check
```

## MCP Server

r8r includes an MCP (Model Context Protocol) server for AI tool integration:

```bash
# Start MCP server
r8r-mcp
```

Available tools:
- `list_workflows` - List available workflows
- `execute_workflow` - Execute a workflow
- `get_execution` - Get execution status

## Configuration

Key environment variables:

```bash
# Server
R8R_CORS_ORIGINS=https://app.example.com
R8R_MAX_CONCURRENT_REQUESTS=200

# Security
R8R_MONITOR_TOKEN=<random-token>
R8R_HEALTH_API_KEY=<api-key>

# Observability
R8R_OTEL_ENABLED=true
R8R_OTEL_ENDPOINT=http://jaeger:4317

# Database
R8R_DATABASE_PATH=/var/lib/r8r/r8r.db
```

See [Environment Variables](docs/ENVIRONMENT_VARIABLES.md) for full reference.

## Security

r8r includes several security features:

- **SSRF Protection** - Blocks requests to internal IPs by default
- **Credential Encryption** - AES-256-GCM with PBKDF2 key derivation
- **Webhook Signatures** - GitHub, Stripe, Slack signature verification
- **Rate Limiting** - Per-workflow rate limiting
- **Input Validation** - JSON Schema validation for workflow inputs

See [Security Audit](SECURITY_AUDIT_REPORT.md) for details.

## Contributing

Contributions are welcome! Please read the contributing guidelines before submitting PRs.

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=r8r=debug cargo run -- server

# Check formatting
cargo fmt --check
cargo clippy
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Acknowledgments

r8r is inspired by workflow automation tools like n8n, Temporal, and Prefect, but designed specifically for the AI agent use case.
