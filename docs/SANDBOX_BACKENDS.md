# Sandbox Backends

r8r's sandbox module uses a pluggable backend architecture. Operators choose the isolation level that fits their deployment.

## Available Backends

| Backend | Feature Flag | Isolation | Platform | Status |
|---------|-------------|-----------|----------|--------|
| Subprocess | `sandbox` | None | macOS, Linux, Windows | Stable |
| Docker | `sandbox-docker` | Container-level | macOS, Linux | Stable |
| Firecracker | `sandbox-firecracker` | Hardware VM (KVM) | Linux only | Stable |

## Example Workflow

The workflow YAML is the same regardless of backend — the backend is an infrastructure choice configured in `r8r.toml` or via `R8R_SANDBOX_BACKEND` env var.

```yaml
name: sandbox-python-example
nodes:
  - id: compute
    type: sandbox
    config:
      runtime: python3
      timeout_seconds: 10
      code: |
        import json
        numbers = [1, 2, 3, 4, 5]
        result = {
            "sum": sum(numbers),
            "count": len(numbers),
            "average": sum(numbers) / len(numbers)
        }
        print(json.dumps(result))

  - id: show-result
    type: debug
    config:
      message: "Result: {{ nodes.compute.output.output }}"
    depends_on: [compute]
```

Output (stdout parsed as JSON automatically):

```json
{
  "output": {
    "sum": 15,
    "count": 5,
    "average": 3.0
  },
  "exit_code": 0,
  "stdout": "{\"sum\": 15, \"count\": 5, \"average\": 3.0}\n",
  "stderr": ""
}
```

## Subprocess

Default backend. Spawns a child process with `kill_on_drop`. No real isolation — the process inherits the host's network, filesystem, and permissions. Use for local development only.

```toml
[sandbox]
backend = "subprocess"
```

## Docker

Container-based isolation via the Docker daemon (bollard SDK). Enforces memory limits, network isolation (`--network none`), read-only root filesystem, PID limits, and writable `/tmp` only.

```toml
[sandbox]
backend = "docker"

[sandbox.docker]
python_image = "python:3.12-slim"
node_image = "node:22-slim"
bash_image = "alpine:3.19"
```

## Firecracker

Hardware-level isolation via Firecracker microVMs. Each execution gets its own Linux kernel, memory space, and (optionally) network stack. Requires Linux with KVM (`/dev/kvm`), the `firecracker` binary, a vmlinux kernel, and a rootfs image with the guest agent.

```toml
[sandbox]
backend = "firecracker"

[sandbox.firecracker]
firecracker_bin = "firecracker"
kernel_path = "/opt/r8r/vmlinux"
rootfs_path = "/opt/r8r/rootfs.ext4"
vcpu_count = 1
memory_mb = 256
```

### Setup

1. Install Firecracker: https://github.com/firecracker-microvm/firecracker/releases
2. Download a pre-built kernel: `wget https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/x86_64/vmlinux-5.10.204 -O /opt/r8r/vmlinux`
3. Build a rootfs image (Alpine + python3/node/bash + guest agent)
4. Ensure `/dev/kvm` is accessible by the r8r process

### Guest Agent

The rootfs must include a guest agent that listens on vsock and executes code. A reference implementation is provided in `tools/firecracker-guest-agent/`.

---

## Future Backends (Not Yet Implemented)

The `SandboxBackend` trait is designed to support additional backends. Below are candidates evaluated for future implementation.

### WASM (wasmtime)

**Feature flag:** `sandbox-wasm`

**What:** Run code in a WebAssembly sandbox using wasmtime (a Rust-native WASM runtime). Sub-millisecond cold start, cross-platform, embeds directly in the r8r binary.

**Isolation:** Memory-safe sandbox. No filesystem or network access unless explicitly granted via WASI capabilities.

**Limitation:** Cannot run arbitrary Python/Node/Bash scripts. Only works with pre-compiled WASM modules, MicroPython (limited stdlib), or languages that compile to WASM (Rust, Go, C).

**Best for:** High-frequency, trusted tool execution where you control the code ahead of time.

**Rust crate:** `wasmtime` (mature, Bytecode Alliance, production-ready)

### Hyperlight (Microsoft)

**Feature flag:** `sandbox-hyperlight`

**What:** Sub-millisecond microVM execution. Pure Rust library from Microsoft (CNCF sandbox project, open-sourced Feb 2025). Creates minimal microVMs without a full guest OS.

**Cold start:** ~900 microseconds for complete function execution.

**Platform:** Linux (KVM) or Windows (Hyper-V). No macOS support.

**Isolation:** Full hardware-level VM isolation with near-zero overhead.

**Limitation:** Early-stage project. API may change. Best for WASM components or native ELF binaries, not arbitrary scripts.

**Best for:** Ultra-low-latency isolated execution where you need VM-level security but can't afford Firecracker's ~125ms cold start.

**Rust crate:** `hyperlight-host` (pure Rust, actively developed)

### libkrun

**Feature flag:** `sandbox-libkrun`

**What:** Lightweight virtualization library from Red Hat. Creates microVMs using the host's hypervisor (KVM on Linux, Apple Hypervisor Framework on macOS).

**Platform:** Linux (KVM) AND macOS (HVF) — the only microVM option that works on both.

**Isolation:** VM-level isolation, lighter than Firecracker.

**Best for:** Development environments where you want better-than-Docker isolation on macOS without requiring Docker Desktop.

**Rust crate:** `libkrun-sys` (FFI bindings)

### Landlock + seccomp (Linux Security Modules)

**Feature flag:** `sandbox-landlock`

**What:** OS-level sandboxing using Linux security modules. Landlock restricts file/network access; seccomp filters system calls. No VM overhead.

**Platform:** Linux only (kernel 5.13+ for Landlock).

**Isolation:** Weak compared to VMs/containers. Kernel vulnerabilities can escape the sandbox.

**Best for:** Trusted code that just needs resource restrictions. An enhanced subprocess backend, not a replacement for Docker/Firecracker.

**Rust crate:** `rust-landlock` (stable, maintained by Landlock upstream)
