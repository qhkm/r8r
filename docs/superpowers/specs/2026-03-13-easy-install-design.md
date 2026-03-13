# Easy Install Experience for r8r

**Date:** 2026-03-13
**Status:** Approved
**Goal:** Reduce install friction from "clone + compile" to a one-liner for all target platforms.

---

## Problem

The current Quick Start requires `git clone && cargo build --release`, which demands a Rust toolchain. This filters out the vast majority of potential users. The install experience must match the simplicity of competing tools (e.g., `npx n8n`).

## Solution Overview

Four parallel install paths, ranked by priority:

| Method | Command | Audience |
|--------|---------|----------|
| Install script | `curl -sSf https://r8r.sh/install.sh \| sh` | Primary — all Unix users |
| Docker | `docker run -d -p 8080:8080 ghcr.io/qhkm/r8r` | Server deployments |
| Cargo | `cargo install r8r` | Rust developers |
| Source | `git clone && cargo build --release` | Contributors |

---

## 1. Release Workflow (`.github/workflows/release.yml`)

### Trigger

Tag push matching `v*` (e.g., `v0.4.0`). Consistent with existing tags `v0.2.0`, `v0.3.0`.

### Build Matrix — 6 Targets

| Target | Runner | Build Method |
|--------|--------|--------------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | Native `cargo build` |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` |
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `cross` |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` | `cross` |
| `aarch64-apple-darwin` | `macos-latest` | Native `cargo build` |
| `x86_64-apple-darwin` | `macos-13` | Native `cargo build` |

**Note on `macos-13`:** This is the last Intel-native macOS runner on GitHub Actions. If deprecated, fall back to `macos-latest` with an explicit `x86_64-apple-darwin` target (Rosetta cross-compile).

### Cross-Compilation: `Cross.toml`

The project depends on `opentelemetry-otlp` → `tonic` → `prost`, which requires `protoc` at build time. The `ring` crate also requires platform-specific C/assembly compilation. A `Cross.toml` file is needed to configure the cross containers:

```toml
[build.env]
passthrough = ["CARGO_TERM_COLOR"]

[target.aarch64-unknown-linux-gnu]
pre-build = ["apt-get update && apt-get install -y protobuf-compiler"]

[target.x86_64-unknown-linux-musl]
pre-build = ["apt-get update && apt-get install -y protobuf-compiler"]

[target.aarch64-unknown-linux-musl]
pre-build = ["apt-get update && apt-get install -y protobuf-compiler"]
```

If `ring` fails on a musl target, ensure `clang` and `perl` are available in the cross build environment (add to `pre-build`).

### Feature Flags

All release builds use default features only (`cargo build --release` / `cross build --release`). No `--all-features`. This avoids needing system libraries like `libpq` (postgres) or runtime deps like Redis.

### Artifacts

Each target produces a tarball containing both binaries:
```
r8r-v0.4.0-x86_64-unknown-linux-gnu.tar.gz
├── r8r
└── r8r-mcp
```

### Release Job

A final job (depends on all build jobs) that:
1. Downloads all 6 tarballs
2. Generates `r8r-v0.4.0-checksums.txt` (SHA256 for each tarball)
3. Creates a GitHub Release attached to the tag
4. Uploads all tarballs + checksums as release assets
5. Auto-generates release notes from commits since previous tag

### Docker Job

**Depends on the build matrix** (not parallel) — reuses pre-built Linux binaries to avoid slow QEMU emulation.

Strategy: instead of `docker buildx` with QEMU (which would compile Rust under emulation for 30-60+ minutes), use a thin Dockerfile that COPYs the pre-built binaries from the matrix jobs:

1. Download `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` tarballs from the build matrix
2. Build per-arch images using a minimal `Dockerfile.release` (no Rust compilation, just COPY binary + runtime deps)
3. Use `docker buildx` with `--platform linux/amd64,linux/arm64` to create a multi-arch manifest
4. Push to GHCR with two tags:
   - `ghcr.io/qhkm/r8r:v0.4.0` (immutable)
   - `ghcr.io/qhkm/r8r:latest` (mutable, points to newest)
5. Authenticate with `github.actor` + `secrets.GITHUB_TOKEN` (with `packages: write` permission), not separate GHCR secrets

**New file: `Dockerfile.release`** — a thin runtime-only image (no builder stage):

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates gosu \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false -d /data r8r
COPY r8r /usr/local/bin/
COPY r8r-mcp /usr/local/bin/
COPY docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
ENV R8R_DB=/data/r8r.db
ENV RUST_LOG=r8r=info
EXPOSE 8080
VOLUME /data
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["r8r", "server", "--port", "8080"]
```

**Manual step (one-time):** Set the GHCR package to public visibility in GitHub package settings.

---

## 2. Install Script (`install.sh`)

A POSIX-compatible shell script in the repo root, served at `https://r8r.sh/install.sh`.

### Behavior

1. Detect OS (`uname -s`) and architecture (`uname -m`)
2. Map to target triple:
   - `Darwin` + `arm64` → `aarch64-apple-darwin`
   - `Darwin` + `x86_64` → `x86_64-apple-darwin`
   - `Linux` + `x86_64` → `x86_64-unknown-linux-musl` (prefer static)
   - `Linux` + `aarch64` → `aarch64-unknown-linux-musl` (prefer static)
3. Resolve version:
   - Default: use redirect-based approach (`curl -sI https://github.com/qhkm/r8r/releases/latest | grep -i location`) to extract the latest tag without consuming GitHub API rate limits
   - Override: `R8R_VERSION=v0.4.0`
4. Download tarball from GitHub Releases (try `curl`, fall back to `wget` if `curl` is unavailable)
5. Download checksums file (`r8r-{version}-checksums.txt`)
6. Verify SHA256 checksum (using `sha256sum` or `shasum -a 256`)
7. Extract to `${R8R_INSTALL_DIR:-$HOME/.r8r/bin}`
8. Print PATH instructions (add `~/.r8r/bin` to shell profile)
9. Print confirmation: `r8r v0.4.0 installed successfully`

### Design Decisions

- **No sudo required** — installs to user home directory by default
- **Linux defaults to musl** — fully static, works on any distro (Alpine, Debian, CentOS, etc.)
- **Checksum verification mandatory** — script fails if checksum doesn't match
- **POSIX sh** — no bash-isms, works on minimal systems
- **No automatic PATH modification** — prints instructions only, user decides
- **Download tool fallback** — tries `curl` first, falls back to `wget`; fails with clear message if neither available
- **Rate-limit safe** — uses GitHub redirect (`/releases/latest` 302) instead of API endpoint to avoid 60 req/hr unauthenticated limit

### Usage

```bash
# Latest
curl -sSf https://r8r.sh/install.sh | sh

# Specific version
curl -sSf https://r8r.sh/install.sh | R8R_VERSION=v0.4.0 sh

# Custom directory
curl -sSf https://r8r.sh/install.sh | R8R_INSTALL_DIR=/usr/local/bin sh
```

---

## 3. Cargo.toml Prep for crates.io

### Changes

Add `exclude` to `[package]` to keep crate download small:

```toml
exclude = [
    ".github/",
    ".internal/",
    "assets/",
    "deploy/",
    "docs/",
    "examples/",
    "infra/",
    "landing/",
    "scripts/",
    "signatures/",
    "tests/",
    "ui/",
    "Dockerfile",
    "Dockerfile.release",
    "docker-entrypoint.sh",
    "docker-compose*.yml",
    "install.sh",
    "Cross.toml",
    "*.md",
    "!README.md",
]
```

### No Changes Needed

- `default = []` features is correct — basic install doesn't need sandbox or postgres
- Metadata (keywords, categories, license, repo, homepage) already present
- Both binaries (`r8r`, `r8r-mcp`) are published by default

### Publishing

Manual only: `cargo publish`. Not automated in CI — deliberate during beta.

---

## 4. Docker Image

### Dockerfile Changes

The existing `Dockerfile` remains for local development builds. A new `Dockerfile.release` (see Section 1, Docker Job) is added for CI release builds — it skips Rust compilation and copies pre-built binaries directly.

### What Changes

- **Built by release workflow** using pre-built binaries (no QEMU emulation)
- **Multi-arch** via `docker buildx` manifest (amd64 + arm64)
- **Public access** on GHCR (one-time manual setting)
- **Dual tagging** — version tag + `latest`
- **Auth** — uses `GITHUB_TOKEN` with `packages: write`, not separate GHCR credentials

### Quick Start

```bash
docker run -d -p 8080:8080 -v r8r-data:/data ghcr.io/qhkm/r8r
```

---

## 5. README Quick Start Update

Replace the current Quick Start section with:

```bash
# Option 1: Install script (recommended)
curl -sSf https://r8r.sh/install.sh | sh

# Option 2: Docker
docker run -d -p 8080:8080 -v r8r-data:/data ghcr.io/qhkm/r8r

# Option 3: Cargo (requires Rust toolchain)
cargo install r8r

# Option 4: Build from source
git clone https://github.com/qhkm/r8r.git && cd r8r
cargo build --release
```

Install script is the primary path. Build from source is last — for contributors.

**Note:** The current README uses port 3000 in examples but the actual default is 8080 (matching the Dockerfile). The updated Quick Start corrects this to 8080.

---

## Files to Create/Modify

| File | Action |
|------|--------|
| `.github/workflows/release.yml` | Create — release workflow |
| `install.sh` | Create — install script |
| `Cross.toml` | Create — cross-compilation config for protobuf |
| `Dockerfile.release` | Create — thin runtime image for CI releases |
| `Cargo.toml` | Modify — add `exclude` |
| `README.md` | Modify — update Quick Start |

---

## Out of Scope

- **Homebrew tap** — deferred, install script covers macOS
- **Windows binaries** — deferred, audience is Linux/macOS
- **Docker Hub** — using GHCR only
- **Automated crates.io publish** — manual during beta
- **r8r.sh hosting setup** — assumed ready, user confirmed
