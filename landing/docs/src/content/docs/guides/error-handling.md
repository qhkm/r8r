---
title: Error Handling
description: Configure retry policies and error handling in r8r workflows.
---

r8r provides per-node error handling with retry policies and fallback strategies.

## Retry Policy

Configure automatic retries for transient failures:

```yaml
- id: call-api
  type: http
  config:
    url: https://api.example.com/data
    method: GET
  retry:
    max_attempts: 3           # 1-10 attempts (default: 3)
    backoff: exponential      # fixed | linear | exponential (default: exponential)
    delay_seconds: 2          # Initial delay, 1-300s (default: 1)
    max_delay_seconds: 60     # Max delay cap, 1-3600s
```

### Backoff Strategies

| Strategy | Behavior |
|----------|----------|
| `fixed` | Same delay between each retry |
| `linear` | Delay increases linearly (delay * attempt) |
| `exponential` | Delay doubles each retry (delay * 2^attempt), capped by `max_delay_seconds` |

Example with `exponential`, `delay_seconds: 2`:
- Attempt 1: 2s delay
- Attempt 2: 4s delay
- Attempt 3: 8s delay

## Error Actions

The `on_error` field controls what happens when a node fails (after all retries are exhausted):

```yaml
on_error:
  action: continue    # fail | continue | fallback
```

### fail (default)

Stop the entire workflow immediately:

```yaml
- id: critical-step
  type: http
  config:
    url: https://api.example.com/important
  on_error:
    action: fail
```

### continue

Skip the failed node and continue executing downstream nodes:

```yaml
- id: optional-notification
  type: slack
  config:
    channel: "#alerts"
    message: "Something happened"
  on_error:
    action: continue
```

Downstream nodes that depend on the failed node will receive no output from it.

### fallback

Use a fallback value as the node's output when it fails:

```yaml
- id: fetch-config
  type: http
  config:
    url: https://config-service.example.com/settings
  on_error:
    action: fallback
    fallback_value:
      theme: "default"
      max_items: 100
```

The `fallback_value` can be any JSON value -- object, array, string, number, or boolean.

## Combining Retry and Error Handling

A common pattern is to retry transient failures, then fall back gracefully:

```yaml
- id: fetch-data
  type: http
  config:
    url: https://api.example.com/data
  retry:
    max_attempts: 3
    backoff: exponential
    delay_seconds: 1
  on_error:
    action: fallback
    fallback_value: []
```

This will:
1. Try the HTTP request up to 3 times with exponential backoff
2. If all retries fail, use `[]` as the node's output instead of failing

## Per-Node Timeout

Set a maximum execution time for individual nodes:

```yaml
- id: slow-operation
  type: http
  config:
    url: https://slow-api.example.com/process
  timeout_seconds: 30    # 1-3600 seconds
```

If the node exceeds the timeout, it fails and error handling kicks in.

## Workflow-Level Timeout

Set a maximum execution time for the entire workflow:

```yaml
settings:
  timeout_seconds: 300   # 5 minutes (1-86400, default: 3600)
```
