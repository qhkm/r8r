---
title: Environment Variables
description: Complete list of r8r environment variables.
---

All r8r environment variables use the `R8R_` prefix.

## Server

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_SERVER_PORT` | `8080` | HTTP server port |
| `R8R_SERVER_HOST` | `127.0.0.1` | Bind address |

## Database

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_DATABASE_PATH` | `~/.local/share/r8r/r8r.db` | SQLite database path |
| `R8R_DB` | (none) | Database path (Docker alias) |
| `R8R_DB_POOL_SIZE` | `4` | Connection pool size |
| `R8R_DB_POOL_TIMEOUT_SECS` | `30` | Pool connection timeout |

## Agent (ZeptoClaw)

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_AGENT_ENDPOINT` | `http://localhost:3000/api/chat` | Agent API endpoint |
| `R8R_AGENT_TIMEOUT_SECONDS` | `30` | Agent call timeout |

## CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_CORS_ORIGINS` | `http://localhost:3000` | Allowed origins (comma-separated) |
| `R8R_CORS_ALLOW_ALL` | `false` | Allow all origins (NOT for production) |

## Security

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_ALLOW_INTERNAL_URLS` | `false` | Disable SSRF protection |
| `R8R_HEALTH_API_KEY` | (none) | API key for /api/health and /api/metrics |
| `R8R_MONITOR_TOKEN` | (none) | WebSocket monitor authentication token |
| `R8R_MONITOR_PUBLIC` | `false` | Allow unauthenticated WebSocket access |
| `R8R_TRUST_REQUEST_ID` | `false` | Trust incoming X-Request-ID headers |

## Rate Limiting

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_MAX_CONCURRENT_REQUESTS` | `100` | Max concurrent API requests |
| `R8R_MAX_REQUEST_BODY_BYTES` | `1048576` | Max request body size (bytes) |

## Observability (OpenTelemetry)

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_OTEL_ENABLED` | `false` | Enable OpenTelemetry tracing |
| `R8R_OTEL_ENDPOINT` | `http://localhost:4317` | OTLP endpoint URL |
| `R8R_OTEL_SERVICE_NAME` | `r8r` | Service name for traces |
| `R8R_OTEL_SAMPLE_RATE` | `1.0` | Trace sampling rate (0.0-1.0) |

## Logging

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `r8r=info` | Log level filter |
| `R8R_ACCESS_LOG` | `true` | Enable HTTP access logging |
| `R8R_ACCESS_LOG_BODY` | `false` | Log request/response bodies (not recommended) |

## Template Engine

| Variable | Default | Description |
|----------|---------|-------------|
| `R8R_ALLOWED_ENV_VARS` | (none) | Additional env vars accessible in `{{ env.* }}` templates (comma-separated) |
