# r8r HTTP API Reference

The r8r HTTP API provides programmatic access to workflow management and execution. All responses are in JSON format.

## Base URL

```
http://localhost:8080
```

## Authentication

Currently, the API does not require authentication for workflow execution. For production deployments, place r8r behind a reverse proxy with authentication.

The WebSocket monitoring endpoint (`/api/monitor`) requires a token unless explicitly disabled. See [Environment Variables](./ENVIRONMENT_VARIABLES.md) for details.

---

## API Version

This document describes the **v1** API contract for r8r. All endpoints follow the v1 schema with standardized error envelopes and trace fields.

---

## Standard Error Response

All errors return a standardized error envelope with machine-parseable codes:

```json
{
  "error": {
    "code": "WORKFLOW_NOT_FOUND",
    "message": "Workflow 'order-processing' not found",
    "category": "client_error",
    "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
    "execution_id": null,
    "request_id": "req-abc123",
    "retry_after_ms": null,
    "details": null
  }
}
```

### Error Categories

| Category | Description | Retry Strategy |
|----------|-------------|----------------|
| `client_error` | Invalid request, missing fields, not found | Don't retry without fixing request |
| `transient` | Temporary server error | Retry with exponential backoff |
| `permanent` | Permanent server error | Fail, don't retry |
| `rate_limit` | Rate limit exceeded | Retry after `retry_after_ms` |
| `conflict` | Idempotency key reuse | Return cached result |

### Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `INVALID_REQUEST` | 400 | Malformed request |
| `MISSING_FIELD` | 400 | Required field missing |
| `WORKFLOW_NOT_FOUND` | 404 | Workflow doesn't exist |
| `EXECUTION_NOT_FOUND` | 404 | Execution doesn't exist |
| `INVALID_WORKFLOW_DEFINITION` | 422 | Workflow YAML invalid |
| `IDEMPOTENCY_KEY_REUSE` | 409 | Duplicate idempotency key |
| `RATE_LIMITED` | 429 | Too many requests |
| `INTERNAL_ERROR` | 500 | Server error |
| `SERVICE_UNAVAILABLE` | 503 | Server shutting down |

---

## Endpoints

### Health Check

#### GET `/api/health`

Check the health status of the r8r server and database.

**Response**:

```json
{
  "status": "ok",
  "foreign_keys_enabled": true,
  "integrity_check": "ok",
  "orphaned_executions": 0,
  "orphaned_node_executions": 0,
  "orphaned_workflow_versions": 0,
  "journal_mode": "wal",
  "busy_timeout_ms": 5000
}
```

**Status Codes**:
- `200 OK` - Server and database are healthy
- `500 Internal Server Error` - Database health check failed

---

### Workflows

#### GET `/api/workflows`

List all workflows.

**Response**:

```json
{
  "workflows": [
    {
      "id": "wf-abc123",
      "name": "order-notification",
      "enabled": true,
      "created_at": "2024-01-15T10:00:00Z",
      "updated_at": "2024-01-20T14:30:00Z",
      "node_count": 4,
      "trigger_count": 1
    }
  ]
}
```

**Status Codes**:
- `200 OK` - Success

---

#### GET `/api/workflows/{name}`

Get details of a specific workflow.

**Parameters**:
- `name` (path) - Workflow name

**Response**:

```json
{
  "id": "wf-abc123",
  "name": "order-notification",
  "enabled": true,
  "created_at": "2024-01-15T10:00:00Z",
  "updated_at": "2024-01-20T14:30:00Z",
  "node_count": 4,
  "trigger_count": 1,
  "definition": "name: order-notification\nnodes:\n  ..."
}
```

**Status Codes**:
- `200 OK` - Success
- `404 Not Found` - Workflow not found

---

#### POST `/api/workflows/{name}/execute`

Execute a workflow.

**Parameters**:
- `name` (path) - Workflow name

**Request Body**:

```json
{
  "input": {
    "order_id": "ORD-12345",
    "customer_email": "customer@example.com"
  },
  "wait": true,
  "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
  "idempotency_key": "order-12345-process"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `input` | object | No | Input data for the workflow |
| `wait` | boolean | No | Whether to wait for completion (default: true) |
| `correlation_id` | string (UUID) | No | End-to-end trace identifier (auto-generated if not provided) |
| `idempotency_key` | string | No | Unique key to prevent duplicate executions |

**Response**:

```json
{
  "execution_id": "exec-def456",
  "status": "completed",
  "output": {
    "notifications_sent": 1
  },
  "error": null,
  "duration_ms": 1250
}
```

**Idempotency Behavior**:
- If `idempotency_key` is provided and matches a previous execution, returns `409 Conflict` with the existing execution ID
- Idempotency keys should be unique per logical operation (e.g., order ID)
- Keys are scoped globally across all workflows

**Status Codes**:
- `200 OK` - Execution completed
- `202 Accepted` - Execution started (if wait=false)
- `404 Not Found` - Workflow not found
- `409 Conflict` - Idempotency key already used (see response for existing execution ID)
- `422 Unprocessable Entity` - Invalid workflow definition
- `500 Internal Server Error` - Execution failed
- `503 Service Unavailable` - Server is shutting down

---

### Executions

#### GET `/api/executions/{id}`

Get details of a specific execution.

**Parameters**:
- `id` (path) - Execution ID

**Response**:

```json
{
  "id": "exec-def456",
  "workflow_id": "wf-abc123",
  "workflow_name": "order-notification",
  "status": "completed",
  "trigger_type": "api",
  "input": {
    "order_id": "ORD-12345"
  },
  "output": {
    "notifications_sent": 1
  },
  "error": null,
  "started_at": "2024-01-20T14:30:00Z",
  "finished_at": "2024-01-20T14:30:01Z",
  "duration_ms": 1250,
  "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
  "idempotency_key": "order-12345-process",
  "origin": "api"
}
```

**Trace Fields**:
- `correlation_id` - End-to-end trace ID for cross-system debugging
- `idempotency_key` - Key used to prevent duplicate execution (if provided)
- `origin` - Source of execution ("api", "webhook", "cron", "mcp", "engine")

**Status Codes**:
- `200 OK` - Success
- `404 Not Found` - Execution not found

---

#### GET `/api/executions/{id}/trace`

Get the execution trace with per-node details.

**Parameters**:
- `id` (path) - Execution ID

**Response**:

```json
{
  "execution_id": "exec-def456",
  "workflow_name": "order-notification",
  "status": "completed",
  "nodes": [
    {
      "node_id": "fetch-order",
      "status": "completed",
      "started_at": "2024-01-20T14:30:00.000Z",
      "finished_at": "2024-01-20T14:30:00.500Z",
      "duration_ms": 500,
      "error": null
    },
    {
      "node_id": "send-email",
      "status": "completed",
      "started_at": "2024-01-20T14:30:00.501Z",
      "finished_at": "2024-01-20T14:30:01.250Z",
      "duration_ms": 749,
      "error": null
    }
  ]
}
```

**Status Codes**:
- `200 OK` - Success
- `404 Not Found` - Execution not found

---

### WebSocket Monitoring

#### GET `/api/monitor?token={token}`

WebSocket endpoint for real-time execution monitoring.

**Query Parameters**:
- `token` (query) - Authentication token (required unless `R8R_MONITOR_PUBLIC=true`)

**WebSocket Messages**:

**Client → Server**:
```json
{
  "action": "subscribe",
  "execution_id": "exec-def456"
}
```

**Server → Client**:

Execution started:
```json
{
  "event": "execution_started",
  "execution_id": "exec-def456",
  "workflow_name": "order-notification",
  "trigger_type": "api"
}
```

Node started:
```json
{
  "event": "node_started",
  "execution_id": "exec-def456",
  "node_id": "fetch-order",
  "node_type": "http"
}
```

Node completed:
```json
{
  "event": "node_completed",
  "execution_id": "exec-def456",
  "node_id": "fetch-order",
  "status": "completed",
  "duration_ms": 500
}
```

Execution completed:
```json
{
  "event": "execution_completed",
  "execution_id": "exec-def456",
  "workflow_name": "order-notification",
  "status": "completed",
  "duration_ms": 1250
}
```

Execution failed:
```json
{
  "event": "execution_failed",
  "execution_id": "exec-def456",
  "workflow_name": "order-notification",
  "error": "HTTP request failed with status 500"
}
```

---

### Webhooks

Workflows can define webhook triggers that create HTTP endpoints:

```yaml
name: webhook-processor
triggers:
  - type: webhook
    path: /custom-webhook  # Optional, defaults to workflow name
    method: POST           # Optional, defaults to POST

nodes:
  - id: process
    type: transform
    config:
      expression: "input"
```

The webhook will be available at:

```
POST /webhooks/webhook-processor
# or with custom path:
POST /webhooks/custom-webhook
```

**Request**: Any JSON body

**Response**:

```json
{
  "execution_id": "exec-ghi789",
  "status": "completed",
  "output": { ... }
}
```

---

## Error Responses

All errors follow a consistent format:

```json
{
  "error": {
    "code": "WORKFLOW_NOT_FOUND",
    "message": "Workflow 'order-processing' not found",
    "category": "client_error",
    "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
    "execution_id": null,
    "request_id": null,
    "retry_after_ms": null,
    "details": null
  }
}
```

**Common HTTP Status Codes**:

| Status | Meaning |
|--------|---------|
| `400` | Bad Request - Invalid request payload or field values |
| `404` | Not Found - Resource doesn't exist |
| `409` | Conflict - Idempotency key was already used |
| `422` | Unprocessable Entity - Invalid workflow definition |
| `413` | Payload Too Large - Request body exceeds limit |
| `429` | Too Many Requests - Concurrency limit reached |
| `500` | Internal Server Error - Unexpected server error |
| `503` | Service Unavailable - Server is shutting down or backend unavailable |

---

## Rate Limiting

The API enforces concurrency limits controlled by `R8R_MAX_CONCURRENT_REQUESTS`. When the limit is reached, new requests will wait until a slot becomes available.

There is no per-IP rate limiting by default. For production deployments, add rate limiting at the reverse proxy level (nginx, traefik, etc.).

---

## Examples

### Execute a workflow with curl

```bash
# Execute workflow
curl -X POST http://localhost:8080/api/workflows/order-notification/execute \
  -H "Content-Type: application/json" \
  -d '{
    "input": {
      "order_id": "ORD-12345",
      "customer_email": "customer@example.com"
    }
  }'
```

### Check execution status

```bash
# Get execution details
curl http://localhost:8080/api/executions/exec-def456

# Get execution trace
curl http://localhost:8080/api/executions/exec-def456/trace
```

### List all workflows

```bash
curl http://localhost:8080/api/workflows
```

### WebSocket monitoring with wscat

```bash
# Install wscat: npm install -g wscat

wscat -c "ws://localhost:8080/api/monitor?token=your-token-here"

> {"action": "subscribe", "execution_id": "exec-def456"}
```

---

## OpenAPI/Swagger

An OpenAPI specification is planned for a future release. For now, refer to this document for API details.

---

*For CLI usage, see `r8r --help`. For environment configuration, see [Environment Variables](./ENVIRONMENT_VARIABLES.md).*
