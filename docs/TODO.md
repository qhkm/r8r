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

## In Progress

### Event Triggers
- [ ] Kafka consumer trigger
- [ ] SQS/SNS triggers

## Planned

### Node Types
- [ ] `discord` - Discord messaging
- [ ] `google-sheets` - Google Sheets integration
- [ ] `telegram` - Telegram Bot API

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

We welcome contributions! Pick an item from "Planned" and open an issue to discuss before starting work.

Priority areas:
1. New node types (especially `discord`, `telegram`)
2. Event trigger sources
3. Documentation improvements
4. Test coverage
