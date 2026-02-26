# r8r Roadmap

## Completed

### Core Engine
- [x] Workflow execution with dependency resolution
- [x] Node types: http, transform, agent, subworkflow, debug, variables, template, circuit_breaker, wait, switch, filter, sort, limit, set, aggregate, split, crypto, datetime, dedupe, summarize, if
- [x] Conditional execution (`condition` field)
- [x] Retry with backoff strategies (fixed, linear, exponential)
- [x] Error handling with fallback values
- [x] For-each iteration with chunked processing
- [x] Workflow timeout enforcement
- [x] Max concurrency limits

### Storage & Versioning
- [x] SQLite storage with WAL mode
- [x] Workflow version history
- [x] Execution trace storage
- [x] Workflow rollback support
- [x] Execution replay from version
- [x] Resume from failed node
- [x] Dead Letter Queue for failed executions

### Triggers
- [x] Cron scheduling
- [x] Webhook endpoints with signature verification
- [x] Manual/API triggers
- [x] Redis pub/sub event source

### Security
- [x] Credential encryption (AES-256-GCM)
- [x] SSRF protection for HTTP node
- [x] Webhook signature verification (GitHub, Stripe, Slack)
- [x] Rate limiting per workflow
- [x] Input validation with JSON Schema
- [x] Health endpoint authentication

### Observability
- [x] Prometheus metrics
- [x] OpenTelemetry tracing
- [x] Structured access logging
- [x] Request ID propagation

### API & Integration
- [x] REST API server
- [x] MCP server for AI tool integration
- [x] WebSocket monitoring
- [x] OpenAPI specification
- [x] Workflow import/export CLI

### Workflow Features
- [x] Workflow templates/blueprints (5 built-in, custom templates supported)
- [x] Parameterized workflows (with type validation & defaults)
- [x] Workflow dependencies (DAG of workflows with cycle detection)

### Sandbox (Code Execution)
- [x] Pluggable `SandboxBackend` trait with feature-flagged backends
- [x] Subprocess backend (`--features sandbox`) — zero deps, works everywhere
- [x] Docker backend (`--features sandbox-docker`) — container isolation via bollard SDK
- [x] Firecracker backend (`--features sandbox-firecracker`) — hardware VM isolation via KVM
- [x] SandboxNode workflow node (`type: sandbox`) with Python, Node, Bash support
- [x] JSON output parsing (stdout → structured data)
- [x] Global security overrides (network kill switch, memory/timeout caps, runtime whitelist)
- [x] Sandbox backends reference doc with future roadmap (WASM, Hyperlight, libkrun, Landlock)

### UI & Developer Experience
- [x] Web dashboard for monitoring
- [x] Visual workflow editor
- [x] CLI autocomplete

### Reliability (Lightweight Durability)
- [x] Checkpoint-based durable execution - Persist workflow state after each node
- [x] Workflow pause/resume API - Manual control over execution (`POST /api/executions/{id}/pause`, `POST /api/executions/{id}/resume`)
- [x] Graceful shutdown with checkpoint - Save state on SIGTERM/SIGINT
- [x] Long-running workflow support - Extended timeout limits with checkpoint recovery

### Event Processing Enhancements
- [x] Event fan-out - One trigger → multiple workflows (parallel or sequential)
- [x] Debouncing for webhooks - Deduplicate rapid events with configurable wait times
- [x] Event filtering/routing - JSONPath filtering, header-based routing, conditional routing
- [x] Delayed event processing - Schedule events for future processing via Redis or SQLite

## Planned (Priority Order)

### 1. Synchronous Workflow Execution
Make r8r seamless for AI agents by returning results inline instead of requiring polling.
- [ ] Webhook response mode — `POST /api/webhooks/:id?wait=true` returns workflow output directly
- [ ] `run_and_wait` MCP tool — agents call one tool, get structured results back
- [ ] Configurable timeout with streaming progress updates
- [ ] Response format options (full trace vs. final output only)

### 2. Workflow Testing & Mocking Framework
No competitor has this — huge DX win for workflow development.
- [ ] `r8r test` CLI command — run workflows against mock fixtures
- [ ] Mock definitions in YAML — stub HTTP responses, agent outputs, external services
- [ ] Assertion syntax — validate node outputs, execution order, error handling
- [ ] Snapshot testing — detect unexpected output changes
- [ ] CI-friendly output (JUnit XML, exit codes)

### 3. Enhanced MCP Tools
Make r8r the best workflow engine for AI agents to operate.
- [ ] `r8r_run_and_wait` tool — synchronous execution with structured output
- [ ] `r8r_discover` tool — parameter discovery, schema introspection
- [ ] Structured error responses — actionable error messages agents can reason about
- [ ] `r8r_lint` tool — validate YAML before submission
- [ ] `r8r_generate` tool — scaffold workflows from natural language descriptions

### 4. Workflow Linting & Generation Helpers
Make it trivial for LLMs to produce valid workflow YAML.
- [ ] `r8r lint` CLI command — validate YAML syntax, node references, dependency cycles
- [ ] JSON Schema for workflow YAML — enables IDE autocomplete
- [ ] `r8r generate` CLI — scaffold workflow from template + parameters
- [ ] Common error suggestions — "did you mean `depends_on` instead of `dependsOn`?"

### 5. Human-in-the-Loop Node
Unlocks enterprise use cases requiring human approval in automated workflows.
- [ ] `approval` node type — pause execution, wait for human input
- [ ] Notification channels — webhook callback, email, Slack
- [ ] Approval UI in dashboard — review pending approvals, approve/reject with comments
- [ ] Timeout with default action — auto-approve/reject after configurable wait
- [ ] Delegation — route approvals to specific users/groups

## Backlog

### Event Triggers
- [ ] Kafka consumer trigger
- [ ] SQS/SNS triggers

### Node Types
- [ ] `discord` - Discord messaging
- [ ] `google-sheets` - Google Sheets integration
- [ ] `telegram` - Telegram Bot API

### Sandbox v2
- [ ] File artifacts — output directory collection from sandbox executions
- [ ] Warm pools — pre-started containers/VMs for sub-100ms cold starts
- [ ] Streaming output — WebSocket streaming of stdout during execution
- [ ] Package caching — pre-installed pip/npm packages in Docker images
- [ ] Firecracker guest agent reference implementation
- [ ] WASM backend (`sandbox-wasm`) — for trusted, high-frequency tool execution

### Scalability
- [ ] Distributed execution (multiple workers)
- [ ] PostgreSQL storage backend
- [ ] Redis-based job queue

### Enterprise Features
- [ ] Multi-tenancy
- [ ] RBAC (Role-Based Access Control)
- [ ] Audit logging
- [ ] SSO integration

## NOT Planned (Intentional Design Decisions)

The following features are intentionally **not** on the roadmap to maintain r8r's focus on lightweight, self-hosted, AI-native workflow automation:

### From Temporal.io (Out of Scope)
- [ ] Deterministic replay - Full event sourcing is too complex for r8r's use cases
- [ ] Multi-language SDKs - YAML-first philosophy; use external SDKs if needed
- [ ] Multi-year workflow durability - Use Temporal for financial transactions requiring years-long durability
- [ ] Exactly-once semantics - At-least-once with idempotency is sufficient

### From Inngest (Philosophy Conflict)
- [ ] Serverless/edge deployment - r8r is self-hosted first
- [ ] Stateless execution - Would require complete architecture rewrite
- [ ] Durable steps (serverless-style) - Requires serverless infrastructure

**Rationale:** We believe these features would add significant complexity without serving r8r's core mission: lightweight, AI-native workflow automation for self-hosted environments.

## Contributing

We welcome contributions! Pick an item from "Planned" or "Backlog" and open an issue to discuss before starting work.

Priority areas:
1. Synchronous execution & MCP tool improvements (items 1 & 3)
2. Workflow testing framework (item 2)
3. Documentation improvements
4. Test coverage
