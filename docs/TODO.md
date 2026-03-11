# r8r Roadmap

## Completed

### Core Engine
- [x] Workflow execution with dependency resolution
- [x] Node types: http, transform, agent, subworkflow, debug, variables, template, circuit_breaker, wait, switch, filter, sort, limit, set, aggregate, split, crypto, datetime, dedupe, summarize, if
- [x] Conditional execution (`condition` field)
- [x] Retry with backoff strategies (fixed, linear, exponential)
- [x] Retry backoff cap (`max_delay_seconds`) and jitter (±25%) to prevent unbounded delays
- [x] Error handling with fallback values
- [x] Fallback node routing — `on_error.fallback_node` executes alternate node on failure
- [x] For-each iteration with chunked processing
- [x] Workflow timeout enforcement
- [x] Max concurrency limits
- [x] Parallel sibling execution — independent nodes at same DAG depth run concurrently via `join_all`
- [x] `store_traces` setting to gate node-level execution record writes

### Storage & Versioning
- [x] SQLite storage with WAL mode
- [x] Workflow version history
- [x] Execution trace storage
- [x] Workflow rollback support
- [x] Execution replay from version
- [x] Resume from failed node
- [x] Dead Letter Queue for failed executions
- [x] DLQ auto-push — failed executions automatically create DLQ entries

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
- [x] Unified audit log — queryable `audit_events` table with REST API (`GET /api/audit`)

### API & Integration
- [x] REST API server
- [x] MCP server for AI tool integration
- [x] WebSocket monitoring
- [x] OpenAPI specification
- [x] Workflow import/export CLI
- [x] REST Approval API — CRUD endpoints (`GET/POST /api/approvals`) for human-in-the-loop without MCP

### Workflow Features
- [x] Workflow templates/blueprints (5 built-in, custom templates supported)
- [x] Parameterized workflows (with type validation & defaults)
- [x] Workflow dependencies (DAG of workflows with cycle detection)
- [x] Prompt-to-workflow generation via CLI (`r8r generate`) and MCP (`r8r_generate`)

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
- [x] Non-builder dashboard UX (overview, runs, approvals, audit)
- [x] Guided workflow studio (template + plain-language form + publish)
- [x] CLI autocomplete
- [x] Interactive REPL with TUI — conversation engine, tool integration, LLM streaming

### Deployment & Operations
- [x] VPS deploy wrapper (`scripts/deploy-vps.sh`) that bootstraps Docker/Compose and starts r8r API + embedded UI
- [x] Post-deploy API/UI health checks in cloud deploy script (`scripts/deploy-cloud.sh`)
- [x] Terraform modules for multi-tenant ECS deployment (per-client services, ALB, autoscaling)
- [x] GitHub Actions CI/CD for cloud deploy and Terraform infra

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

### ~~1. Synchronous Workflow Execution~~ ✅ Complete
Make r8r seamless for AI agents by returning results inline instead of requiring polling.
- [x] Webhook response mode — `response_mode: wait_for_result` on webhook triggers returns output directly
- [x] `r8r_run_and_wait` MCP tool — agents call one tool, get structured results back
- [x] Configurable timeout — `timeout_seconds` on sync execution
- [x] Response format options — final output only via `r8r_run_and_wait`; full trace via standard execution API

### ~~2. Workflow Testing & Mocking Framework~~ ✅ Complete
No competitor has this — huge DX win for workflow development.
- [x] `r8r test <fixture.yaml>` CLI — run workflows against mock fixtures
- [x] Mock definitions in YAML — `mocks:` map of node_id → pinned output (injected as `pinned_data`)
- [x] Assertion syntax — `expected:` + `mode: exact|contains`; `expected_error:` for failure cases
- [x] Snapshot mode — `--update-snapshots` prints actual output for copying into fixture
- [x] CI-friendly output — exit code 1 on failure, `--junit-xml <path>` for JUnit XML report
- [x] `--filter / -k` to run matching test names only

### ~~3. Enhanced MCP Tools~~ ✅ Complete
Make r8r the best workflow engine for AI agents to operate.
- [x] `r8r_run_and_wait` tool — synchronous execution with structured output
- [x] `r8r_discover` tool — parameter discovery, schema introspection
- [x] `r8r_lint` tool — validate YAML before submission
- [x] `r8r_generate` tool — scaffold workflows from natural language descriptions
- [x] Structured error responses — `{ error_code, message, details }` on all tools; codes: `workflow_not_found`, `yaml_parse_error`, `execution_failed`, `approval_required`, `invalid_input`, `resource_not_found`, `internal_error`

### ~~4. Workflow Linting & Generation Helpers~~ ✅ Complete
Make it trivial for LLMs to produce valid workflow YAML.
- [x] `r8r lint <file...>` CLI — validate YAML syntax, node references, dependency cycles
- [x] Common error suggestions — "did you mean `depends_on` instead of `dependsOn`?"
- [x] `--errors-only` flag to suppress warnings; `--json` for machine output
- [x] `r8r generate` / `r8r create` / `r8r prompt` CLI — generate workflow from natural language
- [x] JSON Schema for workflow YAML — `assets/r8r-workflow.schema.json`, emitted via `r8r schema [--output file]`

### ~~5. Human-in-the-Loop Node~~ ✅ Complete
- [x] `approval` node type — pause execution, wait for human input
- [x] REST Approval API — list, decide, with `decided_by` identity tracking
- [x] MCP `r8r_approve` tool with optional `decided_by` parameter
- [x] Approval UI in dashboard — review pending approvals, approve/reject with comments
- [x] Timeout with default action — auto-approve/reject after configurable wait
- [x] Notification channels — `notify:` list in approval node config; supports `webhook`, `slack`, `email` (SMTP relay via `R8R_SMTP_RELAY_URL`)
- [x] Delegation — `assign_to` and `assign_groups` on approval node; filterable via REST API and MCP tool

### ~~6. CLI Guardrails & UX (MVP, Vision-Aligned)~~ ✅ Sprint 1 & 2 Complete
Adopt a safer agent-native CLI flow without drifting into Temporal/Inngest-style durability.

**Sprint 1 ✅ Complete**
- [x] Add top-level command aliases: `prompt`, `run`, `watch`, `runs`, `secrets`, `doctor`
- [x] `r8r prompt` wrapper over create/refine with `--patch`, `--emit`, `--dry-run`, `--yes`, `--json`
- [x] `r8r run` review screen on TTY with side-effect node summary before execution
- [x] Fail-closed non-interactive behavior when `waiting_for_approval` — exits with code `42`
- [x] Add tests + docs for new prompt/run flags and non-interactive exit semantics

**Sprint 2 ✅ Complete**
- [x] Add `r8r runs pending`, `r8r runs show <run_id>`, `r8r runs export <run_id> --format jsonl`
- [x] Add `r8r approve <run_id>` and `r8r deny <run_id>` CLI flow on top of approval requests
- [x] Add `r8r policy` profiles: `lenient`, `standard`, `strict`
- [x] Implement minimum policy knobs: `require_review_on_patch`, `require_idempotency_key_for_run`, `max_runtime`, `deny_unknown_node_types`
- [x] Add policy validation command + docs

**Explicitly deferred (out of Sprint 1/2)**
- [ ] Full tool capability/scope permission matrix
- [ ] Cost estimation and `--max-cost` enforcement
- [ ] Tool version pinning/drift enforcement
- [ ] Vault/keychain provider integration framework
- [ ] Channel-specific/2FA approval constraints (e.g., WhatsApp + OTP)
- [ ] Full execution diff engine

**REPL direction (objective decision: hybrid approach)**
- [ ] Keep current REPL/TUI architecture and avoid full rewrite
- [ ] Optimize default UX for builders (fast iteration, low friction)
- [ ] Add plan-first safety flow incrementally: Draft -> Plan -> Run
- [ ] Ensure REPL never performs write side effects without explicit approval/gate
- [ ] Prioritize guardrails over new complexity when trade-offs conflict

**Builder-first UX policy (default mode)**
- [ ] Keep chat-first interaction as the default (no mandatory mode switching)
- [ ] Preserve minimal core command set for builders (`/run`, `/show`, `/dag`, `/logs`, `/trace`, `/export`, `/help`)
- [ ] Keep low-risk runs fast; only escalate UX friction when write/approval risk is detected
- [ ] Prefer inline safety hints and side-effect summaries over blocking dialogs for routine flows
- [ ] Stream responses and show YAML/DAG immediately; run validation continuously in the background
- [ ] Keep policy/permissions/cost controls out of default builder flow (surface in operator mode)

**REPL guardrails (next after Sprint 1/2 core CLI)**
- [x] Add `/plan` in REPL to create a plan artifact before execution
- [x] Add side-effect summary panel before any write-capable run (write-gate confirmation)
- [x] Add `/arm` with session TTL (default disarmed — safe mode)
- [x] Add type-to-confirm (yes/no) approval for write-intent inputs when disarmed
- [x] Persist transcript-to-run correlation (`session_id`, `run_id`, tool calls redacted)
- [ ] Add fail-closed non-interactive behavior parity for approval-required runs (exit `42`)

**Operator mode (optional/escalated)**
- [x] Add `/operator [on|off]` toggle command — shows arm state in status bar
- [x] Show write arm state prominently when operator mode is on
- [ ] Require stricter confirms only in operator mode or high-risk paths — **deferred**: arm/disarm mechanic is sufficient; revisit when destructive bulk ops exist

## Backlog

### Event Triggers
- [ ] Kafka consumer trigger
- [ ] SQS/SNS triggers

### Node Types
- [ ] `discord` - Discord messaging
- [ ] `google-sheets` - Google Sheets integration
- [ ] `telegram` - Telegram Bot API

### Sandbox v2
- [x] File artifacts — `collect_artifacts: true` + `R8R_OUTPUT_DIR` env; collected per-execution in subprocess and Docker backends
- [x] Streaming output — `execute_streaming()` on `SandboxBackend` trait; subprocess backend streams stdout/stderr line-by-line via mpsc channel
- [x] Package caching — `packages: [...]` in sandbox node config; pip/npm install prefix for subprocess and Docker; `image:` override for custom Docker images
- [ ] Warm pools — pre-started containers/VMs for sub-100ms cold starts (backlogged)
- [ ] Firecracker guest agent reference implementation
- [ ] WASM backend (`sandbox-wasm`) — for trusted, high-frequency tool execution

### Scalability
- [ ] Distributed execution (multiple workers)
- [x] PostgreSQL storage backend (`--features storage-postgres`, `Storage` trait, `Arc<dyn Storage>` everywhere)
- [ ] Redis-based job queue

### Deployment & Operations (Self-Hosted)
- [x] AWS ECS/Fargate reference architecture for per-client deployments (one service per client)
- [x] Terraform modules for ECS service, networking, secrets, logging, and autoscaling defaults
- [ ] Blue/green deployment and rollback runbook for workflow API uptime
- [ ] Per-client cost controls (resource limits, autoscaling caps, tagging, budget alarms)

### Enterprise Features
- [ ] Multi-tenancy
- [ ] RBAC (Role-Based Access Control)
- [x] Audit logging — unified audit_events table with REST API, emitted from executor/approval/timeout paths
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

Priority areas (next):
1. New node types — discord, telegram, google-sheets
2. New triggers — Kafka, SQS/SNS
3. Scalability — distributed workers, Redis job queue
4. Enterprise — multi-tenancy, RBAC, SSO
