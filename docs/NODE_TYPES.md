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
  - [Email](#email)
  - [Slack](#slack)
  - [Database](#database)
  - [S3](#s3)
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

AI agent node — call OpenAI, Anthropic, Ollama, or any OpenAI-compatible endpoint for classification, generation, and reasoning. Supports structured output validation via JSON Schema.

**Type**: `agent`

**Providers**: `zeptoclaw` (default), `openai`, `anthropic`, `ollama`, `custom`

**Configuration**:

```yaml
# OpenAI with structured output
nodes:
  - id: classify-ticket
    type: agent
    config:
      provider: openai
      model: gpt-4o
      credential: openai
      prompt: |
        Classify this support ticket:
        Subject: {{ input.subject }}
        Body: {{ input.body }}
      response_format: json
      json_schema:
        type: object
        required: [priority, reason]
        properties:
          priority: { type: string, enum: [low, medium, high] }
          reason: { type: string }
```

```yaml
# Anthropic
  - id: summarize
    type: agent
    config:
      provider: anthropic
      model: claude-sonnet-4-5-20250929
      system: "You write concise technical summaries."
      prompt: "Summarize: {{ input.document }}"
      max_tokens: 1024
```

```yaml
# Ollama (local, no credentials needed)
  - id: local-classify
    type: agent
    config:
      provider: ollama
      model: llama3
      prompt: "Classify: {{ input }}"
      response_format: json
```

```yaml
# Custom OpenAI-compatible endpoint
  - id: custom
    type: agent
    config:
      provider: custom
      endpoint: https://my-vllm.internal/v1/chat/completions
      credential: vllm_key
      model: my-model
      prompt: "Process: {{ input }}"
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `prompt` | string | Yes | - | Prompt template (supports `{{ input }}`, `{{ nodes.* }}`) |
| `provider` | string | No | `zeptoclaw` | LLM provider: `zeptoclaw`, `openai`, `anthropic`, `ollama`, `custom` |
| `model` | string | No | Provider default | Model name (e.g. `gpt-4o`, `claude-sonnet-4-5-20250929`, `llama3`) |
| `credential` | string | No | Auto-detect | Credential name for API key |
| `endpoint` | string | No | Provider default | API endpoint URL |
| `system` | string | No | - | System prompt |
| `response_format` | string | No | `text` | Expected format: `text` or `json` |
| `json_schema` | object | No | - | JSON Schema to validate response (when `response_format: json`) |
| `temperature` | number | No | - | Generation temperature (0.0–1.0) |
| `max_tokens` | integer | No | Provider default | Maximum response tokens |
| `timeout_seconds` | integer | No | `30` | Request timeout |
| `agent` | string | No | - | ZeptoClaw agent template name |

**Credential resolution**: Explicit `credential` field > provider-specific default (`openai`, `anthropic`) > `llm` fallback. ZeptoClaw and Ollama don't require credentials.

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

### Email

Send emails via SMTP or API providers.

**Type**: `email`

**Configuration**:

```yaml
nodes:
  - id: send-notification
    type: email
    config:
      provider: resend  # smtp, sendgrid, resend, mailgun
      to: "user@example.com"
      from: "noreply@example.com"
      subject: "Order Confirmation"
      body: "Your order #{{ input.order_id }} has been confirmed."
      html: "<h1>Order Confirmed</h1><p>Order #{{ input.order_id }}</p>"
      credential: email_api_key
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | No | smtp | Email provider: smtp, sendgrid, resend, mailgun |
| `to` | string/array | Yes | - | Recipient email(s) |
| `from` | string | Yes | - | Sender email |
| `subject` | string | Yes | - | Email subject |
| `body` | string | No | - | Plain text body |
| `html` | string | No | - | HTML body |
| `cc` | string/array | No | - | CC recipients |
| `bcc` | string/array | No | - | BCC recipients |
| `reply_to` | string | No | - | Reply-to address |
| `credential` | string | No | - | API key credential name |

**SMTP-specific fields**:
- `smtp_host` - SMTP server hostname
- `smtp_port` - SMTP port (default: 587)
- `smtp_username` - SMTP username
- `smtp_password` - SMTP password

---

### Slack

Send messages to Slack channels.

**Type**: `slack`

**Configuration**:

```yaml
nodes:
  - id: notify-slack
    type: slack
    config:
      channel: "#alerts"
      text: "New order received: {{ input.order_id }}"
      blocks:
        - type: section
          text:
            type: mrkdwn
            text: "*New Order*\nOrder ID: {{ input.order_id }}"
      credential: slack_token
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `channel` | string | Yes | Slack channel (e.g., "#general" or channel ID) |
| `text` | string | No* | Message text (*required if no blocks) |
| `blocks` | array | No* | Block Kit blocks (*required if no text) |
| `attachments` | array | No | Legacy attachments |
| `username` | string | No | Bot username override |
| `icon_emoji` | string | No | Bot icon emoji (e.g., ":robot_face:") |
| `icon_url` | string | No | Bot icon URL |
| `thread_ts` | string | No | Thread timestamp (for replies) |
| `reply_broadcast` | boolean | No | Also post to channel when replying |
| `webhook_url` | string | No | Webhook URL (alternative to token) |
| `credential` | string | No | OAuth token credential name |

---

### Database

Execute SQL queries on databases.

**Type**: `database`

**Configuration**:

```yaml
nodes:
  - id: fetch-users
    type: database
    config:
      db_type: sqlite  # sqlite, postgres, mysql
      connection_string: "./data/app.db"
      query: "SELECT * FROM users WHERE status = ?"
      params: ["active"]
      operation: query  # query or execute
      max_rows: 100
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `db_type` | string | No | sqlite | Database type: sqlite, postgres, mysql |
| `connection_string` | string | No | - | Connection string or DSN |
| `query` | string | Yes | - | SQL query to execute |
| `params` | array | No | - | Query parameters (positional) |
| `operation` | string | No | query | Operation: query (SELECT) or execute (INSERT/UPDATE/DELETE) |
| `max_rows` | number | No | 1000 | Maximum rows to return |
| `timeout_seconds` | number | No | 30 | Query timeout |
| `credential` | string | No | - | Credential name for connection string |

**Output** (query):
```json
{
  "success": true,
  "db_type": "sqlite",
  "operation": "query",
  "columns": ["id", "name", "email"],
  "rows": [{ "id": 1, "name": "John", "email": "john@example.com" }],
  "row_count": 1
}
```

**Output** (execute):
```json
{
  "success": true,
  "db_type": "sqlite",
  "operation": "execute",
  "affected_rows": 5
}
```

---

### S3

Perform S3 object storage operations.

**Type**: `s3`

**Configuration**:

```yaml
nodes:
  - id: upload-file
    type: s3
    config:
      operation: put
      bucket: my-bucket
      key: "uploads/{{ input.filename }}"
      content: "{{ input.data }}"
      content_type: "application/json"
      region: us-east-1
      credential: aws_credentials
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `operation` | string | Yes | get, put, delete, list, copy, head, presign |
| `bucket` | string | Yes | S3 bucket name |
| `key` | string | No* | Object key (*required for most operations) |
| `content` | string | No | Content to upload (for put) |
| `content_base64` | string | No | Base64-encoded content (for binary data) |
| `content_type` | string | No | MIME type for uploads |
| `prefix` | string | No | Prefix filter (for list) |
| `max_keys` | number | No | Max keys to return (for list, default: 1000) |
| `region` | string | No | AWS region (default: us-east-1) |
| `endpoint` | string | No | Custom endpoint (for MinIO, R2, etc.) |
| `credential` | string | No | Credential name (format: access_key:secret_key) |

**Operations**:
- `get/download` - Download an object
- `put/upload` - Upload content to S3
- `delete` - Delete an object
- `list` - List objects in bucket
- `copy` - Copy object within S3
- `head` - Get object metadata
- `presign` - Generate presigned URL

**Output** (get):
```json
{
  "success": true,
  "operation": "get",
  "bucket": "my-bucket",
  "key": "data.json",
  "content_type": "application/json",
  "content": "{ ... }",
  "is_base64": false
}
```

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
