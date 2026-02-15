---
title: Installation
description: How to install r8r on your system.
---

## Build from Source

r8r requires Rust 1.70 or later.

```bash
git clone https://github.com/qhkm/r8r.git
cd r8r
cargo build --release
```

The binary will be at `./target/release/r8r`. Optionally copy it to your PATH:

```bash
cp target/release/r8r /usr/local/bin/
cp target/release/r8r-mcp /usr/local/bin/  # MCP server binary
```

## Docker

Pull or build the Docker image:

```bash
# Build locally
docker build -t r8r .

# Run with persistent data
docker run -d \
  --name r8r \
  -p 8080:8080 \
  -v r8r-data:/data \
  r8r:latest
```

Or use Docker Compose:

```bash
docker compose up -d
```

See [Deployment](/guides/deployment) for production Docker setup.

## Verify Installation

```bash
r8r --version
r8r --help
```

## Shell Completions

Generate shell completions for your shell:

```bash
# Bash
r8r completions bash > /etc/bash_completion.d/r8r

# Zsh
r8r completions zsh > ~/.zsh/completions/_r8r

# Fish
r8r completions fish > ~/.config/fish/completions/r8r.fish
```

## Next Steps

Head to the [Quick Start](/getting-started/quick-start) to create your first workflow.
