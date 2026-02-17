# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- Test count increased from 335+ to 370+

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

[Unreleased]: https://github.com/qhkm/r8r/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/qhkm/r8r/releases/tag/v0.1.0
