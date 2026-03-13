<p align="center">
  <h1 align="center">r8r</h1>
  <p align="center">
    <em>Pronounced "rater" — The agent-native workflow automation engine</em>
  </p>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL--3.0-blue.svg" alt="License: AGPL-3.0"></a>
  <a href="https://github.com/qhkm/r8r"><img src="https://img.shields.io/badge/rust-1.70+-orange.svg" alt="Rust"></a>
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Status: Beta">
  <img src="https://img.shields.io/badge/binary-~15MB-brightgreen.svg" alt="Binary: ~15MB">
</p>

> **Beta Software**: r8r is under active development. APIs may change between versions. Not recommended for production workloads yet. We welcome feedback and contributions!

---

**r8r** is an agent-native workflow automation engine written in Rust. While tools like n8n and Zapier were built for humans clicking through visual editors, r8r was built for the AI age — where agents need to create, execute, and orchestrate workflows programmatically. 15MB binary. 50ms cold start. Cloud to edge.

## Why r8r

**Don't burn LLM tokens on deterministic steps.** Tools that route every step through an LLM spend money on tasks that don't need intelligence — HTTP calls, JSON parsing, conditional routing. r8r uses deterministic nodes for deterministic work and only calls the LLM when you actually need reasoning.

```
Fully agentic:  LLM token on every step  →  $$$
r8r:            LLM only where needed    →  $
```

**Runs on anything.** A single static binary with ~15MB RAM footprint and ~50ms cold start. No JVM, no Node.js runtime, no container orchestrator required. Deploy on a cloud VM, a Raspberry Pi, an NVIDIA Jetson, or an industrial gateway — the same binary works everywhere.

**Local AI reasoning.** Call OpenAI, Anthropic, Ollama, or any OpenAI-compatible LLM from within your workflows. Run Ollama on the same edge device for fully offline intelligent automation — no cloud round-trips, no token bills.

## What Makes r8r Different

| Traditional Tools | r8r |
|------------------|-----|
| Visual drag-and-drop | LLM-friendly YAML |
| Heavy (500MB+ RAM) | Lightweight (~15MB RAM) |
| Cloud-only | Cloud to edge |
| Slow startup | ~50ms cold start |
| Locked in database | Git-friendly files |
| Built for humans clicking | Built for AI agents and developers |

## Quick Start

```bash
# Install (recommended)
curl -sSf https://r8r.sh/install.sh | sh

# Pin a specific release if needed
curl -sSf https://r8r.sh/install.sh | R8R_VERSION=v0.3.1 sh

# Or use Docker
docker run -d -p 8080:8080 -v r8r-data:/data ghcr.io/qhkm/r8r

# Or via Cargo (requires Rust)
cargo install r8r

# Or build from source
git clone https://github.com/qhkm/r8r.git && cd r8r
cargo build --release
```

The install script resolves the latest GitHub release automatically and falls back to `R8R_VERSION` when you want to pin a version.

Then create and run a workflow:

```bash
# Create a workflow
cat > hello.yaml << 'EOF'
name: hello-world
nodes:
  - id: greet
    type: transform
    config:
      expression: '"Hello, " + (input.name ?? "World") + "!"'
EOF

# Start the server
r8r server --workflows . &

# Execute it
curl -X POST localhost:8080/api/workflows/hello-world/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"name": "Agent"}}'

# Open the web UI
# Dashboard: http://localhost:8080/
# Guided workflow studio: http://localhost:8080/editor
```

## Deployment Modes

### Cloud / Server Mode

Run `r8r server` for the full-featured automation server — REST API, web dashboard, MCP server, webhooks, cron triggers, and credential management.

```bash
r8r server --workflows ./workflows
```

This is the primary mode today. Supports SQLite (default) or PostgreSQL (`--features storage-postgres`) for execution history and state.

### Edge / Runner Mode (Coming Soon)

`r8r-runner` — a lightweight daemon designed for edge devices. Pulls workflow definitions from a central server or local directory, executes them offline, and syncs results when connectivity is available.

- **Offline-safe**: SQLite-backed execution with store-and-forward
- **Fleet management**: Central dashboard to deploy workflows to hundreds of edge nodes
- **Minimal footprint**: Same ~15MB binary, same ~15MB RAM

See [Runner Plan](docs/RUNNER_PLAN.md) for the roadmap.

## AI-Powered Workflows

The `agent` node drops AI reasoning into any workflow — call OpenAI, Anthropic, Ollama, or any OpenAI-compatible endpoint directly. Use AI where you need it, skip it where you don't.

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
      json_schema:
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

| Provider | Config | Credential | Edge-friendly |
|----------|--------|------------|---------------|
| OpenAI | `provider: openai` | `credential: openai` | Cloud |
| Anthropic | `provider: anthropic` | `credential: anthropic` | Cloud |
| Ollama | `provider: ollama` | None (local) | Local / Edge |
| Custom | `provider: custom` + `endpoint: ...` | `credential: ...` | Any |

For edge deployments, pair r8r with a local Ollama instance for fully offline AI reasoning — no internet connection required.

## MCP Server

r8r includes a built-in **MCP server** (Model Context Protocol) so AI agents can directly create, execute, and manage workflows.

```bash
r8r-mcp
```

**15 tools available:**

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
| `r8r_test` | Test a workflow with mocks and assertions |
| `r8r_list_approvals` | List pending approval requests |
| `r8r_approve` | Approve or reject a pending approval |
| `r8r_generate` | Generate a workflow from natural language |

## r8r vs Alternatives

| Feature | r8r | n8n | Node-RED | AWS Greengrass |
|---------|-----|-----|----------|----------------|
| **Primary user** | AI agents & developers | Human operators | IoT developers | AWS ecosystem |
| **Language** | Rust | TypeScript | Node.js | Java + Python |
| **Binary size** | ~15 MB | ~200 MB+ | ~100 MB+ | ~200 MB+ |
| **Memory (idle)** | ~15 MB | ~500 MB+ | ~80 MB+ | ~200 MB+ |
| **Startup** | ~50ms | Seconds | Seconds | Seconds |
| **Interface** | CLI, API, MCP, Web UI | Visual editor | Visual editor | AWS Console |
| **Workflows as code** | YAML (git-friendly) | Database blobs | JSON (git-friendly) | JSON + AWS config |
| **AI agent nodes** | Multi-provider | None | Community nodes | SageMaker integration |
| **MCP support** | Built-in (15 tools) | None | None | None |
| **Edge deployment** | Single binary | Not designed for edge | Yes (lightweight) | Yes (AWS-only) |
| **Vendor lock-in** | None | None | None | AWS |
| **Sandbox execution** | Docker, Firecracker | None | None | Lambda-based |

## Node Types

| Node | Purpose |
|------|---------|
| `http` | REST API calls |
| `transform` | Data transformation (Rhai expressions) |
| **`agent`** | **AI reasoning — call OpenAI, Anthropic, Ollama, or any LLM** |
| **`sandbox`** | **Execute Python, Node, or Bash in isolated environments** |
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

## Edge & IoT

r8r's Rust foundation makes it uniquely suited for edge computing:

- **Small footprint**: ~15MB binary, ~15MB RAM — fits on any device with an ARM or x86 processor
- **SQLite-backed**: No external database needed, durable execution state on local disk
- **Offline-safe**: Workflows execute without internet; results sync when connectivity returns
- **Fast recovery**: ~50ms cold start means near-instant restart after power cycles

**Target platforms:**

| Platform | Status |
|----------|--------|
| Raspberry Pi 4/5 | Supported (ARM64) |
| NVIDIA Jetson (Nano, Orin) | Supported (ARM64) |
| Intel NUC / x86 gateways | Supported (x86_64) |
| Industrial ARM gateways | Supported (ARM64) |

**Planned edge capabilities:**

- MQTT node — subscribe/publish to MQTT brokers (coming soon)
- GPIO support — read sensors and control actuators (planned)
- Fleet management — deploy and monitor workflows across device fleets (planned)

## Use Cases

**Cloud: Auto-triage GitHub issues with AI**
Webhook receives a new issue, the `agent` node classifies severity and assigns labels, then `http` node posts a Slack notification. Deterministic routing handles the plumbing; LLM handles the judgment call.

**Edge: Smart factory sensor monitoring**
r8r runs on a gateway device, reads sensor data via HTTP polling, uses a local Ollama model for anomaly detection, and triggers alerts when thresholds are exceeded — all without leaving the factory floor.

**Hybrid: Edge collection, cloud aggregation**
Edge devices run lightweight r8r workflows that pre-process and filter data locally. Results sync to a cloud r8r instance that aggregates across sites and generates reports.

**Robotics: Autonomous decision-making**
r8r on an NVIDIA Jetson runs workflows that combine sensor input with local LLM reasoning for real-time navigation decisions — deterministic control loops with AI judgment where it matters.

## CLI Reference

```bash
# Server
r8r server --workflows ./workflows    # Start server
r8r dev --workflows ./workflows       # Hot-reload mode

# Interactive REPL
r8r                                   # Open interactive REPL
r8r chat                              # Same as above
r8r chat --resume <session-id>        # Resume previous session

# Workflows
r8r workflows list                    # List workflows
r8r workflows run <name>              # Execute
r8r workflows run <name> -p key=val   # Execute with parameters
r8r workflows validate <file>         # Lint YAML
r8r workflows history <name>          # Version history
r8r workflows rollback <name> <ver>   # Rollback
r8r workflows dag <name>              # Show dependency graph

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

While a prompt is running, the REPL TUI shows staged progress: planning, active tool calls, latest completed tool, and reply drafting, alongside elapsed time.

## Security

- **SSRF Protection** — Blocks internal IP requests
- **AES-256-GCM** — Encrypted credential storage
- **Webhook Signatures** — GitHub, Stripe, Slack verification
- **Rate Limiting** — Per-workflow throttling
- **Schema Validation** — JSON Schema input validation
- **Sandbox Isolation** — Docker containers or Firecracker microVMs with memory, network, and filesystem restrictions

See [Security Audit](SECURITY_AUDIT_REPORT.md) for details.

## Documentation

| Doc | Description |
|-----|-------------|
| [API Reference](docs/API.md) | REST endpoints |
| [Node Types](docs/NODE_TYPES.md) | Node configuration |
| [Environment Variables](docs/ENVIRONMENT_VARIABLES.md) | Configuration |
| [Deployment Guide](docs/DEPLOYMENT.md) | Server deployment (API + UI) |
| [Edge Deployment](docs/EDGE_DEPLOYMENT.md) | Raspberry Pi, Jetson, and IoT deployment |
| [Use Cases](docs/USE_CASES.md) | 10 concrete workflows (cloud, edge, hybrid, MCP) |
| [Competitive Positioning](docs/COMPETITIVE_POSITIONING.md) | r8r vs n8n, Node-RED, Temporal, Greengrass |
| [Client Deployment Model](docs/CLIENT_DEPLOYMENT_MODEL.md) | Per-client isolation approach |
| [Architecture](docs/ARCHITECTURE.md) | System design |
| [Sandbox Backends](docs/SANDBOX_BACKENDS.md) | Docker and Firecracker sandboxing |
| [Runner Plan](docs/RUNNER_PLAN.md) | Edge/fleet runner roadmap |
| [Roadmap](docs/TODO.md) | Planned features |

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines and our [CLA](CLA.md) (required for first PR).

```bash
cargo test              # Run tests (400+)
cargo clippy            # Lint
cargo fmt               # Format
```

See [Roadmap](docs/TODO.md) for contribution ideas.

## License

This repository uses multiple licenses:

| Component | License |
|-----------|---------|
| Core engine (`src/`) | [AGPL-3.0](LICENSE-AGPL) |
| Client libraries | [Apache 2.0](LICENSE-APACHE) |
| Enterprise features | Commercial (requires license key) |

The source code compiled without the `enterprise` feature flag is open source under AGPLv3. See [LICENSE](LICENSE) for full terms.

**Commercial license** available for teams that need to use r8r without AGPL obligations or need enterprise features (RBAC, SSO, fleet management, HA). [Contact us](mailto:hello@r8r.sh).

---

<p align="center">
  <em>r8r — Because AI agents deserve better than drag-and-drop. Cloud to edge.</em>
</p>
