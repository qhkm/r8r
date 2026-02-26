# Workflow Testing & Mocking Framework Design

## Goal

Add an `r8r_test` MCP tool that lets agents validate workflows they create — run a workflow with test input, mock external calls via `pinned_data`, and assert expected output. One tool, zero extra files, fully isolated execution.

## Architecture

Single MCP tool (`r8r_test`) backed by an isolated test executor. Each test run creates a fresh in-memory SQLite storage and executor — no side effects to the real database. Mocking uses the existing `pinned_data` mechanism already built into the executor (nodes with `pinned_data` skip execution and use the pinned value as output).

## Tool Signature

```
r8r_test(
  workflow_yaml: String,          // Workflow YAML (may include pinned_data for mocking)
  input: Option<Value>,           // Test input (default: {})
  expected_output: Option<Value>, // If set, compare actual vs expected
  mode: Option<String>,           // "exact" (default) or "contains"
)
```

## Response Format

### Dry-run (no expected_output)

```json
{
  "status": "completed",
  "output": { "greeting": "Hello Alice!" },
  "duration_ms": 12,
  "nodes_executed": ["parse-input", "greet"]
}
```

### Test pass

```json
{
  "pass": true,
  "output": { "greeting": "Hello Alice!" },
  "duration_ms": 12,
  "nodes_executed": ["parse-input", "greet"]
}
```

### Test fail

```json
{
  "pass": false,
  "output": { "greeting": "Hello Bob!" },
  "expected": { "greeting": "Hello Alice!" },
  "diff": ["output.greeting: expected \"Hello Alice!\", got \"Hello Bob!\""],
  "duration_ms": 12,
  "nodes_executed": ["parse-input", "greet"]
}
```

### Execution failure

```json
{
  "status": "failed",
  "error": "Node 'fetch-data' failed: connection refused",
  "nodes_executed": ["parse-input"],
  "node_errors": { "fetch-data": "connection refused" }
}
```

## Mocking via pinned_data

No new mocking system. Agents mock external calls by setting `pinned_data` on nodes:

```yaml
nodes:
  - id: fetch-users
    type: http
    config:
      url: https://api.example.com/users
    pinned_data: [{"id": 1, "name": "Alice"}]
  - id: format
    type: transform
    config:
      expression: 'input.len()'
    depends_on: [fetch-users]
```

The `format` node receives pinned data from `fetch-users` as its input and runs normally. This tests workflow logic without making real HTTP calls.

## Comparison Logic

### Exact mode (default)

Deep JSON equality. Actual output must exactly equal expected output.

### Contains mode

Recursive subset check:
- **Objects**: every key in expected must exist in actual with matching value. Extra keys in actual are ignored.
- **Arrays**: exact match (order matters) for predictability.
- **Scalars**: exact equality.

Example: expected `{"greeting": "Hello"}` matches actual `{"greeting": "Hello", "source": "webhook", "timestamp": 123}`.

### Diff output

Flat list of human-readable paths when test fails:
- `"output.greeting: expected \"Hello Alice!\", got \"Hello Bob!\""`
- `"output.items[0].id: expected 1, got 2"`
- `"output.count: missing in actual output"`

## Error Handling

- Invalid YAML: `CallToolResult::error` with parse error + suggestions (reuse `r8r_lint` logic)
- Validation failure: `CallToolResult::error` with validation details
- Execution failure: structured error response with `node_errors` map
- Comparison failure: `pass: false` with diff (valid test result, NOT an error)

All errors use `CallToolResult::error` (tool-level, not protocol-level).

## Test Isolation

Each `r8r_test` call creates:
- Fresh `SqliteStorage::open_in_memory()`
- Fresh `NodeRegistry`
- Fresh `Executor`

No state leaks between test runs. No writes to the real database.

## Scope

This is deliberately minimal — one tool, one purpose. Future additions (test suites, batching, stored test cases, CI output) can build on this primitive if needed.
