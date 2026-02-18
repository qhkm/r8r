# Failure Routing and Idempotency Policy

> Internal operational guide for r8r + ZeptoClaw integration
> Last updated: 2026-02-18

## Overview

This document defines the failure routing policy and idempotency guarantees for the r8r workflow execution system. It establishes clear ownership boundaries for retries, escalation criteria, and cross-repo integration contracts.

## Idempotency Policy

### Definition

An idempotency key is a unique client-provided identifier that ensures exactly-once execution semantics for workflow runs. The system guarantees:

1. **At-most-once execution**: Duplicate requests with the same idempotency key return the cached result
2. **Global scope**: Idempotency keys are unique across all workflows
3. **Permanent storage**: Keys persist until explicitly purged (not TTL-based)

### Usage

```json
POST /api/workflows/order-process/execute
{
  "input": {"order_id": "ORD-12345"},
  "idempotency_key": "order-ORD-12345-process",
  "correlation_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

### Conflict Response

When a duplicate idempotency key is detected:

```json
HTTP 409 Conflict
{
  "error": {
    "code": "IDEMPOTENCY_KEY_REUSE",
    "message": "Request with this idempotency key already processed",
    "category": "conflict",
    "execution_id": "exec-abc123",
    "correlation_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

The client should treat this as success and use the provided `execution_id` to check status.

## Retry Ownership Matrix

### Determining Who Retries

| Layer | Retries | Responsibility | Strategy |
|-------|---------|----------------|----------|
| **Client (ZeptoClaw)** | First-line | User code | Configurable backoff, up to 3 attempts |
| **r8r API** | None | Never | Returns error immediately, logs trace |
| **r8r Engine** | Node-level | Workflow definition | Per-node retry config (fixed, linear, exponential) |
| **r8r DLQ** | Manual | Operator | Scheduled retries via API/UI |
| **r8r-chronicle** | Workflow-level | Platform | Durable orchestration for critical flows |

### Retry Decision Tree

```
Execution Failure
├── Transient Error (5xx, timeout, connection)
│   ├── r8r node retries (if configured) → Success
│   ├── Node retries exhausted → DLQ
│   └── DLQ retry scheduled → Success
│
├── Client Error (4xx, invalid input)
│   └── No retry → Return error to caller
│
├── Permanent Error (logic error, missing dependency)
│   ├── DLQ for manual review
│   └── Optional: Escalate to chronicle if critical
│
└── Idempotency Conflict (409)
    └── Return cached result immediately
```

## Failure Routing Paths

### Path 1: Transient Retry (Automatic)

**Criteria**: Network errors, timeouts, rate limits, temporary unavailability

**Flow**:
1. Node execution fails with transient error
2. Engine applies retry policy (if configured)
3. Success → Continue workflow
4. Exhausted → Add to DLQ with status `pending`

**Example**:
```yaml
nodes:
  - id: fetch-data
    type: http
    retry:
      max_attempts: 3
      backoff: exponential
      initial_delay_ms: 1000
```

### Path 2: Non-Retryable Failure (Terminal)

**Criteria**: Invalid input, workflow definition errors, authentication failures

**Flow**:
1. Execution fails with client error
2. No automatic retry
3. Execution status set to `failed`
4. Error returned to caller immediately

**Response**:
```json
{
  "error": {
    "code": "INVALID_REQUEST",
    "category": "client_error",
    "message": "Missing required field 'order_id'"
  }
}
```

### Path 3: Dead Letter Queue (Manual Recovery)

**Criteria**: Retries exhausted, unknown errors, partial failures

**Flow**:
1. Execution/node fails after all retries
2. Entry added to DLQ with:
   - Original execution context
   - Error details
   - Retry count
   - Next retry time (if scheduled)
3. Status: `pending` → `scheduled` → `resolved`/`discarded`

**DLQ States**:
- `pending`: Awaiting first retry attempt
- `scheduled`: Retry queued for future time
- `resolved`: Successfully retried
- `discarded`: Permanently failed, archived
- `acknowledged`: Operator reviewed, no action

### Path 4: Chronicle Escalation (Critical Flows)

**Criteria**: 
- Workflow marked as `critical: true`
- Multi-step saga patterns
- Must survive node/process restart
- Exactly-once execution guarantees required

**Flow**:
1. Pre-flight check: Is chronicle available?
2. Yes → Hand off to chronicle for durable execution
3. No → Fail fast with `SERVICE_UNAVAILABLE`

**Example**:
```yaml
name: payment-processing
settings:
  critical: true  # Escalate to chronicle
  escalation_policy: chronicle
nodes:
  - id: charge-card
    type: http
    # ...
```

## Cross-Repo Contract

### ZeptoClaw → r8r

**Responsibilities**:
- Generate and propagate `correlation_id`
- Respect `retry_after_ms` from rate limit errors
- Cache responses for `idempotency_key_reuse` conflicts
- Implement circuit breaker for `SERVICE_UNAVAILABLE`

**Expected Behavior**:
```python
# ZeptoClaw client
response = r8r.execute_workflow(
    workflow="order-process",
    input=data,
    idempotency_key=f"order-{order_id}",
    correlation_id=trace_id
)

if response.error.code == "IDEMPOTENCY_KEY_REUSE":
    # Use existing execution
    return get_execution(response.error.execution_id)
elif response.error.category == "transient":
    # Retry with backoff
    retry_with_backoff(response.error.retry_after_ms)
```

### r8r → Chronicle

**Escalation Criteria**:
- Workflow has `critical: true` setting
- Execution timeout > 5 minutes
- Multi-workflow saga coordination needed

**Handoff Protocol**:
1. r8r validates workflow and creates execution record
2. If critical, submits to chronicle with:
   - Full workflow definition
   - Execution context (correlation_id, input)
   - Timeout budget
3. Chronicle takes ownership of execution
4. r8r monitors via polling or webhook

### Error Code Stability

API error codes are part of the v1 contract and will not change without a major version bump:

- **Stable**: `WORKFLOW_NOT_FOUND`, `IDEMPOTENCY_KEY_REUSE`, `RATE_LIMITED`
- **May Evolve**: Error message text (human-readable)
- **Never Breaking**: HTTP status codes for documented scenarios

## SLO and Monitoring

### Success Rate Targets

| Path | Target | Measurement |
|------|--------|-------------|
| Transient retry success | 99.9% | % retried that succeed |
| DLQ resolution | 95% | % manually resolved within 24h |
| Chronicle handoff | 99.99% | % successful escalations |
| End-to-end latency | p95 < 5s | Request to response |

### Key Metrics

```
r8r_executions_total{status, trigger_type}
r8r_execution_duration_seconds_bucket{workflow_name}
r8r_dlq_entries_total{status}
r8r_idempotency_hits_total
r8r_chronicle_escalations_total{result}
```

## Runbooks

### Runbook: DLQ Backlog

**Symptom**: DLQ `pending` count > 100

1. Check external dependencies (databases, APIs)
2. Review error patterns: `SELECT error, COUNT(*) FROM dead_letter_queue WHERE status='pending' GROUP BY error`
3. If rate limit: Increase retry delays
4. If service down: Pause scheduler, fix service, resume
5. Bulk retry valid entries via API

### Runbook: Idempotency Key Collisions

**Symptom**: High rate of `IDEMPOTENCY_KEY_REUSE`

1. Verify clients are generating unique keys per operation
2. Check for clock skew in key generation
3. Review TTL if implemented (not default)
4. Alert if collision rate > 5% of requests

### Runbook: Chronicle Escalation Failures

**Symptom**: Critical workflows failing to escalate

1. Check chronicle service health
2. Verify network connectivity
3. Review escalation timeout configuration
4. If chronicle unavailable > 5 min: Enable "fail open" mode (execute in r8r with warnings)

## Version History

- **v1.0** (2026-02-18): Initial policy for r8r + ZeptoClaw integration
  - Idempotency guarantees
  - DLQ state machine
  - Chronicle escalation criteria
  - Error code contract
