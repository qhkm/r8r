---
title: API Integration
description: Using the r8r REST API to manage and execute workflows.
---

r8r provides a REST API for managing workflows and executions. The server runs on port 8080 by default.

## Starting the Server

```bash
r8r server --port 8080
```

The API is also available as an OpenAPI 3.1 spec at `/api/openapi.json`.

## Endpoints

### Health Check

```bash
GET /api/health
```

Returns server health status including database integrity.

### List Workflows

```bash
GET /api/workflows
```

Response:
```json
{
  "workflows": [
    {
      "id": "uuid",
      "name": "my-workflow",
      "enabled": true,
      "created_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-01T00:00:00Z",
      "node_count": 3,
      "trigger_count": 1
    }
  ]
}
```

### Get Workflow

```bash
GET /api/workflows/{name}
```

Returns workflow details including the YAML definition.

### Execute Workflow

```bash
POST /api/workflows/{name}/execute
Content-Type: application/json

{
  "input": {
    "key": "value"
  }
}
```

Response:
```json
{
  "execution_id": "uuid",
  "status": "completed",
  "output": { "result": "..." },
  "duration_ms": 150
}
```

### Get Execution

```bash
GET /api/executions/{id}
```

Returns execution status, input, output, and timing.

### Get Execution Trace

```bash
GET /api/executions/{id}/trace
```

Returns per-node execution details:
```json
{
  "execution_id": "uuid",
  "workflow_name": "my-workflow",
  "status": "completed",
  "nodes": [
    {
      "node_id": "fetch",
      "status": "completed",
      "started_at": "...",
      "finished_at": "...",
      "duration_ms": 120
    }
  ]
}
```

### Prometheus Metrics

```bash
GET /api/metrics
```

Returns Prometheus-formatted metrics for monitoring.

### Live Monitoring (WebSocket)

```bash
WS /api/monitor?token=<token>
```

Subscribe to live execution events. Set `R8R_MONITOR_TOKEN` for authentication, or set `R8R_MONITOR_PUBLIC=true` for development.

## Webhooks

Workflows with webhook triggers are accessible at:

```bash
POST /webhooks/{workflow-name}
```

The request body becomes the workflow input.

## Example: Complete Integration

```bash
# Create a workflow
r8r workflows create my-workflow.yaml

# Execute via API
EXEC_ID=$(curl -s -X POST http://localhost:8080/api/workflows/my-workflow/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"url": "https://example.com"}}' | jq -r '.execution_id')

# Check execution status
curl http://localhost:8080/api/executions/$EXEC_ID

# View detailed trace
curl http://localhost:8080/api/executions/$EXEC_ID/trace
```
