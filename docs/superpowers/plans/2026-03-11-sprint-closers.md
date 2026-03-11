# Sprint Closers Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close Sprint 1 by adding tests + docs for `r8r prompt` flags and exit-42 behavior, then wire transcript-to-run correlation and credential redaction into the REPL storage layer.

**Architecture:** Two independent workstreams. Item A adds `assert_cmd`-based CLI tests and a `docs/CLI.md` reference. Item B extends the `repl_messages` schema with a `run_id` column, adds a credential redaction helper, and wires the execution ID from tool call results into saved message rows.

**Tech Stack:** Rust, SQLite (rusqlite), `assert_cmd` + `predicates` (new dev deps), `serde_json`

---

## Chunk 1: Item A — Dev deps + CLI unit tests

### Files
- Modify: `Cargo.toml` — add `assert_cmd`, `predicates` dev deps
- Create: `tests/cli_flags_test.rs` — unit-style tests for prompt flag validation and exit-42 logic
- Modify: `src/main.rs` — extract `should_exit_42()` pure function

---

### Task 1: Add dev dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dev deps**

Open `Cargo.toml`, find the `[dev-dependencies]` section (currently has `tokio-test` and `tempfile`), and add:

```toml
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check --tests
```

Expected: no errors. If version conflicts appear, try `assert_cmd = "2.0"`.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add assert_cmd and predicates dev dependencies"
```

---

### Task 2: Extract `should_exit_42` as a testable pure function

**Files:**
- Modify: `src/lib.rs` — add `should_exit_42` as a public library function
- Modify: `src/main.rs:2485-2491` — use the library function
- Create: `tests/cli_flags_test.rs`

> **Why `src/lib.rs` not `src/main.rs`?**
> `src/main.rs` is the binary crate entry point. Items defined there are not reachable from integration tests via `r8r::*`. All testable logic must live in the library crate (`src/lib.rs` or its modules).

- [ ] **Step 1: Write the failing test first**

Create `tests/cli_flags_test.rs`:

```rust
// Tests for r8r CLI flag behavior and exit-code semantics.
// Run with: cargo test --test cli_flags_test

#[test]
fn exit_42_when_waiting_for_approval_non_interactive() {
    assert!(r8r::should_exit_42("waiting_for_approval", false));
}

#[test]
fn no_exit_42_when_interactive() {
    assert!(!r8r::should_exit_42("waiting_for_approval", true));
}

#[test]
fn no_exit_42_when_status_is_completed() {
    assert!(!r8r::should_exit_42("completed", false));
}

#[test]
fn no_exit_42_when_status_is_failed() {
    assert!(!r8r::should_exit_42("failed", false));
}
```

- [ ] **Step 2: Run — expect compile error (function not found)**

```bash
cargo test --test cli_flags_test 2>&1 | head -20
```

Expected: `error[E0425]: cannot find function 'should_exit_42' in crate 'r8r'`

- [ ] **Step 3: Add `should_exit_42` to `src/lib.rs`**

Open `src/lib.rs` and add at the bottom:

```rust
/// Returns true if the process should exit with code 42.
/// Exported for testing.
pub fn should_exit_42(status: &str, is_interactive: bool) -> bool {
    status == "waiting_for_approval" && !is_interactive
}
```

- [ ] **Step 4: Update `src/main.rs` to use the library function**

Find this block (around line 2485):

```rust
// Fail-closed: non-interactive + waiting_for_approval → exit 42
if execution.status.to_string() == "waiting_for_approval" && !is_interactive() {
    eprintln!(
        "Execution {} requires human approval. Exiting with code 42 (non-interactive).",
        execution.id
    );
    std::process::exit(42);
}
```

Replace with:

```rust
if r8r::should_exit_42(&execution.status.to_string(), is_interactive()) {
    eprintln!(
        "Execution {} requires human approval. Exiting with code 42 (non-interactive).",
        execution.id
    );
    std::process::exit(42);
}
```

- [ ] **Step 5: Run tests — expect pass**

```bash
cargo test --test cli_flags_test
```

Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/main.rs tests/cli_flags_test.rs
git commit -m "test: add unit tests for exit-42 logic, extract should_exit_42 to library"
```

---

## Chunk 2: Item A — Integration tests for CLI flags

### Files
- Create: `tests/exit_code_test.rs` — subprocess tests using `assert_cmd`

---

### Task 3: CLI integration tests

These tests spawn the real `r8r` binary via `assert_cmd`. Tests that require a running server or API key are marked `#[ignore]` with a note.

- [ ] **Step 1: Write the test file**

Create `tests/exit_code_test.rs`:

```rust
//! Integration tests for `r8r` CLI flags and exit codes.
//! Run all: `cargo test --test exit_code_test`
//! Run ignoring slow/server tests: `cargo test --test exit_code_test` (ignored are skipped by default)

use assert_cmd::Command;
use predicates::prelude::*;

// ── r8r prompt ────────────────────────────────────────────────────────────────

#[test]
fn prompt_requires_description() {
    // `r8r prompt` with no arguments should exit non-zero and print usage hint.
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.arg("prompt");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_patch_requires_description() {
    // `r8r prompt --patch foo` with no description should exit non-zero.
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--patch", "some-workflow"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please describe the change"));
}

#[test]
fn prompt_emit_flag_is_recognised() {
    // `r8r prompt --emit` with no description should still fail on the description,
    // not on an unknown-flag error. Proves the flag is parsed.
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--emit"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_dry_run_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--dry-run"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_json_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--json"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

#[test]
fn prompt_yes_flag_is_recognised() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--yes"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Please provide a description"));
}

// ── flag output correctness (require LLM API key, skipped in CI) ─────────────

/// Verifies --emit writes valid YAML to stdout and exits 0.
/// Requires R8R_LLM_API_KEY to be set.
/// Run manually: `cargo test --test exit_code_test prompt_emit_writes_yaml_to_stdout -- --ignored`
#[test]
#[ignore = "requires LLM API key"]
fn prompt_emit_writes_yaml_to_stdout() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--emit", "--yes", "fetch HN top stories"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("nodes:"));
}

/// Verifies --dry-run does not save anything to storage.
/// Requires R8R_LLM_API_KEY to be set.
/// Run manually: `cargo test --test exit_code_test prompt_dry_run_saves_nothing -- --ignored`
#[test]
#[ignore = "requires LLM API key"]
fn prompt_dry_run_saves_nothing() {
    use std::env;
    // Use a temp dir as the data directory so we can inspect storage after the run.
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--dry-run", "--yes", "fetch HN top stories"])
        .env("R8R_DATA_DIR", dir.path());
    cmd.assert().success();
    // No .db file should have been written (dry-run skips storage).
    let db_count = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension().map_or(false, |x| x == "db"))
        .count();
    assert_eq!(db_count, 0, "dry-run should not write to storage");
}

// ── exit code 42 ─────────────────────────────────────────────────────────────

/// Full end-to-end exit-42 test. Requires a running r8r server with a workflow
/// containing an `approval` node. Run manually or in a dedicated CI job.
///
/// To run: `cargo test --test exit_code_test exit_42_end_to_end -- --ignored`
#[test]
#[ignore = "requires running r8r server with approval workflow"]
fn exit_42_end_to_end() {
    // Workflow `needs-approval` must contain an approval node.
    // Run non-interactively (piped stdin → not a TTY).
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["run", "needs-approval"])
        .write_stdin("") // piped stdin → is_interactive() returns false
        .env("R8R_BASE_URL", "http://localhost:3000");
    cmd.assert().code(42);
}

// ── help ──────────────────────────────────────────────────────────────────────

#[test]
fn help_flag_exits_zero() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.arg("--help");
    cmd.assert().success();
}

#[test]
fn prompt_help_mentions_emit_flag() {
    let mut cmd = Command::cargo_bin("r8r").unwrap();
    cmd.args(["prompt", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--emit"));
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --test exit_code_test
```

Expected: all non-`#[ignore]` tests pass. The `exit_42_end_to_end` test is skipped.

If a test fails with "binary not found", run `cargo build` first — `assert_cmd` needs the binary compiled.

- [ ] **Step 3: Commit**

```bash
git add tests/exit_code_test.rs
git commit -m "test: add CLI integration tests for prompt flags and exit-42 contract"
```

---

## Chunk 3: Item A — CLI reference docs

### Files
- Create: `docs/CLI.md`

---

### Task 4: Write CLI.md

- [ ] **Step 1: Create `docs/CLI.md`**

```markdown
# r8r CLI Reference

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | Success |
| `1`  | Error (see stderr for details) |
| `42` | Execution requires human approval (non-interactive mode) |

### Exit 42 — Non-Interactive Approval Guard

When `r8r run` is used in a non-interactive context (e.g. CI, piped stdin, scripts) and the workflow reaches an `approval` node, the process exits with code **42** instead of blocking.

This lets orchestrators detect approval-required executions and handle them:

```bash
r8r run my-workflow
EXIT=$?
if [ $EXIT -eq 42 ]; then
  echo "Workflow is waiting for human approval"
  # notify team, open approval UI, etc.
fi
```

---

## `r8r prompt` — Generate or Refine Workflows

```
r8r prompt [FLAGS] <description...>
r8r prompt --patch <workflow-name> [FLAGS] <description...>
```

### Flags

| Flag | Description |
|------|-------------|
| `--patch <name>` | Refine an existing workflow instead of generating a new one |
| `--emit` | Print generated YAML to stdout and exit (no interactive loop) |
| `--dry-run` | Generate and validate YAML but do not save to storage |
| `--yes` | Skip the save confirmation prompt (non-interactive mode) |
| `--json` | Output result as JSON `{"name": "...", "yaml": "..."}` |

### Examples

```bash
# Generate a new workflow interactively
r8r prompt fetch HN top stories and post to Slack

# Emit YAML to stdout for piping or inspection
r8r prompt --emit summarize PDF and send email > workflow.yaml

# Refine an existing workflow non-interactively
r8r prompt --patch hn-to-slack --yes add retry with exponential backoff

# Generate and output as JSON (for agent consumption)
r8r prompt --json --yes create daily report workflow
```

---

## `r8r run` — Execute a Workflow

```
r8r run <workflow-name> [--yes] [--json]
```

In interactive mode, shows a side-effect summary before execution.
In non-interactive mode (`--yes` or piped stdin), skips the confirmation.
```

- [ ] **Step 2: Verify the file renders correctly**

```bash
cat docs/CLI.md
```

- [ ] **Step 3: Commit**

```bash
git add docs/CLI.md
git commit -m "docs: add CLI reference with exit code table and prompt flag docs"
```

- [ ] **Step 4: Update docs/TODO.md — mark Sprint 1 item as complete**

In `docs/TODO.md`, find:

```
- [ ] Add tests + docs for new prompt/run flags and non-interactive exit semantics
```

Change to:

```
- [x] Add tests + docs for new prompt/run flags and non-interactive exit semantics
```

```bash
git add docs/TODO.md
git commit -m "docs: mark sprint 1 CLI tests+docs as complete"
```

---

## Chunk 4: Item B — Schema migration + model update

### Files
- Modify: `src/storage/sqlite.rs` — add migration for `run_id` and `redacted` columns
- Modify: `src/storage/models.rs` — add fields to `ReplMessage`

---

### Task 5: Add `run_id` and `redacted` to `repl_messages`

- [ ] **Step 1: Write a failing test**

In `tests/repl_storage_test.rs` (file already exists), add at the bottom:

```rust
#[tokio::test]
async fn repl_message_stores_run_id() {
    use r8r::storage::{SqliteStorage, Storage};
    // Use in-memory DB — no tempdir needed.
    let db = SqliteStorage::open_in_memory().unwrap();
    // create_repl_session takes a model name and returns the session ID.
    let sess_id = db.create_repl_session("test-model").await.unwrap();
    db.save_repl_message(&sess_id, "user", "hello", None, Some("exec-abc"))
        .await
        .unwrap();
    let msgs = db.list_repl_messages(&sess_id, 10).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].run_id.as_deref(), Some("exec-abc"));
}

#[tokio::test]
async fn repl_message_run_id_defaults_to_none() {
    use r8r::storage::{SqliteStorage, Storage};
    let db = SqliteStorage::open_in_memory().unwrap();
    let sess_id = db.create_repl_session("test-model").await.unwrap();
    db.save_repl_message(&sess_id, "user", "hello", None, None)
        .await
        .unwrap();
    let msgs = db.list_repl_messages(&sess_id, 10).await.unwrap();
    assert_eq!(msgs[0].run_id, None);
}
```

- [ ] **Step 2: Run — expect compile errors**

```bash
cargo test --test repl_storage_test 2>&1 | head -30
```

Expected: errors about wrong number of arguments to `save_repl_message`.

- [ ] **Step 3: Add `run_id` and `redacted` fields to `ReplMessage`**

In `src/storage/models.rs`, find `pub struct ReplMessage` and update it:

```rust
pub struct ReplMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub token_count: Option<i64>,
    pub run_id: Option<String>,   // which execution triggered this message
    pub redacted: bool,           // true if credential fields were redacted
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 4: Add migration in `SqliteStorage::init()` in `src/storage/sqlite.rs`**

Find the `CREATE TABLE IF NOT EXISTS repl_messages` block (around line 191). After it, add these migration statements (SQLite `ALTER TABLE ... ADD COLUMN` is idempotent when wrapped in error-ignore):

```rust
// Migration: add run_id and redacted to repl_messages (idempotent)
let _ = conn.execute(
    "ALTER TABLE repl_messages ADD COLUMN run_id TEXT",
    [],
);
let _ = conn.execute(
    "ALTER TABLE repl_messages ADD COLUMN redacted INTEGER NOT NULL DEFAULT 0",
    [],
);
let _ = conn.execute(
    "CREATE INDEX IF NOT EXISTS idx_repl_messages_run_id ON repl_messages(run_id)",
    [],
);
```

- [ ] **Step 5: Update `save_repl_message` signature in `SqliteStorage`**

Find `pub async fn save_repl_message` (line ~1433). Update signature and SQL:

```rust
pub async fn save_repl_message(
    &self,
    session_id: &str,
    role: &str,
    content: &str,
    token_count: Option<i64>,
    run_id: Option<&str>,
) -> Result<()> {
    let conn = self.conn.lock().await;
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO repl_messages (id, session_id, role, content, token_count, run_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, session_id, role, content, token_count, run_id, now],
    )?;
    conn.execute(
        "UPDATE repl_sessions SET updated_at = ?1 WHERE id = ?2",
        params![now, session_id],
    )?;
    Ok(())
}
```

- [ ] **Step 6: Update `row_to_repl_message` to read the new columns**

Find `fn row_to_repl_message` (line ~1628):

```rust
fn row_to_repl_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReplMessage> {
    Ok(ReplMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        token_count: row.get(4)?,
        run_id: row.get(5)?,
        redacted: row.get::<_, i64>(6).unwrap_or(0) != 0,
        created_at: {
            let s: String = row.get(7)?;
            s.parse().unwrap_or_else(|_| Utc::now())
        },
    })
}
```

Also update the `SELECT` in `list_repl_messages` to include the new columns:

```sql
SELECT id, session_id, role, content, token_count, run_id, redacted, created_at
FROM repl_messages
WHERE session_id = ?1
ORDER BY created_at ASC
LIMIT ?2
```

- [ ] **Step 7: Update trait impl in `SqliteStorage` (the `Storage` trait delegation around line 1910)**

```rust
async fn save_repl_message(
    &self,
    session_id: &str,
    role: &str,
    content: &str,
    token_count: Option<i64>,
    run_id: Option<&str>,
) -> crate::error::Result<()> {
    SqliteStorage::save_repl_message(self, session_id, role, content, token_count, run_id).await
}
```

- [ ] **Step 8: Update the Storage trait signature in `src/storage/mod.rs`**

Find `async fn save_repl_message` in the trait definition and add `run_id: Option<&str>` to match.

- [ ] **Step 9: Fix all existing call sites — pass `None` for `run_id`**

There are 5 call sites in `src/repl/tui.rs` (lines 2372, 2382, 2417, 2586, 2634). For now, add `None` as the last argument to each.

> **Note:** Lines 2372 (tool_call) and 2382 (tool_result) will be updated again in Task 6 to pass the real `run_id`. The `None` here is temporary to make the code compile.

```bash
# Find all call sites to verify
grep -n "save_repl_message" src/repl/tui.rs
```

Update each to end with `, None)` or `, None,` as appropriate for the call style.

- [ ] **Step 10: Update `src/storage/postgres.rs`**

The PostgreSQL storage backend also implements `save_repl_message` directly (around line 1055). Update its signature and SQL to match:

```rust
async fn save_repl_message(
    &self,
    session_id: &str,
    role: &str,
    content: &str,
    token_count: Option<i64>,
    run_id: Option<&str>,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO repl_messages (id, session_id, role, content, token_count, run_id, created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(role)
    .bind(content)
    .bind(token_count)
    .bind(run_id)
    .bind(now)
    .execute(&self.pool)
    .await
    .map_err(sqlx_err)?;
    Ok(())
}
```

> **Note:** The Postgres migration (adding `run_id` and `redacted` columns to the live table) must be handled separately as a SQL migration file in your Postgres migration system. This plan only updates the code. Add the following SQL to your next Postgres migration:
> ```sql
> ALTER TABLE repl_messages ADD COLUMN IF NOT EXISTS run_id TEXT;
> ALTER TABLE repl_messages ADD COLUMN IF NOT EXISTS redacted BOOLEAN NOT NULL DEFAULT FALSE;
> CREATE INDEX IF NOT EXISTS idx_repl_messages_run_id ON repl_messages(run_id);
> ```

- [ ] **Step 11: Run the new tests**

```bash
cargo test --test repl_storage_test repl_message_stores_run_id repl_message_run_id_defaults_to_none
```

Expected: both pass.

- [ ] **Step 12: Compile check**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 13: Commit**

```bash
git add src/storage/models.rs src/storage/sqlite.rs src/storage/mod.rs src/storage/postgres.rs src/repl/tui.rs tests/repl_storage_test.rs
git commit -m "feat: add run_id and redacted columns to repl_messages with schema migration"
```

---

## Chunk 5: Item B — Redaction helper + run_id wiring

### Files
- Modify: `src/repl/tui.rs` — add `redact_credentials()`, add `last_run_id` to `ReplApp`, wire run_id into tool call saves

---

### Task 6: Add `redact_credentials` helper

- [ ] **Step 1: Write the failing test**

In `tests/repl_storage_test.rs`, add:

```rust
#[test]
fn redact_credentials_replaces_sensitive_keys() {
    use r8r::repl::tui::redact_credentials;
    use serde_json::json;

    let input = json!({
        "url": "https://api.example.com",
        "api_key": "sk-super-secret",
        "token": "bearer-abc123",
        "password": "hunter2",
        "username": "alice",
        "nested": {
            "secret": "shh",
            "data": "safe"
        }
    });

    let output = redact_credentials(&input);

    assert_eq!(output["url"], "https://api.example.com");
    assert_eq!(output["username"], "alice");
    assert_eq!(output["api_key"], "[REDACTED]");
    assert_eq!(output["token"], "[REDACTED]");
    assert_eq!(output["password"], "[REDACTED]");
    assert_eq!(output["nested"]["secret"], "[REDACTED]");
    assert_eq!(output["nested"]["data"], "safe");
}
```

- [ ] **Step 2: Run — expect compile error**

```bash
cargo test --test repl_storage_test redact_credentials_replaces_sensitive_keys 2>&1 | head -20
```

Expected: `error[E0425]: cannot find function 'redact_credentials'`

- [ ] **Step 3: Add `redact_credentials` to `src/repl/tui.rs`**

Add this function near the top of the file, after the imports:

```rust
/// Walks a JSON value and replaces values whose keys match common credential
/// patterns with `"[REDACTED]"`. Works recursively for nested objects.
pub fn redact_credentials(v: &serde_json::Value) -> serde_json::Value {
    const SENSITIVE: &[&str] = &[
        "password", "token", "secret", "api_key", "apikey",
        "authorization", "auth", "key", "credential", "private_key",
    ];

    match v {
        serde_json::Value::Object(map) => {
            let redacted: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, val)| {
                    let lower = k.to_lowercase();
                    if SENSITIVE.iter().any(|s| lower.contains(s)) {
                        (k.clone(), serde_json::Value::String("[REDACTED]".to_string()))
                    } else {
                        (k.clone(), redact_credentials(val))
                    }
                })
                .collect();
            serde_json::Value::Object(redacted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_credentials).collect())
        }
        other => other.clone(),
    }
}
```

- [ ] **Step 4: Run the test**

```bash
cargo test --test repl_storage_test redact_credentials_replaces_sensitive_keys
```

Expected: passes.

- [ ] **Step 5: Add `last_run_id` field to `ReplApp`**

In `src/repl/tui.rs`, find `pub struct ReplApp` (around line 258). Add:

```rust
/// Most recent execution_id returned by a tool call in this session.
pub last_run_id: Option<String>,
```

In the `ReplApp::new()` or wherever `ReplApp` is constructed (around line 332), initialise it:

```rust
last_run_id: None,
```

- [ ] **Step 6: Wire run_id and redaction at the tool_call save site**

Find the `StreamUpdate::ToolCallStart` handler (around line 2370):

```rust
if let StreamUpdate::ToolCallStart { name, args } = &update {
    storage
        .save_repl_message(
            &app.session_id,
            "tool_call",
            &json!({"name": name, "args": args}).to_string(),
            None,
            None,
        )
        .await?;
}
```

Replace with:

```rust
if let StreamUpdate::ToolCallStart { name, args } = &update {
    let redacted_args = redact_credentials(args);
    storage
        .save_repl_message(
            &app.session_id,
            "tool_call",
            &serde_json::json!({"name": name, "args": redacted_args}).to_string(),
            None,
            None, // run_id not yet known at call-start time
        )
        .await?;
}
```

- [ ] **Step 7: Wire run_id at the tool_result save site**

Find the `StreamUpdate::ToolCallResult` handler (around line 2380):

```rust
if let StreamUpdate::ToolCallResult { name, result } = &update {
    storage
        .save_repl_message(
            &app.session_id,
            "tool_result",
            &json!({"name": name, "result": result}).to_string(),
            None,
            None,
        )
        .await?;
}
```

Replace with:

```rust
if let StreamUpdate::ToolCallResult { name, result } = &update {
    // Extract execution_id from tool result if present, store for correlation.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(result) {
        if let Some(exec_id) = parsed.get("execution_id").and_then(|v| v.as_str()) {
            app.last_run_id = Some(exec_id.to_string());
        }
    }
    storage
        .save_repl_message(
            &app.session_id,
            "tool_result",
            &serde_json::json!({"name": name, "result": result}).to_string(),
            None,
            app.last_run_id.as_deref(),
        )
        .await?;
}
```

- [ ] **Step 8: Wire run_id at the assistant message save site (around line 2417)**

```rust
storage
    .save_repl_message(
        &app.session_id,
        "assistant",
        &compact,
        None,
        app.last_run_id.as_deref(),
    )
    .await?;
```

- [ ] **Step 9: Compile check**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 10: Run all tests**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 11: Commit**

```bash
git add src/repl/tui.rs
git commit -m "feat: wire run_id correlation and credential redaction into REPL message storage"
```

---

## Chunk 6: Wrap-up

### Task 7: Update TODO.md and memory

- [ ] **Step 1: Update `docs/TODO.md`**

Mark the transcript correlation item as complete. Find:

```
- [ ] Persist transcript-to-run correlation (`session_id`, `run_id`, tool calls redacted)
```

Change to:

```
- [x] Persist transcript-to-run correlation (`session_id`, `run_id`, tool calls redacted)
```

Also update the operator mode item to make clear it's deferred:

```
- [ ] Require stricter confirms only in operator mode or high-risk paths — **deferred**: arm/disarm mechanic is sufficient; revisit when destructive bulk ops exist
```

- [ ] **Step 2: Run the full test suite one final time**

```bash
cargo test
```

Expected: all tests pass, no warnings.

- [ ] **Step 3: Final commit**

```bash
git add docs/TODO.md
git commit -m "docs: mark sprint closers complete, defer operator-mode confirm item"
```

---

## Summary of all files changed

| File | Change |
|------|--------|
| `Cargo.toml` | Add `assert_cmd`, `predicates` dev deps |
| `src/lib.rs` | Add `pub fn should_exit_42()` to library crate |
| `src/main.rs` | Use `r8r::should_exit_42()` instead of inline check |
| `src/storage/models.rs` | Add `run_id`, `redacted` to `ReplMessage` |
| `src/storage/sqlite.rs` | Schema migration, updated `save_repl_message` + `row_to_repl_message` + `list_repl_messages` |
| `src/storage/postgres.rs` | Update `save_repl_message` signature + SQL to include `run_id` |
| `src/storage/mod.rs` | Update trait signature for `save_repl_message` |
| `src/repl/tui.rs` | Add `redact_credentials()`, `last_run_id` to `ReplApp`, wire redaction + run_id |
| `tests/cli_flags_test.rs` | New: unit tests for exit-42 logic |
| `tests/exit_code_test.rs` | New: CLI integration tests for prompt flags (+ `#[ignore]` for LLM-dependent tests) |
| `tests/repl_storage_test.rs` | Add: run_id storage tests + redaction tests |
| `docs/CLI.md` | New: CLI reference with exit codes and flag docs |
| `docs/TODO.md` | Mark items complete / defer operator mode |
