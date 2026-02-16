---
title: Nodes
description: Understanding node types and configuration in r8r.
---

Nodes are the building blocks of r8r workflows. Each node performs a specific action — making HTTP requests, transforming data, routing logic, or integrating with external services.

## Node Structure

Every node requires an `id` and `type`:

```yaml
- id: my-node
  type: http
  config:
    url: https://api.example.com
    method: GET
```

## Node Fields

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `id` | Yes | string | Unique identifier (alphanumeric, hyphens, underscores). Max 64 chars. |
| `type` | Yes | string | One of the 24 built-in node types. |
| `config` | No | object | Node-specific configuration. |
| `depends_on` | No | array | Node IDs that must complete before this node runs. |
| `condition` | No | string | Rhai expression; node only runs if this evaluates to true. |
| `for_each` | No | string | Expression returning an array; the node runs once per item. |
| `retry` | No | object | Retry policy (`max_attempts`, `backoff`, `delay_seconds`). |
| `on_error` | No | object | Error handling (`action`: fail/continue/fallback). |
| `timeout_seconds` | No | integer | Per-node timeout (1-3600 seconds). |

## Built-in Node Types

### Core

| Type | Purpose |
|------|---------|
| `http` | Make HTTP/REST API calls |
| `transform` | Transform data using Rhai expressions |
| `agent` | Invoke LLM/AI agents (via ZeptoClaw) |
| `subworkflow` | Execute another workflow as a sub-step |

### Logic & Routing

| Type | Purpose |
|------|---------|
| `if` | Conditional branching (if/else) |
| `switch` | Multi-branch routing based on value |
| `merge` | Combine outputs from multiple nodes |

### Data Manipulation

| Type | Purpose |
|------|---------|
| `filter` | Filter arrays based on conditions |
| `sort` | Sort arrays by field |
| `limit` | Limit array to N items |
| `set` | Set/override values |
| `aggregate` | Aggregate data (sum, count, avg, etc.) |
| `split` | Split arrays into chunks |
| `dedupe` | Remove duplicate items |
| `variables` | Workflow-scoped state management |

### Integrations

| Type | Purpose |
|------|---------|
| `email` | Send emails (SMTP, SendGrid, Resend, Mailgun) |
| `slack` | Send Slack messages |
| `database` | Execute SQL queries (SQLite, PostgreSQL, MySQL) |
| `s3` | S3-compatible object storage operations |

### Utility

| Type | Purpose |
|------|---------|
| `wait` | Delay/sleep for a duration |
| `crypto` | Hashing, encryption, HMAC operations |
| `datetime` | Date/time parsing and formatting |
| `debug` | Log data for development |
| `summarize` | Summarize text using LLM |

## Dependencies

Use `depends_on` to create execution order:

```yaml
nodes:
  - id: fetch
    type: http
    config:
      url: https://api.example.com/data

  - id: process
    type: transform
    config:
      expression: "input.items.len()"
    depends_on: [fetch]

  - id: notify
    type: slack
    config:
      channel: "#alerts"
      message: "Processed {{ nodes.process }} items"
    depends_on: [process]
```

Nodes without dependencies run in parallel automatically.

## Conditional Execution

Use `condition` with Rhai expressions:

```yaml
- id: alert
  type: email
  config:
    to: admin@example.com
    subject: "High error rate"
  depends_on: [check]
  condition: "nodes.check.error_rate > 0.05"
```

## Iteration with for_each

Process arrays item by item:

```yaml
- id: process-items
  type: http
  config:
    url: "https://api.example.com/items/{{ item.id }}"
    method: PUT
  depends_on: [fetch]
  for_each: "nodes.fetch.items"
```

The current item is available as `item` within the node.
