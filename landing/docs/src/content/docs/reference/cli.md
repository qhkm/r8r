---
title: CLI Reference
description: Complete reference for the r8r command-line interface.
---

## Overview

```bash
r8r <command> [options]
```

## Commands

### server

Start the API server with scheduler and web dashboard:

```bash
r8r server [--port 8080] [--no-ui]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--port`, `-p` | `8080` | HTTP port to listen on |
| `--no-ui` | false | Disable the web dashboard (API only) |

### dev

Watch a workflow file and validate on changes (hot reload):

```bash
r8r dev <file>
```

### workflows

Manage workflows and executions.

#### workflows list

```bash
r8r workflows list
```

#### workflows create

```bash
r8r workflows create <file.yaml>
```

Parse, validate, and store a workflow from a YAML file.

#### workflows run

```bash
r8r workflows run <name> [--input '{"key":"value"}'] [-p key=value] [--wait]
```

| Flag | Description |
|------|-------------|
| `--input`, `-i` | JSON input data |
| `--param`, `-p` | Parameter key=value (repeatable) |
| `--wait`, `-w` | Wait for completion (default: true) |

#### workflows logs

```bash
r8r workflows logs <name> [--limit 10]
```

Show recent execution logs for a workflow.

#### workflows search

```bash
r8r workflows search [options]
```

| Flag | Description |
|------|-------------|
| `--workflow` | Filter by workflow name |
| `--status` | Filter: pending, running, completed, failed, cancelled |
| `--trigger` | Filter by trigger type |
| `--search` | Full-text search in input/output/error |
| `--started-after` | RFC3339 timestamp lower bound |
| `--started-before` | RFC3339 timestamp upper bound |
| `--limit` | Page size (default: 20) |
| `--offset` | Pagination offset (default: 0) |

#### workflows replay

```bash
r8r workflows replay <execution-id> [--input '{"key":"value"}']
```

Re-run a previous execution, optionally with different input.

#### workflows resume

```bash
r8r workflows resume <execution-id>
```

Resume a failed execution from its last checkpoint. Completed nodes are skipped.

#### workflows history

```bash
r8r workflows history <name> [--limit 20]
```

Show version history with changelog entries.

#### workflows rollback

```bash
r8r workflows rollback <name> <version>
```

Roll back a workflow to a previous version.

#### workflows trace

```bash
r8r workflows trace <execution-id>
```

Show per-node execution trace with timing and errors.

#### workflows show

```bash
r8r workflows show <name>
```

Display workflow details: description, version, triggers, and nodes.

#### workflows delete

```bash
r8r workflows delete <name>
```

#### workflows validate

```bash
r8r workflows validate <file.yaml>
```

Validate a workflow file without creating it.

#### workflows export

```bash
r8r workflows export <name> [--output file.yaml]
```

Export a workflow to YAML. Outputs to stdout if no `--output` specified.

#### workflows export-all

```bash
r8r workflows export-all [--output ./workflows]
```

Export all workflows to a directory (default: `./workflows`).

#### workflows dag

```bash
r8r workflows dag <name> [--order]
```

Show the workflow dependency graph. Use `--order` to display topological execution order.

### credentials

Manage encrypted credential storage.

#### credentials set

```bash
r8r credentials set <service> [--key key] [--value value]
```

If `--value` is omitted, reads from stdin.

#### credentials list

```bash
r8r credentials list
```

Lists stored credentials with masked values.

#### credentials delete

```bash
r8r credentials delete <service>
```

### templates

Manage workflow templates.

#### templates list

```bash
r8r templates list [--by-category]
```

#### templates show

```bash
r8r templates show <name>
```

Shows template details including required variables.

#### templates use

```bash
r8r templates use <name> [--output file.yaml] [--var key=value] [--create]
```

| Flag | Description |
|------|-------------|
| `--output`, `-o` | Write to file (stdout if omitted) |
| `--var`, `-v` | Set template variable (repeatable) |
| `--create` | Also create the workflow in the database |

### db

Database maintenance.

#### db check

```bash
r8r db check
```

Run integrity checks, foreign key validation, and orphan detection.

### completions

```bash
r8r completions <shell>
```

Generate shell completions. Supported shells: `bash`, `zsh`, `fish`, `power-shell`, `elvish`.
