# r8r Node Types Reference

This document describes all built-in node types available in r8r.

## Table of Contents

- [Core Nodes](#core-nodes)
  - [HTTP](#http)
  - [Transform](#transform)
  - [Filter](#filter)
  - [Switch](#switch)
  - [If](#if)
  - [Merge](#merge)
  - [Split](#split)
  - [Set](#set)
  - [Variables](#variables)
  - [Wait](#wait)
- [Data Processing](#data-processing)
  - [Sort](#sort)
  - [Limit](#limit)
  - [Dedupe](#dedupe)
  - [Aggregate](#aggregate)
  - [Summarize](#summarize)
- [Integration Nodes](#integration-nodes)
  - [Agent](#agent)
  - [SubWorkflow](#subworkflow)
- [Utility Nodes](#utility-nodes)
  - [Debug](#debug)
  - [DateTime](#datetime)
  - [Crypto](#crypto)

---

## Core Nodes

### HTTP

Make HTTP requests to external APIs.

**Type**: `http`

**Configuration**:

```yaml
nodes:
  - id: api-call
    type: http
    config:
      url: "https://api.example.com/data"
      method: POST
      headers:
        Content-Type: application/json
        X-Custom-Header: "{{ input.header_value }}"
      body:
        key: "{{ input.value }}"
      timeout_seconds: 30
      raw_response: false
      credential: api_key
      auth_type: bearer
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | Yes | - | URL to request |
| `method` | string | No | GET | HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD) |
| `headers` | object | No | - | HTTP headers |
| `body` | any | No | - | Request body (JSON) |
| `timeout_seconds` | number | No | 30 | Request timeout |
| `raw_response` | boolean | No | false | Return raw text instead of parsing JSON |
| `credential` | string | No | - | Credential name from credential store |
| `auth_type` | string | No | bearer | Authentication type (bearer, basic, api_key, header:name) |

**Output**:

```json
{
  "status": 200,
  "headers": { "content-type": "application/json" },
  "body": { ... }
}
```

**Authentication Types**:
- `bearer`: Adds `Authorization: Bearer <credential>` header
- `basic`: Credential format `username:password`, adds Basic auth header
- `api_key`: Adds `X-API-Key: <credential>` header
- `header:<name>`: Adds custom header with specified name

---

### Transform

Transform data using Rhai expressions.

**Type**: `transform`

**Configuration**:

```yaml
nodes:
  - id: transform-data
    type: transform
    config:
      expression: |
        input.items.map(|item| item.name)
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `expression` | string | Yes | Rhai expression to evaluate |

**Variables Available**:
- `input` - Node input data
- `nodes.<id>` - Output from previous nodes

**Output**: Result of the expression

---

### Filter

Filter array items based on a condition.

**Type**: `filter`

**Configuration**:

```yaml
nodes:
  - id: filter-active
    type: filter
    config:
      condition: "item.status == 'active'"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `condition` | string | Yes | Rhai expression returning boolean |

**Variables Available**:
- `item` - Current array item
- `index` - Current index
- `input` - Full input

**Output**: Filtered array

---

### Switch

Route execution based on multiple conditions.

**Type**: `switch`

**Configuration**:

```yaml
nodes:
  - id: route-by-type
    type: switch
    config:
      field: "{{ input.type }}"
      cases:
        email: process-email
        sms: process-sms
      default: process-default
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `field` | string | Yes | Value to match |
| `cases` | object | Yes | Map of values to node IDs |
| `default` | string | No | Default node ID if no match |

**Output**: The matched value

---

### If

Conditional execution (binary switch).

**Type**: `if`

**Configuration**:

```yaml
nodes:
  - id: check-condition
    type: if
    config:
      condition: "input.count > 10"
      true_node: handle-large
      false_node: handle-small
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `condition` | string | Yes | Rhai boolean expression |
| `true_node` | string | No | Node ID if condition is true |
| `false_node` | string | No | Node ID if condition is false |

---

### Merge

Merge multiple inputs into one.

**Type**: `merge`

**Configuration**:

```yaml
nodes:
  - id: combine-data
    type: merge
    config:
      strategy: object  # or "array"
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `strategy` | string | No | object | Merge strategy: object, array |

**Output**: Merged data

---

### Split

Split an array into individual items for processing.

**Type**: `split`

**Configuration**:

```yaml
nodes:
  - id: split-items
    type: split
    config:
      path: "input.items"  # Optional, defaults to input
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | No | JSON path to array |

**Note**: Use with `for_each: true` on the next node to process items.

---

### Set

Set constant values or transform data.

**Type**: `set`

**Configuration**:

```yaml
nodes:
  - id: set-values
    type: set
    config:
      values:
        status: "processed"
        timestamp: "{{ $now }}"
        message: "Hello {{ input.name }}"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `values` | object | Yes | Key-value pairs to set |

**Special Variables**:
- `{{ $now }}` - Current ISO8601 timestamp
- `{{ $uuid }}` - Random UUID
- `{{ input.field }}` - Reference input data

---

### Variables

Manage workflow variables.

**Type**: `variables`

**Configuration**:

```yaml
nodes:
  - id: manage-vars
    type: variables
    config:
      operation: set  # set, get, merge, delete, list
      values:
        counter: 42
      names: ["counter"]  # for get/delete operations
      passthrough: true
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `operation` | string | No | set | Operation: set, get, merge, delete, list |
| `values` | object | No | - | Values for set/merge operations |
| `names` | array | No | - | Variable names for get/delete |
| `passthrough` | boolean | No | true | Pass input through to output |

---

### Wait

Pause execution for a duration or until a specific time.

**Type**: `wait`

**Configuration**:

```yaml
nodes:
  - id: wait-5-seconds
    type: wait
    config:
      milliseconds: 5000
      # or
      seconds: 5
      minutes: 1
      until: "2024-12-25T00:00:00Z"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `milliseconds` | number | No | Duration in milliseconds |
| `seconds` | number | No | Duration in seconds |
| `minutes` | number | No | Duration in minutes |
| `until` | string | No | ISO8601 timestamp to wait until |

**Note**: Maximum wait time is 24 hours.

---

## Data Processing

### Sort

Sort an array by a field.

**Type**: `sort`

**Configuration**:

```yaml
nodes:
  - id: sort-by-date
    type: sort
    config:
      path: "input.orders"
      field: "created_at"
      direction: desc
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `path` | string | No | input | Path to array |
| `field` | string | No | - | Field to sort by |
| `direction` | string | No | asc | Sort direction: asc, desc |

---

### Limit

Limit the number of items in an array.

**Type**: `limit`

**Configuration**:

```yaml
nodes:
  - id: top-10
    type: limit
    config:
      path: "input.items"
      count: 10
      offset: 0
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `path` | string | No | input | Path to array |
| `count` | number | Yes | - | Maximum items to return |
| `offset` | number | No | 0 | Number of items to skip |

---

### Dedupe

Remove duplicate items from an array.

**Type**: `dedupe`

**Configuration**:

```yaml
nodes:
  - id: unique-items
    type: dedupe
    config:
      path: "input.orders"
      field: "order_id"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | No | Path to array |
| `field` | string | No | Field to check for uniqueness |

---

### Aggregate

Aggregate data (sum, avg, min, max, count).

**Type**: `aggregate`

**Configuration**:

```yaml
nodes:
  - id: total-sales
    type: aggregate
    config:
      path: "input.orders"
      field: "amount"
      operation: sum
      group_by: "region"
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `path` | string | No | input | Path to array |
| `field` | string | Yes | - | Field to aggregate |
| `operation` | string | Yes | - | Operation: sum, avg, min, max, count |
| `group_by` | string | No | - | Group by field |

---

### Summarize

Generate statistical summary of data.

**Type**: `summarize`

**Configuration**:

```yaml
nodes:
  - id: stats
    type: summarize
    config:
      path: "input.data"
      fields: ["age", "income"]
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | No | Path to array |
| `fields` | array | Yes | Fields to summarize |

**Output**: Statistics (count, min, max, avg, median, stddev) for each field

---

## Integration Nodes

### Agent

Call an AI agent (ZeptoClaw) for intelligent processing.

**Type**: `agent`

**Configuration**:

```yaml
nodes:
  - id: classify-ticket
    type: agent
    config:
      endpoint: "http://localhost:3000/api/chat"
      prompt: |
        Classify this support ticket:
        Subject: {{ input.subject }}
        Body: {{ input.body }}
        
        Respond with JSON: {"priority": "low|medium|high"}
      response_format: json
      model: "claude-sonnet-4"
      credential: ai_api_key
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `endpoint` | string | Yes | AI service endpoint |
| `prompt` | string | Yes | Prompt template |
| `response_format` | string | No | Expected response format (json) |
| `model` | string | No | Model to use |
| `credential` | string | No | API key credential name |

---

### SubWorkflow

Execute another workflow.

**Type**: `subworkflow`

**Configuration**:

```yaml
nodes:
  - id: run-subflow
    type: subworkflow
    config:
      workflow: common-validation
      input:
        data: "{{ input }}"
      inherit_credentials: true
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `workflow` | string | Yes | - | Workflow name or ID |
| `input` | object | No | - | Input for sub-workflow |
| `inherit_credentials` | boolean | No | true | Pass credentials to sub-workflow |

---

## Utility Nodes

### Debug

Log data for debugging.

**Type**: `debug`

**Configuration**:

```yaml
nodes:
  - id: debug-input
    type: debug
    config:
      label: "Before transformation"
      level: info  # trace, debug, info, warn, error
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `label` | string | No | - | Label for the log message |
| `level` | string | No | debug | Log level |

---

### DateTime

Date and time operations.

**Type**: `datetime`

**Configuration**:

```yaml
nodes:
  - id: format-date
    type: datetime
    config:
      operation: format
      input: "{{ input.timestamp }}"
      format: "%Y-%m-%d %H:%M:%S"
      timezone: "Asia/Kuala_Lumpur"
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `operation` | string | Yes | - | now, parse, format, add, diff |
| `input` | string | No | - | Input timestamp |
| `format` | string | No | ISO8601 | Format string or preset |
| `timezone` | string | No | UTC | Target timezone |

**Operations**:
- `now` - Current timestamp
- `parse` - Parse a date string
- `format` - Format a timestamp
- `add` - Add duration to timestamp
- `diff` - Calculate difference between timestamps

---

### Crypto

Cryptographic operations.

**Type**: `crypto`

**Configuration**:

```yaml
nodes:
  - id: hash-data
    type: crypto
    config:
      operation: hash
      algorithm: sha256
      data: "{{ input.data }}"
      encoding: hex  # hex, base64
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `operation` | string | Yes | hash, hmac, encrypt, decrypt |
| `algorithm` | string | Yes | Algorithm name |
| `data` | string | Yes | Input data |
| `encoding` | string | No | Output encoding |
| `key` | string | No | Key for HMAC/encryption |

**Algorithms**:
- Hash: `md5`, `sha256`, `sha512`, `blake3`
- HMAC: `hmac-sha256`

---

## Common Patterns

### Processing a List of Items

```yaml
nodes:
  - id: fetch-orders
    type: http
    config:
      url: "https://api.example.com/orders"
  
  - id: process-each
    type: transform
    config:
      expression: "input.orders"
    depends_on: [fetch-orders]
  
  - id: send-notification
    type: http
    config:
      url: "https://api.example.com/notify"
      method: POST
      body:
        order_id: "{{ item.id }}"
    depends_on: [process-each]
    for_each: true
```

### Conditional Execution

```yaml
nodes:
  - id: check-status
    type: if
    config:
      condition: "input.priority == 'high'"
      true_node: urgent-processing
      false_node: normal-processing
```

### Error Handling

```yaml
nodes:
  - id: risky-operation
    type: http
    config:
      url: "https://unreliable-api.example.com/data"
    retry:
      max_attempts: 3
      delay_seconds: 5
      backoff: exponential
    on_error:
      action: fallback
      fallback_value:
        status: "error"
        data: null
```

---

*For more examples, see the [examples/](../examples/) directory.*
