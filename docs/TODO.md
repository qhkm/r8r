# r8r TODO

## CLI Placeholders

These commands are intentionally present but not implemented yet.
They should not report fake success.

1. `server` runtime
- Implement Axum HTTP server and real endpoints:
  - `GET /api/workflows`
  - `POST /api/workflows/:id/execute`
  - `GET /api/executions/:id`
- Wire scheduler/trigger lifecycle into server startup/shutdown.

2. `dev` mode
- Implement file watching for workflow YAML.
- Add hot reload and validation on changes.
- Add clear non-zero exit behavior for invalid workflows.

3. `credentials` commands
- Implement secure local credential storage (encrypted at rest).
- Add CRUD operations and masked list output.
- Add integration path for node credential resolution.

## Engine Reliability

1. Apply `condition`, `retry`, `timeout_seconds`, and `max_concurrency` semantics.
2. Ensure `for_each` non-array input fails deterministically (or is explicitly skipped) and always finalizes node execution state.
3. Add regression tests for failure paths and retry behavior.

## Improvements Over n8n

Completed in this phase:
- Workflow version snapshots on each definition change (`workflow_versions` table).
- Workflow rollback support (`r8r workflows rollback`).
- Workflow version history support (`r8r workflows history`).
- Execution replay support (`r8r workflows replay`).
- Execution trace retrieval support (`r8r workflows trace`).

Remaining:
1. Resume from failed node checkpoint (partial re-run).
2. Streaming executor for very large datasets (chunked processing).
3. Inline per-node fallback actions beyond `continue_on_error`.
4. Search/filter API for execution history.
