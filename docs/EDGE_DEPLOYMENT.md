# Edge Deployment Guide

Deploy r8r on edge devices, IoT gateways, and resource-constrained hardware.

## Why r8r on Edge

r8r is built in Rust with an embedded SQLite database, making it exceptionally well-suited for edge deployment:

- **Tiny footprint** -- 15 MB binary (release, stripped, LTO), ~15 MB RAM idle
- **Fast cold start** -- ~50 ms to first workflow execution
- **No external dependencies** -- embedded SQLite, no database server needed
- **Offline-capable** -- executes workflows without internet connectivity
- **Cross-compiles to ARM** -- runs natively on Raspberry Pi, Jetson, industrial gateways
- **Local AI** -- connect to Ollama for on-device LLM inference

### Comparison with Alternatives

| | r8r | Node-RED | AWS Greengrass | Azure IoT Edge |
|---|---|---|---|---|
| Binary size | ~15 MB | ~200 MB (Node.js + deps) | ~200 MB + Lambda runtime | ~300 MB + container runtime |
| Idle RAM | ~15 MB | ~80-120 MB | ~100-200 MB | ~150-300 MB |
| Cold start | ~50 ms | ~2-5 s | ~5-15 s | ~10-30 s |
| Language | Rust (native) | JavaScript (interpreted) | Python/JS (interpreted) | C#/Python (interpreted) |
| External DB | None (embedded SQLite) | None (JSON files) | Required (cloud-connected) | Required (cloud-connected) |
| Offline mode | Full execution | Partial | Limited | Limited |
| Local LLM | Yes (via agent node + Ollama) | Plugin required | Not natively supported | Not natively supported |
| License | AGPL-3.0 (dual) | Apache 2.0 | Proprietary | Proprietary |

---

## Supported Platforms

| Platform | Architecture | Status | Notes |
|----------|-------------|--------|-------|
| Raspberry Pi 4/5 | aarch64 | Supported | 2 GB+ RAM recommended |
| NVIDIA Jetson (Nano/Orin) | aarch64 | Supported | Ideal for local LLM workflows |
| Intel NUC / x86 mini-PCs | x86_64 | Supported | Full feature set |
| Industrial gateways (ARM64) | aarch64 | Supported | Tested on Advantech, Kontron |
| Docker on ARM | aarch64 | Supported | Multi-arch images available |
| ARMv7 (older Pi, embedded) | armv7 | Planned | Community interest tracked |

> **Note:** "Supported" means the cross-compilation target is defined and tested manually. Pre-built ARM binaries via GitHub Releases and automated CI cross-compilation are planned but not yet available -- you will need to cross-compile yourself for now.

---

## Installation on Edge Devices

### Option A: Cross-compile from source (recommended)

Build on your development machine and copy the binary to the device.

```bash
# Install the cross-compilation toolchain
rustup target add aarch64-unknown-linux-gnu

# Using cross (recommended -- handles sysroot and linker automatically)
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu

# Binary is at:
# target/aarch64-unknown-linux-gnu/release/r8r

# Copy to device
scp target/aarch64-unknown-linux-gnu/release/r8r pi@raspberrypi:~/
```

### Option B: Build on device

The Raspberry Pi 5 (8 GB) has enough horsepower to compile r8r directly.

```bash
# On the Pi itself
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://github.com/qhkm/r8r.git && cd r8r
cargo build --release

# Takes 10-20 minutes on Pi 5, longer on Pi 4
# Binary at target/release/r8r
```

### Option C: Pre-built binaries (Planned)

Pre-built ARM64 and x86_64 binaries will be published to GitHub Releases. This is not yet available.

### Option D: Docker

```bash
# Multi-arch image (planned -- build locally for now)
docker run -d --name r8r \
  -v ~/workflows:/workflows \
  -v ~/r8r-data:/data \
  -p 3000:3000 \
  ghcr.io/qhkm/r8r:latest \
  server --workflows /workflows
```

To build the Docker image yourself for ARM64:

```bash
docker buildx build --platform linux/arm64 -t r8r:edge .
```

---

## Edge-Optimized Configuration

### r8r.toml for constrained devices

```toml
# Edge-optimized r8r configuration

[server]
host = "0.0.0.0"
port = 3000
# Disable web UI to save resources (flag planned)
# no_ui = true

[sandbox]
backend = "subprocess"  # No Docker needed on edge

[execution]
max_concurrent_workflows = 4
default_timeout_seconds = 60
```

### Environment variables for edge

```bash
# Data and workflow directories
export R8R_DATA_DIR=/opt/r8r/data
export R8R_WORKFLOWS_DIR=/opt/r8r/workflows

# For local LLM via Ollama
export R8R_AGENT_PROVIDER=ollama
export R8R_AGENT_ENDPOINT=http://localhost:11434
export R8R_AGENT_MODEL=llama3.2:3b  # Small model suitable for edge

# Optional: disable features to save resources
export R8R_SANDBOX_BACKEND=subprocess
```

### systemd service (Linux)

Create `/etc/systemd/system/r8r.service`:

```ini
[Unit]
Description=r8r workflow engine
After=network.target

[Service]
Type=simple
User=r8r
ExecStart=/usr/local/bin/r8r serve
Environment=R8R_DATA_DIR=/opt/r8r/data
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now r8r
```

---

## Local AI on Edge

r8r's `agent` node connects to any OpenAI-compatible API, including Ollama running locally on the device.

### Raspberry Pi 5 (8 GB)

```bash
# Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# Pull a small model
ollama pull llama3.2:1b    # 1.3 GB -- fits comfortably in 8 GB RAM
ollama pull phi3:mini       # 2.3 GB -- good reasoning for its size
ollama pull gemma2:2b       # 1.6 GB -- fast inference
```

Best models for Pi 5 (8 GB): `llama3.2:1b`, `gemma2:2b`, `phi3:mini` (3.8B).

### NVIDIA Jetson

```bash
# Install Ollama (or use TensorRT-LLM for GPU-accelerated inference)
curl -fsSL https://ollama.com/install.sh | sh

# Jetson Nano (4-8 GB): small models
ollama pull llama3.2:3b

# Jetson Orin (16-64 GB): can run larger models with GPU acceleration
ollama pull mistral:7b
ollama pull llama3.1:8b
```

Jetson devices provide CUDA-accelerated inference, making them ideal for real-time LLM workflows.

### Example: Edge Anomaly Detection Workflow

```yaml
name: edge-anomaly-detector
description: Detect anomalies in sensor data using local LLM

triggers:
  - type: cron
    schedule: "*/5 * * * *"  # Every 5 minutes

nodes:
  - id: read-sensors
    type: http
    config:
      url: "http://localhost:8081/api/sensors/latest"
      method: GET

  - id: check-anomaly
    type: agent
    config:
      provider: ollama
      model: llama3.2:3b
      prompt: |
        Analyze this sensor data and determine if there is an anomaly.
        Temperature: {{ nodes.read-sensors.output.temperature }} C
        Humidity: {{ nodes.read-sensors.output.humidity }} %
        Pressure: {{ nodes.read-sensors.output.pressure }} hPa

        Normal ranges: temp 18-28 C, humidity 30-70%, pressure 980-1040 hPa.

        Respond with JSON: {"anomaly": true/false, "severity": "low/medium/high", "reason": "..."}
      response_format: json
    depends_on: [read-sensors]

  - id: alert-if-anomaly
    type: http
    config:
      url: "http://monitoring-server:9090/api/alerts"
      method: POST
      body: |
        {
          "device_id": "{{ env.DEVICE_ID }}",
          "anomaly": {{ nodes.check-anomaly.output }},
          "timestamp": "{{ now() }}"
        }
    depends_on: [check-anomaly]
    condition: "nodes.check_anomaly.output.anomaly == true"
```

---

## Offline Operation

r8r is designed offline-first. Every component works without internet connectivity.

### What works offline

- **Workflow execution** -- all triggers (cron, webhook) and node types execute locally
- **Data storage** -- SQLite stores workflows, execution history, and results on disk
- **Credential encryption** -- AES-256-GCM with a local key, no cloud KMS required
- **Local LLM inference** -- Ollama runs entirely on-device
- **Sandbox execution** -- subprocess backend needs no network

### Sync when connected

When the device regains connectivity:

- Execution results buffered locally can be forwarded to a central server via HTTP nodes
- New workflow definitions can be pulled from a remote endpoint
- Logs and metrics can be shipped to your observability stack

### Planned: r8r-runner offline mode

The planned `r8r-runner` daemon (see [RUNNER_PLAN.md](RUNNER_PLAN.md)) will formalize offline operation:

- Pull signed workflow bundles when connected
- Execute offline with local triggers
- Buffer execution reports for upload on reconnect
- Automatic rollback to last-known-good bundle on failure

---

## Fleet Management (Planned)

> **Status: Planned / Coming Soon** -- This section describes the vision for fleet management. Implementation has not started. See [RUNNER_PLAN.md](RUNNER_PLAN.md) for the full design.

The fleet management model uses a central control plane with lightweight runners on each device:

```
                    ┌──────────────────────┐
                    │    Control Plane      │
                    │  (workflow registry,  │
                    │   rollout policy,     │
                    │   observability)      │
                    └──────────┬───────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
              ▼                ▼                ▼
     ┌────────────┐   ┌────────────┐   ┌────────────┐
     │ r8r-runner  │   │ r8r-runner  │   │ r8r-runner  │
     │ Device A    │   │ Device B    │   │ Device C    │
     └────────────┘   └────────────┘   └────────────┘
```

### Planned capabilities

- **Signed workflow bundles** -- integrity verification before activation (SHA-256 + signature)
- **Staged rollout** -- canary -> ring -> fleet deployment strategy
- **Health heartbeats** -- devices report version, status, and capability flags
- **Automatic rollback** -- revert to last-known-good bundle on repeated failure
- **Policy enforcement** -- node allowlist/denylist, egress domain restrictions, resource limits
- **Offline resilience** -- 24h+ disconnected operation with backlog sync on reconnect

---

## Edge Use Cases

### 1. Smart Factory Monitoring

Sensor data from PLCs and IoT devices flows through r8r for anomaly detection using a local LLM, triggering alerts to SCADA systems or operator dashboards.

**Flow:** Sensor HTTP endpoint -> transform -> agent node (local Ollama) -> conditional alert

### 2. Retail Edge Analytics

Point-of-sale and foot-traffic data processed on-premises for real-time inventory decisions and loss prevention, without sending data to the cloud.

**Flow:** POS webhook -> aggregate -> agent node (classify patterns) -> dashboard push

### 3. Agricultural IoT

Weather stations and soil moisture sensors feed into r8r for automated irrigation scheduling, with LLM-powered reasoning about weather forecasts.

**Flow:** Sensor poll (cron) -> transform -> agent node (irrigation decision) -> actuator HTTP call

### 4. Autonomous Robots

On-robot r8r instances process perception data and coordinate multi-step actions, using local LLMs for reasoning in environments with unreliable connectivity.

**Flow:** Perception API -> agent node (reasoning) -> action HTTP call -> report buffer

### 5. Building Automation

HVAC, lighting, and occupancy sensors optimized by r8r workflows running on a building gateway, keeping all data on-premises for privacy compliance.

**Flow:** BMS poll (cron) -> merge sensor data -> agent node (optimize) -> control API call

---

## Resource Requirements

| Deployment | Min RAM | Min Storage | CPU | GPU |
|-----------|---------|------------|-----|-----|
| r8r only (no AI) | 64 MB | 50 MB | 1 core | None |
| r8r + small LLM (1-3B params) | 2 GB | 2 GB | 2 cores | Optional |
| r8r + medium LLM (7-8B params) | 8 GB | 8 GB | 4 cores | Recommended |
| r8r + large LLM (13B+ params) | 16 GB+ | 16 GB+ | 4+ cores | Required (Jetson/GPU) |

Notes:
- r8r itself uses very little (64 MB RAM is generous). The LLM dominates resource usage.
- Storage requirements grow with execution history. Budget extra for SQLite WAL and workflow bundles.
- GPU is not required for r8r -- only for LLM inference performance. CPU inference works but is slower.

---

## Planned Edge Features

These features are on the roadmap but not yet implemented:

| Feature | Description | Priority |
|---------|-------------|----------|
| MQTT node | Pub/sub for IoT message brokers (Mosquitto, HiveMQ) | High |
| GPIO node | Direct Raspberry Pi hardware pin control | Medium |
| Modbus node | Industrial protocol support (RTU/TCP) | Medium |
| BLE node | Bluetooth Low Energy device communication | Medium |
| Pre-built ARM64 binaries | GitHub Releases for aarch64-unknown-linux-gnu | High |
| Pre-built ARMv7 binaries | GitHub Releases for armv7-unknown-linux-gnueabihf | Low |
| `r8r-runner` fleet daemon | Edge fleet management (see [RUNNER_PLAN.md](RUNNER_PLAN.md)) | High |
| WASM sandbox backend | Lightweight isolation without Docker or VMs | Medium |
| `--no-ui` flag | Disable web UI to save memory on headless devices | Low |
| OTA updates | Over-the-air binary updates for runner fleet | Low |

---

## Troubleshooting

### Cross-compilation fails with linker errors

Install the cross-compilation linker:

```bash
# Ubuntu/Debian
sudo apt install gcc-aarch64-linux-gnu

# Or use cross (handles everything in Docker)
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
```

### Binary too large

Strip the binary and enable LTO:

```bash
# In Cargo.toml [profile.release] (already configured)
# lto = true
# strip = true

# Manual strip after build
strip target/aarch64-unknown-linux-gnu/release/r8r
```

### Ollama out of memory on Pi

Use a smaller model or enable swap:

```bash
# Add 4 GB swap
sudo fallocate -l 4G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile

# Use a 1B parameter model instead
ollama pull llama3.2:1b
export R8R_AGENT_MODEL=llama3.2:1b
```

### r8r crashes on ARM with illegal instruction

Ensure you are compiling for the correct target. Raspberry Pi 4/5 is `aarch64-unknown-linux-gnu`, not `armv7-unknown-linux-gnueabihf`. Check with:

```bash
uname -m
# Should show: aarch64
```
