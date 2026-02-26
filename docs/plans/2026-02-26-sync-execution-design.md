# Synchronous Workflow Execution — Design

**Goal:** Make r8r seamless for AI agents and external services by returning workflow results inline, without polling.

**Architecture:** Enhance existing sync execution paths (already working) with better response formats. No new execution engine changes needed.

**Scope:** Webhook response mode + enhanced MCP tools.

---

## 1. Webhook Response Mode

### Current Behavior

`handle_webhook()` awaits execution but always returns execution metadata:

```json
{"execution_id": "...", "status": "...", "output": {...}, "duration_ms": ...}
```

### New Behavior

Add `response_mode` field to webhook trigger config:

```yaml
triggers:
  - type: webhook
    config:
      path: /api/summarize
      method: POST
      response_mode: output  # return last node's output as response body
```

- `response_mode: output` — success returns **200** with raw output JSON; failure returns **500** with `{"error": "..."}`; timeout returns **504** with `{"error": "workflow execution timed out"}`
- `response_mode: full` (default) — current behavior, backward-compatible
- Debounce + `response_mode: output` is invalid (debounce is fire-and-forget) — log warning at startup, fall back to `full`

### Files

- `src/triggers/webhook.rs` — parse `response_mode`, change response format in `execute_webhook_workflow()`

---

## 2. Enhanced MCP Tools

### Existing: `r8r_execute` — Fix Error Handling

- On failure: set `is_error: true` on `CallToolResult` (currently always false)
- Response format unchanged

### New: `r8r_run_and_wait`

- Parameters: `workflow` (name), `input` (optional JSON), `timeout_seconds` (optional)
- Returns: just the output value on success (not wrapped in execution metadata)
- On failure: `is_error: true` with actionable error message
- Description: "Execute a workflow and return the result. Blocks until completion."

### New: `r8r_discover`

- Parameters: `workflow` (name)
- Returns: workflow parameters (names, types, defaults, required), node list, trigger info
- On not found: `is_error: true`

### New: `r8r_lint`

- Parameters: `yaml` (workflow YAML string)
- Returns: list of errors/warnings, or "valid" confirmation
- Reuses existing validation logic with better error messages
- Validation errors are not tool errors (`is_error: false`)

### Files

- `src/mcp/tools.rs` — fix `r8r_execute`, add 3 new tools
- `src/mcp/mod.rs` — register new tools

---

## 3. Error Handling

| Scenario | Webhook (output mode) | MCP |
|----------|----------------------|-----|
| Success | 200 + raw output | `is_error: false` + output |
| Workflow failure | 500 + `{"error": "..."}` | `is_error: true` + error message |
| Timeout | 504 + `{"error": "..."}` | `is_error: true` + timeout message |
| Not found | 404 (unchanged) | `is_error: true` + not found |
| Debounce + output mode | Warning at startup, fall back to full | N/A |

---

## 4. Testing

- Unit tests for webhook response mode (mock executor, verify response format)
- Unit tests for each new MCP tool
- Integration test: webhook `response_mode: output` returns raw output
- Test debounce + response_mode: output warning
- Test `r8r_execute` returns `is_error: true` on failure

## 5. Non-Goals

- No streaming/SSE (separate future feature)
- No new execution engine changes
- No breaking changes to existing behavior
