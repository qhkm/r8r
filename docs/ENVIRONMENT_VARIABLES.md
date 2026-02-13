# Environment Variables Reference

This document describes all environment variables used by r8r for configuration.

## Table of Contents

- [Core Configuration](#core-configuration)
- [Security Settings](#security-settings)
- [API Server Settings](#api-server-settings)
- [CORS Configuration](#cors-configuration)
- [Database Settings](#database-settings)
- [Monitoring & Observability](#monitoring--observability)
- [Template Security](#template-security)
- [Advanced Settings](#advanced-settings)

---

## Core Configuration

### `R8R_DATA_DIR`

**Default**: Platform-specific data directory

- Linux: `~/.local/share/r8r`
- macOS: `~/Library/Application Support/r8r`
- Windows: `%APPDATA%/r8r`

**Description**: Directory where r8r stores its data files including the SQLite database and credentials.

**Example**:
```bash
export R8R_DATA_DIR=/var/lib/r8r
```

---

## Security Settings

### `R8R_MONITOR_TOKEN`

**Required**: No (unless `R8R_MONITOR_PUBLIC` is not set to `true`)

**Description**: Authentication token for WebSocket monitoring endpoint (`/api/monitor`). Clients must provide this token via the `?token=<value>` query parameter.

**Security Note**: Keep this token secret. Use a strong random value in production.

**Example**:
```bash
export R8R_MONITOR_TOKEN=$(openssl rand -hex 32)
```

### `R8R_MONITOR_PUBLIC`

**Default**: `false`

**Description**: When set to `true`, disables authentication for the WebSocket monitoring endpoint. **Not recommended for production.**

**Example**:
```bash
# Development only!
export R8R_MONITOR_PUBLIC=true
```

### `R8R_ALLOWED_ENV_VARS`

**Default**: None (only `R8R_*` prefixed variables are allowed)

**Description**: Comma-separated list of additional environment variable names that can be accessed in templates. By default, only variables starting with `R8R_` are accessible to prevent accidental exposure of sensitive environment variables.

**Example**:
```bash
# Allow specific variables for templates
export R8R_ALLOWED_ENV_VARS="API_URL,WEBHOOK_SECRET,FEATURE_FLAG"
```

### `R8R_ALLOW_INTERNAL_URLS`

**Default**: `false`

**Description**: When set to `true`, disables SSRF protection in the HTTP node, allowing requests to localhost, private IP ranges, and internal hostnames. **Only use for testing/development.**

**Example**:
```bash
# Testing only - allows access to internal services
export R8R_ALLOW_INTERNAL_URLS=true
```

---

## API Server Settings

### `R8R_MAX_CONCURRENT_REQUESTS`

**Default**: `100`

**Description**: Maximum number of concurrent HTTP requests the API server will handle. Additional requests will wait until a slot is available.

**Example**:
```bash
# Increase for high-traffic deployments
export R8R_MAX_CONCURRENT_REQUESTS=500
```

### `R8R_MAX_REQUEST_BODY_BYTES`

**Default**: `1048576` (1 MiB)

**Description**: Maximum size of HTTP request bodies in bytes. Requests exceeding this size will be rejected with a 413 Payload Too Large error.

**Example**:
```bash
# Allow larger payloads for file processing workflows
export R8R_MAX_REQUEST_BODY_BYTES=10485760  # 10 MiB
```

---

## Request Tracing

### `R8R_TRUST_REQUEST_ID`

**Default**: `false`

**Description**: When set to `true`, trust incoming `X-Request-ID` headers from clients. When `false` (default), a new UUID is always generated for each request. Enable only behind a trusted proxy that sets this header.

**Example**:
```bash
# Trust X-Request-ID from load balancer
export R8R_TRUST_REQUEST_ID=true
```

### `R8R_ACCESS_LOG`

**Default**: `true`

**Description**: Enable structured JSON access logging for all requests. Logs include method, path, status, duration, request ID, user agent, and client IP.

**Example**:
```bash
# Disable access logging (not recommended)
export R8R_ACCESS_LOG=false
```

### `R8R_HEALTH_API_KEY`

**Default**: None (no authentication required)

**Description**: When set, requires authentication for `/api/health` and `/api/metrics` endpoints. Clients must provide the key via `Authorization: Bearer <key>` or `Authorization: ApiKey <key>` header.

**Security Note**: Use a strong random value. Prevents health endpoint enumeration and information disclosure to unauthorized parties.

**Example**:
```bash
# Require authentication for health endpoints
export R8R_HEALTH_API_KEY=$(openssl rand -hex 32)
```

---

## CORS Configuration

### `R8R_CORS_ORIGINS`

**Default**: `http://localhost:3000`

**Description**: Comma-separated list of allowed origins for CORS requests. Each origin must be a valid URL.

**Example**:
```bash
# Multiple origins
export R8R_CORS_ORIGINS="https://app.example.com,https://admin.example.com"
```

### `R8R_CORS_ALLOW_ALL`

**Default**: `false`

**Description**: When set to `true`, allows all origins for CORS. **Not recommended for production** as it disables an important security protection.

**Example**:
```bash
# Development only
export R8R_CORS_ALLOW_ALL=true
```

---

## OpenTelemetry Tracing

### `R8R_OTEL_ENABLED`

**Default**: `false`

**Description**: Enable OpenTelemetry distributed tracing. When enabled, traces are exported to an OTLP-compatible collector.

**Example**:
```bash
export R8R_OTEL_ENABLED=true
```

### `R8R_OTEL_ENDPOINT`

**Default**: `http://localhost:4317`

**Description**: OTLP endpoint URL for trace export. Supports gRPC endpoints.

**Example**:
```bash
# Jaeger
export R8R_OTEL_ENDPOINT=http://jaeger:4317

# Grafana Tempo
export R8R_OTEL_ENDPOINT=http://tempo:4317
```

### `R8R_OTEL_SERVICE_NAME`

**Default**: `r8r`

**Description**: Service name for identifying traces in the collector.

**Example**:
```bash
export R8R_OTEL_SERVICE_NAME=r8r-production
```

### `R8R_OTEL_SAMPLE_RATE`

**Default**: `1.0`

**Description**: Sampling rate for traces (0.0 to 1.0). Use lower values in high-traffic environments.

**Example**:
```bash
# Sample 10% of traces
export R8R_OTEL_SAMPLE_RATE=0.1
```

---

## Database Settings

### `R8R_DATABASE_PATH`

**Default**: `{R8R_DATA_DIR}/r8r.db`

**Description**: Path to the SQLite database file. Can be an absolute or relative path.

**Example**:
```bash
# Custom database location
export R8R_DATABASE_PATH=/data/r8r/production.db
```

### `R8R_DB_POOL_SIZE`

**Default**: `4`

**Description**: Number of connections in the optional connection pool. Only used when explicitly using `ConnectionPool` instead of `SqliteStorage`.

**Example**:
```bash
# Increase pool size for high-concurrency workloads
export R8R_DB_POOL_SIZE=8
```

### `R8R_DB_POOL_TIMEOUT_SECS`

**Default**: `30`

**Description**: Timeout in seconds for acquiring a connection from the pool.

**Example**:
```bash
export R8R_DB_POOL_TIMEOUT_SECS=10
```

---

## Monitoring & Observability

### `RUST_LOG`

**Default**: `r8r=info`

**Description**: Logging configuration using the `tracing-subscriber` format. Controls the verbosity of logs.

**Levels**: `error`, `warn`, `info`, `debug`, `trace`

**Example**:
```bash
# Debug logging for r8r, warnings for everything else
export RUST_LOG="r8r=debug,warn"

# Trace logging for specific modules
export RUST_LOG="r8r::engine=trace,r8r::api=debug,r8r=info"
```

---

## Template Security

The HTTP and Agent nodes support template rendering with the following variables:

- `{{ input }}` - The node's input data
- `{{ nodes.<node_id>.output }}` - Output from a previous node
- `{{ env.VARNAME }}` - Environment variables (restricted, see below)

### Environment Variable Access

By default, only environment variables starting with `R8R_` are accessible in templates. This prevents accidental exposure of sensitive credentials like:

- `PATH`
- `HOME`
- `AWS_SECRET_ACCESS_KEY`
- `DATABASE_URL`

To allow additional variables, add them to `R8R_ALLOWED_ENV_VARS`:

```bash
export R8R_ALLOWED_ENV_VARS="API_ENDPOINT,SERVICE_TOKEN"
```

Then in your workflow:

```yaml
nodes:
  - id: fetch-data
    type: http
    config:
      url: "{{ env.API_ENDPOINT }}/data"
      headers:
        Authorization: "Bearer {{ env.SERVICE_TOKEN }}"
```

---

## Advanced Settings

### HTTP Node Timeouts

The HTTP node uses these default timeouts:

- **Connection timeout**: 10 seconds
- **Request timeout**: 30 seconds

These can be overridden per-node in the workflow configuration:

```yaml
nodes:
  - id: slow-api
    type: http
    config:
      url: "https://slow-api.example.com/data"
      timeout_seconds: 120
```

### Workflow Settings

Individual workflows can configure these settings in their YAML definition:

```yaml
name: my-workflow
settings:
  timeout_seconds: 3600        # Maximum execution time (default: 3600)
  max_concurrency: 10          # Concurrent node executions (default: 10)
  max_for_each_items: 10000    # Max items in for_each loops (default: 10000)
  chunk_size: 100              # Process for_each in batches (default: 0)
  store_traces: true           # Store execution traces (default: true)
  debug: false                 # Enable debug mode (default: false)

nodes:
  # ... node definitions
```

---

## Production Deployment Checklist

Before deploying r8r to production, ensure:

- [ ] `R8R_MONITOR_TOKEN` is set to a strong random value
- [ ] `R8R_MONITOR_PUBLIC` is NOT set (defaults to false)
- [ ] `R8R_CORS_ALLOW_ALL` is NOT set (defaults to false)
- [ ] `R8R_CORS_ORIGINS` lists only your actual domains
- [ ] `R8R_ALLOW_INTERNAL_URLS` is NOT set (defaults to false)
- [ ] `R8R_ALLOWED_ENV_VARS` only includes necessary variables
- [ ] `R8R_HEALTH_API_KEY` is set if health endpoints should be protected
- [ ] `R8R_TRUST_REQUEST_ID` is only enabled behind a trusted proxy
- [ ] `RUST_LOG` is set to an appropriate level (info or warn)
- [ ] Database file has proper permissions (readable only by r8r user)
- [ ] Credentials file (`credentials.json`) has proper permissions

---

## Example Production Configuration

```bash
#!/bin/bash
# /etc/r8r/env

# Core
export R8R_DATA_DIR=/var/lib/r8r
export R8R_DATABASE_PATH=/var/lib/r8r/r8r.db

# Security (generate with: openssl rand -hex 32)
export R8R_MONITOR_TOKEN="your-64-character-hex-token-here"
export R8R_HEALTH_API_KEY="your-health-api-key-here"

# CORS - only allow your actual domains
export R8R_CORS_ORIGINS="https://workflows.example.com"

# Request Tracing
export R8R_TRUST_REQUEST_ID=true  # Enable if behind trusted proxy
export R8R_ACCESS_LOG=true

# API Limits
export R8R_MAX_CONCURRENT_REQUESTS=200
export R8R_MAX_REQUEST_BODY_BYTES=5242880  # 5 MiB

# Template Security
export R8R_ALLOWED_ENV_VARS="API_BASE_URL,WEBHOOK_SECRET"

# Logging
export RUST_LOG="r8r=info,r8r::access=info,warn"
```

---

*For more information, see the [Architecture](./ARCHITECTURE.md) and [Security Audit](../SECURITY_AUDIT_REPORT.md) documents.*
