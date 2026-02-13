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

## In Progress

### Event Triggers
- [ ] Kafka consumer trigger
- [ ] SQS/SNS triggers

## Planned

### Node Types
- [ ] `email` - Send emails via SMTP/API
- [ ] `slack` - Slack messaging
- [ ] `discord` - Discord messaging
- [ ] `database` - SQL query execution
- [ ] `s3` - S3 object operations
- [ ] `google-sheets` - Google Sheets integration
- [ ] `telegram` - Telegram Bot API

### Workflow Features
- [ ] Workflow templates/blueprints
- [ ] Parameterized workflows
- [ ] Workflow dependencies (DAG of workflows)

### UI & Developer Experience
- [x] Web dashboard for monitoring
- [x] Visual workflow editor
- [x] CLI autocomplete

### Scalability
- [ ] Distributed execution (multiple workers)
- [ ] PostgreSQL storage backend
- [ ] Redis-based job queue

### Enterprise Features
- [ ] Multi-tenancy
- [ ] RBAC (Role-Based Access Control)
- [ ] Audit logging
- [ ] SSO integration

## Contributing

We welcome contributions! Pick an item from "Planned" and open an issue to discuss before starting work.

Priority areas:
1. New node types (especially `email`, `slack`, `database`)
2. Event trigger sources
3. Documentation improvements
4. Test coverage
