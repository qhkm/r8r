# Sprint Closers Design
**Date:** 2026-03-11
**Status:** Approved
**Items:** CLI tests/docs (Sprint 1 closer) + Transcript-to-run correlation

---

## Item A: Tests + Docs for Exit-42 and `r8r prompt` Flags

### Problem
The exit-42 non-interactive behavior and `r8r prompt` flags (`--patch`, `--emit`, `--dry-run`, `--yes`, `--json`) are implemented but have no automated test coverage. Regressions would go undetected.

### Approach
Option 1: `assert_cmd` + `predicates` dev dependencies for ergonomic CLI subprocess testing, plus unit tests for pure logic.

### New Dependencies
```toml
[dev-dependencies]
assert_cmd = "2"
predicates = "3"
```

### Unit Tests — `tests/cli_flags_test.rs`
- `--emit`: assert YAML written to stdout, no storage write
- `--dry-run`: assert nothing saved
- `--json`: assert output is valid JSON with `name`/`yaml` keys
- Empty description: assert non-zero exit + error message on stderr

### Integration Tests — `tests/exit_code_test.rs`
- Spawn `r8r run <workflow>` with piped (non-TTY) stdin on a `waiting_for_approval` execution → assert exit code 42
- Spawn `r8r prompt --emit "fetch HN"` → assert stdout contains `nodes:`, exit 0
- Spawn `r8r prompt --yes "fetch HN"` with non-TTY stdin → assert no interactive prompt

### Docs — `docs/CLI.md` (new file)
- Table of all `r8r prompt` flags with description + example
- Exit code reference: `0` = success, `1` = error, `42` = waiting_for_approval (non-interactive)

---

## Item B: Transcript-to-Run Correlation + Redaction

### Problem
REPL sessions save chat messages to `repl_messages` but there is no link between a message and the workflow execution it triggered. Tool call arguments containing credentials (API keys, tokens) are also stored in plaintext.

### Approach
Option 1: Schema migration to add `run_id` column, update storage trait, wire `execution_id` from tool call responses into message rows, and redact credential-like fields before saving.

### Schema Migration
```sql
ALTER TABLE repl_messages ADD COLUMN run_id TEXT;
ALTER TABLE repl_messages ADD COLUMN redacted INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_repl_messages_run_id ON repl_messages(run_id);
```

### Model Change — `storage/models.rs`
```rust
pub struct ReplMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub token_count: Option<i64>,
    pub run_id: Option<String>,   // new
    pub redacted: bool,           // new
    pub created_at: DateTime<Utc>,
}
```

### Storage Trait Change
`save_repl_message` gains `run_id: Option<&str>` parameter. All existing call sites pass `None` by default; tool-call saves pass the correlated execution ID.

### Redaction Helper — `src/repl/tui.rs`
```rust
fn redact_credentials(v: &Value) -> Value { ... }
```
- Walks JSON recursively
- Keys matching `password|token|secret|key|api_key|authorization` → value replaced with `"[REDACTED]"`
- Sets `redacted = true` on the saved message row if any field was redacted

### Wiring in `tui.rs`
- Add `last_run_id: Option<String>` field to `ReplApp`
- When a tool call response contains `execution_id`, store it in `app.last_run_id`
- Pass `app.last_run_id.as_deref()` as `run_id` when saving the tool call message
- Run content through `redact_credentials()` before saving

---

## Out of Scope
- Operator mode stricter confirms — deferred/watching, arm/disarm mechanic is sufficient for now
- Querying sessions by `run_id` via REST API — follow-on work
