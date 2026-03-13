# Symbiotic Automation: ZeptoClaw + r8r

> Strategy document for pairing ZeptoClaw (AI brain) with r8r (workflow engine) to build best-in-class operations automation.

## Overview

ZeptoClaw and r8r form a complementary stack where each project handles what it does best:

| Dimension | ZeptoClaw | r8r |
|-----------|-----------|-----|
| **Role** | AI reasoning, conversation, decisions | Deterministic execution, orchestration, scheduling |
| **Binary size** | ~6MB | ~15MB |
| **RAM** | ~6MB idle | ~15MB idle |
| **Startup** | ~50ms | ~50ms |
| **Strength** | Multi-channel comms (9 channels), LLM reasoning (9 providers), memory, safety (9 layers) | DAG execution, data transformation (Rhai), triggers (cron/webhook/event), 30+ node types |
| **Weakness** | Expensive per-token for plumbing tasks | No conversational ability, no reasoning |
| **Config** | `~/.zeptoclaw/config.json` | `~/.config/r8r/config.toml` |
| **License** | Apache 2.0 | AGPL-3.0 (dual license available) |

### The Core Principle

> Don't burn LLM tokens on deterministic work. Use r8r for plumbing and call ZeptoClaw only when you need reasoning.

A single LLM call costs ~$0.01-0.10. An r8r HTTP node costs $0.00. A 10-step workflow with 1 agent step saves 90% of token costs vs. having the LLM do everything.

---

## Existing Integration Points

### ZeptoClaw -> r8r

ZeptoClaw includes an `R8rTool` that triggers r8r workflows:

```
User (Telegram) -> ZeptoClaw -> "This needs a workflow" -> R8rTool -> r8r executes -> result back to ZeptoClaw -> response to user
```

- Default endpoint: `http://localhost:8080`
- Configurable via `R8R_ENDPOINT` env var
- ZeptoClaw passes parameters, receives structured output

### r8r -> ZeptoClaw

r8r's `agent` node calls ZeptoClaw (or any LLM) for reasoning steps within a workflow:

```yaml
nodes:
  - id: classify
    type: agent
    provider: zeptoclaw  # or openai, anthropic, ollama
    config:
      prompt: "Classify this support ticket: {{input.body}}"
```

- ZeptoClaw and Ollama providers don't require credentials
- Supports JSON Schema response validation
- Token usage tracking per node

### Shared Infrastructure

- Both use AES-256-GCM for credential encryption
- Both support SSRF protection
- Both deploy as single binaries (no runtime deps)
- Both run on edge hardware (Raspberry Pi, Jetson)

---

## Gap Analysis: What's Missing

### 1. Channel -> Workflow Bridge (High Impact)

**Current state:** ZeptoClaw receives messages on 9 channels. To trigger an r8r workflow, ZeptoClaw's agent loop must reason about it, decide to use the R8rTool, construct parameters — all costing tokens.

**Desired state:** Declarative routing from channel events directly to r8r workflows, with ZeptoClaw reasoning only when needed.

```toml
# Proposed: ~/.zeptoclaw/routines/deploy.toml
[[routines]]
trigger = { channel = "telegram", pattern = "^deploy (\\w+)" }
action = { r8r_workflow = "deploy-service", params = { service = "$1" } }
confirm = true  # ZeptoClaw asks "Deploy {service} to staging?" before executing
```

**Why it matters:** Eliminates token cost for pattern-matchable commands. ZeptoClaw still handles ambiguous requests via normal agent loop.

### 2. Approval Channel Bridge (High Impact)

**Current state:** r8r's `approval` nodes pause workflows and expose pending approvals via API/MCP. But there's no automatic way to route approval requests to humans on their preferred channel.

**Desired state:** r8r approval events automatically appear in ZeptoClaw channels. User responds conversationally, ZeptoClaw resolves the approval.

```
r8r workflow pauses at approval node
  -> emits event: { type: "approval_needed", workflow: "deploy-prod", data: {...} }
  -> ZeptoClaw routine catches event
  -> ZeptoClaw sends to Telegram: "Deploy to prod requested by CI. Approve? (reply yes/no)"
  -> User replies "yes"
  -> ZeptoClaw calls r8r MCP tool: r8r_approve(id, "approved")
  -> Workflow resumes
```

**Why it matters:** Approval workflows are one of the highest-value automation patterns. Currently requires manual API calls or dashboard interaction.

### 3. Shared Credential Store (Medium Impact)

**Current state:** ZeptoClaw encrypts secrets in `~/.zeptoclaw/config.json`. r8r encrypts in its own store. API keys configured twice.

**Desired state:** Single credential store at `~/.config/kitakod/credentials.enc` that both tools read.

### 4. Unified Event Bus (Medium Impact)

**Current state:** r8r has an event/pub-sub system (in-memory or Redis). ZeptoClaw has a message bus. They don't talk to each other natively.

**Desired state:** Shared event namespace. ZeptoClaw channel events can trigger r8r workflows. r8r execution events can trigger ZeptoClaw routines.

```
┌─────────────┐    events    ┌──────────────┐
│  ZeptoClaw   │ <==========> │     r8r      │
│  Message Bus │              │  Event System │
└─────────────┘              └──────────────┘

Event types:
  zeptoclaw.channel.message    -> r8r can trigger workflows
  zeptoclaw.tool.completed     -> r8r can react to tool results
  r8r.execution.completed      -> ZeptoClaw can notify user
  r8r.approval.requested       -> ZeptoClaw can route to channel
  r8r.execution.failed         -> ZeptoClaw can alert + diagnose
```

### 5. Combined CLI / Unified Experience (Low-Medium Impact)

**Current state:** Two separate binaries (`zeptoclaw`, `r8r`) with independent CLIs.

**Options:**
- A) `kitakod` wrapper CLI that delegates to both
- B) `zeptoclaw workflow` subcommand that delegates to r8r
- C) Keep separate but ensure consistent UX (flags, output format, config paths)

Option B is most pragmatic — ZeptoClaw is the user-facing tool, r8r is the engine.

---

## Concrete Automation Tools

### Tier 1: Immediate Value (Build First)

#### ChatOps Commander

Receive commands from any channel, execute deterministic workflows, report results.

```
┌──────────┐     ┌───────────┐     ┌─────────┐     ┌──────────┐
│ Telegram │ --> │ ZeptoClaw │ --> │   r8r   │ --> │ ZeptoClaw│
│ "deploy  │     │ classifies│     │ runs    │     │ reports  │
│  staging"│     │ intent    │     │ workflow│     │ result   │
└──────────┘     └───────────┘     └─────────┘     └──────────┘
```

**ZeptoClaw handles:** Intent classification, parameter extraction, confirmation prompts, error explanation
**r8r handles:** Git operations, build pipeline, health checks, rollback logic, Slack notifications

Example r8r workflow (`deploy-service.yaml`):
```yaml
name: deploy-service
triggers:
  - type: event
    event: chatops.deploy

nodes:
  - id: validate
    type: http
    config:
      url: "https://api.github.com/repos/{{params.repo}}/branches/{{params.branch}}"
      headers:
        Authorization: "Bearer {{credentials.github_token}}"

  - id: build
    type: sandbox
    depends_on: [validate]
    config:
      runtime: docker
      image: node:20
      command: "npm ci && npm run build && npm test"

  - id: approve-prod
    type: approval
    depends_on: [build]
    config:
      message: "Build passed. Deploy {{params.service}} to {{params.env}}?"
      timeout: 30m
    # Only require approval for production
    when: "params.env == 'production'"

  - id: deploy
    type: http
    depends_on: [approve-prod]
    config:
      method: POST
      url: "https://deploy.internal/api/deploy"
      body:
        service: "{{params.service}}"
        env: "{{params.env}}"
        commit: "{{nodes.validate.output.sha}}"

  - id: notify
    type: slack
    depends_on: [deploy]
    config:
      channel: "#deployments"
      message: "Deployed {{params.service}} to {{params.env}} ({{nodes.validate.output.sha}})"
```

#### Incident Responder

Automated runbook execution triggered by alerts, with AI-powered diagnosis.

```
Alert (PagerDuty/Grafana webhook)
  -> r8r workflow starts
  -> HTTP nodes: check service health, fetch logs, check metrics
  -> Agent node (ZeptoClaw): "Given these logs and metrics, what's the likely cause?"
  -> Switch node: route by diagnosis
  -> Action nodes: restart service / scale up / page oncall
  -> ZeptoClaw: post summary to Slack with diagnosis + actions taken
```

**ZeptoClaw handles:** Log analysis, root cause reasoning, incident summary generation
**r8r handles:** Health checks, metric collection, service restarts, escalation chains, status page updates

#### Email Triage Pipeline

Process inbound emails, classify, route to correct workflow.

```
Email arrives (ZeptoClaw IMAP channel)
  -> ZeptoClaw: extract intent + entities
  -> Emit event to r8r with classification
  -> r8r switch node routes by type:
     invoice -> process-invoice.yaml (OCR, validate, store)
     support -> create-ticket.yaml (Jira/Linear, assign, SLA)
     meeting -> schedule-meeting.yaml (calendar check, propose times)
  -> ZeptoClaw: draft and send response
```

**ZeptoClaw handles:** Email parsing, intent classification, entity extraction, response drafting
**r8r handles:** Routing, API calls (Jira, calendar), data validation, SLA tracking

---

### Tier 2: High Value (Build Next)

#### Data Pipeline Monitor

Scheduled data quality checks with intelligent alerting.

```yaml
# r8r workflow: data-quality-check.yaml
name: data-quality-check
triggers:
  - type: cron
    expression: "0 */6 * * *"  # Every 6 hours

nodes:
  - id: fetch-metrics
    type: http
    config:
      url: "https://warehouse.internal/api/metrics"

  - id: fetch-history
    type: database
    config:
      query: "SELECT * FROM metric_history WHERE timestamp > datetime('now', '-7 days')"

  - id: analyze
    type: agent
    depends_on: [fetch-metrics, fetch-history]
    config:
      provider: zeptoclaw
      prompt: |
        Compare current metrics to 7-day history.
        Flag anomalies (>2 std dev from mean).
        Return JSON: { "anomalies": [...], "severity": "low|medium|high", "summary": "..." }
      response_format: json

  - id: route
    type: switch
    depends_on: [analyze]
    config:
      field: "nodes.analyze.output.severity"
      cases:
        high:
          - id: page-oncall
            type: http
            config:
              method: POST
              url: "https://api.pagerduty.com/incidents"
        medium:
          - id: slack-alert
            type: slack
            config:
              channel: "#data-alerts"
              message: "{{nodes.analyze.output.summary}}"
        low:
          - id: log-only
            type: debug
            config:
              message: "Minor anomaly detected: {{nodes.analyze.output.summary}}"
```

#### Multi-Channel Content Pipeline

Create content, review with AI, publish across platforms.

**r8r orchestrates:** Fetch data sources -> Generate draft (agent node) -> Review (agent node with different persona) -> Format per platform (transform nodes) -> Publish (HTTP nodes to Twitter, LinkedIn, etc.) -> Track engagement (scheduled follow-up)

**ZeptoClaw handles:** Content generation, editorial review, engagement analysis

#### IoT Sensor Pipeline

For edge deployments (Raspberry Pi, Jetson).

```
Sensor (MQTT/Serial)
  -> ZeptoClaw (MQTT channel) receives reading
  -> Emits event to r8r
  -> r8r workflow: transform -> threshold check -> aggregate -> route
  -> If anomaly: ZeptoClaw sends alert via Telegram
  -> If normal: r8r stores in database, updates dashboard
```

Both binaries run on the same device. Total footprint: ~21MB binary, ~21MB RAM.

---

### Tier 3: Strategic (Build When Foundation is Solid)

| Tool | Description |
|------|-------------|
| **Multi-Tenant SaaS Ops** | Per-tenant ZeptoClaw agent + r8r workflows. Tenant onboarding, usage monitoring, billing workflows. |
| **Compliance Auditor** | r8r fetches audit data on schedule, ZeptoClaw analyzes for compliance gaps, r8r generates reports and files tickets. |
| **Customer Success Bot** | ZeptoClaw monitors customer channels, r8r runs health-score workflows, ZeptoClaw proactively reaches out when scores drop. |
| **Self-Healing Infrastructure** | r8r monitors health metrics, ZeptoClaw diagnoses failures, r8r executes remediation playbooks. Closed-loop automation. |

---

## Implementation Roadmap

### Phase 1: Foundation (Weeks 1-2)

**Goal:** Make the existing integration production-solid.

| Task | Project | Description |
|------|---------|-------------|
| Standardize event schema | Both | Define shared event types (JSON Schema) for cross-project events |
| ZeptoClaw routine for r8r approvals | ZeptoClaw | Auto-route approval requests to configured channel |
| r8r event trigger for ZeptoClaw | r8r | Accept events from ZeptoClaw message bus |
| Shared credential format | Both | Document and implement `~/.config/kitakod/credentials.enc` |
| Health check integration | Both | `zeptoclaw status` shows r8r health, `r8r diagnostics` shows ZeptoClaw |

### Phase 2: Channel Router (Weeks 3-4)

**Goal:** Declarative channel-to-workflow routing without token cost.

| Task | Project | Description |
|------|---------|-------------|
| Routine pattern matcher | ZeptoClaw | Regex/glob matching on channel messages to trigger r8r |
| Parameter extraction | ZeptoClaw | Extract named groups from patterns as workflow params |
| Confirmation flow | ZeptoClaw | Optional "Are you sure?" before triggering destructive workflows |
| Fallback to agent | ZeptoClaw | Unmatched messages go through normal LLM reasoning |

### Phase 3: Automation Templates (Weeks 5-6)

**Goal:** Ship ready-to-use automation recipes.

| Template | Files |
|----------|-------|
| ChatOps Deploy | `workflows/chatops-deploy.yaml` + ZeptoClaw routine |
| Incident Response | `workflows/incident-response.yaml` + alert webhook |
| Email Triage | `workflows/email-triage.yaml` + ZeptoClaw email channel config |
| Data Monitor | `workflows/data-quality.yaml` + Slack alert config |
| Approval Bot | `workflows/approval-flow.yaml` + channel bridge config |

### Phase 4: Unified Experience (Weeks 7-8)

**Goal:** Feel like one product, not two.

| Task | Project | Description |
|------|---------|-------------|
| `zeptoclaw workflow` subcommand | ZeptoClaw | Delegates to r8r binary for workflow operations |
| Combined status dashboard | Both | Single TUI/web view showing agents + workflows |
| Shared config documentation | Both | `~/.config/kitakod/` as unified config home |
| Installation script | Both | Single `curl` install that sets up both |

---

## Shared Event Schema (Draft)

All events follow this structure:

```json
{
  "id": "evt_abc123",
  "type": "r8r.execution.completed",
  "source": "r8r",
  "timestamp": "2026-03-14T10:30:00Z",
  "data": {
    "workflow": "deploy-service",
    "execution_id": "exec_xyz",
    "status": "success",
    "output": { ... }
  },
  "metadata": {
    "tenant_id": "default",
    "correlation_id": "corr_456"
  }
}
```

### Event Types

| Event | Source | Consumer | Description |
|-------|--------|----------|-------------|
| `zeptoclaw.channel.message` | ZeptoClaw | r8r | Message received on any channel |
| `zeptoclaw.channel.command` | ZeptoClaw | r8r | Parsed command from channel (pattern-matched) |
| `zeptoclaw.tool.completed` | ZeptoClaw | r8r | Tool execution finished |
| `r8r.execution.started` | r8r | ZeptoClaw | Workflow execution began |
| `r8r.execution.completed` | r8r | ZeptoClaw | Workflow finished (success/failure) |
| `r8r.execution.failed` | r8r | ZeptoClaw | Workflow failed with error details |
| `r8r.approval.requested` | r8r | ZeptoClaw | Approval gate reached, waiting for decision |
| `r8r.approval.timeout` | r8r | ZeptoClaw | Approval timed out, escalation needed |
| `r8r.node.error` | r8r | ZeptoClaw | Individual node failed (for diagnosis) |

---

## Cost Model

### Token Savings

For a typical 10-step automation:

| Approach | Token Cost | Latency | Reliability |
|----------|-----------|---------|-------------|
| Pure LLM (ZeptoClaw does everything) | ~$0.05-0.50/run | 5-15s | Medium (LLM can hallucinate steps) |
| Hybrid (r8r + 1 agent step) | ~$0.005-0.05/run | 1-3s | High (deterministic + targeted reasoning) |
| Pure r8r (no LLM) | $0.00/run | <1s | Highest (fully deterministic) |

**Rule of thumb:** If a step can be expressed as a config (HTTP call, data transform, conditional), use r8r. If it requires judgment (classification, summarization, diagnosis), use an agent node.

### Resource Footprint

| Deployment | Binary Size | RAM (idle) | RAM (active) |
|------------|-------------|------------|--------------|
| ZeptoClaw only | 6MB | 6MB | 15-30MB |
| r8r only | 15MB | 15MB | 25-50MB |
| Both (same host) | 21MB | 21MB | 40-80MB |
| Both on Raspberry Pi 4 (4GB) | 21MB | 21MB | Works fine |

---

## Decision Framework: Which Tool Handles What?

```
Is the task deterministic (same input -> same output)?
  YES -> r8r workflow
    Examples: HTTP calls, data transforms, routing, scheduling, notifications

  NO -> Does it need conversation/memory?
    YES -> ZeptoClaw agent loop
      Examples: Chat support, email drafting, meeting scheduling

    NO -> r8r workflow with agent node
      Examples: Log analysis, content classification, anomaly detection
```

### Quick Reference

| Task | Tool | Why |
|------|------|-----|
| "Fetch data from API every hour" | r8r | Deterministic, scheduled |
| "Analyze this customer complaint" | ZeptoClaw | Requires reasoning, context |
| "Fetch metrics, detect anomalies, alert" | r8r + agent node | Pipeline is deterministic, detection needs AI |
| "Deploy to staging" | r8r (triggered by ZeptoClaw) | Deterministic steps, conversational trigger |
| "Help me write a workflow" | ZeptoClaw + r8r REPL | Creative task, uses r8r's generate tool |
| "Process invoices from email" | Both | ZeptoClaw extracts data, r8r processes pipeline |
| "Monitor Slack for deploy requests" | ZeptoClaw channel -> r8r | Channel handling + deterministic execution |
| "Restart service if health check fails" | r8r | Fully deterministic runbook |
| "Diagnose why the service is failing" | r8r collects data -> ZeptoClaw analyzes | Data collection is deterministic, diagnosis needs AI |

---

## Related Documentation

- [r8r Architecture](ARCHITECTURE.md) — System design and ZeptoClaw integration basics
- [r8r Node Types](NODE_TYPES.md) — All 30+ node configurations including agent node
- [r8r Use Cases](USE_CASES.md) — Concrete workflow examples
- [r8r Failure Routing](FAILURE_ROUTING.md) — Error handling between r8r and ZeptoClaw
- [ZeptoClaw README](https://github.com/kitakod/zeptoclaw) — Agent framework documentation
- [ZeptoClaw CLAUDE.md](../../zeptoclaw/CLAUDE.md) — Architecture reference
