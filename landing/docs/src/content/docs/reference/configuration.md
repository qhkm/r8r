---
title: Configuration
description: r8r configuration file reference.
---

r8r loads configuration from multiple sources in this priority order:

1. Environment variables (`R8R_*`) — highest priority
2. Config file (`~/.config/r8r/config.toml`)
3. Shared Kitakod config (`~/.config/kitakod/config.toml` under `[r8r]` section)
4. Built-in defaults — lowest priority

## Config File

Location: `~/.config/r8r/config.toml`

```toml
[server]
port = 8080
host = "127.0.0.1"

[storage]
database_path = "/path/to/r8r.db"

[agent]
endpoint = "http://localhost:3000/api/chat"
timeout_seconds = 30
```

### Server Section

| Key | Default | Description |
|-----|---------|-------------|
| `port` | `8080` | HTTP server port |
| `host` | `127.0.0.1` | Bind address |

### Storage Section

| Key | Default | Description |
|-----|---------|-------------|
| `database_path` | `~/.local/share/r8r/r8r.db` | SQLite database file path |

### Agent Section

| Key | Default | Description |
|-----|---------|-------------|
| `endpoint` | `http://localhost:3000/api/chat` | ZeptoClaw agent API endpoint |
| `timeout_seconds` | `30` | Timeout for agent API calls |

## Shared Kitakod Config

If you use multiple Kitakod tools, you can share config at `~/.config/kitakod/config.toml`:

```toml
[r8r]
[r8r.server]
port = 8080

[r8r.agent]
endpoint = "http://localhost:3000/api/chat"
```

## Environment Variable Overrides

Environment variables take precedence over config files:

| Variable | Config Key | Default |
|----------|-----------|---------|
| `R8R_SERVER_PORT` | `server.port` | `8080` |
| `R8R_SERVER_HOST` | `server.host` | `127.0.0.1` |
| `R8R_DATABASE_PATH` | `storage.database_path` | `~/.local/share/r8r/r8r.db` |
| `R8R_AGENT_ENDPOINT` | `agent.endpoint` | `http://localhost:3000/api/chat` |
| `R8R_AGENT_TIMEOUT_SECONDS` | `agent.timeout_seconds` | `30` |

See [Environment Variables](/reference/environment) for the complete list.

## Data Directory

r8r stores its database in the system data directory:

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/r8r/` |
| Linux | `~/.local/share/r8r/` |

## Credentials Storage

Every stored credential and OAuth2 token is encrypted with AES-256-GCM. The key comes from `R8R_ENCRYPTION_KEY` when it is set; otherwise r8r generates one on first use and keeps it in `encryption.key` next to `credentials.json`, readable only by you.

With the generated key file, encryption protects a copied or backed-up `credentials.json` on its own, not someone who can read the whole data directory. On servers, set `R8R_ENCRYPTION_KEY` from a secrets manager to keep the key off the disk. **Back up the key:** without it, stored credentials cannot be recovered.

Manage credentials with:

```bash
r8r credentials set <service> --value <secret> --allowed-hosts <host,...>
r8r credentials scope <service> --allowed-hosts <host,...>
r8r credentials list
r8r credentials delete <service>
```
