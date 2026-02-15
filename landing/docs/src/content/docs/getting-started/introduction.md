---
title: Introduction
description: What is r8r and why does it exist?
---

**r8r** (pronounced "rater") is an agent-first workflow automation engine written in Rust. While tools like n8n and Zapier were built for humans clicking through visual editors, r8r was designed for the AI age — where agents create, execute, and orchestrate workflows programmatically.

## Why r8r?

| Traditional Tools | r8r |
|------------------|-----|
| Visual drag-and-drop | LLM-friendly YAML |
| Heavy (500MB+ RAM) | Lightweight (~15MB RAM) |
| Slow startup | ~50ms cold start |
| Locked in database | Git-friendly files |
| Built for humans | Built for agents |

## r8r vs n8n

| Feature | r8r | n8n |
|---------|-----|-----|
| **Primary User** | AI agents & developers | Human operators |
| **Interface** | CLI, API, MCP | Visual drag-and-drop |
| **Language** | Rust | TypeScript |
| **Binary Size** | 24 MB | ~200 MB+ |
| **Memory (idle)** | ~15 MB | ~500 MB+ |
| **Startup** | ~50ms | Seconds |
| **Storage** | SQLite (embedded) | PostgreSQL/MySQL |
| **Workflows** | YAML files (git-friendly) | Database blobs |
| **MCP Support** | Built-in | None |
| **License** | AGPL-3.0 | Sustainable Use License |

## Key Concepts

### Workflows

A workflow is a YAML file that defines a directed acyclic graph (DAG) of nodes:

```yaml
name: my-workflow
description: A simple workflow

triggers:
  - type: webhook
    path: /incoming

nodes:
  - id: fetch-data
    type: http
    config:
      url: https://api.example.com/data
      method: GET

  - id: process
    type: transform
    config:
      expression: 'input.items.filter(|item| item.active)'
    depends_on: [fetch-data]
```

### Node Types

r8r ships with 24 built-in node types:

| Category | Nodes |
|----------|-------|
| **Core** | `http`, `transform`, `agent`, `subworkflow` |
| **Logic** | `if`, `switch`, `merge`, `filter`, `sort`, `limit` |
| **Data** | `set`, `aggregate`, `split`, `dedupe`, `variables` |
| **Integrations** | `email`, `slack`, `database`, `s3` |
| **Utility** | `wait`, `crypto`, `datetime`, `debug`, `summarize` |

### MCP Integration

r8r includes a built-in MCP (Model Context Protocol) server so AI agents can discover and execute workflows directly.

## License

r8r is licensed under **AGPL-3.0**. Free to use, modify, and distribute. If you run a modified version as a network service, you must make the source available.

**Commercial licensing** is available for organizations that cannot use AGPL.
