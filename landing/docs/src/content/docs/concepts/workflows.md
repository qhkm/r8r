---
title: Workflows
description: Understanding r8r workflow structure and configuration.
---

A workflow is a YAML file that defines a directed acyclic graph (DAG) of nodes. Each workflow has a name, optional triggers, and a list of nodes that execute based on their dependency order.

## Workflow Structure

```yaml
name: order-processor
description: Process incoming orders
version: "1.0.0"

settings:
  timeout_seconds: 300
  max_concurrency: 5
  debug: true

triggers:
  - type: webhook
    path: /orders
  - type: cron
    schedule: "0 * * * *"

input_schema:
  type: object
  required: [order_id]
  properties:
    order_id:
      type: string

nodes:
  - id: validate
    type: transform
    config:
      expression: |
        if input.amount > 0 { input } else { throw "Invalid amount" }

  - id: notify
    type: http
    config:
      url: https://slack.com/api/chat.postMessage
      method: POST
      body: '{"text": "New order: {{ nodes.validate.amount }}"}'
    depends_on: [validate]
    retry:
      max_attempts: 3
      backoff: exponential
    on_error:
      action: continue
```

## Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Unique workflow name (alphanumeric, hyphens, underscores). Must start with a letter. Max 128 chars. |
| `nodes` | array | At least one node definition. |

## Optional Fields

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Human-readable description. Max 1024 chars. |
| `version` | string | Semantic version (e.g., `1.0.0`). |
| `settings` | object | Workflow-level settings. |
| `triggers` | array | List of trigger definitions. |
| `input_schema` | object | JSON Schema for validating workflow input. |

## Settings

Control workflow execution behavior:

```yaml
settings:
  timeout_seconds: 3600     # Max execution time (1-86400, default: 3600)
  max_concurrency: 10       # Max parallel node executions (1-1000, default: 10)
  max_for_each_items: 10000 # Max items in for_each (1-100000, default: 10000)
  chunk_size: 100           # Batch size for for_each processing
  store_traces: true        # Store execution traces (default: true)
  debug: false              # Enable debug logging (default: false)
  timezone: "America/New_York"  # IANA timezone for cron triggers
```

## Node Definition

Each node in the `nodes` array defines a step in the workflow:

```yaml
- id: my-node           # Required: unique identifier
  type: http             # Required: node type
  config:                # Node-specific configuration
    url: https://api.example.com
    method: GET
  depends_on: [prev-node]  # Nodes that must complete first
  condition: "input.status == 200"  # Rhai expression for conditional execution
  for_each: "nodes.prev-node.items"  # Iterate over array
  timeout_seconds: 60    # Per-node timeout (1-3600)
  retry:                 # Retry policy
    max_attempts: 3
    backoff: exponential
    delay_seconds: 2
  on_error:              # Error handling
    action: continue
```

## Template Expressions

Use `{{ }}` syntax to reference data from other nodes and environment variables:

```yaml
# Reference another node's output
url: "https://api.example.com/{{ nodes.fetch.id }}"

# Reference environment variables (R8R_* or allowlisted)
headers:
  Authorization: "Bearer {{ env.R8R_API_TOKEN }}"
```

## Error Handling

Each node can define error handling behavior:

```yaml
on_error:
  action: fail        # fail (default) | continue | fallback
  fallback_value: []  # Value to use when action is 'fallback'
```

- **fail**: Stop the workflow immediately
- **continue**: Skip the failed node, continue execution
- **fallback**: Use `fallback_value` as the node's output

## Retry Policy

Configure automatic retries for transient failures:

```yaml
retry:
  max_attempts: 3          # 1-10 (default: 3)
  backoff: exponential     # fixed | linear | exponential (default: exponential)
  delay_seconds: 2         # Initial delay, 1-300 (default: 1)
  max_delay_seconds: 60    # Cap for exponential backoff, 1-3600
```
