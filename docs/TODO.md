# r8r TODO

## CLI Placeholders

These commands are intentionally present but not implemented yet.
They should not report fake success.

1. ~~`server` runtime~~ ✓ DONE
- ~~Implement Axum HTTP server and real endpoints~~ ✓
  - GET /api/health
  - GET /api/workflows
  - GET /api/workflows/:name
  - POST /api/workflows/:name/execute
  - GET /api/executions/:id
  - GET /api/executions/:id/trace
- ~~Wire scheduler/trigger lifecycle into server startup/shutdown~~ ✓ DONE
  - Scheduler loads all enabled workflows with cron triggers on startup
  - Cron jobs execute workflows automatically
  - Graceful shutdown stops all scheduled jobs

2. ~~`dev` mode~~ ✓ DONE
- ~~Implement file watching for workflow YAML~~ ✓
- ~~Add hot reload and validation on changes~~ ✓
- Validation errors displayed but don't exit (for continuous development)

3. ~~`credentials` commands~~ ✓ DONE
- ~~Implement secure local credential storage~~ ✓ (base64-encoded in ~/.r8r/credentials.json)
- ~~Add CRUD operations and masked list output~~ ✓
- ~~Add integration path for node credential resolution~~ ✓ DONE
  - Credentials loaded from store during workflow execution
  - HTTP node supports `credential` and `auth_type` config fields
  - Auth types: bearer, basic, api_key, header:<name>
- ~~Add true encryption with master key (ring crate)~~ ✓ DONE
  - AES-256-GCM encryption for credential values
  - PBKDF2 key derivation from user password (100k iterations)
  - Master key stored encrypted in ~/.r8r/master.key
  - Backward compatible with legacy base64 credentials
  - Migration support: `migrate_to_encrypted()` method

## Engine Reliability

1. ~~Apply `condition`, `retry`, `timeout_seconds`, and `max_concurrency` semantics.~~ ✓ DONE
2. ~~Ensure `for_each` non-array input fails deterministically (or is explicitly skipped) and always finalizes node execution state.~~ ✓ DONE
3. ~~Add regression tests for failure paths and retry behavior.~~ ✓ DONE (24 executor tests)

## Improvements Over n8n

Completed in this phase:
- Workflow version snapshots on each definition change (`workflow_versions` table).
- Workflow rollback support (`r8r workflows rollback`).
- Workflow version history support (`r8r workflows history`).
- Execution replay support (`r8r workflows replay`).
- Execution trace retrieval support (`r8r workflows trace`).
- Cascading foreign keys for workflow/execution/version relationships.
- One-time startup migration to rebuild legacy non-cascade FK tables.
- One-time orphan-row repair during storage initialization.
- Database health command (`r8r db check`).
- Inline per-node fallback actions (`on_error.action: fallback` + `fallback_value`).
- Search/filter execution history API (`query_executions`) + CLI (`r8r workflows search`).

Remaining:
1. ~~Resume from failed node checkpoint (partial re-run).~~ ✓ DONE (`r8r workflows resume <execution_id>`)
2. ~~Streaming executor for very large datasets (chunked processing).~~ ✓ DONE (`settings.chunk_size`)

## Trigger System

1. ~~Cron trigger~~ ✓ DONE (via Scheduler)
   - Automatically registers cron jobs on server startup
   - Executes workflows on schedule
   - Graceful shutdown

2. ~~Webhook trigger~~ ✓ DONE
   - Dynamic route registration based on workflow definitions
   - Supports GET/POST/PUT/DELETE/PATCH methods
   - Custom path support: `/webhooks/{workflow_name}` or custom
   - Request body, headers, and method passed as workflow input

3. Event trigger - TODO
   - Redis pub/sub or similar message queue
   - Event filtering based on trigger config
