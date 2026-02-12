# r8r Dockerfile
# Multi-stage build for minimal image size
#
# Build: docker build -t r8r .
# Run:   docker run -v r8r-data:/data r8r

# =============================================================================
# Stage 1: Build
# =============================================================================
FROM rust:1.85-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock* ./

# Create dummy src to build dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/r8r-mcp.rs && \
    echo "pub fn lib() {}" > src/lib.rs

# Build dependencies (cached layer)
RUN cargo build --release && rm -rf src

# Copy actual source
COPY src ./src

# Touch files to ensure rebuild
RUN touch src/main.rs src/lib.rs src/bin/r8r-mcp.rs

# Build release binaries
RUN cargo build --release --bin r8r --bin r8r-mcp

# =============================================================================
# Stage 2: Runtime (minimal)
# =============================================================================
FROM debian:bookworm-slim AS runtime

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false r8r

# Copy binaries from builder
COPY --from=builder /app/target/release/r8r /usr/local/bin/
COPY --from=builder /app/target/release/r8r-mcp /usr/local/bin/

# Create data directory
RUN mkdir -p /data && chown r8r:r8r /data

# Switch to non-root user
USER r8r

# Set environment
ENV R8R_DB=/data/r8r.db
ENV RUST_LOG=r8r=info

# Expose default port
EXPOSE 8080

# Data volume
VOLUME /data

# Default command
CMD ["r8r", "server", "--port", "8080"]
