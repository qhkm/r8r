---
title: Triggers
description: How to trigger workflow execution in r8r.
---

Triggers define when and how a workflow is executed. r8r supports 4 trigger types.

## Trigger Types

### Cron

Schedule workflows on a recurring basis:

```yaml
triggers:
  - type: cron
    schedule: "*/5 * * * *"   # Every 5 minutes
  - type: cron
    schedule: "0 9 * * MON"   # Every Monday at 9 AM
```

Cron expressions follow standard 5-field format: `minute hour day-of-month month day-of-week`.

Use the `settings.timezone` field to control which timezone the cron schedule uses:

```yaml
settings:
  timezone: "America/New_York"
```

### Webhook

Trigger workflows via HTTP POST requests:

```yaml
triggers:
  - type: webhook
    path: /orders
```

When the server is running, POST to `http://localhost:8080/webhooks/<workflow-name>` to trigger the workflow. The request body becomes the workflow input.

#### Webhook Signature Verification

Verify webhook authenticity using signature schemes:

```yaml
triggers:
  - type: webhook
    path: /github-events
    signature:
      scheme: github          # github | stripe | slack | generic
      secret_env: GITHUB_WEBHOOK_SECRET  # Read from environment
```

Supported schemes:
- **github**: Validates `X-Hub-Signature-256` header
- **stripe**: Validates Stripe webhook signatures
- **slack**: Validates Slack request signatures
- **generic**: HMAC-SHA256 with configurable header

You can also reference stored credentials:

```yaml
    signature:
      scheme: github
      secret_credential: github-webhook  # From r8r credentials store
```

### Event

Subscribe to events via Redis pub/sub:

```yaml
triggers:
  - type: event
    event: order.created
```

When an event matching the name is published, the workflow is triggered with the event payload as input. Requires a Redis connection.

### Manual

Explicitly mark a workflow as manually triggerable:

```yaml
triggers:
  - type: manual
```

Manual triggers allow execution via the CLI (`r8r workflows run`) or API (`POST /api/workflows/{name}/execute`).

## Multiple Triggers

A workflow can have multiple triggers:

```yaml
triggers:
  - type: webhook
    path: /incoming
  - type: cron
    schedule: "0 * * * *"
  - type: manual
```

## Triggering via CLI

```bash
# Run any workflow manually
r8r workflows run my-workflow

# With parameters
r8r workflows run my-workflow -p key=value

# With JSON input
r8r workflows run my-workflow --input '{"key": "value"}'
```

## Triggering via API

```bash
curl -X POST http://localhost:8080/api/workflows/my-workflow/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"key": "value"}}'
```
