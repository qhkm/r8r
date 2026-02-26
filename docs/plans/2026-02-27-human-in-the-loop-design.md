# Human-in-the-Loop Node Design

## Goal

Add an `approval` node type that pauses workflow execution until a human (or agent) approves or rejects. Agents manage the approval loop via MCP tools. Supports configurable timeouts with default actions.

## Architecture

Leverages the existing pause/resume/checkpoint infrastructure in the executor. The `approval` node creates an approval request in the DB, then the executor pauses at the node boundary. An agent (or human via API) calls `r8r_approve` to resolve the request and resume execution. A background timeout checker handles expired approvals.

## Approval Node

### YAML Config

```yaml
- id: manager-approval
  type: approval
  config:
    title: "Approve large order"
    description: "Order #{{ input.order_id }} exceeds $1000 threshold"
    timeout_seconds: 3600
    default_action: "reject"
```

- `title` (required): Short label for the approval request
- `description` (optional): Details about what needs approval, supports template expressions
- `timeout_seconds` (optional): Auto-resolve after this many seconds
- `default_action` (required if timeout set): "approve" or "reject"

### Execution Flow

1. Executor calls `ApprovalNode::execute()`
2. Node creates an `ApprovalRequest` record in DB with status "pending"
3. Node returns `NodeResult` with `{"approval_id": "...", "status": "pending"}`
4. Executor detects node type is "approval" and pauses execution (saves checkpoint)
5. Execution status becomes `Paused`

### Resume Flow

1. Agent calls `r8r_approve(approval_id, decision, comment)`
2. System updates approval request with decision
3. System updates the approval node's output to include the decision
4. System resumes execution from checkpoint
5. Downstream nodes receive: `{"approval_id": "...", "decision": "approved", "comment": "LGTM", "decided_by": "agent"}`

## Storage Schema

### `approval_requests` Table

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUID |
| execution_id | TEXT | Links to execution |
| node_id | TEXT | Which approval node |
| workflow_name | TEXT | For display |
| title | TEXT | Human-readable title |
| description | TEXT | Details |
| status | TEXT | "pending", "approved", "rejected", "expired" |
| decision_comment | TEXT | Optional comment |
| decided_by | TEXT | Who decided (agent name, "timeout") |
| context_data | TEXT (JSON) | Snapshot of node input |
| created_at | DATETIME | When created |
| expires_at | DATETIME | Timeout deadline (null = no timeout) |
| decided_at | DATETIME | When resolved |

## MCP Tools

### `r8r_list_approvals`

```
r8r_list_approvals(status: Option<String>)
```

Lists approval requests filtered by status (default: "pending"). Returns array of `{id, workflow_name, title, description, status, created_at, expires_at}`.

### `r8r_approve`

```
r8r_approve(approval_id: String, decision: String, comment: Option<String>)
```

Resolves an approval. `decision` must be "approve" or "reject". Updates the approval request, injects decision into the approval node's output, and resumes the paused execution.

## Timeout Checker

Background task spawned on server startup. Runs every 30 seconds:

1. Query pending approvals where `expires_at < now()`
2. For each expired approval: set status to "expired", apply `default_action`
3. Resume the associated execution with the default decision

## Executor Integration

After a node of type "approval" executes, the executor checks if the result indicates a pending approval. If so, it pauses execution using the existing pause mechanism (saves checkpoint, sets status to Paused).

## Testing

1. **ApprovalNode unit tests** - creates request, returns correct output
2. **MCP tool tests** - list empty, list with pending, approve, reject
3. **Timeout test** - expired approval auto-resolves
4. **E2E test** - workflow with approval node pauses, approve via MCP, downstream receives decision

## Scope

No dashboard UI, no email/Slack notifications, no delegation/routing. Agents manage approvals via MCP tools. Notifications can be added as a separate enhancement.
