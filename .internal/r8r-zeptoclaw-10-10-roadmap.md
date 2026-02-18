# r8r + ZeptoClaw: 10/10 Integration Roadmap

> Internal strategy note
> Last updated: 2026-02-18

## Goal

Make `r8r + zeptoclaw` production-grade as a combined system:

- ZeptoClaw handles reasoning and dynamic decisions
- r8r handles deterministic workflow execution
- r8r-chronicle handles durable/critical orchestration when needed

Success target: clear operational confidence, stable contracts, and low-friction developer use.

## Discussion Summary (2026-02-18)

### Usefulness Ratings Discussed

- `r8r + zeptoclaw` today (dev/internal): **7/10**
- `r8r` for mission-critical orchestration today: **4/10**
- Single-tenant production (non-compliance-heavy): **~6/10**
- If r8r passes beta with strong stability/testing:
  - `r8r + zeptoclaw`: **~8.5/10**
  - Single-tenant production: **~7.5/10**
  - Enterprise-critical in r8r alone: **~5.5-6.5/10**

### Clarifications Agreed

- Distributed execution, PostgreSQL backend, RBAC, audit, and SSO are valid enterprise-track features.
- Lower production score was based on **current operational maturity**, not product direction.
- Temporal-class durability and high-assurance orchestration belong to **`r8r-chronicle`**, not core `r8r`.
- With `zeptoclaw + r8r + r8r-chronicle` combined:
  - Current expected fit: **~8/10**
  - Hardened integration target: **~9/10**

### Main Gap to Reach 10/10

Integration rigor between repos, especially:

- versioned contracts
- retry/idempotency ownership
- cross-repo conformance tests
- unified tracing/correlation IDs
- deterministic failure routing

## What Must Be True for "10/10"

1. Versioned, stable integration contracts across API/MCP/workflow payloads.
2. Explicit retry ownership and idempotency policy across all boundaries.
3. End-to-end conformance tests across repositories, not just unit tests.
4. Unified tracing and correlation IDs from agent request to workflow node.
5. Deterministic failure routing (retry, DLQ, escalate to chronicle, or ask agent).
6. Tight security posture for agent-triggered execution and secrets handling.
7. SLO-backed operations with runbooks and drills.
8. Backward-compatible upgrade path with migration/deprecation policy.
9. Performance budgets with regression gates in CI.
10. Strong DX templates so agents generate valid workflows reliably.

## 30/60/90 Plan

### Day 0-30: Contract and Control

- Freeze `v1` request/response schemas for:
  - execute workflow
  - execution status
  - trace output
  - structured error envelopes
- Define ownership matrix:
  - who retries (agent vs r8r vs chronicle)
  - where idempotency keys are generated/validated
  - timeout budget per layer
- Add mandatory fields:
  - `correlation_id`
  - `idempotency_key`
  - `origin` and `trigger_type`
- Publish failure routing policy:
  - transient retry path
  - non-retryable failure path
  - chronicle escalation criteria

Exit criteria:

- One canonical contract doc and schema artifacts checked into repo(s).
- Integration RFC approved by maintainers of all touched repos.
- New flows reject missing correlation/idempotency metadata.

### Day 31-60: Conformance and Observability

- Add cross-repo integration test harness that runs:
  - happy path execution
  - duplicate request replay
  - timeout propagation
  - partial failure + recovery
  - dependency outage scenarios
- Enforce conformance checks in CI for both directions:
  - ZeptoClaw -> r8r
  - r8r -> ZeptoClaw
- Standardize logs/traces:
  - shared correlation ID propagation
  - machine-parseable error codes
  - node-level trace linkage

Exit criteria:

- CI fails on contract drift or behavior drift.
- A single trace ID can reconstruct a full round-trip request path.
- Error classes are stable and documented.

### Day 61-90: Production Hardening and DX

- Define and enforce SLOs:
  - success rate
  - p95 latency for agent -> r8r -> result
  - recovery time for failed executions
- Add runbooks and game days:
  - queue backlog
  - external provider outage
  - stale execution recovery
- Add compatibility guarantees:
  - schema deprecation window
  - migration playbooks
  - rollback guidance
- Improve agent-authoring DX:
  - gold-path workflow templates
  - linting/validation presets
  - example prompts with expected YAML output

Exit criteria:

- SLO dashboards are live and reviewed regularly.
- Failure drills complete with action items closed.
- Agent-generated workflow success rate reaches agreed benchmark.

## Ownership Suggestion

- r8r maintainers: execution contract, trace model, retry semantics.
- ZeptoClaw maintainers: tool-call contract compliance, correlation propagation.
- Chronicle maintainers: escalation boundary and durable execution handoff.
- Shared platform owner: cross-repo CI and SLO governance.

## Risk Register (Current)

- Contract drift across repos without version pinning.
- Duplicate execution if idempotency policy is inconsistent.
- Ambiguous responsibility during failures (agent vs workflow vs chronicle).
- Incomplete observability causing slow incident triage.

## Definition of Done (Program-Level)

- Contract tests green across all integrated repos.
- Retry/idempotency behavior is deterministic and documented.
- On-call can triage from one correlation ID.
- SLOs are met for two consecutive release cycles.
