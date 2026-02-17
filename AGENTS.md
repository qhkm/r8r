# AGENTS.md

Operational guide for coding agents working in this repository.

## Mission

Build and maintain `r8r` as a lightweight, agent-first workflow engine:

- YAML + API first
- Self-hosted first
- Fast startup and low memory
- Deterministic workflow execution (without Temporal-level platform complexity)

## Product Scope (Hard Boundary)

This repository is for `r8r`, not the durable-orchestration product line.

- In scope: workflow engine improvements, nodes, triggers, API, observability, safety, DX.
- Out of scope here: full event-sourced deterministic replay platform, distributed durable workers, multi-year workflow durability, exactly-once platform semantics.
- If work targets Temporal/Inngest-class durability, route it to the separate Chronicle repo.

Reference: `docs/TODO.md` ("NOT Planned" section).

## Source of Truth

- Product and usage: `README.md`
- Architecture and runtime model: `docs/ARCHITECTURE.md`
- Public API contracts: `docs/API.md`
- Node behavior and configuration: `docs/NODE_TYPES.md`
- Environment/config knobs: `docs/ENVIRONMENT_VARIABLES.md`
- Strategy notes (internal): `.internal/`

## Repo Map

- Core execution: `src/engine/`
- Workflow schema/parser/validation: `src/workflow/`
- Built-in nodes: `src/nodes/`
- Triggers (cron/webhook/events): `src/triggers/`
- API server: `src/api/`
- Storage layer: `src/storage/`
- MCP server: `src/mcp/`
- CLI entrypoints: `src/main.rs`, `src/bin/`
- UI assets: `ui/`

## Working Rules

1. Prefer the smallest safe change over broad refactors.
2. Keep behavior explicit and machine-readable (structured errors/logs, stable APIs).
3. Preserve backward compatibility for workflow YAML whenever possible.
4. Do not silently broaden scope into Chronicle-level architecture.
5. When changing user-visible behavior, update docs in the same change.

## Validation Requirements

Run these before finishing Rust code changes:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

If UI files under `ui/` change, also run the relevant frontend checks/tests for that package.

## Required Quality Gates

All gates below are mandatory unless the user explicitly waives them.

1. Run the validation commands in this file for every Rust change.
2. Add or update tests for each behavior change (not only new features).
3. Update impacted docs in the same change.
4. Call out any untested risk explicitly in handoff notes.
5. Do not merge logic changes with failing lint/tests.

## Touched-Module Enforcement Matrix

Use this matrix as a minimum bar. If multiple areas are touched, satisfy all relevant rows.

| Touched Area | Minimum Tests Required | Required Docs/Contracts Update |
|---|---|---|
| `src/workflow/` | parser/validation tests for new schema or constraints | `docs/NODE_TYPES.md` (if node-facing), `README.md` examples if syntax changed |
| `src/nodes/` | happy path + failure path + retry/error behavior coverage | `docs/NODE_TYPES.md` |
| `src/triggers/` | trigger routing/filter edge cases + restart/failure behavior tests | `docs/API.md` for webhook/event contract changes |
| `src/engine/` | execution ordering/state transition regression tests | `docs/ARCHITECTURE.md` for behavior model changes |
| `src/storage/` | compatibility/migration tests for schema/state changes | migration notes + relevant `docs/` pages |
| `src/api/` | endpoint contract/status/error tests | `docs/API.md` |
| `src/mcp/` | tool contract tests for input/output stability | MCP usage docs in `README.md` or `docs/` |
| `ui/` | component/integration tests for changed flows | user-facing docs/screenshots only if behavior changed materially |

## Risk Classification and Release Bar

Classify each change before handoff:

- `low`: internal refactor, no contract or behavior change.
- `medium`: behavior change without persistence/contract break.
- `high`: storage/execution semantics, API contract, security-sensitive logic.

Release bar:

1. `low`: standard validation commands.
2. `medium`: standard validation + targeted tests for touched module.
3. `high`: standard validation + targeted tests + explicit rollback/compatibility note in handoff.

## Change Checklists

### Node changes (`src/nodes/`)

- Update node docs in `docs/NODE_TYPES.md` when config/behavior changes.
- Add or update tests for happy path + failure path.
- Validate retry/error-handling interaction with executor behavior.

### Trigger changes (`src/triggers/`)

- Verify restart behavior and failure semantics.
- Confirm API/webhook contracts still match `docs/API.md`.
- Add tests for filtering/routing logic and edge cases.

### Storage/execution changes (`src/storage/`, `src/engine/`)

- Protect existing execution/checkpoint compatibility unless intentionally migrated.
- Document schema or behavior changes in docs and migration notes.
- Watch for blocking operations on async paths.

## Performance and Safety Guardrails

- Avoid unnecessary allocations and blocking calls in hot paths.
- Keep dependencies minimal and justified.
- Maintain SSRF and secret-handling protections in security-sensitive code paths.
- Prefer idempotent behavior at integration boundaries.

## Documentation Discipline

- Keep strategy and planning notes in `.internal/`.
- Keep public product docs in `README.md` and `docs/`.
- If a behavior is important enough to implement, it is important enough to document.
