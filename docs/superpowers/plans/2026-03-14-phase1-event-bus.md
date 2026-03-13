# Phase 1: WebSocket Event Bus & Approval Bridge — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish a bidirectional WebSocket event bus between r8r and ZeptoClaw, enabling real-time approval routing to user channels and cross-project health visibility.

**Architecture:** ZeptoClaw connects as a WebSocket client to r8r's new `/api/ws/events` endpoint. Both sides push JSON events on the same connection. r8r emits approval/execution events; ZeptoClaw emits approval decisions and workflow triggers. A replay buffer ensures at-least-once delivery on reconnect.

**Tech Stack:** Rust, Tokio, Axum (r8r server), tokio-tungstenite (ZeptoClaw client), serde_json, chrono, uuid

**Repos:**
- r8r: `~/ios/r8r` (Tasks 1-5)
- ZeptoClaw: `~/ios/zeptoclaw` (Tasks 6-9)

**Spec:** `docs/superpowers/specs/2026-03-14-phase1-event-bus-design.md`

---

## File Structure

### r8r (~/ios/r8r)

| File | Responsibility |
|------|---------------|
| New: `src/bridge/mod.rs` | Module root, re-exports, `BridgeState` struct (shared state for the bridge connection) |
| New: `src/bridge/events.rs` | `BridgeEvent` enum, `BridgeEventEnvelope` struct, serialization, event constructors |
| New: `src/bridge/buffer.rs` | `ReplayBuffer` — VecDeque-based ring buffer of unacked events with 5-min TTL and 256-event cap |
| New: `src/bridge/handlers.rs` | Handle incoming ZeptoClaw events: approval decisions, workflow triggers, health pings |
| New: `src/bridge/websocket.rs` | `/api/ws/events` WebSocket handler, single-client enforcement, send/receive loop |
| Modify: `src/lib.rs` | Add `pub mod bridge;` |
| Modify: `src/api/mod.rs` | Register `/api/ws/events` route, pass `BridgeState` to handler |
| Modify: `src/engine/executor.rs` | Emit bridge events on approval creation and execution completion/failure |
| Modify: `src/triggers/approval_timeout.rs` | Emit `r8r.approval.timeout` bridge event on expiry |
| Modify: `src/cli/diagnostics.rs` | Show bridge connection status |
| New: `tests/bridge_events_test.rs` | Event serde round-trips |
| New: `tests/bridge_buffer_test.rs` | Replay buffer unit tests |
| New: `tests/bridge_websocket_test.rs` | WebSocket integration tests |

### ZeptoClaw (~/ios/zeptoclaw)

| File | Responsibility |
|------|---------------|
| New: `src/r8r_bridge/mod.rs` | Module root, `R8rBridge` struct, WebSocket client lifecycle, reconnect loop |
| New: `src/r8r_bridge/events.rs` | Mirrored event types from r8r (same schema, independent definitions) |
| New: `src/r8r_bridge/approval.rs` | Approval routing to channels, response parsing, pending approval tracking |
| New: `src/r8r_bridge/health.rs` | Health ping loop, r8r status tracking |
| New: `src/r8r_bridge/dedup.rs` | `Deduplicator` — LRU set of recent event IDs (200 cap / 10 min TTL) |
| Modify: `src/config/mod.rs` | Add `R8rBridgeConfig` section |
| Modify: `src/lib.rs` | Add `pub mod r8r_bridge;` |
| Modify: `src/bus/mod.rs` | Route bridge events through message bus |
| New: `tests/r8r_bridge_test.rs` | Unit + integration tests |

---

## Chunk 1: r8r Bridge Events & Replay Buffer

### Task 1: Bridge Event Types (`src/bridge/events.rs`)

**Files:**
- Create: `src/bridge/events.rs`
- Create: `src/bridge/mod.rs`
- Modify: `src/lib.rs`
- Test: `tests/bridge_events_test.rs`

- [ ] **Step 1: Write failing test for event envelope serialization**

Create `tests/bridge_events_test.rs`:

```rust
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};
use serde_json;

#[test]
fn test_approval_requested_serializes_correctly() {
    let event = BridgeEventEnvelope::new(
        BridgeEvent::ApprovalRequested {
            approval_id: "appr_9d2e".to_string(),
            workflow: "deploy-prod".to_string(),
            execution_id: "exec_7f3a".to_string(),
            node_id: "approve-deploy".to_string(),
            message: "Deploy to prod?".to_string(),
            timeout_secs: 1800,
            requester: Some("ci-bot".to_string()),
            context: serde_json::json!({"env": "production"}),
        },
        Some("corr_001".to_string()),
    );

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["type"], "r8r.approval.requested");
    assert!(parsed["id"].as_str().unwrap().starts_with("evt_"));
    assert_eq!(parsed["data"]["approval_id"], "appr_9d2e");
    assert_eq!(parsed["data"]["timeout_secs"], 1800);
    assert_eq!(parsed["correlation_id"], "corr_001");
}

#[test]
fn test_approval_decision_deserializes_correctly() {
    let json = r#"{
        "id": "evt_ad_001",
        "type": "zeptoclaw.approval.decision",
        "timestamp": "2026-03-14T10:31:00Z",
        "data": {
            "approval_id": "appr_9d2e",
            "execution_id": "exec_7f3a",
            "node_id": "approve-deploy",
            "decision": "approved",
            "reason": "",
            "decided_by": "dr.nora",
            "channel": "telegram"
        },
        "correlation_id": "corr_001"
    }"#;

    let envelope: BridgeEventEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(envelope.event_type(), "zeptoclaw.approval.decision");

    match envelope.event {
        BridgeEvent::ApprovalDecision { decision, decided_by, .. } => {
            assert_eq!(decision, "approved");
            assert_eq!(decided_by, "dr.nora");
        }
        _ => panic!("Expected ApprovalDecision"),
    }
}

#[test]
fn test_ack_serializes_correctly() {
    let ack = r8r::bridge::events::Ack {
        event_id: "evt_abc123".to_string(),
    };
    let json = serde_json::to_string(&ack).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "ack");
    assert_eq!(parsed["event_id"], "evt_abc123");
}

#[test]
fn test_all_event_types_roundtrip() {
    let events = vec![
        BridgeEvent::ApprovalRequested {
            approval_id: "a".into(), workflow: "w".into(), execution_id: "e".into(),
            node_id: "n".into(), message: "m".into(), timeout_secs: 60,
            requester: None, context: serde_json::Value::Null,
        },
        BridgeEvent::ApprovalTimeout {
            workflow: "w".into(), execution_id: "e".into(),
            node_id: "n".into(), elapsed_secs: 60,
        },
        BridgeEvent::ExecutionCompleted {
            workflow: "w".into(), execution_id: "e".into(),
            status: "success".into(), duration_ms: 1000, node_count: 3,
        },
        BridgeEvent::ExecutionFailed {
            workflow: "w".into(), execution_id: "e".into(),
            status: "failed".into(), error_code: "NODE_TIMEOUT".into(),
            error_message: "timeout".into(), failed_node: "n".into(),
            duration_ms: 5000,
        },
        BridgeEvent::HealthStatus {
            version: "0.5.0".into(), uptime_secs: 3600,
            active_executions: 1, pending_approvals: 0, workflows_loaded: 5,
        },
        BridgeEvent::ApprovalDecision {
            approval_id: "a".into(), execution_id: "e".into(),
            node_id: "n".into(), decision: "approved".into(),
            reason: "".into(), decided_by: "user".into(), channel: "telegram".into(),
        },
        BridgeEvent::WorkflowTrigger {
            workflow: "w".into(), params: serde_json::json!({}),
            triggered_by: "user".into(), channel: "telegram".into(),
        },
        BridgeEvent::HealthPing,
    ];

    for event in events {
        let envelope = BridgeEventEnvelope::new(event, None);
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: BridgeEventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.id, parsed.id);
        assert_eq!(envelope.event_type(), parsed.event_type());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test bridge_events_test 2>&1 | head -20`
Expected: Compilation error — `bridge` module doesn't exist

- [ ] **Step 3: Create bridge module skeleton**

Create `src/bridge/mod.rs`:

```rust
pub mod events;
pub(crate) mod buffer;
pub(crate) mod handlers;
pub(crate) mod websocket;

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::bridge::buffer::ReplayBuffer;

/// Shared state for the bridge WebSocket connection.
///
/// Passed into the Axum handler via State. Holds the replay buffer
/// and a handle to the current client sender (if connected).
pub struct BridgeState {
    /// Replay buffer for unacknowledged outbound events.
    pub(crate) buffer: Arc<Mutex<ReplayBuffer>>,
    /// Sender half of the current WebSocket connection (None if no client).
    pub(crate) client_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<String>>>>,
    /// Timestamp of last health ping from ZeptoClaw.
    pub(crate) last_ping: Arc<Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    /// Bridge auth token (injected at construction from config/env).
    pub(crate) token: Option<String>,
    /// Storage reference for resolving approvals and triggering workflows.
    pub(crate) storage: Arc<Mutex<Option<Arc<dyn crate::storage::Storage>>>>,
}

impl BridgeState {
    pub fn new(token: Option<String>) -> Self {
        // Read token from param, fall back to env vars
        let resolved_token = token
            .or_else(|| std::env::var("R8R_BRIDGE_TOKEN").ok())
            .or_else(|| std::env::var("R8R_API_KEY").ok());

        Self {
            buffer: Arc::new(Mutex::new(ReplayBuffer::new(256, 300))),
            client_tx: Arc::new(Mutex::new(None)),
            last_ping: Arc::new(Mutex::new(None)),
            token: resolved_token,
            storage: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_storage(self, storage: Arc<dyn crate::storage::Storage>) -> Self {
        *self.storage.blocking_lock() = Some(storage);
        self
    }

    /// Returns true if a bridge client is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.client_tx.lock().await.is_some()
    }

    /// Returns the last health ping timestamp, if any.
    pub async fn last_ping_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_ping.lock().await
    }

    /// Send an event to the connected bridge client.
    /// Always buffers the event (removed on ack) for at-least-once delivery.
    /// Returns false if no client is connected or send fails.
    pub async fn send_event(&self, envelope: &events::BridgeEventEnvelope) -> bool {
        let json = match serde_json::to_string(envelope) {
            Ok(j) => j,
            Err(_) => return false,
        };

        // Always add to replay buffer first (removed when acked)
        {
            let mut buf = self.buffer.lock().await;
            buf.push(envelope.clone());
        }

        let tx_guard = self.client_tx.lock().await;
        if let Some(tx) = tx_guard.as_ref() {
            // Use try_send to avoid blocking. If buffer full, log and drop.
            match tx.try_send(json) {
                Ok(()) => true,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!("Bridge: send buffer full (256), dropping event {}", envelope.id);
                    false
                }
                Err(_) => false, // Channel closed
            }
        } else {
            false // No client connected — event is in replay buffer
        }
    }
}
```

- [ ] **Step 4: Implement event types**

Create `src/bridge/events.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Envelope wrapping every event on the bridge WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEventEnvelope {
    /// Unique event ID, prefixed "evt_".
    pub id: String,
    /// Dot-namespaced event type string (e.g. "r8r.approval.requested").
    #[serde(rename = "type")]
    pub event_type_str: String,
    /// ISO 8601 timestamp.
    pub timestamp: DateTime<Utc>,
    /// Event-specific payload.
    pub data: Value,
    /// Optional correlation ID linking related events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,

    /// Parsed event (skip serialization — reconstructed from type + data).
    #[serde(skip)]
    pub event: BridgeEvent,
}

impl BridgeEventEnvelope {
    pub fn new(event: BridgeEvent, correlation_id: Option<String>) -> Self {
        let event_type_str = event.event_type().to_string();
        let data = serde_json::to_value(&event).unwrap_or(Value::Null);
        Self {
            id: format!("evt_{}", Uuid::new_v4().as_simple()),
            event_type_str,
            timestamp: Utc::now(),
            data,
            correlation_id,
            event,
        }
    }

    pub fn event_type(&self) -> &str {
        &self.event_type_str
    }
}

/// All bridge event variants (both directions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BridgeEvent {
    // r8r -> ZeptoClaw
    ApprovalRequested {
        approval_id: String,
        workflow: String,
        execution_id: String,
        node_id: String,
        message: String,
        timeout_secs: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        requester: Option<String>,
        #[serde(default)]
        context: Value,
    },
    ApprovalTimeout {
        workflow: String,
        execution_id: String,
        node_id: String,
        elapsed_secs: u64,
    },
    ExecutionCompleted {
        workflow: String,
        execution_id: String,
        status: String,
        duration_ms: i64,
        node_count: usize,
    },
    ExecutionFailed {
        workflow: String,
        execution_id: String,
        status: String,
        error_code: String,
        error_message: String,
        failed_node: String,
        duration_ms: i64,
    },
    HealthStatus {
        version: String,
        uptime_secs: u64,
        active_executions: usize,
        pending_approvals: usize,
        workflows_loaded: usize,
    },

    // ZeptoClaw -> r8r
    ApprovalDecision {
        approval_id: String,
        execution_id: String,
        node_id: String,
        decision: String,
        reason: String,
        decided_by: String,
        channel: String,
    },
    WorkflowTrigger {
        workflow: String,
        params: Value,
        triggered_by: String,
        channel: String,
    },
    HealthPing,
}

impl BridgeEvent {
    /// Returns the dot-namespaced event type string.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ApprovalRequested { .. } => "r8r.approval.requested",
            Self::ApprovalTimeout { .. } => "r8r.approval.timeout",
            Self::ExecutionCompleted { .. } => "r8r.execution.completed",
            Self::ExecutionFailed { .. } => "r8r.execution.failed",
            Self::HealthStatus { .. } => "r8r.health.status",
            Self::ApprovalDecision { .. } => "zeptoclaw.approval.decision",
            Self::WorkflowTrigger { .. } => "zeptoclaw.workflow.trigger",
            Self::HealthPing => "zeptoclaw.health.ping",
        }
    }

    /// Parse a BridgeEvent from its type string and data value.
    pub fn from_type_and_data(event_type: &str, data: &Value) -> Option<Self> {
        match event_type {
            "r8r.approval.requested" => serde_json::from_value(data.clone()).ok(),
            "r8r.approval.timeout" => serde_json::from_value(data.clone()).ok(),
            "r8r.execution.completed" => serde_json::from_value(data.clone()).ok(),
            "r8r.execution.failed" => serde_json::from_value(data.clone()).ok(),
            "r8r.health.status" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.approval.decision" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.workflow.trigger" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.health.ping" => Some(BridgeEvent::HealthPing),
            _ => None,
        }
    }
}

/// Acknowledgment message sent by the receiver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ack {
    #[serde(rename = "type")]
    pub ack_type: String,
    pub event_id: String,
}

impl Ack {
    pub fn new(event_id: String) -> Self {
        Self {
            ack_type: "ack".to_string(),
            event_id,
        }
    }
}

// Fix: the Ack struct has a fixed "type" field, update serialization
// Actually the #[serde(rename)] handles it. But we need the test to
// reference the struct correctly. Let's keep it simple.
```

**Important serde design note:** The `BridgeEvent` enum must NOT use `#[serde(untagged)]` — several variants share overlapping fields (`workflow`, `execution_id`, `status`) which causes serde to silently deserialize into the wrong variant. Instead:

- Each variant's inner fields are defined as a separate data struct (e.g., `ApprovalRequestedData`, `ExecutionCompletedData`)
- `BridgeEvent::event_type()` returns the type string for serialization into the envelope's `type` field
- `BridgeEvent::from_type_and_data()` dispatches by type string and deserializes `data` into the specific struct (not the enum), then wraps in the variant
- The enum is only ever serialized via `serde_json::to_value(&inner_struct)`, never as a tagged/untagged enum

Example fix for `from_type_and_data`:
```rust
pub fn from_type_and_data(event_type: &str, data: &Value) -> Option<Self> {
    match event_type {
        "r8r.approval.requested" => {
            // Deserialize into the specific struct, not the enum
            let d: ApprovalRequestedData = serde_json::from_value(data.clone()).ok()?;
            Some(Self::ApprovalRequested(d))
        }
        // ... same pattern for each variant
    }
}
```

This avoids the serde untagged ambiguity entirely. The envelope's `type` field drives dispatch, not serde.

- [ ] **Step 5: Add module to lib.rs**

In `src/lib.rs`, add after `pub mod api;`:

```rust
pub mod bridge;
```

- [ ] **Step 6: Create placeholder files for other bridge submodules**

Create `src/bridge/buffer.rs` (placeholder — implemented in Task 2):
```rust
use crate::bridge::events::BridgeEventEnvelope;

pub struct ReplayBuffer {
    // Implemented in Task 2
    events: Vec<(BridgeEventEnvelope, std::time::Instant)>,
    max_events: usize,
    ttl_secs: u64,
}

impl ReplayBuffer {
    pub fn new(max_events: usize, ttl_secs: u64) -> Self {
        Self { events: Vec::new(), max_events, ttl_secs }
    }

    pub fn push(&mut self, _event: BridgeEventEnvelope) {
        // Task 2
    }
}
```

Create `src/bridge/handlers.rs` (placeholder):
```rust
// Implemented in Task 3
```

Create `src/bridge/websocket.rs` (placeholder):
```rust
// Implemented in Task 3
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --test bridge_events_test`
Expected: All 4 tests pass

- [ ] **Step 8: Commit**

```bash
git add src/bridge/ src/lib.rs tests/bridge_events_test.rs
git commit -m "feat(bridge): add event types and envelope for ZeptoClaw bridge"
```

---

### Task 2: Replay Buffer (`src/bridge/buffer.rs`)

**Files:**
- Modify: `src/bridge/buffer.rs`
- Test: `tests/bridge_buffer_test.rs`

- [ ] **Step 1: Write failing tests for replay buffer**

Create `tests/bridge_buffer_test.rs`:

```rust
use r8r::bridge::buffer::ReplayBuffer;
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};

fn make_event(id_suffix: &str) -> BridgeEventEnvelope {
    let mut env = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
    env.id = format!("evt_{}", id_suffix);
    env
}

#[test]
fn test_push_and_drain_unacked() {
    let mut buf = ReplayBuffer::new(100, 300);
    buf.push(make_event("a"));
    buf.push(make_event("b"));

    let events = buf.unacked_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "evt_a");
    assert_eq!(events[1].id, "evt_b");
}

#[test]
fn test_ack_removes_event() {
    let mut buf = ReplayBuffer::new(100, 300);
    buf.push(make_event("a"));
    buf.push(make_event("b"));
    buf.push(make_event("c"));

    buf.ack("evt_b");

    let events = buf.unacked_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "evt_a");
    assert_eq!(events[1].id, "evt_c");
}

#[test]
fn test_max_events_cap() {
    let mut buf = ReplayBuffer::new(3, 300);
    for i in 0..5 {
        buf.push(make_event(&i.to_string()));
    }

    let events = buf.unacked_events();
    // Oldest events dropped when cap exceeded
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].id, "evt_2");
    assert_eq!(events[1].id, "evt_3");
    assert_eq!(events[2].id, "evt_4");
}

#[test]
fn test_expired_events_pruned() {
    // TTL of 0 seconds means everything expires immediately
    let mut buf = ReplayBuffer::new(100, 0);
    buf.push(make_event("a"));

    // Sleep briefly to ensure expiry
    std::thread::sleep(std::time::Duration::from_millis(10));
    buf.prune_expired();

    let events = buf.unacked_events();
    assert_eq!(events.len(), 0);
}

#[test]
fn test_ack_nonexistent_is_noop() {
    let mut buf = ReplayBuffer::new(100, 300);
    buf.push(make_event("a"));
    buf.ack("evt_nonexistent"); // Should not panic
    assert_eq!(buf.unacked_events().len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test bridge_buffer_test 2>&1 | head -20`
Expected: Compilation error — `unacked_events`, `ack`, `prune_expired` don't exist

- [ ] **Step 3: Implement replay buffer**

Replace `src/bridge/buffer.rs`:

```rust
use std::collections::VecDeque;
use std::time::Instant;

use crate::bridge::events::BridgeEventEnvelope;

/// Ring buffer of unacknowledged outbound events (VecDeque for O(1) front removal).
///
/// Events are stored with their insertion timestamp. On reconnect,
/// r8r replays all events that haven't been acked and haven't expired.
/// Capacity is 256 per the spec's backpressure requirement.
pub struct ReplayBuffer {
    events: VecDeque<(BridgeEventEnvelope, Instant)>,
    max_events: usize,
    ttl_secs: u64,
}

impl ReplayBuffer {
    pub fn new(max_events: usize, ttl_secs: u64) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events),
            max_events,
            ttl_secs,
        }
    }

    /// Add an event to the buffer. Drops oldest if at capacity and logs a warning.
    pub fn push(&mut self, event: BridgeEventEnvelope) {
        if self.events.len() >= self.max_events {
            if let Some((dropped, _)) = self.events.pop_front() {
                tracing::warn!("Bridge: replay buffer full ({}), dropping event {}", self.max_events, dropped.id);
            }
        }
        self.events.push_back((event, Instant::now()));
    }

    /// Mark an event as acknowledged and remove it.
    pub fn ack(&mut self, event_id: &str) {
        self.events.retain(|(e, _)| e.id != event_id);
    }

    /// Return all unacknowledged, non-expired events (oldest first).
    pub fn unacked_events(&self) -> Vec<BridgeEventEnvelope> {
        let cutoff = Instant::now() - std::time::Duration::from_secs(self.ttl_secs);
        self.events
            .iter()
            .filter(|(_, ts)| *ts >= cutoff)
            .map(|(e, _)| e.clone())
            .collect()
    }

    /// Remove events older than TTL.
    pub fn prune_expired(&mut self) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(self.ttl_secs);
        while let Some((_, ts)) = self.events.front() {
            if *ts < cutoff {
                self.events.pop_front();
            } else {
                break; // VecDeque is ordered by insertion time
            }
        }
    }

    /// Number of events currently buffered.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test bridge_buffer_test`
Expected: All 5 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/bridge/buffer.rs tests/bridge_buffer_test.rs
git commit -m "feat(bridge): implement replay buffer with TTL and ack tracking"
```

---

## Chunk 2: r8r WebSocket Endpoint & Handlers

### Task 3: WebSocket Endpoint & Event Handlers

**Files:**
- Modify: `src/bridge/websocket.rs`
- Modify: `src/bridge/handlers.rs`
- Modify: `src/api/mod.rs`
- Test: `tests/bridge_websocket_test.rs`

- [ ] **Step 1: Write failing integration test for WebSocket connection**

Create `tests/bridge_websocket_test.rs`:

```rust
use axum::Router;
use r8r::api::AppState;
use r8r::bridge::BridgeState;
use r8r::bridge::events::{BridgeEventEnvelope, BridgeEvent, Ack};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};

/// Helper: start a test server and return its address.
/// Token is injected via BridgeState::new(Some(token)), NOT env vars.
async fn start_test_server(bridge: Arc<BridgeState>) -> String {
    // Minimal AppState for testing — uses in-memory SQLite
    let storage = r8r::storage::sqlite::SqliteStorage::new_in_memory()
        .await
        .unwrap();
    let storage = Arc::new(storage);
    let registry = Arc::new(r8r::nodes::registry::NodeRegistry::new());

    let app_state = AppState {
        storage: storage.clone(),
        registry,
        monitor: None,
        shutdown: Arc::new(r8r::shutdown::ShutdownCoordinator::new()),
        pause_registry: r8r::engine::executor::PauseRegistry::new(),
        event_backend: None,
    };

    let app = r8r::api::create_api_routes()
        .with_state(app_state)
        .merge(r8r::bridge::websocket::create_bridge_routes(bridge));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("ws://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn test_bridge_connection_and_health_ping() {
    // Token injected via constructor — no env var race conditions
    let bridge = Arc::new(BridgeState::new(Some("test-token-123".to_string())));
    let base_url = start_test_server(bridge.clone()).await;

    // Connect with valid token
    let url = format!("{}/api/ws/events", base_url);
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", "Bearer test-token-123")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "localhost")
        .body(())
        .unwrap();

    let (mut ws, _) = connect_async(request).await.unwrap();

    // Send health ping
    let ping = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
    ws.send(Message::Text(serde_json::to_string(&ping).unwrap())).await.unwrap();

    // Should receive health status response
    if let Some(Ok(Message::Text(text))) = ws.next().await {
        let response: BridgeEventEnvelope = serde_json::from_str(&text).unwrap();
        assert_eq!(response.event_type(), "r8r.health.status");
    } else {
        panic!("Expected health status response");
    }

    assert!(bridge.is_connected().await);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn test_bridge_rejects_invalid_token() {
    let bridge = Arc::new(BridgeState::new(Some("correct-token".to_string())));
    let base_url = start_test_server(bridge.clone()).await;

    let url = format!("{}/api/ws/events", base_url);
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", "Bearer wrong-token")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "localhost")
        .body(())
        .unwrap();

    // Connection should be rejected
    let result = connect_async(request).await;
    assert!(result.is_err());

    assert!(!bridge.is_connected().await);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test bridge_websocket_test 2>&1 | head -20`
Expected: Compilation error — `create_bridge_routes` doesn't exist

- [ ] **Step 3: Implement WebSocket endpoint**

Replace `src/bridge/websocket.rs`:

```rust
use std::sync::Arc;

use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn, error};

use crate::bridge::BridgeState;
use crate::bridge::events::{Ack, BridgeEventEnvelope};
use crate::bridge::handlers;

/// Create the bridge WebSocket route.
/// This is merged into the main Axum router separately from AppState routes
/// because it uses BridgeState.
pub fn create_bridge_routes(bridge: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/api/ws/events", axum::routing::get(ws_handler))
        .with_state(bridge)
}

/// WebSocket upgrade handler. Validates bridge token before upgrading.
/// Token is checked via constant-time comparison to prevent timing attacks.
async fn ws_handler(
    State(bridge): State<Arc<BridgeState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Validate bridge token
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let expected = bridge.token.as_deref()  // Injected at construction, not env::var
        .or_else(|| None);  // No fallback in handler — configured at startup

    match (token, expected) {
        (Some(provided), Some(expected)) => {
            // Constant-time comparison (matches existing monitor pattern)
            let provided_bytes = provided.as_bytes();
            let expected_bytes = expected.as_bytes();
            let matches = provided_bytes.len() == expected_bytes.len()
                && provided_bytes
                    .iter()
                    .zip(expected_bytes.iter())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0;
            if !matches {
                warn!("Bridge connection rejected: invalid token");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
        (_, None) => {
            // No token configured — allow connection (dev mode)
            warn!("Bridge connection accepted without token (no R8R_BRIDGE_TOKEN set)");
        }
        (None, Some(_)) => {
            warn!("Bridge connection rejected: no token provided");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_bridge_socket(socket, bridge))
        .into_response()
}

async fn handle_bridge_socket(socket: ws::WebSocket, bridge: Arc<BridgeState>) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Replace any existing client (single-client enforcement)
    // Send close frame 4002 to old client before replacing
    {
        let mut existing = bridge.client_tx.lock().await;
        if let Some(old_tx) = existing.take() {
            warn!("Replacing existing bridge client — sending close 4002 to old client");
            // Send a sentinel that the sender task interprets as "close with 4002"
            let _ = old_tx.send("__CLOSE_4002__".to_string()).await;
            // Small delay to let the close frame send
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let (tx, mut rx) = mpsc::channel::<String>(256);
        *existing = Some(tx);

        // Spawn sender task: reads from channel, writes to WebSocket
        // Handles __CLOSE_4002__ sentinel for single-client replacement
        let sender_bridge = bridge.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if msg == "__CLOSE_4002__" {
                    // Send close frame with code 4002 (replaced by new connection)
                    let _ = ws_sender.send(ws::Message::Close(Some(ws::CloseFrame {
                        code: 4002,
                        reason: "Replaced by new connection".into(),
                    }))).await;
                    break;
                }
                if msg == "__WS_PING__" {
                    // WebSocket-level ping for dead connection detection (30s interval)
                    let _ = ws_sender.send(ws::Message::Ping(vec![])).await;
                    continue;
                }
                if ws_sender.send(ws::Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            // Connection closed — clean up
            let mut client = sender_bridge.client_tx.lock().await;
            *client = None;
            info!("Bridge client sender loop ended");
        });

        // Spawn WebSocket-level ping task (30s interval per spec)
        // Detects dead TCP connections independent of application-level health pings
        let ping_bridge = bridge.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if !ping_bridge.is_connected().await {
                    break;
                }
                // Axum's WebSocket handles ping/pong at the protocol level
                // Send an empty ping frame
                let tx = ping_bridge.client_tx.lock().await;
                if let Some(tx) = tx.as_ref() {
                    if tx.try_send("__WS_PING__".to_string()).is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });
    }

    // Replay unacked events to the newly connected client
    {
        let buf = bridge.buffer.lock().await;
        let replay = buf.unacked_events();
        if !replay.is_empty() {
            info!("Replaying {} unacked events to bridge client", replay.len());
            for event in replay {
                bridge.send_event(&event).await;
            }
        }
    }

    info!("Bridge client connected");

    // Receive loop: read messages from the bridge client
    while let Some(Ok(msg)) = ws_receiver.next().await {
        match msg {
            ws::Message::Text(text) => {
                let text_str: &str = text.as_ref();
                // Try parsing as Ack first
                if let Ok(ack) = serde_json::from_str::<Ack>(text_str) {
                    if ack.ack_type == "ack" {
                        let mut buf = bridge.buffer.lock().await;
                        buf.ack(&ack.event_id);
                        continue;
                    }
                }

                // Try parsing as event envelope
                match serde_json::from_str::<serde_json::Value>(text_str) {
                    Ok(value) => {
                        let event_type = value.get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let data = value.get("data").cloned().unwrap_or(serde_json::Value::Null);

                        handlers::handle_incoming_event(
                            event_type, &data, &bridge,
                        ).await;
                    }
                    Err(e) => {
                        warn!("Bridge: malformed message: {}", e);
                    }
                }
            }
            ws::Message::Close(_) => {
                info!("Bridge client sent close frame");
                break;
            }
            ws::Message::Ping(data) => {
                // Axum handles pong automatically for ws::Message::Ping
                let _ = data;
            }
            _ => {}
        }
    }

    // Clean up on disconnect
    let mut client = bridge.client_tx.lock().await;
    *client = None;
    info!("Bridge client disconnected");
}
```

- [ ] **Step 4: Implement event handlers**

Replace `src/bridge/handlers.rs`:

```rust
use serde_json::Value;
use tracing::{info, warn};

use crate::bridge::BridgeState;
use crate::bridge::events::{BridgeEvent, BridgeEventEnvelope};

/// Dispatch an incoming event from ZeptoClaw.
pub async fn handle_incoming_event(
    event_type: &str,
    data: &Value,
    bridge: &BridgeState,
) {
    match event_type {
        "zeptoclaw.health.ping" => handle_health_ping(bridge).await,

        "zeptoclaw.approval.decision" => {
            if let Some(event) = BridgeEvent::from_type_and_data(event_type, data) {
                if let BridgeEvent::ApprovalDecision {
                    approval_id,
                    decision,
                    reason,
                    decided_by,
                    ..
                } = event
                {
                    handle_approval_decision(
                        &approval_id, &decision, &reason, &decided_by,
                    ).await;
                }
            } else {
                warn!("Bridge: failed to parse approval decision: {:?}", data);
            }
        }

        "zeptoclaw.workflow.trigger" => {
            if let Some(event) = BridgeEvent::from_type_and_data(event_type, data) {
                if let BridgeEvent::WorkflowTrigger {
                    workflow,
                    params,
                    triggered_by,
                    channel,
                } = event
                {
                    handle_workflow_trigger(
                        &workflow, &params, &triggered_by, &channel,
                    ).await;
                }
            } else {
                warn!("Bridge: failed to parse workflow trigger: {:?}", data);
            }
        }

        other => {
            warn!("Bridge: unknown event type: {}", other);
        }
    }
}

async fn handle_health_ping(bridge: &BridgeState) {
    // Update last ping timestamp
    let mut last_ping = bridge.last_ping.lock().await;
    *last_ping = Some(chrono::Utc::now());

    // Respond with health status
    let status = BridgeEvent::HealthStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: 0, // TODO: track actual uptime
        active_executions: 0, // TODO: query from storage
        pending_approvals: 0, // TODO: query from storage
        workflows_loaded: 0,  // TODO: query from storage
    };

    let envelope = BridgeEventEnvelope::new(status, None);
    bridge.send_event(&envelope).await;
}

async fn handle_approval_decision(
    approval_id: &str,
    decision: &str,
    reason: &str,
    decided_by: &str,
) {
    // TODO (Task 4): Wire into existing approval resolution logic
    // from src/api/mod.rs decide_approval_handler
    info!(
        "Bridge: approval decision received: id={}, decision={}, by={}",
        approval_id, decision, decided_by
    );
    let _ = reason; // Used when wiring to storage
}

async fn handle_workflow_trigger(
    workflow: &str,
    params: &Value,
    triggered_by: &str,
    channel: &str,
) {
    // TODO (Task 4): Wire into existing workflow execution logic
    info!(
        "Bridge: workflow trigger received: workflow={}, by={}, channel={}",
        workflow, triggered_by, channel
    );
    let _ = params;
}
```

- [ ] **Step 5: Update Cargo.toml if needed**

Check that `tokio-tungstenite` and `futures-util` are in dev-dependencies for tests. If not, add:

```toml
[dev-dependencies]
tokio-tungstenite = { version = "0.21", features = ["native-tls"] }
futures-util = "0.3"
```

- [ ] **Step 6: Register bridge route in api/mod.rs**

In `src/api/mod.rs`, the bridge route is merged separately because it uses `BridgeState` not `AppState`. The main.rs or server startup code should merge both routers. For now, the test creates the merged router directly.

No changes needed to `src/api/mod.rs` at this point — `create_bridge_routes` is called by the server startup code (modified in Task 4) and by the test helper.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test --test bridge_websocket_test`
Expected: Both tests pass (connection + rejection)

- [ ] **Step 8: Commit**

```bash
git add src/bridge/websocket.rs src/bridge/handlers.rs tests/bridge_websocket_test.rs Cargo.toml
git commit -m "feat(bridge): WebSocket endpoint with auth, single-client, and event dispatch"
```

---

## Chunk 3: r8r Event Emitters & Diagnostics

### Task 4: Wire Bridge into Executor & Server Startup

**Files:**
- Modify: `src/engine/executor.rs`
- Modify: `src/triggers/approval_timeout.rs`
- Modify: `src/main.rs` (server startup)
- Modify: `src/bridge/handlers.rs` (wire approval decision to storage)

- [ ] **Step 1: Add BridgeState to Executor**

In `src/engine/executor.rs`, add a `bridge` field to the `Executor` struct:

```rust
use crate::bridge::BridgeState;

// In the Executor struct, add:
bridge: Option<Arc<BridgeState>>,

// Add builder method:
pub fn with_bridge(mut self, bridge: Arc<BridgeState>) -> Self {
    self.bridge = Some(bridge);
    self
}
```

Initialize `bridge: None` in `Executor::new()`.

- [ ] **Step 2: Emit approval event when approval node completes**

In `src/engine/executor.rs`, find where `ExecutionStatus::WaitingForApproval` is set (around line 1242). After the existing `emit_execution_finished` call, add:

```rust
// Emit bridge event for approval
if let Some(bridge) = self.bridge.as_ref() {
    if let Some(approval_output) = last_output.as_object() {
        let approval_id = approval_output.get("approval_id")
            .and_then(|v| v.as_str()).unwrap_or("").to_string();
        let title = approval_output.get("title")
            .and_then(|v| v.as_str()).unwrap_or("").to_string();

        let event = crate::bridge::events::BridgeEvent::ApprovalRequested {
            approval_id,
            workflow: execution.workflow_name.clone(),
            execution_id: execution.id.clone(),
            node_id: current_node_id.clone(),
            message: title,
            timeout_secs: approval_output.get("expires_at")
                .and_then(|_| Some(1800)).unwrap_or(0),
            requester: approval_output.get("assignee")
                .and_then(|v| v.as_str()).map(String::from),
            context: last_output.clone(),
        };
        let envelope = crate::bridge::events::BridgeEventEnvelope::new(
            event,
            Some(execution.id.clone()),
        );
        bridge.send_event(&envelope).await;
    }
}
```

- [ ] **Step 3: Emit execution completed/failed events**

In `src/engine/executor.rs`, find `emit_execution_finished` (around line 3210). Add bridge emission alongside monitor emission:

```rust
fn emit_bridge_execution_finished(
    bridge: Option<&Arc<BridgeState>>,
    execution: &Execution,
) {
    let Some(bridge) = bridge else { return; };
    let duration_ms = execution
        .finished_at
        .map(|f| (f - execution.started_at).num_milliseconds())
        .unwrap_or(0);

    // node_count and failed_node should be passed as parameters
    // from the executor context where they are known
    let event = match execution.status {
        ExecutionStatus::Completed => BridgeEvent::ExecutionCompleted {
            workflow: execution.workflow_name.clone(),
            execution_id: execution.id.clone(),
            status: "success".to_string(),
            duration_ms,
            node_count, // Passed from executor (count of executed nodes)
        },
        ExecutionStatus::Failed => BridgeEvent::ExecutionFailed {
            workflow: execution.workflow_name.clone(),
            execution_id: execution.id.clone(),
            status: "failed".to_string(),
            error_code: "EXECUTION_FAILED".to_string(),
            error_message: execution.error.clone().unwrap_or_default(),
            failed_node: failed_node_id.unwrap_or("unknown").to_string(), // From executor context
            duration_ms,
        },
        _ => return,
    };

    let envelope = BridgeEventEnvelope::new(event, Some(execution.id.clone()));
    let bridge = bridge.clone();
    tokio::spawn(async move {
        bridge.send_event(&envelope).await;
    });
}
```

Call `emit_bridge_execution_finished(self.bridge.as_ref(), &execution)` alongside the existing `emit_execution_finished` calls.

- [ ] **Step 4: Emit approval timeout event**

In `src/triggers/approval_timeout.rs`, after the timeout is processed (around line 207), add:

```rust
// Emit bridge event for approval timeout
if let Some(bridge) = bridge.as_ref() {
    let event = crate::bridge::events::BridgeEvent::ApprovalTimeout {
        workflow: approval.workflow_name.clone(),
        execution_id: approval.execution_id.clone(),
        node_id: resolved_node_id.clone().unwrap_or_default(),
        elapsed_secs: approval.expires_at
            .map(|e| (chrono::Utc::now() - approval.created_at).num_seconds() as u64)
            .unwrap_or(0),
    };
    let envelope = crate::bridge::events::BridgeEventEnvelope::new(
        event,
        Some(approval.execution_id.clone()),
    );
    bridge.send_event(&envelope).await;
}
```

The `ApprovalTimeoutChecker` needs a `bridge: Option<Arc<BridgeState>>` field, set via a new `with_bridge()` method.

- [ ] **Step 5: Wire bridge into server startup in main.rs**

In `src/main.rs`, where the server is started (the `server` subcommand handler), add:

```rust
use r8r::bridge::BridgeState;

let bridge = Arc::new(BridgeState::new());

// When creating executor:
let executor = executor.with_bridge(bridge.clone());

// When creating approval timeout checker:
let timeout_checker = timeout_checker.with_bridge(bridge.clone());

// When building the Axum app, merge bridge routes:
let app = app.merge(r8r::bridge::websocket::create_bridge_routes(bridge.clone()));
```

- [ ] **Step 6: Wire approval decision handler to storage**

Update `src/bridge/handlers.rs` `handle_approval_decision` to accept storage and resolve the approval:

The handler needs access to storage. Update `handle_incoming_event` signature to accept `Arc<dyn Storage>` and pass it through. Or, add storage to `BridgeState`.

Simplest approach: add `storage: Option<Arc<dyn Storage>>` to `BridgeState`. Set it during server startup.

```rust
// In BridgeState:
pub(crate) storage: Arc<Mutex<Option<Arc<dyn Storage>>>>,

// In handle_approval_decision, use storage to resolve:
async fn handle_approval_decision(
    bridge: &BridgeState,
    approval_id: &str,
    decision: &str,
    reason: &str,
    decided_by: &str,
) {
    let storage_guard = bridge.storage.lock().await;
    let Some(storage) = storage_guard.as_ref() else {
        warn!("Bridge: no storage configured, cannot process approval");
        return;
    };

    // Load approval request
    let approval = match storage.get_approval_request(approval_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            warn!("Bridge: approval {} not found", approval_id);
            return;
        }
        Err(e) => {
            error!("Bridge: failed to load approval {}: {}", approval_id, e);
            return;
        }
    };

    // Validate approval is still pending
    if approval.status != "pending" {
        warn!("Bridge: approval {} already resolved (status: {})", approval_id, approval.status);
        return;
    }

    // Check if expired
    if let Some(expires_at) = approval.expires_at {
        if chrono::Utc::now() > expires_at {
            warn!("Bridge: approval {} has expired", approval_id);
            return;
        }
    }

    // Resolve the approval using the correct Storage trait method:
    // decide_approval_request(id, new_status, decided_by, decision_comment, decided_at)
    let status = if decision == "approved" || decision == "approve" {
        "approved"
    } else {
        "rejected"
    };

    let comment = if reason.is_empty() { None } else { Some(reason.to_string()) };

    if let Err(e) = storage.decide_approval_request(
        approval_id,
        status,
        Some(decided_by.to_string()),
        comment,
        Some(chrono::Utc::now()),
    ).await {
        error!("Bridge: failed to decide approval {}: {}", approval_id, e);
        return;
    }

    info!("Bridge: approval {} resolved as {} by {}", approval_id, status, decided_by);

    // Resume execution from checkpoint
    // Extract the shared approval-resume logic from:
    //   src/api/mod.rs (decide_approval_handler)
    //   src/triggers/approval_timeout.rs (process_expired_approvals)
    // into a shared function: crate::bridge::handlers::resume_after_approval()
    // This loads the checkpoint, injects the decision, saves, and resumes.
    // TODO: Wire this using the same inject_approval_decision_into_checkpoint
    // pattern from approval_timeout.rs
}
```

- [ ] **Step 7: Run existing tests to verify nothing is broken**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: All existing tests still pass

- [ ] **Step 8: Commit**

```bash
git add src/bridge/ src/engine/executor.rs src/triggers/approval_timeout.rs src/main.rs
git commit -m "feat(bridge): wire event emitters into executor, approvals, and server startup"
```

---

### Task 5: Diagnostics Update

**Files:**
- Modify: `src/cli/diagnostics.rs`

- [ ] **Step 1: Add bridge status to diagnostics output**

In `src/cli/diagnostics.rs`, find the diagnostics output section. Add a bridge status section:

```rust
// Bridge status
if let Some(bridge) = bridge_state {
    let connected = bridge.is_connected().await;
    let last_ping = bridge.last_ping_time().await;

    if connected {
        let ping_ago = last_ping
            .map(|t| {
                let secs = (chrono::Utc::now() - t).num_seconds();
                format!("last ping {}s ago", secs)
            })
            .unwrap_or_else(|| "no pings yet".to_string());
        println!("  Bridge:      connected ({})", ping_ago);
    } else {
        println!("  Bridge:      not connected");
    }
} else {
    println!("  Bridge:      disabled");
}
```

This follows the existing diagnostics format with aligned labels.

- [ ] **Step 2: Run diagnostics to verify output**

Run: `cargo run -- diagnostics 2>&1`
Expected: Shows "Bridge: disabled" or "Bridge: not connected"

- [ ] **Step 3: Commit**

```bash
git add src/cli/diagnostics.rs
git commit -m "feat(bridge): show bridge connection status in diagnostics"
```

---

## Chunk 4: ZeptoClaw Bridge Client

### Task 6: ZeptoClaw Event Types & Config

**Files (in ~/ios/zeptoclaw):**
- Create: `src/r8r_bridge/mod.rs`
- Create: `src/r8r_bridge/events.rs`
- Create: `src/r8r_bridge/dedup.rs`
- Modify: `src/config/mod.rs`
- Modify: `src/lib.rs`
- Test: `tests/r8r_bridge_events_test.rs`

- [ ] **Step 1: Write failing tests for event types and deduplicator**

Create `tests/r8r_bridge_events_test.rs`:

```rust
use zeptoclaw::r8r_bridge::events::{BridgeEvent, BridgeEventEnvelope};
use zeptoclaw::r8r_bridge::dedup::Deduplicator;

#[test]
fn test_event_envelope_deserialization() {
    let json = r#"{
        "id": "evt_ap_001",
        "type": "r8r.approval.requested",
        "timestamp": "2026-03-14T10:30:00Z",
        "data": {
            "approval_id": "appr_9d2e",
            "workflow": "deploy-prod",
            "execution_id": "exec_7f3a",
            "node_id": "approve-deploy",
            "message": "Deploy to prod?",
            "timeout_secs": 1800
        },
        "correlation_id": "corr_001"
    }"#;

    let envelope: BridgeEventEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(envelope.event_type(), "r8r.approval.requested");
}

#[test]
fn test_dedup_skips_duplicate() {
    let mut dedup = Deduplicator::new(200, 600);
    assert!(dedup.is_new("evt_001")); // First time: new
    assert!(!dedup.is_new("evt_001")); // Second time: duplicate
    assert!(dedup.is_new("evt_002")); // Different ID: new
}

#[test]
fn test_dedup_evicts_oldest() {
    let mut dedup = Deduplicator::new(3, 600);
    dedup.is_new("evt_001");
    dedup.is_new("evt_002");
    dedup.is_new("evt_003");
    dedup.is_new("evt_004"); // Evicts evt_001

    assert!(dedup.is_new("evt_001")); // Evicted, so it's "new" again
    assert!(!dedup.is_new("evt_004")); // Still tracked
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_events_test 2>&1 | head -20`
Expected: Compilation error — module doesn't exist

- [ ] **Step 3: Create module structure**

Create `src/r8r_bridge/mod.rs`:

```rust
pub mod events;
pub mod dedup;
pub(crate) mod approval;
pub(crate) mod health;

// Re-export for convenience
pub use events::{BridgeEvent, BridgeEventEnvelope};
```

Create `src/r8r_bridge/events.rs` — mirror of r8r's events.rs (same schema, independent code):

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEventEnvelope {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type_str: String,
    pub timestamp: DateTime<Utc>,
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

impl BridgeEventEnvelope {
    pub fn new(event: BridgeEvent, correlation_id: Option<String>) -> Self {
        let event_type_str = event.event_type().to_string();
        let data = serde_json::to_value(&event).unwrap_or(Value::Null);
        Self {
            id: format!("evt_{}", Uuid::new_v4().as_simple()),
            event_type_str,
            timestamp: Utc::now(),
            data,
            correlation_id,
        }
    }

    pub fn event_type(&self) -> &str {
        &self.event_type_str
    }

    /// Parse the event data based on the type string.
    pub fn parse_event(&self) -> Option<BridgeEvent> {
        BridgeEvent::from_type_and_data(self.event_type(), &self.data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BridgeEvent {
    ApprovalRequested {
        approval_id: String,
        workflow: String,
        execution_id: String,
        node_id: String,
        message: String,
        timeout_secs: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        requester: Option<String>,
        #[serde(default)]
        context: Value,
    },
    ApprovalTimeout {
        workflow: String,
        execution_id: String,
        node_id: String,
        elapsed_secs: u64,
    },
    ExecutionCompleted {
        workflow: String,
        execution_id: String,
        status: String,
        duration_ms: i64,
        node_count: usize,
    },
    ExecutionFailed {
        workflow: String,
        execution_id: String,
        status: String,
        error_code: String,
        error_message: String,
        failed_node: String,
        duration_ms: i64,
    },
    HealthStatus {
        version: String,
        uptime_secs: u64,
        active_executions: usize,
        pending_approvals: usize,
        workflows_loaded: usize,
    },
    ApprovalDecision {
        approval_id: String,
        execution_id: String,
        node_id: String,
        decision: String,
        reason: String,
        decided_by: String,
        channel: String,
    },
    WorkflowTrigger {
        workflow: String,
        params: Value,
        triggered_by: String,
        channel: String,
    },
    HealthPing,
}

impl BridgeEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ApprovalRequested { .. } => "r8r.approval.requested",
            Self::ApprovalTimeout { .. } => "r8r.approval.timeout",
            Self::ExecutionCompleted { .. } => "r8r.execution.completed",
            Self::ExecutionFailed { .. } => "r8r.execution.failed",
            Self::HealthStatus { .. } => "r8r.health.status",
            Self::ApprovalDecision { .. } => "zeptoclaw.approval.decision",
            Self::WorkflowTrigger { .. } => "zeptoclaw.workflow.trigger",
            Self::HealthPing => "zeptoclaw.health.ping",
        }
    }

    pub fn from_type_and_data(event_type: &str, data: &Value) -> Option<Self> {
        match event_type {
            "r8r.approval.requested" => serde_json::from_value(data.clone()).ok(),
            "r8r.approval.timeout" => serde_json::from_value(data.clone()).ok(),
            "r8r.execution.completed" => serde_json::from_value(data.clone()).ok(),
            "r8r.execution.failed" => serde_json::from_value(data.clone()).ok(),
            "r8r.health.status" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.approval.decision" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.workflow.trigger" => serde_json::from_value(data.clone()).ok(),
            "zeptoclaw.health.ping" => Some(BridgeEvent::HealthPing),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ack {
    #[serde(rename = "type")]
    pub ack_type: String,
    pub event_id: String,
}

impl Ack {
    pub fn new(event_id: String) -> Self {
        Self {
            ack_type: "ack".to_string(),
            event_id,
        }
    }
}
```

Create `src/r8r_bridge/dedup.rs`:

```rust
use std::collections::VecDeque;
use std::time::Instant;

/// LRU deduplicator tracking recently processed event IDs.
/// Prevents duplicate processing when events are replayed on reconnect.
pub struct Deduplicator {
    seen: VecDeque<(String, Instant)>,
    max_entries: usize,
    ttl_secs: u64,
}

impl Deduplicator {
    pub fn new(max_entries: usize, ttl_secs: u64) -> Self {
        Self {
            seen: VecDeque::with_capacity(max_entries),
            max_entries,
            ttl_secs,
        }
    }

    /// Returns true if this event ID has NOT been seen recently.
    /// Adds it to the tracking set.
    pub fn is_new(&mut self, event_id: &str) -> bool {
        self.prune_expired();

        // Check if already seen
        if self.seen.iter().any(|(id, _)| id == event_id) {
            return false;
        }

        // Evict oldest if at capacity
        if self.seen.len() >= self.max_entries {
            self.seen.pop_front();
        }

        self.seen.push_back((event_id.to_string(), Instant::now()));
        true
    }

    fn prune_expired(&mut self) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(self.ttl_secs);
        while let Some((_, ts)) = self.seen.front() {
            if *ts < cutoff {
                self.seen.pop_front();
            } else {
                break;
            }
        }
    }
}
```

Create placeholder files:

`src/r8r_bridge/approval.rs`:
```rust
// Implemented in Task 8
```

`src/r8r_bridge/health.rs`:
```rust
// Implemented in Task 8
```

- [ ] **Step 4: Add module to lib.rs and config**

In `src/lib.rs`, add:
```rust
pub mod r8r_bridge;
```

In `src/config/mod.rs`, add config section (follow existing pattern for optional config sections):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct R8rBridgeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bridge_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub default_channel: Option<ChannelTarget>,
    #[serde(default)]
    pub approval_routing: HashMap<String, ChannelTarget>,
    #[serde(default = "default_reconnect_max")]
    pub reconnect_max_interval_secs: u64,
    #[serde(default = "default_health_interval")]
    pub health_ping_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTarget {
    pub channel: String,
    pub chat_id: String,
}

fn default_bridge_endpoint() -> String {
    "ws://localhost:8080/api/ws/events".to_string()
}

fn default_reconnect_max() -> u64 { 30 }
fn default_health_interval() -> u64 { 60 }
```

Add `r8r_bridge: R8rBridgeConfig` to the main config struct.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_events_test`
Expected: All 3 tests pass

- [ ] **Step 6: Commit**

```bash
cd ~/ios/zeptoclaw
git add src/r8r_bridge/ src/lib.rs src/config/mod.rs tests/r8r_bridge_events_test.rs
git commit -m "feat(r8r_bridge): add event types, deduplicator, and config for r8r bridge"
```

---

### Task 7: ZeptoClaw WebSocket Client

**Files (in ~/ios/zeptoclaw):**
- Modify: `src/r8r_bridge/mod.rs`
- Test: `tests/r8r_bridge_client_test.rs`

- [ ] **Step 1: Write failing test for bridge client connection**

Create `tests/r8r_bridge_client_test.rs`:

```rust
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::accept_async;
use zeptoclaw::r8r_bridge::R8rBridge;
use zeptoclaw::r8r_bridge::events::{BridgeEventEnvelope, BridgeEvent};

/// Start a mock r8r WebSocket server that echoes health pings as health status.
async fn start_mock_r8r(port: u16) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>(100);
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let ws = accept_async(stream).await.unwrap();
            let (mut ws_tx, mut ws_rx) = ws.split();

            while let Some(Ok(msg)) = ws_rx.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    tx.send(text.clone()).await.ok();

                    // If it's a health ping, respond with health status
                    if text.contains("zeptoclaw.health.ping") {
                        let status = BridgeEventEnvelope::new(
                            BridgeEvent::HealthStatus {
                                version: "0.5.0".into(),
                                uptime_secs: 100,
                                active_executions: 0,
                                pending_approvals: 0,
                                workflows_loaded: 5,
                            },
                            None,
                        );
                        let json = serde_json::to_string(&status).unwrap();
                        ws_tx.send(tokio_tungstenite::tungstenite::Message::Text(json))
                            .await
                            .ok();
                    }
                }
            }
        }
    });

    rx
}

#[tokio::test]
async fn test_bridge_connects_and_sends_health_ping() {
    let port = 19876; // Use a random high port for testing
    let mut server_rx = start_mock_r8r(port).await;

    // Give server time to bind
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let bridge = R8rBridge::new(
        format!("ws://127.0.0.1:{}", port),
        Some("test-token".to_string()),
    );

    // Connect
    bridge.connect().await.unwrap();

    // Send health ping
    bridge.send_health_ping().await.unwrap();

    // Verify server received it
    let received = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        server_rx.recv(),
    ).await.unwrap().unwrap();

    assert!(received.contains("zeptoclaw.health.ping"));

    bridge.disconnect().await;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_client_test 2>&1 | head -20`
Expected: Compilation error — `R8rBridge` doesn't exist

- [ ] **Step 3: Implement R8rBridge WebSocket client**

Update `src/r8r_bridge/mod.rs`:

```rust
pub mod events;
pub mod dedup;
pub(crate) mod approval;
pub(crate) mod health;

pub use events::{BridgeEvent, BridgeEventEnvelope};

use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite;
use futures_util::{SinkExt, StreamExt};
use tracing::{info, warn, error};

use crate::r8r_bridge::dedup::Deduplicator;
use crate::r8r_bridge::events::Ack;

pub struct R8rBridge {
    endpoint: String,
    token: Option<String>,
    sender: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    dedup: Arc<Mutex<Deduplicator>>,
    health_status: Arc<Mutex<Option<events::BridgeEvent>>>,
    connected: Arc<std::sync::atomic::AtomicBool>,
}

impl R8rBridge {
    pub fn new(endpoint: String, token: Option<String>) -> Self {
        Self {
            endpoint,
            token,
            sender: Arc::new(Mutex::new(None)),
            dedup: Arc::new(Mutex::new(Deduplicator::new(200, 600))),
            health_status: Arc::new(Mutex::new(None)),
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn connect(&self) -> Result<(), String> {
        // Extract host from endpoint URL for the Host header
        let host = url::Url::parse(&self.endpoint)
            .ok()
            .and_then(|u| u.host_str().map(|h| {
                if let Some(port) = u.port() {
                    format!("{}:{}", h, port)
                } else {
                    h.to_string()
                }
            }))
            .unwrap_or_else(|| "localhost".to_string());

        let mut request = tungstenite::http::Request::builder()
            .uri(&self.endpoint)
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .header("Sec-WebSocket-Version", "13")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Host", &host);

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let request = request.body(()).map_err(|e| e.to_string())?;

        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| format!("Failed to connect to r8r: {}", e))?;

        let (ws_sink, mut ws_stream) = ws.split();

        // Create send channel
        let (tx, mut rx) = mpsc::channel::<String>(256);
        *self.sender.lock().await = Some(tx);
        self.connected.store(true, std::sync::atomic::Ordering::Relaxed);

        // Sender task
        let ws_sink = Arc::new(Mutex::new(ws_sink));
        let sender_sink = ws_sink.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let mut sink = sender_sink.lock().await;
                if sink.send(tungstenite::Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        });

        // Receiver task
        let dedup = self.dedup.clone();
        let health_status = self.health_status.clone();
        let connected = self.connected.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_stream.next().await {
                if let tungstenite::Message::Text(text) = msg {
                    // Parse envelope
                    if let Ok(envelope) = serde_json::from_str::<BridgeEventEnvelope>(&text) {
                        // Dedup check
                        let is_new = dedup.lock().await.is_new(&envelope.id);
                        if !is_new {
                            info!("Bridge: skipping duplicate event {}", envelope.id);
                            continue;
                        }

                        // Send ack
                        let ack = Ack::new(envelope.id.clone());
                        if let Ok(ack_json) = serde_json::to_string(&ack) {
                            if let Some(tx) = sender.lock().await.as_ref() {
                                let _ = tx.send(ack_json).await;
                            }
                        }

                        // Handle event
                        if let Some(event) = envelope.parse_event() {
                            match &event {
                                BridgeEvent::HealthStatus { .. } => {
                                    *health_status.lock().await = Some(event);
                                }
                                BridgeEvent::ApprovalRequested { .. } => {
                                    // TODO: route to channel (Task 8)
                                    info!("Bridge: approval requested: {}", envelope.id);
                                }
                                BridgeEvent::ApprovalTimeout { .. } => {
                                    info!("Bridge: approval timeout: {}", envelope.id);
                                }
                                BridgeEvent::ExecutionCompleted { .. } => {
                                    info!("Bridge: execution completed: {}", envelope.id);
                                }
                                BridgeEvent::ExecutionFailed { .. } => {
                                    info!("Bridge: execution failed: {}", envelope.id);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // Connection closed
            connected.store(false, std::sync::atomic::Ordering::Relaxed);
            *sender.lock().await = None;
            info!("Bridge: disconnected from r8r");
        });

        info!("Bridge: connected to r8r at {}", self.endpoint);
        Ok(())
    }

    pub async fn disconnect(&self) {
        let mut sender = self.sender.lock().await;
        *sender = None;
        self.connected.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Send a raw event envelope to r8r.
    pub async fn send(&self, envelope: &BridgeEventEnvelope) -> Result<(), String> {
        let json = serde_json::to_string(envelope).map_err(|e| e.to_string())?;
        let sender = self.sender.lock().await;
        if let Some(tx) = sender.as_ref() {
            tx.send(json).await.map_err(|e| e.to_string())
        } else {
            Err("Not connected to r8r".to_string())
        }
    }

    pub async fn send_health_ping(&self) -> Result<(), String> {
        let envelope = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
        self.send(&envelope).await
    }

    /// Get the last known r8r health status.
    pub async fn last_health_status(&self) -> Option<events::BridgeEvent> {
        self.health_status.lock().await.clone()
    }

    /// Start the bridge with automatic reconnect on disconnect.
    /// Uses exponential backoff: 1s, 2s, 4s, 8s, 16s, max 30s.
    /// This is the main entry point — call this instead of connect() directly.
    pub async fn run(&self, max_interval_secs: u64) {
        let mut backoff_secs: u64 = 1;

        loop {
            match self.connect().await {
                Ok(()) => {
                    backoff_secs = 1; // Reset on successful connect
                    info!("Bridge: connected to r8r");

                    // Wait until disconnected
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if !self.is_connected() {
                            break;
                        }
                    }
                    warn!("Bridge: disconnected from r8r, will reconnect");
                }
                Err(e) => {
                    warn!("Bridge: connection failed: {}", e);
                }
            }

            // Exponential backoff
            info!("Bridge: reconnecting in {}s", backoff_secs);
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(max_interval_secs);
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_client_test`
Expected: Test passes

- [ ] **Step 5: Commit**

```bash
cd ~/ios/zeptoclaw
git add src/r8r_bridge/ tests/r8r_bridge_client_test.rs
git commit -m "feat(r8r_bridge): WebSocket client with dedup, acks, and reconnect support"
```

---

## Chunk 5: ZeptoClaw Approval Routing & Health

### Task 8: Approval Routing & Response Parsing

**Files (in ~/ios/zeptoclaw):**
- Modify: `src/r8r_bridge/approval.rs`
- Test: `tests/r8r_bridge_approval_test.rs`

- [ ] **Step 1: Write failing tests for approval response parsing**

Create `tests/r8r_bridge_approval_test.rs`:

```rust
use zeptoclaw::r8r_bridge::approval::{parse_approval_response, ApprovalDecision};

#[test]
fn test_approve_keywords() {
    for word in &["approve", "approved", "yes", "y", "lgtm", "ok"] {
        let result = parse_approval_response(word);
        assert!(result.is_some(), "Failed for: {}", word);
        let d = result.unwrap();
        assert_eq!(d.decision, "approved");
        assert!(d.reason.is_empty());
    }
}

#[test]
fn test_reject_keywords() {
    for word in &["reject", "rejected", "no", "n", "deny", "denied"] {
        let result = parse_approval_response(word);
        assert!(result.is_some(), "Failed for: {}", word);
        let d = result.unwrap();
        assert_eq!(d.decision, "rejected");
        assert!(d.reason.is_empty());
    }
}

#[test]
fn test_reject_with_reason() {
    let result = parse_approval_response("reject bad build").unwrap();
    assert_eq!(result.decision, "rejected");
    assert_eq!(result.reason, "bad build");
}

#[test]
fn test_approve_with_reason() {
    let result = parse_approval_response("approve looks good").unwrap();
    assert_eq!(result.decision, "approved");
    assert_eq!(result.reason, "looks good");
}

#[test]
fn test_case_insensitive() {
    let result = parse_approval_response("APPROVE").unwrap();
    assert_eq!(result.decision, "approved");
}

#[test]
fn test_unrecognized_returns_none() {
    assert!(parse_approval_response("deploy staging").is_none());
    assert!(parse_approval_response("hello").is_none());
    assert!(parse_approval_response("").is_none());
}

#[test]
fn test_format_approval_message() {
    let msg = zeptoclaw::r8r_bridge::approval::format_approval_message(
        "deploy-prod", "exec_7f3a", "Deploy api-service to production?",
    );
    assert!(msg.contains("[r8r]"));
    assert!(msg.contains("deploy-prod"));
    assert!(msg.contains("exec_7f"));
    assert!(msg.contains("approve"));
    assert!(msg.contains("reject"));
}

#[test]
fn test_format_timeout_message() {
    let msg = zeptoclaw::r8r_bridge::approval::format_timeout_message(
        "deploy-prod", "exec_7f3a", 1800,
    );
    assert!(msg.contains("[r8r]"));
    assert!(msg.contains("expired"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_approval_test 2>&1 | head -20`
Expected: Compilation error

- [ ] **Step 3: Implement approval module**

Replace `src/r8r_bridge/approval.rs`:

```rust
/// Parsed approval decision from a user's channel response.
pub struct ApprovalDecision {
    pub decision: String,  // "approved" or "rejected"
    pub reason: String,
}

const APPROVE_WORDS: &[&str] = &["approve", "approved", "yes", "y", "lgtm", "ok"];
const REJECT_WORDS: &[&str] = &["reject", "rejected", "no", "n", "deny", "denied"];

/// Parse a user's text response into an approval decision.
/// Returns None if the response is not recognized.
pub fn parse_approval_response(text: &str) -> Option<ApprovalDecision> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let lower = text.to_lowercase();
    let first_word = lower.split_whitespace().next()?;
    let reason = text
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");

    if APPROVE_WORDS.contains(&first_word.as_str()) {
        Some(ApprovalDecision {
            decision: "approved".to_string(),
            reason,
        })
    } else if REJECT_WORDS.contains(&first_word.as_str()) {
        Some(ApprovalDecision {
            decision: "rejected".to_string(),
            reason,
        })
    } else {
        None
    }
}

/// Format the approval request message sent to the user's channel.
pub fn format_approval_message(
    workflow: &str,
    execution_id: &str,
    message: &str,
) -> String {
    let short_id = if execution_id.len() > 8 {
        &execution_id[..8]
    } else {
        execution_id
    };

    format!(
        "[r8r] Approval needed for {} ({})\n> {}\nReply: approve / reject [reason]",
        workflow, short_id, message
    )
}

/// Format the timeout notification message.
pub fn format_timeout_message(
    workflow: &str,
    execution_id: &str,
    elapsed_secs: u64,
) -> String {
    let short_id = if execution_id.len() > 8 {
        &execution_id[..8]
    } else {
        execution_id
    };

    let elapsed_str = if elapsed_secs >= 3600 {
        format!("{}h {}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    } else if elapsed_secs >= 60 {
        format!("{}m", elapsed_secs / 60)
    } else {
        format!("{}s", elapsed_secs)
    };

    format!(
        "[r8r] Approval for {} ({}) expired after {}.\nRe-run the workflow to try again.",
        workflow, short_id, elapsed_str
    )
}

/// Format the "unrecognized response" help message.
pub fn format_help_message() -> String {
    "I didn't understand that. Reply **approve** or **reject [reason]**.".to_string()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd ~/ios/zeptoclaw && cargo test --test r8r_bridge_approval_test`
Expected: All 7 tests pass

- [ ] **Step 5: Commit**

```bash
cd ~/ios/zeptoclaw
git add src/r8r_bridge/approval.rs tests/r8r_bridge_approval_test.rs
git commit -m "feat(r8r_bridge): approval response parsing and channel message formatting"
```

---

### Task 9: Health Ping Loop & CLI Status

**Files (in ~/ios/zeptoclaw):**
- Modify: `src/r8r_bridge/health.rs`
- Modify: `src/cli/mod.rs` (or `status.rs`)

- [ ] **Step 1: Implement health ping loop**

Replace `src/r8r_bridge/health.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{info, warn};

use crate::r8r_bridge::R8rBridge;

/// Start a background task that sends health pings at the configured interval.
/// Returns a JoinHandle that can be used to cancel the loop.
pub fn start_health_ping_loop(
    bridge: Arc<R8rBridge>,
    interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;

            if !bridge.is_connected() {
                continue;
            }

            if let Err(e) = bridge.send_health_ping().await {
                warn!("Bridge: failed to send health ping: {}", e);
            }
        }
    })
}

/// Format r8r health status for CLI display.
pub fn format_health_status(bridge: &R8rBridge, status: &Option<crate::r8r_bridge::events::BridgeEvent>) -> String {
    if !bridge.is_connected() {
        return "r8r bridge: not connected".to_string();
    }

    match status {
        Some(crate::r8r_bridge::events::BridgeEvent::HealthStatus {
            version,
            uptime_secs,
            active_executions,
            pending_approvals,
            workflows_loaded,
        }) => {
            let uptime = format_uptime(*uptime_secs);
            format!(
                "r8r bridge: connected\n  version: {}\n  uptime: {}\n  active executions: {}\n  pending approvals: {}\n  workflows loaded: {}",
                version, uptime, active_executions, pending_approvals, workflows_loaded
            )
        }
        _ => "r8r bridge: connected (no status yet)".to_string(),
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}
```

- [ ] **Step 2: Add bridge status to CLI status command**

In ZeptoClaw's CLI status output (likely `src/cli/status.rs` or equivalent), add after the existing health sections:

```rust
// r8r Bridge status
if let Some(bridge) = &r8r_bridge {
    let health = bridge.last_health_status().await;
    let status_str = crate::r8r_bridge::health::format_health_status(bridge, &health);
    println!("  {}", status_str);
} else {
    println!("  r8r bridge: disabled");
}
```

- [ ] **Step 3: Run full test suite to verify nothing broken**

Run: `cd ~/ios/zeptoclaw && cargo test --lib 2>&1 | tail -5`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
cd ~/ios/zeptoclaw
git add src/r8r_bridge/health.rs src/cli/
git commit -m "feat(r8r_bridge): health ping loop and CLI status display"
```

---

## Chunk 6: Integration Testing

### Task 10: Cross-Project Integration Test

**Files:**
- Create: `tests/bridge_integration_test.rs` (in r8r repo)

This test validates the full approval flow: r8r emits approval -> mock ZeptoClaw client receives -> sends decision -> r8r resolves.

- [ ] **Step 1: Write integration test**

Create `tests/bridge_integration_test.rs` (in r8r):

```rust
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite};
use futures_util::{SinkExt, StreamExt};
use r8r::bridge::BridgeState;
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope, Ack};

#[tokio::test]
async fn test_full_approval_roundtrip() {
    // Start r8r server with bridge — token injected via constructor
    let bridge = Arc::new(BridgeState::new(Some("integration-test-token".to_string())));
    let storage = r8r::storage::sqlite::SqliteStorage::new_in_memory()
        .await
        .unwrap();
    let storage = Arc::new(storage);
    let registry = Arc::new(r8r::nodes::registry::NodeRegistry::new());

    let app_state = r8r::api::AppState {
        storage: storage.clone(),
        registry,
        monitor: None,
        shutdown: Arc::new(r8r::shutdown::ShutdownCoordinator::new()),
        pause_registry: r8r::engine::executor::PauseRegistry::new(),
        event_backend: None,
    };

    let app = r8r::api::create_api_routes()
        .with_state(app_state)
        .merge(r8r::bridge::websocket::create_bridge_routes(bridge.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Connect as ZeptoClaw client
    let url = format!("ws://127.0.0.1:{}/api/ws/events", addr.port());
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", "Bearer integration-test-token")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "localhost")
        .body(())
        .unwrap();

    let (mut ws, _) = connect_async(request).await.unwrap();

    // Simulate r8r pushing an approval event
    let approval_event = BridgeEventEnvelope::new(
        BridgeEvent::ApprovalRequested {
            approval_id: "appr_test_001".to_string(),
            workflow: "test-workflow".to_string(),
            execution_id: "exec_test_001".to_string(),
            node_id: "approve-step".to_string(),
            message: "Approve this test?".to_string(),
            timeout_secs: 300,
            requester: Some("test-user".to_string()),
            context: serde_json::json!({}),
        },
        Some("corr_test_001".to_string()),
    );

    // Push event through bridge
    bridge.send_event(&approval_event).await;

    // ZeptoClaw client should receive the event
    if let Some(Ok(tungstenite::Message::Text(text))) = ws.next().await {
        let received: BridgeEventEnvelope = serde_json::from_str(&text).unwrap();
        assert_eq!(received.event_type(), "r8r.approval.requested");

        // Send ack
        let ack = Ack::new(received.id.clone());
        ws.send(tungstenite::Message::Text(
            serde_json::to_string(&ack).unwrap(),
        )).await.unwrap();

        // Send approval decision
        let decision = BridgeEventEnvelope::new(
            BridgeEvent::ApprovalDecision {
                approval_id: "appr_test_001".to_string(),
                execution_id: "exec_test_001".to_string(),
                node_id: "approve-step".to_string(),
                decision: "approved".to_string(),
                reason: "".to_string(),
                decided_by: "test-user".to_string(),
                channel: "test".to_string(),
            },
            Some("corr_test_001".to_string()),
        );

        ws.send(tungstenite::Message::Text(
            serde_json::to_string(&decision).unwrap(),
        )).await.unwrap();

        // Small delay to let handler process
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    } else {
        panic!("Expected to receive approval event");
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn test_replay_on_reconnect() {
    let bridge = Arc::new(BridgeState::new(Some("replay-test-token".to_string())));

    // Push events while no client is connected (they go to buffer)
    let event1 = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
    let event2 = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
    let event1_id = event1.id.clone();
    let event2_id = event2.id.clone();

    bridge.send_event(&event1).await; // Goes to buffer
    bridge.send_event(&event2).await; // Goes to buffer

    // Start server
    let storage = r8r::storage::sqlite::SqliteStorage::new_in_memory()
        .await.unwrap();
    let app_state = r8r::api::AppState {
        storage: Arc::new(storage),
        registry: Arc::new(r8r::nodes::registry::NodeRegistry::new()),
        monitor: None,
        shutdown: Arc::new(r8r::shutdown::ShutdownCoordinator::new()),
        pause_registry: r8r::engine::executor::PauseRegistry::new(),
        event_backend: None,
    };
    let app = r8r::api::create_api_routes()
        .with_state(app_state)
        .merge(r8r::bridge::websocket::create_bridge_routes(bridge.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Connect — should receive replayed events
    let url = format!("ws://127.0.0.1:{}/api/ws/events", addr.port());
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", "Bearer replay-test-token")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "localhost")
        .body(())
        .unwrap();

    let (mut ws, _) = connect_async(request).await.unwrap();

    // Should receive both buffered events
    let mut received_ids = Vec::new();
    for _ in 0..2 {
        if let Some(Ok(tungstenite::Message::Text(text))) =
            tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await.ok().flatten()
        {
            let env: BridgeEventEnvelope = serde_json::from_str(&text).unwrap();
            received_ids.push(env.id);
        }
    }

    assert!(received_ids.contains(&event1_id));
    assert!(received_ids.contains(&event2_id));

    ws.close(None).await.ok();
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test bridge_integration_test`
Expected: Both tests pass

- [ ] **Step 3: Run full r8r test suite**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass (existing + new)

- [ ] **Step 4: Commit**

```bash
git add tests/bridge_integration_test.rs
git commit -m "test(bridge): add integration tests for approval roundtrip and event replay"
```

---

## Post-Implementation Checklist

- [ ] All r8r tests pass: `cargo test`
- [ ] All ZeptoClaw tests pass: `cd ~/ios/zeptoclaw && cargo test`
- [ ] Bridge connects when both services run locally
- [ ] Health ping/pong works end-to-end
- [ ] `r8r diagnostics` shows bridge status
- [ ] `zeptoclaw status` shows r8r bridge status
- [ ] No behavior change when bridge is not configured
