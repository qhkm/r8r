# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.2] - 2026-10-26

### Changed

- **Published to crates.io as `r8r`** — `cargo install r8r` installs the `r8r` and `r8r-mcp` commands.
- **Package name is `r8r` again** — the crates.io name `r8r` is now ours, so the `r8r-engine` rename from 0.8.1 is reverted.

### Fixed

- **crates.io package builds** — the three embedded example workflows and the workflow JSON schema are now included in the published package.

## [0.8.1] - 2026-10-26

### Added

- **Global `[llm]` defaults for agent nodes** — provider, endpoint and model fall back to `~/.config/r8r/config.toml` when a workflow omits them.
- **`r8r chat --reset`** — clears the persisted REPL LLM config.
- **Create-time node config validation** — catches hallucinated node configs when workflows are generated.

### Changed

- **Published as `r8r-engine`** — the crates.io name `r8r` belongs to an unrelated project. The `r8r` and `r8r-mcp` commands and the `r8r` library name are unchanged.
- **Rust edition 2024.**

### Security

- **SSRF hardening** — bracketed IPv6 literals, IPv4-mapped/compatible/NAT64 addresses and more reserved ranges are blocked; DNS results are filtered; every redirect is re-checked. The `document` node's URL download now goes through the same checks.
- **Dependency upgrades for RustSec advisories** — `rustls` 0.23.46, `quinn-proto` 0.11.19, `sqlx` 0.8.6 (drops `rustls` 0.21), `quick-xml` 0.41, `wasmtime` 49 (`sandbox-wasm`), with `rusqlite` 0.32 / `r2d2_sqlite` 0.25 to match.

### Fixed

- **Approval races** — the approval decision and its resume checkpoint are written in one transaction, so concurrent approvals (API, bridge, MCP, CLI, timeout) can no longer overwrite each other.
- **Resume runs at most once** — the paused execution is claimed atomically before resuming; the original is marked `cancelled` ("Superseded by resume from checkpoint").
- **Parallel failures keep sibling results** — successful siblings are recorded and checkpointed before the failure is reported.
- **Legacy SQLite databases open again** — trace indexes are created after the migration adds their columns.

## [0.8.0] - 2026-08-19

### Added

#### WASM Sandbox Backend

- **In-process WASI execution** (`--features sandbox-wasm`) — wasmtime 47 runtime; base64 module in `SandboxRequest.code`; epoch-based interruption → `Timeout` error; 64 MiB memory cap; output artifacts via `$R8R_OUTPUT_DIR`; select via `sandbox.backend = "wasm"` (default subprocess).

#### IMAP Trigger

- **Email polling trigger** (`--features imap`) — pure-Rust TLS stack (async-imap + tokio-rustls); watches mailboxes (default UNSEEN), mark-seen/delete options, attachment capture via `include_attachments`.

#### Document Node

- **`document` node** (default feature) — `pdf_text` extraction (pure Rust) and `vision` OCR (BYOK credentials); input via base64 `data`, `data_field`, or `url`.

#### WhatsApp Inbound Webhook

- **Meta Cloud API webhook** (`--features whatsapp-inbound`) — `/webhooks/whatsapp` endpoint with `hub.verify_token` handshake + `X-Hub-Signature-256` verification; publishes `whatsapp.inbound` / `whatsapp.status` events.

#### HTTP / Integrations

- **Multipart request bodies** — `body_type: multipart` on HTTP nodes (data-URI → binary file parts); **OpenAI `transcribe_audio`** (whisper-1) operation.
- **Operation-level headers** — headers now resolve `{{ params.* }}` per operation (e.g. `Xero-tenant-id`).
- **New built-in integrations** — Runware (image/video generation), Threads ops added to meta-graph (me, create post, publish, media status), Xero + QuickBooks (bank feeds, invoices).
- **7 example workflows** — voice-note transcription, review→ticket, compliance kit (regulation impact, document check, cross-check, gap analysis, evidence mapping).

#### Platform / DX

- **Visual workflow builder v2** — open/edit existing workflows (YAML/JSON ⇄ canvas), full 30-node palette grouped, trigger editor, JSON node config + delete, Save = API upsert.
- **Built-in YAML loader fails loudly** — malformed built-in YAML panics at startup with file path + error instead of silently dropping the service.

### Fixed

- **Credential store isolation in tests** — test suite no longer overwrites the real `credentials.json` (path override + `R8R_DATA_DIR` support).
- Clippy `useless_vec` in document test helper; loader ops count test updated for OpenAI `transcribe_audio`.

#### PostgreSQL Storage Backend

- **`Storage` trait** — backend-agnostic async trait (`src/storage/trait.rs`) with ~50 methods covering workflows, executions, checkpoints, approvals, REPL sessions, delayed events, audit log, and DLQ; blanket `impl Storage for Arc<S>` so `Arc<dyn Storage>` itself implements `Storage`
- **`PostgresStorage`** — full `sqlx`-based PostgreSQL implementation (`src/storage/postgres.rs`) using `PgPool` connection pooling; schema auto-created on first connect; enabled via `--features storage-postgres`
- **Dynamic backend selection** — `get_storage()` in `main.rs` checks `DATABASE_URL` env var at runtime: if it starts with `postgres://` or `postgresql://`, uses `PostgresStorage`; otherwise falls back to `SqliteStorage`
- **`Arc<dyn Storage>` everywhere** — all callers updated (`AppState`, `Executor`, `NodeContext`, `R8rService`, triggers, REPL) to use `Arc<dyn Storage>` instead of concrete `SqliteStorage`; enables zero-copy sharing across async tasks

#### Sandbox v2

- **File artifacts** — `collect_artifacts: true` on sandbox nodes; code writes files to `$R8R_OUTPUT_DIR`; all files are collected after execution as `artifacts` in the node result (base64-encoded + decoded `text` for UTF-8 files)
- **Streaming output** — `execute_streaming()` added to `SandboxBackend` trait; subprocess backend streams stdout/stderr line-by-line via `mpsc::Sender<SandboxStreamEvent>`; default trait impl buffers for non-streaming backends
- **Package caching** — `packages: [requests, numpy]` on sandbox nodes; pip/npm installed before code execution (subprocess: inline install prefix; Docker: shell-wrapped cmd); `image: custom/image:tag` for Docker to use pre-built images with packages
- **Custom Docker images** — `image:` field in sandbox node config overrides the backend default image per-execution
- **Bind-mount code execution** — Docker backend now writes code to a host temp file and bind-mounts it as `/r8r_sandbox/code.<ext>`, avoiding shell-quoting issues and CLI arg size limits
- **`SandboxArtifact`** — new struct with `path`, `content: Vec<u8>`, `size`; `collect_output_artifacts()` shared helper walks output dir recursively
- **`SandboxStreamEvent`** — enum with `Stdout`, `Stderr`, `Done`, `Error` variants for streaming consumers

#### Approval Delegation

- **`assign_to` field** on approval nodes — route a pending approval to a specific user identity (e.g. email or username)
- **`assign_groups` field** on approval nodes — route to one or more groups; any member of a listed group can act
- **`GET /api/approvals?assignee=<id>`** — REST filter by assignee
- **`GET /api/approvals?group=<name>`** — REST filter by group membership
- **`r8r_list_approvals` MCP tool** — now accepts `assignee` and `group` params for delegation filtering
- **`r8r runs pending`** — now shows assignee/group routing info per approval
- **SQLite migration** — `assignee` and `groups` columns added automatically via `ALTER TABLE` on startup

#### Job Queue & Distributed Workers

- **`JobQueue` trait** (`src/queue/mod.rs`) — backend-agnostic async trait: `enqueue`, `dequeue`, `ack`, `requeue`, `pending_len`, `requeue_stale`; `RunJob` payload with `job_id`, `workflow_id`, pinned `version`, `input`, `attempts`, `correlation_id`, `idempotency_key`, `origin`
- **`MemoryQueue`** — in-process FIFO with blocking dequeue + notify wakeups; default backend, zero extra dependencies
- **`RedisQueue`** (`--features redis`) — reliable producer/consumer via `BRPOPLPUSH` (pending/processing lists, stale-job recovery on worker startup, bounded retry)
- **`Worker`** (`src/queue/worker.rs`) — dequeue → execute → ack/requeue loop; graceful shutdown (`ctrl_c`); `MAX_ATTEMPTS = 3` then drop; reuses `Executor` + `Storage` so node semantics match direct runs
- **CLI** — `r8r enqueue <workflow>` (input JSON, version pin, idempotency key) and `r8r worker [-c N] [--once]` (single-shot mode for cron/systemd); backend resolution `--redis-url` > `R8R_REDIS_URL` > `REDIS_URL` > memory fallback

#### JSON Schema & REPL Guardrails

- **`r8r schema`** — emits the JSON Schema for workflow YAML to stdout or `--output <file>`. Enables IDE autocomplete in VSCode/Cursor with the YAML Language Server.
- **`assets/r8r-workflow.schema.json`** — full JSON Schema Draft 2020-12 covering all triggers, node types, retry, error handling, settings, and workflow-level fields.
- **REPL `/plan`** — slash command that loads a pre-execution plan into the Log panel, showing what write operations will be triggered and the current gate state.
- **REPL `/arm` / `/disarm`** — session-scoped write gate. Default: disarmed (safe). When disarmed and a message has write intent (create/run/approve/delete), the REPL shows a side-effect summary and requires typing `yes` to confirm before executing.
- **REPL `/operator [on|off]`** — operator mode toggle. Shows write-gate arm state in the status bar. Default: off (builder mode).
- **SSRF protection shared module** — extracted `validate_url` and `is_private_or_special_ip` into `src/ssrf.rs`, shared by `nodes/http.rs` and `notifications.rs`.
- **Notification security** — webhook/Slack/SMTP URLs now SSRF-validated; email bodies HTML-escaped; all notification clients use a 10-second timeout.

#### CLI Guardrails & UX (Sprint 1 & 2)
- **`r8r prompt`** — wrapper over workflow create/refine with `--patch`, `--emit`, `--dry-run`, `--yes`, `--json` flags
- **`r8r run`** — TTY review screen showing side-effect nodes (http, agent, approval) before execution with optional skip via `--yes`; exits with code `42` in non-interactive mode when workflow is waiting for approval
- **`r8r runs`** — list recent executions with status, duration, and timing
- **`r8r runs pending`** — filter executions awaiting approval (status: `waiting_for_approval`)
- **`r8r runs show <run_id>`** — full execution detail with `--trace` flag for per-node results
- **`r8r runs export <run_id>`** — export execution data as JSONL or JSON with `--output` path
- **`r8r approve <id>`** — approve a pending run by approval_id or execution_id with optional `--comment` and `--by` identity
- **`r8r deny <id>`** — deny a pending run by approval_id or execution_id with optional `--comment` and `--by` identity
- **`r8r policy show`** — display current active policy profile and settings
- **`r8r policy set <profile>`** — activate `lenient`, `standard`, or `strict` policy profile
- **`r8r policy validate <file>`** — validate a workflow YAML file against the active policy, reporting violations
- **Policy profiles** — three built-in profiles with knobs for `require_review_on_patch`, `require_idempotency_key_for_run`, `max_runtime_seconds`, `deny_unknown_node_types`; stored at `~/.config/r8r/policy.toml`
- **`r8r watch`** — alias for monitoring running executions
- **`r8r secrets`** — subcommand for credential management (`list`, `set`, `delete`)
- **`r8r doctor`** — health check: verifies DB connectivity, LLM configuration, credential store, and registered node types

#### Workflow Testing (`r8r test`)
- **`r8r test <fixture.yaml>`** — run workflows against mock fixtures in isolated in-memory storage
- **Mock injection** — `mocks:` map of node_id → pinned output injected as `pinned_data` before execution
- **Assertion modes** — `expected:` + `mode: exact|contains`; `expected_error:` for failure cases
- **Snapshot mode** — `--update-snapshots` prints actual output for easy fixture authoring
- **CI-friendly output** — exits with code `1` on failure; `--junit-xml <path>` emits JUnit XML
- **Test filtering** — `--filter / -k` to run matching test names only

#### Workflow Linting (`r8r lint`)
- **`r8r lint <file...>`** — validate YAML syntax, node references, and dependency cycles
- **Smart suggestions** — e.g., "did you mean `depends_on` instead of `dependsOn`?"
- **`--errors-only`** flag to suppress warnings; `--json` for machine-readable output

#### Structured MCP Error Codes
- All MCP tools now return `{ error_code, message, details }` on failure
- Error codes: `workflow_not_found`, `yaml_parse_error`, `validation_error`, `execution_failed`, `execution_timeout`, `approval_required`, `resource_not_found`, `invalid_input`, `internal_error`

#### Approval Notification Channels
- **`notify:` field** on approval nodes — list of channels to notify when approval is requested
- **Webhook** — HTTP POST with approval payload JSON
- **Slack** — Block Kit message with workflow name, approval ID, description, and expiry
- **Email** — HTML email via SMTP relay configured with `R8R_SMTP_RELAY_URL` env var

## [0.7.0] - 2026-08-15

### Changed

- **CI/release pipeline** — GitHub Actions bumped to v5 + Node 24 runtime; `aws-actions/configure-aws-credentials` → v6 (clears Node 20 deprecations).
- **Intel macOS builds** — `x86_64-apple-darwin` now built on `macos-14` (arm64) runners via native cross-compilation; macos-13 runners were sunset by GitHub and could leave jobs queued indefinitely.

## [0.6.0] - 2026-08-15

### Fixed

- **Circuit-breaker deadlock on macOS** — `CircuitBreakerRegistry::get_or_create` no longer holds a write lock while acquiring a read lock on the same `RwLock` (glibc/Linux tolerated the pattern; `pthread_rwlock` on macOS deadlocked every run).
- **Flaky `test_http_get`** — switched the external test endpoint from httpbin.org (frequent 503/rate-limit) to a stable JSON endpoint.
- **Intel macOS (x86_64) release binaries** — release matrix now builds `x86_64-apple-darwin` (runner `macos-13`); installer detects `uname -m` on Darwin instead of assuming ARM. v0.5.0 assets backfilled.

### Added

- **Real bridge health metrics** — ZeptoClaw `health.ping` now returns live runtime values (uptime, active executions, pending approvals, loaded workflows) instead of stub zeros.

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

[0.8.0]: https://github.com/qhkm/r8r/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/qhkm/r8r/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/qhkm/r8r/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/qhkm/r8r/compare/v0.4.1...v0.5.0
[0.3.0]: https://github.com/qhkm/r8r/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/qhkm/r8r/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qhkm/r8r/releases/tag/v0.1.0
