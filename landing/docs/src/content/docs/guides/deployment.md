---
title: Deployment
description: Deploy r8r to production with Docker.
---

## Docker

### Build the Image

```bash
docker build -t r8r .
```

The multi-stage build produces a minimal Debian-based image (~50MB) containing the `r8r` and `r8r-mcp` binaries.

### Run

```bash
docker run -d \
  --name r8r \
  -p 8080:8080 \
  -v r8r-data:/data \
  -e RUST_LOG=r8r=info \
  r8r:latest
```

### Docker Compose

Create a `docker-compose.yml`:

```yaml
services:
  r8r:
    build: .
    image: r8r:latest
    container_name: r8r
    ports:
      - "8080:8080"
    volumes:
      - r8r-data:/data
    environment:
      - RUST_LOG=r8r=info
      - R8R_DB=/data/r8r.db
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "r8r", "workflows", "list"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 5s

volumes:
  r8r-data:
    name: r8r-data
```

Start with:

```bash
docker compose up -d
docker compose logs -f  # View logs
```

## Environment Variables

Key environment variables for production:

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `r8r=info` | Log level |
| `R8R_DB` | `/data/r8r.db` | Database path (Docker) |
| `R8R_DATABASE_PATH` | `~/.local/share/r8r/r8r.db` | Database path (native) |
| `R8R_SERVER_PORT` | `8080` | HTTP port |
| `R8R_SERVER_HOST` | `127.0.0.1` | Bind address |
| `R8R_CORS_ORIGINS` | `http://localhost:3000` | Allowed CORS origins (comma-separated) |
| `R8R_MAX_CONCURRENT_REQUESTS` | `100` | Request concurrency limit |
| `R8R_MAX_REQUEST_BODY_BYTES` | `1048576` | Max request body (1MB) |
| `R8R_MONITOR_TOKEN` | (none) | WebSocket auth token |
| `R8R_HEALTH_API_KEY` | (none) | API key for health endpoints |

See [Environment Variables](/reference/environment) for the full list.

## Port Configuration

The default port is **8080**. Override with:

```bash
# CLI flag
r8r server --port 9090

# Environment variable
R8R_SERVER_PORT=9090 r8r server
```

## Credentials

Store secrets securely with the credentials manager:

```bash
# Store a credential
r8r credentials set slack-webhook --value "https://hooks.slack.com/..."

# List stored credentials (values are masked)
r8r credentials list
```

Credentials are encrypted with AES-256-GCM and stored locally.

## Health Check

The `/api/health` endpoint checks database integrity:

```bash
curl http://localhost:8080/api/health
```

For production monitoring, optionally protect with an API key:

```bash
R8R_HEALTH_API_KEY=your-secret-key r8r server
```

## Reverse Proxy (Nginx)

```nginx
server {
    listen 80;
    server_name r8r.example.com;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/monitor {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```
