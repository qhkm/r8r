# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-03-01

### Added

#### Enterprise Feature Gaps
- **Unified audit log** — queryable `audit_events` table with REST API (`GET /api/audit`), filterable by event type, execution, workflow, actor, and time range with pagination
- **REST Approval API** — full CRUD endpoints (`GET/POST /api/approvals/{id}/decide`) for human-in-the-loop without MCP, with `decided_by` identity tracking
- **DLQ auto-push** — failed executions automatically create dead letter queue entries for retry
- **Retry backoff cap** — `max_delay_seconds` field prevents unbounded retry delays; `jitter` adds ±25% randomization to avoid thundering herd
- **Fallback node routing** — `on_error.fallback_node` executes an alternate node on failure, with cascading fallback to `fallback_value`
- **Parallel sibling execution** — independent nodes at the same DAG depth run concurrently via `futures::join_all`, respecting `max_concurrency` setting
- **`store_traces` gating** — when `store_traces: false`, node-level execution records are skipped for performance
- **`decided_by` on `r8r_approve`** — optional parameter to identify the decision-maker (defaults to "agent")

#### Interactive REPL
- **`r8r repl` command** — interactive TUI with ratatui for workflow management
- **Conversation engine** — multi-turn conversation with history and context
- **Tool integration** — workflow CRUD, execution, and LLM generation from the REPL
- **LLM streaming** — `StreamEvent` enum and `call_llm_streaming()` for SSE/NDJSON streaming across OpenAI, Anthropic, and Ollama providers
- **`generate_with_config()`** — allows REPL runtime LLM config overrides

#### Infrastructure
- **Terraform modules** — multi-tenant ECS deployment with per-client services, ALB, DNS, autoscaling
- **GitHub Actions** — cloud deploy and Terraform infra CI/CD workflows

### Changed
- Test count increased from 483 to 583
- Audit events emitted from executor, approval, and timeout paths
- Validator now accepts `fallback_node` as valid alternative to `fallback_value` for fallback error action

## [0.2.0] - 2026-02-27

### Added

#### Synchronous Execution
- **`response_mode: wait_for_result`** on webhook triggers — returns workflow output directly in the HTTP response
- **`r8r_run_and_wait` MCP tool** — executes a workflow and returns just the output (no execution metadata)
- Sync execution support across CLI, API, and MCP interfaces

#### Workflow Testing & Mocking
- **`r8r_test` MCP tool** — test workflows with input assertions and `pinned_data` mocking, fully isolated in-memory
- **`json_diff` engine** — exact match and subset comparison for test assertions with readable diff output

#### Human-in-the-Loop Approval
- **`approval` node type** — pauses workflow execution until a human or agent approves/rejects
- **`r8r_list_approvals` MCP tool** — query pending approval requests by status
- **`r8r_approve` MCP tool** — resolve an approval and resume the paused execution
- **Background timeout checker** — auto-resolves expired approvals using configured `default_action` (polls every 30s)
- Full storage layer with `approval_requests` table, indexes, and expiry queries

#### Prompt-to-Workflow Generation
- **`r8r create <prompt>` CLI command** — generate workflows from natural language descriptions via LLM
- **`r8r refine <workflow-name> <prompt>` CLI command** — refine existing workflows with natural language feedback
- **`r8r_generate` MCP tool** — prompt-to-workflow generation for AI agents
- **Shared LLM client** (`src/llm.rs`) — supports OpenAI, Anthropic, Ollama, and custom endpoints
- Context-aware prompt assembly with node catalog, credentials, and example workflows
- Auto-fix retry: validation failures are fed back to LLM for self-correction

#### Reliability & Durability
- **Checkpoint-based durable execution** - Workflows automatically persist state after each node, enabling recovery from crashes and graceful shutdowns
- **Workflow pause/resume API** - New endpoints `POST /api/executions/{id}/pause` and `POST /api/executions/{id}/resume` for manual execution control
- **Graceful shutdown** - SIGTERM/SIGINT handling saves checkpoints before exit
- **Long-running workflow support** - Extended timeout limits with checkpoint recovery for workflows running hours or days
- **Configurable checkpointing** - Per-workflow `enable_checkpoints` setting and global `R8R_ENABLE_CHECKPOINTS` environment variable

#### Event Processing Enhancements
- **Event fan-out** - One event trigger can now execute multiple workflows in parallel or sequential mode
- **Webhook debouncing** - Deduplicate rapid webhook requests with configurable wait times and max wait limits
- **Event filtering & routing** - JSONPath filtering, header-based routing, and conditional routing with Rhai expressions
- **Delayed event processing** - Schedule events for future processing (seconds to days) via Redis or SQLite backend

#### Infrastructure
- GitHub Actions CI pipeline (test, clippy, fmt, release build)
- API key authentication middleware (`R8R_API_KEY` environment variable)
- CHANGELOG.md following Keep a Changelog format

### Fixed
- Landing page: corrected binary size (15MB -> 24MB), integrations count, license reference
- Cargo.toml: fixed repository URL, added homepage/authors/keywords/categories
- Security audit report: updated test count to 370+

### Changed
- Landing page license footer updated from MIT to AGPL-3.0
- Default retry delay changed from 60s to 1s for faster iteration
- Test count increased from 335+ to 483
- MCP tool count increased from 11 to 15

## [0.1.0] - 2026-02-13

### Added
- 24 built-in node types (HTTP, Script, Transform, Delay, Conditional, Parallel, Cache, and more)
- MCP (Model Context Protocol) server for AI agent integration
- YAML-based workflow definitions with LLM-friendly syntax
- Cron and webhook triggers with Redis pub/sub event triggers
- SQLite-backed workflow storage with versioning
- Real-time WebSocket monitoring dashboard
- OpenAPI 3.1 specification auto-generation
- Shell completions for Bash, Zsh, Fish, PowerShell, and Elvish
- Prometheus metrics and OpenTelemetry tracing
- Health check authentication middleware
- Request ID propagation and structured access logging
- CORS configuration with environment variables
- Concurrency and request body size limits
- Comprehensive security audit with 335+ tests
- AGPL-3.0 license with dual licensing option

[0.3.0]: https://github.com/qhkm/r8r/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/qhkm/r8r/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qhkm/r8r/releases/tag/v0.1.0
