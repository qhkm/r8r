# Easy Install Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce r8r install friction from "clone + compile" to a one-liner across 6 target platforms.

**Architecture:** A GitHub Actions release workflow triggered by `v*` tags cross-compiles binaries for 6 targets, publishes them as GitHub Release assets with checksums, builds a multi-arch Docker image on GHCR, and an install script at `r8r.sh/install.sh` auto-detects OS/arch and downloads the right binary.

**Tech Stack:** GitHub Actions, `cross` (Rust cross-compilation), `docker buildx` (multi-arch images), POSIX shell (install script)

**Spec:** `docs/superpowers/specs/2026-03-13-easy-install-design.md`

---

## Chunk 1: Cross-Compilation Config + Cargo.toml

### Task 1: Create `Cross.toml`

**Files:**
- Create: `Cross.toml`

- [ ] **Step 1: Create `Cross.toml` with protobuf pre-build**

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

Write this to `Cross.toml` in the repo root.

- [ ] **Step 2: Commit**

```bash
git add Cross.toml
git commit -m "build: add Cross.toml for cross-compilation with protobuf support"
```

---

### Task 2: Add `exclude` to `Cargo.toml`

**Files:**
- Modify: `Cargo.toml:1-13` (inside `[package]` section)

- [ ] **Step 1: Add exclude list after `readme = "README.md"` (line 13)**

Add this block immediately after line 13 (`readme = "README.md"`):

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

- [ ] **Step 2: Verify the package compiles**

Run: `cargo check`
Expected: compiles successfully (exclude only affects `cargo publish` packaging)

- [ ] **Step 3: Verify exclude works**

Run: `cargo package --list 2>/dev/null | head -30`
Expected: output should NOT contain `ui/`, `docs/`, `.github/`, `Dockerfile`, etc. Should contain `src/`, `Cargo.toml`, `README.md`, `Cargo.lock`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "build: add crate exclude list to keep crates.io package small"
```

---

## Chunk 2: Dockerfile.release

### Task 3: Create `Dockerfile.release`

**Files:**
- Create: `Dockerfile.release`

- [ ] **Step 1: Create `Dockerfile.release`**

This is a thin runtime-only image used by CI. It does NOT compile Rust — it receives pre-built binaries as build context. Write to `Dockerfile.release`:

```dockerfile
# Dockerfile.release — thin runtime image for CI releases
# Pre-built binaries are copied in as build context (no Rust compilation)
#
# Usage (CI only):
#   docker buildx build -f Dockerfile.release --platform linux/amd64 .

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    gosu \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false -d /data r8r

COPY r8r /usr/local/bin/
COPY r8r-mcp /usr/local/bin/
COPY docker-entrypoint.sh /usr/local/bin/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh \
    /usr/local/bin/r8r \
    /usr/local/bin/r8r-mcp

ENV R8R_DB=/data/r8r.db
ENV RUST_LOG=r8r=info

EXPOSE 8080
VOLUME /data

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["r8r", "server", "--port", "8080"]
```

- [ ] **Step 2: Commit**

```bash
git add Dockerfile.release
git commit -m "build: add Dockerfile.release for CI multi-arch image builds"
```

---

## Chunk 3: Release Workflow

### Task 4: Create `.github/workflows/release.yml`

**Files:**
- Create: `.github/workflows/release.yml`

This is the largest single file. It has three jobs: `build` (matrix of 6 targets), `release` (collect artifacts + create GitHub Release), and `docker` (multi-arch GHCR image).

- [ ] **Step 1: Create the release workflow**

Write to `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - "v*"

permissions:
  contents: write
  packages: write

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: Build ${{ matrix.target }}
    runs-on: ${{ matrix.runner }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            runner: ubuntu-latest
            use_cross: false
          - target: aarch64-unknown-linux-gnu
            runner: ubuntu-latest
            use_cross: true
          - target: x86_64-unknown-linux-musl
            runner: ubuntu-latest
            use_cross: true
          - target: aarch64-unknown-linux-musl
            runner: ubuntu-latest
            use_cross: true
          - target: aarch64-apple-darwin
            runner: macos-latest
            use_cross: false
          - target: x86_64-apple-darwin
            runner: macos-13
            use_cross: false
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        run: |
          rustup toolchain install stable --profile minimal
          rustup target add ${{ matrix.target }}

      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-${{ matrix.target }}-cargo-release-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: ${{ runner.os }}-${{ matrix.target }}-cargo-release-

      - name: Install protoc (native Linux builds)
        if: "!matrix.use_cross && runner.os == 'Linux'"
        run: sudo apt-get update && sudo apt-get install -y protobuf-compiler

      - name: Install cross
        if: matrix.use_cross
        run: cargo install cross --version 0.2.5

      - name: Build release binary
        run: |
          if [ "${{ matrix.use_cross }}" = "true" ]; then
            cross build --release --target ${{ matrix.target }}
          else
            cargo build --release --target ${{ matrix.target }}
          fi

      - name: Package artifacts
        run: |
          VERSION="${GITHUB_REF_NAME}"
          ARCHIVE="r8r-${VERSION}-${{ matrix.target }}.tar.gz"
          cd target/${{ matrix.target }}/release
          tar czf "../../../${ARCHIVE}" r8r r8r-mcp
          cd ../../..
          echo "ARCHIVE=${ARCHIVE}" >> "$GITHUB_ENV"

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: r8r-${{ matrix.target }}
          path: ${{ env.ARCHIVE }}

  release:
    name: Create Release
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts
          merge-multiple: true

      - name: Generate checksums
        run: |
          VERSION="${GITHUB_REF_NAME}"
          cd artifacts
          sha256sum r8r-*.tar.gz > "r8r-${VERSION}-checksums.txt"
          cat "r8r-${VERSION}-checksums.txt"

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          generate_release_notes: true
          files: |
            artifacts/r8r-*.tar.gz
            artifacts/r8r-*-checksums.txt

  docker:
    name: Docker Image
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download Linux amd64 artifact
        uses: actions/download-artifact@v4
        with:
          name: r8r-x86_64-unknown-linux-gnu
          path: docker-amd64

      - name: Download Linux arm64 artifact
        uses: actions/download-artifact@v4
        with:
          name: r8r-aarch64-unknown-linux-gnu
          path: docker-arm64

      - name: Extract binaries
        run: |
          VERSION="${GITHUB_REF_NAME}"
          mkdir -p context-amd64 context-arm64

          tar xzf docker-amd64/r8r-${VERSION}-x86_64-unknown-linux-gnu.tar.gz -C context-amd64
          tar xzf docker-arm64/r8r-${VERSION}-aarch64-unknown-linux-gnu.tar.gz -C context-arm64

          cp docker-entrypoint.sh context-amd64/
          cp docker-entrypoint.sh context-arm64/

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Set up QEMU
        uses: docker/setup-qemu-action@v3

      - name: Login to GHCR
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Push amd64 image
        run: |
          docker buildx build \
            --platform linux/amd64 \
            -f Dockerfile.release \
            -t ghcr.io/${{ github.repository_owner }}/r8r:amd64 \
            --push \
            context-amd64

      - name: Push arm64 image
        run: |
          docker buildx build \
            --platform linux/arm64 \
            -f Dockerfile.release \
            -t ghcr.io/${{ github.repository_owner }}/r8r:arm64 \
            --push \
            context-arm64

      - name: Create and push multi-arch manifests
        run: |
          VERSION="${GITHUB_REF_NAME}"
          IMAGE="ghcr.io/${{ github.repository_owner }}/r8r"

          # Create versioned multi-arch manifest
          docker buildx imagetools create \
            -t "${IMAGE}:${VERSION}" \
            "${IMAGE}:amd64" \
            "${IMAGE}:arm64"

          # Create latest multi-arch manifest
          docker buildx imagetools create \
            -t "${IMAGE}:latest" \
            "${IMAGE}:amd64" \
            "${IMAGE}:arm64"

          echo "Published ${IMAGE}:${VERSION} and ${IMAGE}:latest"
```

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: no error (valid YAML)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow for cross-compiled binaries and Docker image"
```

---

## Chunk 4: Install Script

### Task 5: Create `install.sh`

**Files:**
- Create: `install.sh`

- [ ] **Step 1: Create the install script**

Write to `install.sh`:

```sh
#!/bin/sh
# r8r installer — https://r8r.sh
# Usage: curl -sSf https://r8r.sh/install.sh | sh
#
# Environment variables:
#   R8R_VERSION      — version to install (default: latest)
#   R8R_INSTALL_DIR  — install directory (default: ~/.r8r/bin)

set -e

REPO="qhkm/r8r"
DEFAULT_INSTALL_DIR="${HOME}/.r8r/bin"
INSTALL_DIR="${R8R_INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

main() {
    detect_platform
    resolve_version
    download_and_verify
    install_binaries
    print_success
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "${OS}" in
        Linux)
            case "${ARCH}" in
                x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
                aarch64) TARGET="aarch64-unknown-linux-musl" ;;
                arm64)   TARGET="aarch64-unknown-linux-musl" ;;
                *)       err "Unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        Darwin)
            case "${ARCH}" in
                x86_64)  TARGET="x86_64-apple-darwin" ;;
                arm64)   TARGET="aarch64-apple-darwin" ;;
                *)       err "Unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        *)
            err "Unsupported OS: ${OS}. r8r supports Linux and macOS."
            ;;
    esac

    say "Detected platform: ${OS} ${ARCH} -> ${TARGET}"
}

resolve_version() {
    if [ -n "${R8R_VERSION:-}" ]; then
        VERSION="${R8R_VERSION}"
        say "Using specified version: ${VERSION}"
        return
    fi

    say "Resolving latest version..."

    # Use redirect-based approach to avoid GitHub API rate limits
    LATEST_URL="https://github.com/${REPO}/releases/latest"

    if has curl; then
        VERSION="$(curl -sI "${LATEST_URL}" 2>/dev/null | grep -i '^location:' | sed 's/.*tag\///' | tr -d '\r\n')"
    elif has wget; then
        VERSION="$(wget --spider -S "${LATEST_URL}" 2>&1 | grep -i 'location:' | sed 's/.*tag\///' | tr -d '\r\n')"
    else
        err "curl or wget is required to download r8r"
    fi

    if [ -z "${VERSION}" ]; then
        err "Could not determine latest version. Set R8R_VERSION manually."
    fi

    say "Latest version: ${VERSION}"
}

download_and_verify() {
    ARCHIVE="r8r-${VERSION}-${TARGET}.tar.gz"
    CHECKSUMS="r8r-${VERSION}-checksums.txt"
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

    R8R_TMP="$(mktemp -d)"
    trap 'rm -rf "${R8R_TMP}"' EXIT

    say "Downloading ${ARCHIVE}..."
    download "${BASE_URL}/${ARCHIVE}" "${R8R_TMP}/${ARCHIVE}"

    say "Downloading checksums..."
    download "${BASE_URL}/${CHECKSUMS}" "${R8R_TMP}/${CHECKSUMS}"

    say "Verifying checksum..."
    verify_checksum "${R8R_TMP}/${ARCHIVE}" "${R8R_TMP}/${CHECKSUMS}"
}

install_binaries() {
    mkdir -p "${INSTALL_DIR}"

    say "Installing to ${INSTALL_DIR}..."
    tar xzf "${R8R_TMP}/${ARCHIVE}" -C "${INSTALL_DIR}"
    chmod +x "${INSTALL_DIR}/r8r" "${INSTALL_DIR}/r8r-mcp"
}

print_success() {
    say ""
    say "r8r ${VERSION} installed successfully!"
    say ""
    say "  r8r:     ${INSTALL_DIR}/r8r"
    say "  r8r-mcp: ${INSTALL_DIR}/r8r-mcp"
    say ""

    # Check if install dir is in PATH
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            say "Add r8r to your PATH by adding this to your shell profile:"
            say ""
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            say ""
            say "Then restart your shell or run:"
            say ""
            say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

# --- Utility functions ---

download() {
    URL="$1"
    DEST="$2"

    if has curl; then
        curl -sSfL "${URL}" -o "${DEST}"
    elif has wget; then
        wget -q "${URL}" -O "${DEST}"
    else
        err "curl or wget is required to download r8r"
    fi
}

verify_checksum() {
    FILE="$1"
    CHECKSUMS_FILE="$2"
    FILENAME="$(basename "${FILE}")"

    EXPECTED="$(grep "${FILENAME}" "${CHECKSUMS_FILE}" | awk '{print $1}')"
    if [ -z "${EXPECTED}" ]; then
        err "Checksum not found for ${FILENAME}"
    fi

    if has sha256sum; then
        ACTUAL="$(sha256sum "${FILE}" | awk '{print $1}')"
    elif has shasum; then
        ACTUAL="$(shasum -a 256 "${FILE}" | awk '{print $1}')"
    else
        err "sha256sum or shasum is required for checksum verification"
    fi

    if [ "${EXPECTED}" != "${ACTUAL}" ]; then
        err "Checksum verification failed!
  Expected: ${EXPECTED}
  Got:      ${ACTUAL}"
    fi

    say "Checksum verified."
}

has() {
    command -v "$1" > /dev/null 2>&1
}

say() {
    printf '%s\n' "$1"
}

err() {
    say "error: $1" >&2
    exit 1
}

main
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x install.sh`

- [ ] **Step 3: Test the script parses correctly (dry run)**

Run: `sh -n install.sh`
Expected: no output (valid POSIX shell syntax)

- [ ] **Step 4: Commit**

```bash
git add install.sh
git commit -m "feat: add install script for one-liner binary installation"
```

---

## Chunk 5: README Update

### Task 6: Update README Quick Start

**Files:**
- Modify: `README.md:45-71`

- [ ] **Step 1: Replace Quick Start section**

In `README.md`, find the `## Quick Start` section (lines 45-71, from `## Quick Start` through the closing triple-backtick fence). Replace the ENTIRE section with the following content. Note: this fixes the port from 3000 to 8080 (the actual default — see `src/main.rs:23`).

**Old text to find** (lines 45-71):

    ## Quick Start

    ```bash
    # Install
    git clone https://github.com/qhkm/r8r.git && cd r8r
    cargo build --release
    ...entire code block...
    ```

**New text to replace with:**

    ## Quick Start

    ```bash
    # Install (recommended)
    curl -sSf https://r8r.sh/install.sh | sh

    # Or use Docker
    docker run -d -p 8080:8080 -v r8r-data:/data ghcr.io/qhkm/r8r

    # Or via Cargo (requires Rust)
    cargo install r8r

    # Or build from source
    git clone https://github.com/qhkm/r8r.git && cd r8r
    cargo build --release
    ```

    Then create and run a workflow:

    ```bash
    # Create a workflow
    cat > hello.yaml << 'EOF'
    name: hello-world
    nodes:
      - id: greet
        type: transform
        config:
          expression: '"Hello, " + (input.name ?? "World") + "!"'
    EOF

    # Start the server
    r8r server --workflows . &

    # Execute it
    curl -X POST localhost:8080/api/workflows/hello-world/execute \
      -H "Content-Type: application/json" \
      -d '{"input": {"name": "Agent"}}'

    # Open the web UI
    # Dashboard: http://localhost:8080/
    # Guided workflow studio: http://localhost:8080/editor
    ```

- [ ] **Step 2: Verify no broken markdown**

Run: `head -80 README.md`
Expected: Quick Start section looks correct with proper markdown formatting

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: update Quick Start with install script, Docker, and cargo install options

Also fixes port from 3000 to 8080 to match actual default."
```

---

## Summary

| Task | File(s) | Description |
|------|---------|-------------|
| 1 | `Cross.toml` | Cross-compilation config with protobuf |
| 2 | `Cargo.toml` | Add crate exclude list |
| 3 | `Dockerfile.release` | Thin runtime image for CI |
| 4 | `.github/workflows/release.yml` | Full release workflow (build + release + docker) |
| 5 | `install.sh` | POSIX install script |
| 6 | `README.md` | Updated Quick Start + port fix |

**After all tasks:** Push to a branch, create PR, and test the release workflow by pushing a `v0.4.0` tag after merge.

**Manual steps (one-time, post-first-release):**
1. Set GHCR package `r8r` to public visibility in GitHub package settings
2. Configure `r8r.sh` to serve `install.sh` from the repo (or proxy to raw GitHub)
3. Publish to crates.io: `cargo publish` (manual, after verifying the release works)
