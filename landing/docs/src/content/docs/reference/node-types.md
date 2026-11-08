---
title: Node Types Reference
description: Complete reference for all r8r built-in node types.
---

r8r ships with 24 built-in node types.

## Core

### http

Make HTTP/REST API calls.

```yaml
- id: call-api
  type: http
  config:
    url: https://api.example.com/data
    method: GET                    # GET, POST, PUT, PATCH, DELETE
    headers:
      Authorization: "Bearer {{ env.R8R_API_TOKEN }}"
      Content-Type: application/json
    body: '{"key": "value"}'       # Request body (POST/PUT/PATCH)
    timeout_seconds: 30
```

SSRF protection blocks requests to internal IPs by default. Set `R8R_ALLOW_INTERNAL_URLS=true` to disable.

### transform

Transform data using Rhai expressions.

```yaml
- id: process
  type: transform
  config:
    expression: |
      let items = input.data;
      items.filter(|item| item.active)
```

See [Custom Logic with Rhai](/guides/custom-nodes) for expression syntax.

### agent

Invoke an LLM/AI agent via the ZeptoClaw API.

```yaml
- id: analyze
  type: agent
  config:
    prompt: "Summarize the following data: {{ nodes.fetch }}"
    model: gpt-4
```

Requires `R8R_AGENT_ENDPOINT` to be configured.

### subworkflow

Execute another workflow as a nested step.

```yaml
- id: run-child
  type: subworkflow
  config:
    workflow: child-workflow-name
    input:
      key: "{{ nodes.prev.value }}"
```

## Logic & Routing

### if

Conditional branching:

```yaml
- id: check
  type: if
  config:
    condition: "input.amount > 100"
    true_branch: high-value
    false_branch: standard
```

### switch

Multi-branch routing:

```yaml
- id: route
  type: switch
  config:
    value: "input.category"
    cases:
      electronics: process-electronics
      clothing: process-clothing
    default: process-other
```

### merge

Combine outputs from multiple upstream nodes:

```yaml
- id: combine
  type: merge
  depends_on: [branch-a, branch-b]
```

## Data Manipulation

### filter

Filter arrays:

```yaml
- id: active-only
  type: filter
  config:
    expression: "item.status == 'active'"
  depends_on: [fetch]
```

### sort

Sort arrays:

```yaml
- id: sorted
  type: sort
  config:
    field: "created_at"
    order: desc        # asc | desc
  depends_on: [fetch]
```

### limit

Limit array size:

```yaml
- id: top-ten
  type: limit
  config:
    count: 10
  depends_on: [sorted]
```

### set

Set or override values:

```yaml
- id: enrich
  type: set
  config:
    values:
      processed: true
      timestamp: "{{ datetime.now }}"
```

### aggregate

Aggregate data:

```yaml
- id: stats
  type: aggregate
  config:
    operations:
      - field: amount
        function: sum     # sum, count, avg, min, max
        alias: total
  depends_on: [fetch]
```

### split

Split arrays into chunks:

```yaml
- id: batches
  type: split
  config:
    chunk_size: 100
  depends_on: [fetch]
```

### dedupe

Remove duplicate items:

```yaml
- id: unique
  type: dedupe
  config:
    field: email
  depends_on: [fetch]
```

### variables

Workflow-scoped state management:

```yaml
- id: init-vars
  type: variables
  config:
    set:
      counter: 0
      status: "started"
```

## Integrations

### email

Send emails through an API provider:

```yaml
- id: send-email
  type: email
  config:
    provider: resend         # resend, sendgrid or mailgun
    to: user@example.com
    subject: "Report Ready"
    body: "Your report is ready."
```

`provider` defaults to `smtp`, which is not implemented yet: a node using it is rejected when the workflow is saved, so always set `provider`. For Mailgun, the sending domain defaults to the domain of `from`; set `mailgun_domain` to override it and `mailgun_base_url: https://api.eu.mailgun.net` for EU accounts.

### slack

Send Slack messages:

```yaml
- id: notify
  type: slack
  config:
    channel: "#alerts"
    message: "Workflow completed: {{ nodes.process.count }} items"
```

### database

Execute SQL queries against a SQLite database:

```yaml
- id: query
  type: database
  config:
    db_type: sqlite
    connection_string: "./data/app.db"
    query: "SELECT * FROM users WHERE active = ?"
    params: [1]
```

PostgreSQL and MySQL are not implemented yet; a node with `db_type: postgres` or `mysql` is rejected when the workflow is saved. For Supabase or PostgREST, use the `supabase` integration or an `http` node.

### s3

S3-compatible object storage:

```yaml
- id: upload
  type: s3
  config:
    operation: put           # get, put, delete, list
    bucket: my-bucket
    key: "reports/{{ datetime.today }}.json"
    body: "{{ nodes.generate }}"
```

Requests are sent unsigned, which works for public buckets and S3-compatible endpoints that don't require auth. AWS request signing and `presign` are not implemented yet, so a node that sets `credential`, `access_key_id` or `secret_access_key`, or uses `presign`, is rejected when the workflow is saved rather than sending requests without its keys.

## Utility

### wait

Delay execution:

```yaml
- id: pause
  type: wait
  config:
    seconds: 5
```

### crypto

Cryptographic operations:

```yaml
- id: hash
  type: crypto
  config:
    operation: hash          # hash, hmac, encrypt, decrypt
    algorithm: sha256
    input: "{{ nodes.fetch.data }}"
```

### datetime

Date/time operations:

```yaml
- id: parse-date
  type: datetime
  config:
    operation: parse
    input: "2025-01-15T10:30:00Z"
    format: "%Y-%m-%dT%H:%M:%SZ"
```

### debug

Log data during development:

```yaml
- id: log
  type: debug
  config:
    message: "Current data: {{ nodes.fetch }}"
    level: info              # info, debug, warn, error
```

### summarize

Summarize text using an LLM:

```yaml
- id: summary
  type: summarize
  config:
    text: "{{ nodes.fetch.body }}"
    max_length: 200
```

Requires `R8R_AGENT_ENDPOINT` to be configured.
