# Phase 1: WebSocket Event Bus & Approval Bridge

> Spec for the foundation layer connecting ZeptoClaw (AI brain) and r8r (workflow engine) via a bidirectional WebSocket event bus.

## Context

ZeptoClaw and r8r are complementary tools: ZeptoClaw handles AI reasoning and multi-channel communication, r8r handles deterministic workflow execution. Today they integrate via ZeptoClaw's `R8rTool` (trigger workflows) and r8r's `agent` node (call LLMs). But there's no real-time event flow between them — no way for r8r to push approval requests to users on Telegram, or for ZeptoClaw to react to workflow failures instantly.

Phase 1 establishes the event bus foundation and builds the first high-value feature on top: the approval channel bridge.

See also: [Symbiotic Automation Strategy](../SYMBIOTIC_AUTOMATION.md)

## Goals

1. Bidirectional real-time events between ZeptoClaw and r8r over a single WebSocket connection
2. r8r approval requests automatically routed to users on their preferred ZeptoClaw channel
3. Health visibility — each tool knows the other's status
4. Minimal scope — only the event types needed for approvals and health

## Non-Goals

- Shared credential store (deferred)
- Channel-to-workflow routing / pattern matching (Phase 2)
- Workflow generation from ZeptoClaw (Phase 2)
- Redis/multi-host event fan-out (later, if needed)
- Replacing r8r's existing REST API or MCP tools for approvals

---

## Architecture

### Connection Topology

```
ZeptoClaw (client) ---WebSocket---> r8r (server)
         <----bidirectional events---->
```

- ZeptoClaw initiates the connection to r8r's `/api/ws/events` endpoint
- Single persistent connection, bidirectional message flow
- r8r is always the server (it already has an Axum HTTP server)
- ZeptoClaw is always the client (connects on startup or when configured)
- r8r accepts at most one bridge client; a new connection replaces the old one (close frame sent to previous client first, logged as warning)

### Authentication

ZeptoClaw sends a bearer token on the WebSocket upgrade request:

```
GET /api/ws/events HTTP/1.1
Upgrade: websocket
Authorization: Bearer <R8R_BRIDGE_TOKEN>
```

r8r validates against the `R8R_BRIDGE_TOKEN` environment variable (a dedicated token for bridge connections, separate from `R8R_API_KEY`). Falls back to `R8R_API_KEY` if `R8R_BRIDGE_TOKEN` is not set. Rejects with 401 if invalid.

The endpoint is under `/api/` so it is covered by r8r's existing `api_auth_middleware`. The handler additionally validates the bridge token specifically to distinguish bridge clients from general API consumers.

### Connection Lifecycle

1. **Connect:** ZeptoClaw connects on startup (if r8r bridge is configured)
2. **Heartbeat:** Ping/pong every 30 seconds to detect dead connections
3. **Reconnect:** On disconnect, ZeptoClaw retries with exponential backoff (1s, 2s, 4s, 8s, 16s, max 30s)
4. **Replay:** On reconnect, r8r replays unacknowledged events from the last 5 minutes
5. **Graceful shutdown:** Both sides send a close frame before shutting down

---

## Event Schema

All messages on the WebSocket follow this envelope:

```json
{
  "id": "evt_<uuid>",
  "type": "r8r.approval.requested",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {},
  "correlation_id": "corr_<optional>"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique event ID, prefixed `evt_` |
| `type` | string | Yes | Dot-namespaced event type |
| `timestamp` | string (ISO 8601) | Yes | When the event was created |
| `data` | object | Yes | Event-specific payload |
| `correlation_id` | string | No | Links related events (e.g., approval request and decision) |

### Acknowledgment

Receiver sends an ack for each event:

```json
{
  "type": "ack",
  "event_id": "evt_abc123"
}
```

Unacknowledged events are replayed on reconnect (5-minute window, max 100 events). Replay is triggered only on reconnect, not on a timer.

### Deduplication

ZeptoClaw must maintain a set of recently processed event IDs (last 200 IDs or last 10 minutes, whichever is smaller) and skip events whose `id` it has already handled. This prevents duplicate channel messages when ZeptoClaw crashes after processing an event but before sending the ack.

### Backpressure

Each side maintains a send buffer of at most 256 events. If the buffer fills (receiver is not consuming fast enough), the oldest unacknowledged events are dropped and a warning is logged. This prevents unbounded memory growth on either side.

### Close Codes

| Code | Meaning |
|------|---------|
| `1000` | Normal closure (graceful shutdown) |
| `4001` | Authentication failure |
| `4002` | Replaced by new connection |
| `4003` | Protocol error (malformed event) |

---

## Event Types (Phase 1)

### r8r -> ZeptoClaw

#### `r8r.approval.requested`

Emitted when a workflow execution reaches an approval node.

```json
{
  "id": "evt_ap_001",
  "type": "r8r.approval.requested",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {
    "approval_id": "appr_9d2e",
    "workflow": "deploy-prod",
    "execution_id": "exec_7f3a",
    "node_id": "approve-deploy",
    "message": "Build passed. Deploy api-service to production?",
    "timeout_secs": 1800,
    "requester": "ci-bot",
    "context": {
      "service": "api-service",
      "env": "production",
      "commit": "abc1234"
    }
  },
  "correlation_id": "corr_deploy_001"
}
```

#### `r8r.approval.timeout`

Emitted when an approval request expires without a decision.

```json
{
  "id": "evt_at_001",
  "type": "r8r.approval.timeout",
  "timestamp": "2026-03-14T11:00:00Z",
  "data": {
    "workflow": "deploy-prod",
    "execution_id": "exec_7f3a",
    "node_id": "approve-deploy",
    "elapsed_secs": 1800
  },
  "correlation_id": "corr_deploy_001"
}
```

#### `r8r.execution.completed`

Emitted when a workflow execution finishes successfully.

```json
{
  "id": "evt_ec_001",
  "type": "r8r.execution.completed",
  "timestamp": "2026-03-14T10:35:00Z",
  "data": {
    "workflow": "deploy-prod",
    "execution_id": "exec_7f3a",
    "status": "success",
    "duration_ms": 45200,
    "node_count": 6
  },
  "correlation_id": "corr_deploy_001"
}
```

#### `r8r.execution.failed`

Emitted when a workflow execution fails.

```json
{
  "id": "evt_ef_001",
  "type": "r8r.execution.failed",
  "timestamp": "2026-03-14T10:32:00Z",
  "data": {
    "workflow": "deploy-prod",
    "execution_id": "exec_7f3a",
    "status": "failed",
    "error_code": "NODE_TIMEOUT",
    "error_message": "Node 'build' timed out after 300s",
    "failed_node": "build",
    "duration_ms": 300100
  },
  "correlation_id": "corr_deploy_001"
}
```

#### `r8r.health.status`

Response to a health ping from ZeptoClaw.

```json
{
  "id": "evt_hs_001",
  "type": "r8r.health.status",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {
    "version": "0.5.0",
    "uptime_secs": 86400,
    "active_executions": 3,
    "pending_approvals": 1,
    "workflows_loaded": 12
  }
}
```

### ZeptoClaw -> r8r

#### `zeptoclaw.approval.decision`

User responded to an approval request.

```json
{
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
  "correlation_id": "corr_deploy_001"
}
```

`decision` is one of: `"approved"`, `"rejected"`.

#### `zeptoclaw.workflow.trigger`

User requested a workflow run from a ZeptoClaw channel.

**Phase 1 auth model:** The bridge connection is fully trusted (equivalent to API key access). Any connected ZeptoClaw instance can trigger any workflow. Per-workflow trigger authorization is deferred to Phase 2.

```json
{
  "id": "evt_wt_001",
  "type": "zeptoclaw.workflow.trigger",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {
    "workflow": "deploy-service",
    "params": {
      "service": "api",
      "env": "staging"
    },
    "triggered_by": "dr.nora",
    "channel": "telegram"
  }
}
```

#### `zeptoclaw.health.ping`

Periodic health check request (every 60 seconds).

```json
{
  "id": "evt_hp_001",
  "type": "zeptoclaw.health.ping",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {}
}
```

---

## Approval Channel Bridge

### End-to-End Flow

```
1. r8r executor hits approval node
2. r8r stores approval in DB (existing behavior, unchanged)
3. r8r emits r8r.approval.requested on WebSocket
4. ZeptoClaw r8r_bridge receives event
5. ZeptoClaw resolves target channel:
   a. data.requester matches a known user -> their preferred channel
   b. r8r_bridge config has a default_channel -> use that
   c. fallback -> first configured channel
6. ZeptoClaw formats and sends message to channel
7. User replies (e.g., "approve" or "reject bad build")
8. ZeptoClaw parses response (keyword match)
9. ZeptoClaw emits zeptoclaw.approval.decision on WebSocket
10. r8r receives decision, resolves approval (same as REST API path)
11. Workflow resumes
```

### Channel Message Format

Sent to user on their channel:

```
[r8r] Approval needed for {workflow} (exec_{short_id})
> {message}
Reply: approve / reject [reason]
```

### Response Parsing

ZeptoClaw matches user response against:

| User says | Parsed as |
|-----------|-----------|
| `approve`, `approved`, `yes`, `y`, `lgtm`, `ok` | `decision: "approved"` |
| `reject`, `rejected`, `no`, `n`, `deny`, `denied` | `decision: "rejected"` |
| `reject bad build` | `decision: "rejected", reason: "bad build"` |

First word is the decision keyword, remaining text is the reason.

Unrecognized responses get: "I didn't understand that. Reply **approve** or **reject [reason]**."

### Timeout Handling

When `r8r.approval.timeout` arrives, ZeptoClaw sends to the same channel:

```
[r8r] Approval for {workflow} (exec_{short_id}) expired after {elapsed}.
Re-run the workflow to try again.
```

### Configuration

ZeptoClaw side (`~/.zeptoclaw/config.json`):

```json
{
  "r8r_bridge": {
    "enabled": true,
    "endpoint": "ws://localhost:8080/api/ws/events",
    "token": "your-r8r-bridge-token",
    "default_channel": {
      "channel": "telegram",
      "chat_id": "123456789"
    },
    "approval_routing": {
      "ci-bot": {
        "channel": "slack",
        "chat_id": "#deployments"
      },
      "dr.nora": {
        "channel": "telegram",
        "chat_id": "123456789"
      }
    },
    "reconnect_max_interval_secs": 30,
    "health_ping_interval_secs": 60
  }
}
```

Each routing entry specifies both the `channel` (which ZeptoClaw channel to use) and `chat_id` (the target chat/conversation within that channel). If a requester is not in `approval_routing`, the `default_channel` is used.

---

## Health Check Integration

### Mechanism

- ZeptoClaw sends `zeptoclaw.health.ping` every 60 seconds
- r8r responds with `r8r.health.status`
- Connection state itself is the primary health signal

### ZeptoClaw Exposure

`zeptoclaw status` output gains a new section:

```
r8r bridge: connected (last ping 12s ago)
  version: 0.5.0
  uptime: 1d 0h 0m
  active executions: 3
  pending approvals: 1
  workflows loaded: 12
```

Panel API: `GET /api/health` includes `r8r_bridge` field.

### r8r Exposure

`r8r diagnostics` output gains a new line:

```
Bridge: connected (ZeptoClaw, last ping 12s ago)
```

`GET /api/health` response includes:

```json
{
  "bridge": {
    "connected": true,
    "client": "zeptoclaw",
    "last_ping": "2026-03-14T10:30:00Z"
  }
}
```

---

## Implementation Changes

### r8r Changes

| File | Change |
|------|--------|
| `src/api/websocket.rs` | Add `/api/ws/events` route with auth, event dispatch, single-client enforcement |
| `src/api/mod.rs` | Register new WebSocket route |
| `src/engine/executor.rs` | Emit `r8r.approval.requested` when hitting approval node |
| `src/engine/executor.rs` | Emit `r8r.execution.completed` and `r8r.execution.failed` on finish |
| `src/triggers/approval_timeout.rs` | Emit `r8r.approval.timeout` on expiry |
| `src/cli/diagnostics.rs` | Show bridge connection status |
| New: `src/bridge/mod.rs` | Bridge state, WebSocket event loop, replay buffer, ack tracking, backpressure |
| New: `src/bridge/events.rs` | Event enum, schema definitions, serialization |
| New: `src/bridge/handlers.rs` | Handle incoming events: `zeptoclaw.approval.decision` (resolve via existing approval logic), `zeptoclaw.workflow.trigger` (trigger execution), `zeptoclaw.health.ping` (respond with status) |

### ZeptoClaw Changes

| File | Change |
|------|--------|
| New: `src/r8r_bridge/mod.rs` | WebSocket client, connection lifecycle, reconnect logic |
| New: `src/r8r_bridge/events.rs` | Shared event types (mirrored from r8r) |
| New: `src/r8r_bridge/approval.rs` | Approval routing, channel formatting, response parsing |
| New: `src/r8r_bridge/health.rs` | Health ping loop, status tracking |
| `src/config/mod.rs` | Add `r8r_bridge` config section |
| `src/cli/mod.rs` | Show r8r bridge status in `zeptoclaw status` |
| `src/bus/mod.rs` | Route bridge events to/from message bus |
| `src/channels/*.rs` | No changes — bridge uses existing channel dispatch |

### Shared

Event schema JSON Schema file at `schemas/bridge-events.json` in both repos (or a shared location). Kept in sync manually for now — no shared crate.

---

## Testing Strategy

### r8r Tests

- Unit: Event serialization/deserialization round-trips
- Unit: Replay buffer (stores events, replays on reconnect, expires after 5 min)
- Unit: Ack tracking (marks events as acknowledged, excludes from replay)
- Integration: WebSocket connection with auth (valid token, invalid token, no token)
- Integration: Emit approval event -> receive on WS client -> send decision -> approval resolved
- Integration: Health ping/pong cycle

### ZeptoClaw Tests

- Unit: Event parsing and dispatch
- Unit: Event deduplication (skip already-processed IDs, eviction after 200 IDs / 10 min)
- Unit: Approval response keyword matching (approve, reject, edge cases, unrecognized input)
- Unit: Channel routing logic (user mapping, default, fallback)
- Unit: Reconnect backoff timing (1s, 2s, 4s... max 30s)
- Integration: Connect to mock r8r WS server, receive approval, respond
- Integration: Connection drop and reconnect with event replay + dedup

### End-to-End Test

1. Start r8r with a workflow that has an approval node
2. Start ZeptoClaw with r8r_bridge enabled (using a test channel)
3. Trigger the workflow
4. Verify ZeptoClaw receives approval request on test channel
5. Send approval response
6. Verify workflow resumes and completes

---

## Rollout

1. Both features ship behind config flags (`r8r_bridge.enabled` on ZeptoClaw, WebSocket events route exists but only emits if a client is connected on r8r)
2. No behavior change for users who don't configure the bridge
3. Existing REST API and MCP approval flows unchanged
4. Document setup in both READMEs

---

## Future Extensions (Not Phase 1)

- `zeptoclaw.channel.command` — pattern-matched channel messages trigger workflows directly
- `r8r.node.error` — per-node errors for ZeptoClaw diagnosis
- Redis transport option for multi-host deployments
- Shared event types crate (Rust) for compile-time schema validation
- Multiple ZeptoClaw clients connecting to one r8r instance (multi-tenant)
