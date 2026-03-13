# Competitive Positioning

## Overview

r8r occupies a unique position in the workflow automation landscape — lightweight enough for edge devices (15 MB binary, ~15 MB RAM idle, ~50 ms cold start), powerful enough for cloud workflows (durable execution, approval gates, circuit breakers), and AI-native from day one (agent node with multi-provider LLM support, MCP server with 15 tools). Where most platforms force a choice between simplicity and power, or between cloud and edge, r8r bridges all three categories: agent frameworks, workflow engines, and edge platforms.

---

## Detailed Comparisons

### r8r vs n8n

| Feature | r8r | n8n |
|---|---|---|
| Primary user | Developers, DevOps, AI engineers | Non-technical users, ops teams, marketers |
| Interface | CLI, TUI/REPL, web dashboard | Visual drag-and-drop editor |
| Language | Rust | TypeScript / Node.js |
| Binary size | 15 MB single binary | ~400 MB+ (Node.js runtime + dependencies) |
| Memory (idle) | ~15 MB | ~300-500 MB |
| Cold start | ~50 ms | Several seconds |
| Storage | SQLite (default), PostgreSQL (optional) | SQLite, PostgreSQL, MySQL |
| Workflow format | YAML files (git-friendly) | JSON stored in database |
| AI / agent support | Native agent node (OpenAI, Anthropic, Ollama, custom) | AI nodes via community packages |
| MCP support | Built-in MCP server with 15 tools | None |
| Code sandbox | Subprocess, Docker, Firecracker | Limited (Code node in JS/Python) |
| Edge deployment | Yes — single binary, minimal resources | Impractical — too heavy for constrained devices |
| Integrations | ~25 node types | 400+ pre-built integrations |
| Self-hosted | Yes (single binary) | Yes (Docker recommended) |
| Pricing model | Open source (Apache 2.0) | Open source (Sustainable Use License) + paid cloud |

**Where n8n wins:** Visual editor, 400+ integrations, larger community, lower barrier for non-technical users.
**Where r8r wins:** Performance (20x less memory), edge deployment, native AI/MCP support, git-friendly YAML workflows, sub-second cold start.

---

### r8r vs Node-RED

| Feature | r8r | Node-RED |
|---|---|---|
| Language | Rust | JavaScript (Node.js) |
| Memory footprint | ~15 MB idle | ~80-150 MB idle |
| Binary / install | 15 MB single binary | Node.js runtime + npm install |
| AI / LLM support | Native agent node, multi-provider | Community nodes (limited, fragmented) |
| Workflow format | YAML (text, diffable) | JSON (visual flow export) |
| Debugging | TUI/REPL, structured logs, web dashboard | Visual debug in browser editor |
| Error handling | Circuit breakers, retry policies, dead-letter | Catch nodes, status nodes |
| Durable execution | Yes — persistent state, resume after crash | No — in-memory by default |
| Fleet management | Planned | None built-in (needs external tooling) |
| Community nodes | ~25 node types | 4,000+ community-contributed nodes |
| Edge deployment | Yes | Yes (runs on Raspberry Pi) |
| Triggers | Cron, webhook, manual, Redis pub/sub | HTTP, MQTT, TCP, serial, GPIO, WebSocket |

**Where Node-RED wins:** Massive ecosystem (4,000+ nodes), visual flow editor, mature IoT protocol support (MQTT, serial, GPIO), runs on Raspberry Pi today.
**Where r8r wins:** Lower memory footprint, durable execution, native AI integration, circuit breakers, YAML workflows in version control.

---

### r8r vs Temporal

| Feature | r8r | Temporal |
|---|---|---|
| Complexity | Low — YAML workflows, single binary | High — requires cluster (server + database + workers) |
| Resource requirements | 15 MB binary, ~15 MB RAM | Multiple services, 1+ GB RAM minimum |
| Learning curve | Write YAML, run binary | Learn SDK (Go/Java/Python/TypeScript), understand activities/workflows |
| Workflow definition | YAML (declarative) | Code-first SDK (imperative) |
| AI-native | Yes — agent node, MCP server | No — build it yourself with activities |
| Pricing | Open source (Apache 2.0) | Open source + Temporal Cloud ($200+/mo) |
| Edge deployment | Yes | No — too resource-heavy |
| Durable execution | Yes | Yes — industry-leading |
| Long-running workflows | Minutes to days | Minutes to years |
| Enterprise adoption | Early stage | Widely adopted (Stripe, Netflix, Snap) |
| Observability | Web dashboard, structured logs | Temporal UI, metrics, tracing |

**Where Temporal wins:** Battle-tested enterprise durability, years-long workflow support, massive enterprise adoption, mature multi-language SDKs.
**Where r8r wins:** Simplicity (YAML vs SDK), resource efficiency (100x less), edge deployment, native AI support, zero-infrastructure setup.

---

### r8r vs AWS IoT Greengrass

| Feature | r8r | AWS IoT Greengrass |
|---|---|---|
| Vendor lock-in | None — open source, runs anywhere | Deep AWS ecosystem dependency |
| Resource footprint | 15 MB binary, ~15 MB RAM | 128 MB+ RAM, requires Linux with glibc |
| AI / LLM support | Native agent node, multi-provider | SageMaker Neo integration (AWS models only) |
| Workflow capabilities | 25+ node types, durable execution | Lambda functions, limited orchestration |
| Offline operation | Full operation with SQLite | Partial — syncs when connectivity returns |
| Cost model | Free (self-hosted) | Per-device pricing + AWS service costs |
| Open source | Yes (Apache 2.0) | Partially (Greengrass Core is open source) |
| Fleet management | Planned | Yes — mature, at-scale device management |
| Cloud integration | Any cloud or none | AWS services (IoT Core, S3, Lambda, etc.) |
| Setup complexity | Download binary, run | AWS account, IAM roles, certificates, provisioning |

**Where Greengrass wins:** Managed fleet management at massive scale, deep AWS service integration, mature device provisioning and OTA updates.
**Where r8r wins:** No vendor lock-in, simpler setup, lower resource footprint, native AI/LLM support, free and open source.

---

### r8r vs Windmill

| Feature | r8r | Windmill |
|---|---|---|
| Language | Rust | Rust backend, Svelte frontend |
| Edge deployment | Yes — 15 MB binary | No — designed for server/cloud |
| AI support | Native agent node, MCP server | AI code generation for scripts |
| Workflow format | YAML (git-native) | Scripts + flows in database (git sync available) |
| Performance | ~50 ms cold start, ~15 MB RAM | Sub-second, ~200-500 MB RAM |
| Interface | CLI, TUI, web dashboard | Polished web IDE with code editor |
| Script execution | Subprocess, Docker, Firecracker | Dedicated workers per language |
| Integrations | ~25 node types | 100+ integrations + any language script |
| Self-hosted | Single binary | Docker Compose (multiple services) |

**Where Windmill wins:** Polished web IDE, multi-language script execution, more integrations, script marketplace.
**Where r8r wins:** Edge deployment, native AI/MCP support, lighter resource footprint, single-binary simplicity.

---

### r8r vs LangGraph / CrewAI

| Feature | r8r | LangGraph | CrewAI |
|---|---|---|---|
| What it is | Workflow runtime / engine | Agent orchestration framework | Multi-agent framework |
| Primary purpose | Run and schedule workflows | Design agent state machines | Design multi-agent teams |
| Deployment | Single binary, anywhere | Python app (you manage hosting) | Python app (you manage hosting) |
| Persistence | Built-in (SQLite / PostgreSQL) | Optional checkpointing | In-memory by default |
| Observability | Web dashboard, TUI, structured logs | LangSmith integration | CrewAI+ dashboard |
| Edge support | Yes | No | No |
| Triggers / scheduling | Cron, webhook, manual, Redis pub/sub | None built-in | None built-in |
| Workflow types | Deterministic + AI-augmented | Agent graphs | Agent pipelines |
| Language | Rust (YAML workflows) | Python | Python |

**Key insight:** These are complementary, not competitive. LangGraph and CrewAI design agent logic; r8r is the runtime that deploys, schedules, and monitors those agents in production. Use them together: build your agent in LangGraph or CrewAI, deploy it as a node in an r8r workflow with triggers, persistence, and observability.

---

## When to Use What

**Use r8r when:**
- You need lightweight, self-hosted workflow automation
- AI agents trigger and manage your workflows (via MCP)
- You are deploying on edge devices or constrained environments
- You want workflows in version control (YAML + git)
- You need deterministic workflows with selective AI reasoning
- You prefer code/CLI over visual editors

**Use n8n when:**
- Your team prefers visual drag-and-drop workflow building
- You need 400+ pre-built SaaS integrations immediately
- Your users are non-technical (marketing, ops, support)

**Use Temporal when:**
- You need enterprise-grade durable execution for financial transactions
- You have years-long running workflows
- Your team has strong DevOps and is comfortable with SDKs
- Budget is not a concern

**Use Node-RED when:**
- You need a visual editor for IoT device programming
- You want the largest community node ecosystem
- JavaScript is your team's primary language

**Use AWS Greengrass when:**
- You are already deep in the AWS ecosystem
- You need managed fleet management at massive scale
- Budget allows for cloud vendor costs

**Use LangGraph/CrewAI when:**
- You are building the agent logic itself
- You need multi-agent conversation and reasoning
- Consider using r8r as the deployment runtime for your agents

---

## Architecture Position

```
Agent Frameworks          Workflow Engines           Edge Platforms
(design agents)           (run workflows)            (manage devices)

LangGraph ─────┐    ┌──── n8n                 ┌──── AWS Greengrass
CrewAI ────────┤    │     Temporal             │     Azure IoT Edge
AutoGen ───────┘    │     Windmill             │     Balena
                    │                          │
                    └──── r8r ─────────────────┘
                          ↑
                    Bridges all three:
                    * Runs agent workflows (like frameworks)
                    * Orchestrates deterministic steps (like engines)
                    * Deploys to edge devices (like edge platforms)
```

r8r is not trying to replace any single category. It sits at the intersection — a workflow engine that is small enough for edge, capable enough for cloud, and AI-native enough to serve as the runtime layer for agent frameworks.

---

## Honest Limitations

r8r is early-stage software. Here is what it does not do well (yet):

- **Fewer integrations** — ~25 node types vs n8n's 400+ or Node-RED's 4,000+ community nodes. You will need to write custom nodes for many SaaS integrations.
- **No visual workflow editor** — The web dashboard monitors workflows but does not offer drag-and-drop editing like n8n or Node-RED.
- **Not suited for years-long workflows** — For multi-year durable execution with millions of events, Temporal is the right choice.
- **No managed cloud offering** — Self-hosted only. You provision and maintain the infrastructure.
- **Edge features are planned, not shipped** — MQTT, GPIO, and fleet management are on the roadmap but not available today.
- **Smaller community** — Fewer contributors, fewer answered questions, fewer tutorials compared to established tools.
- **Beta software** — APIs may change between releases. Production use requires careful version pinning.
