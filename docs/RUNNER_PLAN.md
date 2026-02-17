# r8r Runner Plan (Edge/Fleet Execution)

## Goal

Build a lightweight `r8r-runner` that executes workflows locally on edge devices while a separate control plane manages rollout, policy, and observability.

This document covers the runner side only (open runtime scope), not proprietary fleet-control features.

## Product Boundary

### In Scope

- Local workflow execution on constrained devices
- Pull-and-verify workflow bundles from a remote endpoint
- Offline-safe execution with buffered status/report uploads
- Policy enforcement on node types and network egress
- Deterministic execution traces compatible with existing r8r APIs/storage model

### Out of Scope

- Hosted control plane implementation (device registry, billing, enterprise tenancy)
- Chronicle-level event-sourced replay and long-horizon durability semantics
- Remote shell/remote arbitrary command execution from cloud control plane

## Runner Architecture (v1)

1. **Runner daemon**
   - New binary: `r8r-runner`
   - Runs as long-lived process/service on device
   - Executes workflows using existing engine/node runtime
2. **Bundle sync**
   - Poll remote manifest endpoint (`GET /runner/v1/manifest`)
   - Download workflow bundle only when version changes
   - Verify signature before activation
3. **Local state**
   - Persist active bundle version, last sync time, and pending outbound reports
   - Reuse SQLite for execution/report queues
4. **Execution loop**
   - Trigger locally (cron/event/webhook adapters)
   - Execute with local policy guardrails
   - Emit structured run reports/events for upload when connected
5. **Report uploader**
   - Background retry with exponential backoff
   - Bounded disk queue with drop policy + warning telemetry

## Phased Delivery

## Phase 1: Single-device runner MVP

### Deliverables

- `r8r-runner` binary with local config file
- Manifest poll + signed bundle download + activation
- Local cron/webhook triggers from active bundle
- Run report uploader with offline buffering

### Exit Criteria

- Device can run disconnected for 24h and sync backlog after reconnect
- Invalid signature is always rejected and logged
- Runner survives process restart with correct active bundle + queue state

## Phase 2: Policy and safety hardening

### Deliverables

- Node allowlist/denylist policy
- Egress domain allowlist for HTTP node
- Execution resource limits (timeout, memory/concurrency caps)
- Policy violation reporting

### Exit Criteria

- Disallowed nodes never execute
- Policy mismatch blocks activation before workflow starts
- Security events are observable through reports/metrics

## Phase 3: Rollout-aware protocol support

### Deliverables

- Ring/channel metadata support in manifest (runner reads assignment only)
- Health heartbeat payload with version and capability flags
- Rollback-to-last-known-good bundle on repeated startup/execution failure

### Exit Criteria

- Runner can downgrade automatically to previous valid bundle
- Heartbeats expose enough metadata for staged rollout decisions upstream

## Interface Contracts (Runner Side)

## Manifest response (minimum)

- `version`: monotonic release identifier
- `bundle_url`: download URL
- `bundle_sha256`: integrity hash
- `signature`: detached signature
- `policy`: node/egress/resource constraints
- `min_runner_version`: compatibility gate

## Run report payload (minimum)

- `device_id`
- `runner_version`
- `bundle_version`
- `execution_id`
- `workflow_id`
- `status`
- `started_at` / `finished_at`
- `error_code` / `error_message` (if failed)
- `metrics` (duration_ms, retry_count, node_count)

## Security Requirements

- Signature verification mandatory before activation
- Reject expired or downgraded manifests unless explicitly rollback-approved
- Credential material remains local; runner uploads only execution metadata by default
- TLS required for remote endpoints; pinning support planned for hardened deployments

## Testing Strategy

Minimum tests required for runner additions:

- Happy path: sync, activate, execute, upload report
- Failure path: bad signature, network outage, partial download, incompatible version
- Restart behavior: restore active bundle and queued reports
- Policy behavior: blocked node/egress/resource violation cases

Validation commands for Rust changes:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

## Milestone Backlog (Implementation Order)

1. Add `src/bin/r8r-runner.rs` skeleton + config loader
2. Implement manifest client and bundle storage layout
3. Add signature/hash verification module
4. Add activation + rollback state machine
5. Wire local trigger/execution loop
6. Add report queue/uploader
7. Add policy enforcement hooks
8. Add integration tests for offline/recovery scenarios

## Risks and Mitigations

- **Risk:** Queue growth on long offline periods
  - **Mitigation:** bounded queue + compression + operator alerts
- **Risk:** Partial bundle corruption
  - **Mitigation:** atomic download-to-temp + verify + atomic swap
- **Risk:** Policy drift between bundle and runner capability
  - **Mitigation:** capability handshake + `min_runner_version` gate

## Immediate Next Step

Open an implementation epic with these first three tasks:

1. `runner-core`: binary skeleton, config, lifecycle
2. `runner-sync`: manifest/bundle/download/verify/activate
3. `runner-reporting`: local queue + upload retry loop
