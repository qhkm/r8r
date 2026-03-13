# World-Class CLI Improvements Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 5 high-ROI CLI improvements — Elm-style error diagnostics, global `--json`, init wizard, token usage tracking, and contextual suggestions.

**Architecture:** Layer improvements into the existing Clap 4 + ratatui architecture. New `src/cli/` module for output and diagnostics abstractions. Thread `Output` through `cmd_*` functions. Extend LLM streaming to propagate token usage to TUI.

**Tech Stack:** Rust, Clap 4, ratatui, strsim (new), serde_json, reqwest (existing)

**Spec:** `docs/superpowers/specs/2026-03-13-world-class-cli-design.md`

---

## Chunk 1: Foundation (Output + Diagnostics + Dependency)

### Task 1: Add `strsim` dependency

**Files:**
- Modify: `Cargo.toml` (add under `# Utilities` section, near `uuid`)

- [ ] **Step 1: Add strsim to Cargo.toml**

Add under the `# Utilities` section after `uuid`:

```toml
strsim = "0.11"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles successfully (strsim has no transitive deps)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add strsim dependency for fuzzy matching"
```

---

### Task 2: Create `src/cli/mod.rs` module

**Files:**
- Create: `src/cli/mod.rs`
- Modify: `src/lib.rs` (add `pub mod cli;` in alphabetical order among existing `pub mod` declarations)

- [ ] **Step 1: Create the cli module directory**

```bash
mkdir -p src/cli
```

- [ ] **Step 2: Write `src/cli/mod.rs`**

```rust
pub mod diagnostics;
pub mod output;
```

- [ ] **Step 3: Register the module in `src/lib.rs`**

Add `pub mod cli;` at line 39 (before `pub mod api;` or in alphabetical order among the existing modules):

```rust
pub mod api;
pub mod cli;
pub mod config;
```

- [ ] **Step 4: Create empty placeholder files so the project compiles**

Create `src/cli/output.rs`:
```rust
// Output abstraction for CLI commands — human-readable or JSON.
```

Create `src/cli/diagnostics.rs`:
```rust
// Elm-style error diagnostics with fuzzy matching and recovery hints.
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles (empty modules are valid)

- [ ] **Step 6: Commit**

```bash
git add src/cli/ src/lib.rs
git commit -m "feat(cli): scaffold cli module with output and diagnostics stubs"
```

---

### Task 3: Implement `src/cli/output.rs`

**Files:**
- Modify: `src/cli/output.rs`

- [ ] **Step 1: Write the test**

Create `tests/cli_output_test.rs`:

```rust
use r8r::cli::output::{Output, OutputMode};
use serde::Serialize;

#[derive(Serialize)]
struct TestItem {
    name: String,
    count: u32,
}

#[test]
fn test_output_human_success() {
    let out = Output::new(OutputMode::Human);
    // success() prints to stdout — just verify it doesn't panic
    out.success("done");
}

#[test]
fn test_output_json_success() {
    let out = Output::new(OutputMode::Json);
    out.success("done");
}

#[test]
fn test_output_json_list() {
    let out = Output::new(OutputMode::Json);
    let items = vec![
        TestItem { name: "a".into(), count: 1 },
        TestItem { name: "b".into(), count: 2 },
    ];
    out.list(
        &["NAME", "COUNT"],
        &items,
        |item| vec![item.name.clone(), item.count.to_string()],
    );
}

#[test]
fn test_output_suggest_suppressed_in_json() {
    let out = Output::new(OutputMode::Json);
    // Should not panic or print anything
    out.suggest(&[("r8r help", "show help")]);
}

#[test]
fn test_output_is_json() {
    let human = Output::new(OutputMode::Human);
    let json = Output::new(OutputMode::Json);
    assert!(!human.is_json());
    assert!(json.is_json());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_output_test 2>&1 | tail -10`
Expected: FAIL — `Output`, `OutputMode` not found

- [ ] **Step 3: Implement `Output`**

Write `src/cli/output.rs`:

```rust
use serde::Serialize;

/// Output mode — human-readable text or machine-readable JSON.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

/// Abstraction for CLI command output.
///
/// Commands use this instead of raw `println!` to support both
/// human-readable tables and `--json` structured output.
pub struct Output {
    mode: OutputMode,
}

impl Output {
    pub fn new(mode: OutputMode) -> Self {
        Self { mode }
    }

    pub fn is_json(&self) -> bool {
        matches!(self.mode, OutputMode::Json)
    }

    /// Print a success message.
    /// Human: "✓ {msg}"    JSON: {"ok": true, "message": "..."}
    pub fn success(&self, msg: &str) {
        match self.mode {
            OutputMode::Human => println!("\u{2713} {}", msg),
            OutputMode::Json => {
                let obj = serde_json::json!({"ok": true, "message": msg});
                println!("{}", obj);
            }
        }
    }

    /// Print a list of items.
    /// Human: formatted table with headers.   JSON: array of objects.
    pub fn list<T: Serialize>(
        &self,
        headers: &[&str],
        rows: &[T],
        format_row: fn(&T) -> Vec<String>,
    ) {
        match self.mode {
            OutputMode::Human => {
                if rows.is_empty() {
                    return;
                }
                // Calculate column widths from headers and data
                let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
                let formatted: Vec<Vec<String>> = rows.iter().map(format_row).collect();
                for row in &formatted {
                    for (i, cell) in row.iter().enumerate() {
                        if i < widths.len() {
                            widths[i] = widths[i].max(cell.len());
                        }
                    }
                }
                // Print header
                let header_line: String = headers
                    .iter()
                    .zip(&widths)
                    .map(|(h, w)| format!("{:<width$}", h, width = w))
                    .collect::<Vec<_>>()
                    .join("  ");
                println!("{}", header_line);
                println!("{}", "\u{2500}".repeat(header_line.len()));
                // Print rows
                for row in &formatted {
                    let line: String = row
                        .iter()
                        .zip(&widths)
                        .map(|(cell, w)| format!("{:<width$}", cell, width = w))
                        .collect::<Vec<_>>()
                        .join("  ");
                    println!("{}", line);
                }
            }
            OutputMode::Json => {
                let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                println!("{}", json);
            }
        }
    }

    /// Print a single item.
    /// Human: custom format.   JSON: serialized object.
    pub fn item<T: Serialize>(&self, item: &T, format_human: fn(&T) -> String) {
        match self.mode {
            OutputMode::Human => println!("{}", format_human(item)),
            OutputMode::Json => {
                let json = serde_json::to_string_pretty(item)
                    .unwrap_or_else(|_| "{}".into());
                println!("{}", json);
            }
        }
    }

    /// Print contextual next-step hints.
    /// Suppressed in JSON mode. Printed to stderr so stdout stays clean for piping.
    pub fn suggest(&self, hints: &[(&str, &str)]) {
        if self.is_json() {
            return;
        }
        eprintln!();
        eprintln!("  Next steps:");
        for (cmd, desc) in hints {
            eprintln!("    {}  \u{2014} {}", cmd, desc);
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli_output_test 2>&1 | tail -10`
Expected: all 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli/output.rs tests/cli_output_test.rs
git commit -m "feat(cli): implement Output abstraction for human/JSON dual-mode"
```

---

### Task 4: Implement `src/cli/diagnostics.rs`

**Files:**
- Modify: `src/cli/diagnostics.rs`
- Create: `tests/cli_diagnostics_test.rs`

- [ ] **Step 1: Write the tests**

Create `tests/cli_diagnostics_test.rs`:

```rust
use r8r::cli::diagnostics::{Diagnostic, DiagnosticKind, classify_error, fuzzy_match};

#[test]
fn test_classify_workflow_not_found() {
    let kind = classify_error("WORKFLOW_ERROR", "Workflow error: workflow 'my-wf' not found");
    match kind {
        DiagnosticKind::WorkflowNotFound { name } => assert_eq!(name, "my-wf"),
        other => panic!("Expected WorkflowNotFound, got {:?}", other),
    }
}

#[test]
fn test_classify_no_llm_configured() {
    let kind = classify_error("CONFIG_ERROR", "Configuration error: no LLM provider configured");
    assert!(matches!(kind, DiagnosticKind::NoLlmConfigured));
}

#[test]
fn test_classify_yaml_parse_error() {
    let kind = classify_error("YAML_ERROR", "YAML error: mapping values are not allowed in this context at line 5 column 3");
    match kind {
        DiagnosticKind::YamlParseError { line, column } => {
            assert_eq!(line, Some(5));
            assert_eq!(column, Some(3));
        }
        other => panic!("Expected YamlParseError, got {:?}", other),
    }
}

#[test]
fn test_classify_generic_fallback() {
    let kind = classify_error("INTERNAL_ERROR", "something unexpected");
    assert!(matches!(kind, DiagnosticKind::Generic));
}

#[test]
fn test_fuzzy_match_finds_similar() {
    let candidates = vec!["my-workflow", "other-workflow", "test-wf"];
    let matches = fuzzy_match("my-workflo", &candidates, 0.7);
    assert!(matches.contains(&"my-workflow".to_string()));
}

#[test]
fn test_fuzzy_match_no_match_below_threshold() {
    let candidates = vec!["completely-different"];
    let matches = fuzzy_match("my-workflow", &candidates, 0.7);
    assert!(matches.is_empty());
}

#[test]
fn test_diagnostic_from_error_enriches_workflow_not_found() {
    let err = r8r::error::Error::Workflow("workflow 'test-wf' not found".into());
    let diag = Diagnostic::from_error(&err, &["test-workflow", "test-wf-2"]);
    assert!(!diag.hints.is_empty());
    assert!(diag.hints.iter().any(|h| h.contains("workflows list")));
}

#[test]
fn test_diagnostic_from_error_generic_has_no_suggestions() {
    let err = r8r::error::Error::Internal("something broke".into());
    let diag = Diagnostic::from_error(&err, &[]);
    assert!(diag.suggestions.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_diagnostics_test 2>&1 | tail -10`
Expected: FAIL — types not found

- [ ] **Step 3: Implement diagnostics module**

Write `src/cli/diagnostics.rs`:

```rust
use crate::cli::output::OutputMode;
use crate::error::Error;

/// Specific error scenarios for enrichment.
#[derive(Debug)]
pub enum DiagnosticKind {
    WorkflowNotFound { name: String },
    NoLlmConfigured,
    YamlParseError { line: Option<usize>, column: Option<usize> },
    CredentialMissing { service: String },
    ConnectionRefused { url: String },
    DbNotInitialized,
    Generic,
}

/// An enriched error with recovery hints and fuzzy suggestions.
#[derive(Debug)]
pub struct Diagnostic {
    pub code: &'static str,
    pub kind: DiagnosticKind,
    pub message: String,
    pub hints: Vec<String>,
    pub suggestions: Vec<String>,
}

impl Diagnostic {
    /// Build a diagnostic from an r8r Error, with optional candidate names for fuzzy matching.
    pub fn from_error(err: &Error, candidates: &[&str]) -> Self {
        let code = err.code();
        let message = err.to_string();
        let kind = classify_error(code, &message);
        let mut diag = Diagnostic {
            code,
            kind,
            message,
            hints: vec![],
            suggestions: vec![],
        };
        diag.enrich(candidates);
        diag
    }

    fn enrich(&mut self, candidates: &[&str]) {
        match &self.kind {
            DiagnosticKind::WorkflowNotFound { name } => {
                self.suggestions = fuzzy_match(name, candidates, 0.7);
                self.hints.push("r8r workflows list    \u{2014} show all workflows".into());
                self.hints.push("r8r create \"...\"      \u{2014} generate a new one".into());
            }
            DiagnosticKind::NoLlmConfigured => {
                self.hints.push("r8r credentials set openai --key <your-key>".into());
                self.hints.push("ollama serve           \u{2014} start local Ollama".into());
                self.hints.push("r8r init               \u{2014} interactive setup".into());
            }
            DiagnosticKind::YamlParseError { .. } => {
                self.hints.push("r8r lint <file>        \u{2014} validate workflow YAML".into());
            }
            DiagnosticKind::CredentialMissing { service } => {
                self.hints.push(format!("r8r credentials set {} -v <value>", service));
            }
            DiagnosticKind::ConnectionRefused { .. } => {
                self.hints.push("r8r server             \u{2014} start the server".into());
                self.hints.push("Check that the port is not in use".into());
            }
            DiagnosticKind::DbNotInitialized => {
                self.hints.push("r8r doctor             \u{2014} run health checks".into());
            }
            DiagnosticKind::Generic => {}
        }
    }

    /// Render to stderr for human consumption (with color when supported).
    pub fn print(&self) {
        eprintln!();
        eprintln!("error[{}]: {}", self.code, self.message);
        if !self.suggestions.is_empty() {
            eprintln!();
            eprintln!("  Similar names:");
            for s in &self.suggestions {
                eprintln!("    \u{2022} {}", s);
            }
        }
        if !self.hints.is_empty() {
            eprintln!();
            for (i, hint) in self.hints.iter().enumerate() {
                if i == 0 {
                    eprintln!("  hint: {}", hint);
                } else {
                    eprintln!("        {}", hint);
                }
            }
        }
        eprintln!();
    }

    /// Render to stderr as JSON (for `--json` mode).
    pub fn print_json(&self) {
        let obj = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "hints": self.hints,
                "suggestions": self.suggestions,
            }
        });
        eprintln!("{}", obj);
    }

    /// Render using the appropriate mode.
    pub fn render(&self, mode: OutputMode) {
        match mode {
            OutputMode::Human => self.print(),
            OutputMode::Json => self.print_json(),
        }
    }
}

/// Classify an error by its code and message content into a specific DiagnosticKind.
pub fn classify_error(code: &str, message: &str) -> DiagnosticKind {
    match code {
        "WORKFLOW_ERROR" if message.contains("not found") => {
            DiagnosticKind::WorkflowNotFound {
                name: extract_quoted_name(message).unwrap_or_default(),
            }
        }
        "CONFIG_ERROR"
            if message.contains("LLM")
                || message.contains("provider")
                || message.contains("llm") =>
        {
            DiagnosticKind::NoLlmConfigured
        }
        "YAML_ERROR" => {
            let (line, column) = extract_yaml_position(message);
            DiagnosticKind::YamlParseError { line, column }
        }
        "CONFIG_ERROR" if message.contains("credential") => {
            DiagnosticKind::CredentialMissing {
                service: extract_quoted_name(message).unwrap_or_default(),
            }
        }
        "HTTP_ERROR" if message.contains("Connection refused") || message.contains("connect") => {
            DiagnosticKind::ConnectionRefused {
                url: extract_quoted_name(message).unwrap_or_default(),
            }
        }
        "DATABASE_ERROR" | "STORAGE_ERROR"
            if message.contains("no such table") || message.contains("not initialized") =>
        {
            DiagnosticKind::DbNotInitialized
        }
        _ => DiagnosticKind::Generic,
    }
}

/// Extract a single-quoted name from an error message (e.g., "workflow 'foo' not found" → "foo").
fn extract_quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Extract line and column from YAML error messages like "at line 5 column 3".
fn extract_yaml_position(message: &str) -> (Option<usize>, Option<usize>) {
    let line = extract_number_after(message, "line ");
    let column = extract_number_after(message, "column ");
    (line, column)
}

fn extract_number_after(text: &str, prefix: &str) -> Option<usize> {
    let idx = text.find(prefix)?;
    let rest = &text[idx + prefix.len()..];
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Fuzzy match an input against candidates using Jaro-Winkler similarity.
/// Returns candidates with similarity above the threshold, sorted by similarity (best first).
pub fn fuzzy_match(input: &str, candidates: &[&str], threshold: f64) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = candidates
        .iter()
        .filter_map(|&c| {
            let score = strsim::jaro_winkler(input, c);
            if score >= threshold {
                Some((score, c))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, name)| name.to_string()).collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli_diagnostics_test 2>&1 | tail -15`
Expected: all 8 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli/diagnostics.rs tests/cli_diagnostics_test.rs
git commit -m "feat(cli): implement Elm-style error diagnostics with fuzzy matching"
```

---

## Chunk 2: Global `--json` Flag + Main Integration

### Task 5: Add global `--json` flag to Cli struct

**Files:**
- Modify: `src/main.rs` (`Cli` struct, `Commands` enum, `main()` function)

- [ ] **Step 1: Add `json` field to `Cli` struct**

In `src/main.rs`, modify the `Cli` struct (currently has only `command: Option<Commands>`):

```rust
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Output results as JSON
    #[arg(long, global = true)]
    json: bool,
}
```

- [ ] **Step 2: Add `Init` variant to `Commands` enum**

In `src/main.rs`, add to the `Commands` enum (after `Schema` variant, before the closing `}`):

```rust
    /// Set up r8r — detect services, configure LLM provider, install completions
    Init {
        /// Skip interactive prompts, auto-detect everything
        #[arg(long)]
        yes: bool,

        /// Force re-initialization even if config exists
        #[arg(long)]
        force: bool,
    },
```

- [ ] **Step 3: Create Output from global flag in main()**

In `src/main.rs`, modify `main()` to create Output from the json flag:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let for_chat_tui = matches!(cli.command, None | Some(Commands::Chat { .. }));
    init_logging(for_chat_tui);

    let output = r8r::cli::output::Output::new(if cli.json {
        r8r::cli::output::OutputMode::Json
    } else {
        r8r::cli::output::OutputMode::Human
    });

    match cli.command {
```

- [ ] **Step 4: Add Init match arm in main()**

In the `match cli.command` block, add before the closing of the match:

```rust
        Some(Commands::Init { yes, force }) => cmd_init(&output, yes, force).await?,
```

- [ ] **Step 5: Add stub `cmd_init` function**

At the bottom of `src/main.rs` (before the last `}`), add:

```rust
async fn cmd_init(
    _output: &r8r::cli::output::Output,
    _yes: bool,
    _force: bool,
) -> anyhow::Result<()> {
    println!("r8r init is not yet implemented");
    Ok(())
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles successfully

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): add global --json flag and Init command stub"
```

---

### Task 6: Migrate `cmd_workflows_list` to use Output

**Files:**
- Modify: `src/main.rs` (`WorkflowActions::List` call site and `cmd_workflows_list` function)

This demonstrates the migration pattern. Other commands follow the same pattern and can be migrated incrementally.

- [ ] **Step 1: Update the call site**

In the `WorkflowActions::List` match arm, change:
```rust
WorkflowActions::List => cmd_workflows_list().await?,
```
to:
```rust
WorkflowActions::List => cmd_workflows_list(&output).await?,
```

- [ ] **Step 2: Update function signature and body**

Replace the `cmd_workflows_list` function:

```rust
async fn cmd_workflows_list(output: &r8r::cli::output::Output) -> anyhow::Result<()> {
    let storage = get_storage()?;
    let workflows = storage.list_workflows().await?;

    if workflows.is_empty() {
        output.success("No workflows found. Create one with: r8r workflows create <file.yaml>");
        return Ok(());
    }

    output.list(
        &["NAME", "ENABLED", "UPDATED"],
        &workflows,
        |wf| {
            vec![
                wf.name.clone(),
                if wf.enabled { "yes".into() } else { "no".into() },
                wf.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            ]
        },
    );

    Ok(())
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): migrate cmd_workflows_list to use Output (--json support)"
```

---

### Task 7: Migrate `cmd_credentials_list` to use Output

**Files:**
- Modify: `src/main.rs` (call site in `CredentialActions::List` and `SecretsActions::List` match arms, and `cmd_credentials_list` function)

**Important:** `CredentialStore::list()` returns `Vec<&Credential>` (references). The `Credential` struct contains an encrypted `value` field that must NOT appear in JSON output. We create a redacted summary struct for serialization.

- [ ] **Step 1: Update call sites**

Change both `CredentialActions::List` and `SecretsActions::List` arms to pass `&output`:
```rust
CredentialActions::List => cmd_credentials_list(&output).await?,
// ...
SecretsActions::List => cmd_credentials_list(&output).await?,
```

- [ ] **Step 2: Update function with redacted projection struct**

```rust
async fn cmd_credentials_list(output: &r8r::cli::output::Output) -> anyhow::Result<()> {
    use r8r::credentials::CredentialStore;

    let store = CredentialStore::load().await?;
    let credentials = store.list();

    if credentials.is_empty() {
        output.success("No credentials stored. Add one with: r8r credentials set <service> -v <value>");
        return Ok(());
    }

    // Redacted projection — excludes the encrypted `value` field for safe JSON output
    #[derive(serde::Serialize)]
    struct CredentialSummary {
        service: String,
        key: String,
        updated_at: String,
    }

    let summaries: Vec<CredentialSummary> = credentials
        .iter()
        .map(|c| CredentialSummary {
            service: c.service.clone(),
            key: c.key.as_deref().unwrap_or("-").to_string(),
            updated_at: c.updated_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    output.list(
        &["SERVICE", "KEY", "UPDATED"],
        &summaries,
        |s| vec![s.service.clone(), s.key.clone(), s.updated_at.clone()],
    );

    Ok(())
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): migrate cmd_credentials_list to use Output"
```

---

### Task 8: Migrate `cmd_doctor` to use Output

**Files:**
- Modify: `src/main.rs` (`Commands::Doctor` call site and `cmd_doctor` function)

- [ ] **Step 1: Update call site**

Change:
```rust
Some(Commands::Doctor) => cmd_doctor().await?,
```
to:
```rust
Some(Commands::Doctor) => cmd_doctor(&output).await?,
```

- [ ] **Step 2: Update function signature**

Change the function signature to accept `output`:

```rust
async fn cmd_doctor(output: &r8r::cli::output::Output) -> anyhow::Result<()> {
```

At the end of `cmd_doctor`, after the existing summary `println!`, add JSON support:

```rust
    if output.is_json() {
        let checks_json: Vec<serde_json::Value> = checks
            .iter()
            .map(|(name, passed, msg)| {
                serde_json::json!({"name": name, "pass": passed, "message": msg})
            })
            .collect();
        println!("{}", serde_json::json!({"checks": checks_json, "pass": pass, "fail": fail}));
    }
```

Note: This requires collecting check results into a `checks: Vec<(String, bool, String)>` — modify the `report` closure to push into this vec as well as printing.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): migrate cmd_doctor to use Output (--json support)"
```

---

### Task 9: Wire diagnostic errors into main()

**Files:**
- Modify: `src/main.rs` (`main()` function — split into `main()` + `run_command()`)

- [ ] **Step 1: Replace the default error handler**

Currently `main()` returns `anyhow::Result<()>` and Clap/anyhow print errors. Wrap the match block to catch errors and render diagnostics:

```rust
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let for_chat_tui = matches!(cli.command, None | Some(Commands::Chat { .. }));
    init_logging(for_chat_tui);

    let output_mode = if cli.json {
        r8r::cli::output::OutputMode::Json
    } else {
        r8r::cli::output::OutputMode::Human
    };
    let output = r8r::cli::output::Output::new(output_mode);

    let result = run_command(cli.command, &output).await;

    if let Err(err) = result {
        // Try to downcast to r8r::Error for enriched diagnostics
        if let Some(r8r_err) = err.downcast_ref::<r8r::error::Error>() {
            let diag = r8r::cli::diagnostics::Diagnostic::from_error(r8r_err, &[]);
            diag.render(output_mode);
        } else {
            // Fallback for non-r8r errors
            eprintln!("error: {}", err);
        }
        std::process::exit(1);
    }
}

async fn run_command(
    command: Option<Commands>,
    output: &r8r::cli::output::Output,
) -> anyhow::Result<()> {
    match command {
        // ... existing match arms, now using output parameter ...
    }
}
```

This splits the existing `main()` into `main()` (error handling) and `run_command()` (dispatch). Move all the existing match arms into `run_command()`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 3: Run existing tests**

Run: `cargo test 2>&1 | tail -20`
Expected: all existing tests still pass

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): wire diagnostic error handler into main with Output"
```

---

## Chunk 3: Init Wizard

### Task 10: Implement `r8r init` wizard

**Files:**
- Create: `src/cli/init.rs`
- Modify: `src/cli/mod.rs` (add `pub mod init;`)
- Modify: `src/main.rs` (replace `cmd_init` stub)

- [ ] **Step 1: Write the test**

Create `tests/cli_init_test.rs`:

```rust
use r8r::cli::init::{detect_ollama, detect_shell, detect_env_key, DetectionResult};

#[test]
fn test_detect_shell() {
    let result = detect_shell();
    // Should always return a result on any system
    assert!(result.detected);
    assert!(!result.detail.is_empty());
}

#[test]
fn test_detect_env_key_missing() {
    // Use a key that certainly doesn't exist
    let result = detect_env_key("R8R_TEST_NONEXISTENT_KEY_12345");
    assert!(!result.detected);
}

#[test]
fn test_detect_env_key_present() {
    std::env::set_var("R8R_TEST_INIT_KEY", "test-value");
    let result = detect_env_key("R8R_TEST_INIT_KEY");
    assert!(result.detected);
    std::env::remove_var("R8R_TEST_INIT_KEY");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli_init_test 2>&1 | tail -10`
Expected: FAIL — module not found

- [ ] **Step 3: Add `pub mod init;` to `src/cli/mod.rs`**

```rust
pub mod diagnostics;
pub mod init;
pub mod output;
```

- [ ] **Step 4: Implement `src/cli/init.rs`**

```rust
use crate::cli::output::Output;
use crate::config;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// Result of detecting a service or capability.
pub struct DetectionResult {
    pub name: &'static str,
    pub detected: bool,
    pub detail: String,
}

/// Detect the user's shell from $SHELL.
pub fn detect_shell() -> DetectionResult {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = if shell.contains("zsh") {
        "zsh"
    } else if shell.contains("bash") {
        "bash"
    } else if shell.contains("fish") {
        "fish"
    } else {
        "unknown"
    };
    DetectionResult {
        name: "Shell",
        detected: !shell.is_empty(),
        detail: format!("{} detected", name),
    }
}

/// Detect an environment variable key.
pub fn detect_env_key(key: &str) -> DetectionResult {
    let present = std::env::var(key).is_ok();
    DetectionResult {
        name: "EnvKey",
        detected: present,
        detail: if present {
            "configured".into()
        } else {
            "not found".into()
        },
    }
}

/// Detect Ollama by calling its API.
pub async fn detect_ollama() -> DetectionResult {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let models: Vec<String> = body["models"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect();
            let detail = if models.is_empty() {
                "running (no models pulled)".into()
            } else {
                format!("running ({})", models.first().unwrap())
            };
            DetectionResult {
                name: "Ollama",
                detected: true,
                detail,
            }
        }
        _ => DetectionResult {
            name: "Ollama",
            detected: false,
            detail: "not running".into(),
        },
    }
}

/// Detect Docker by running `docker info`.
pub async fn detect_docker() -> DetectionResult {
    match tokio::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => DetectionResult {
            name: "Docker",
            detected: true,
            detail: "running (for sandboxed execution)".into(),
        },
        _ => DetectionResult {
            name: "Docker",
            detected: false,
            detail: "not available".into(),
        },
    }
}

fn print_detection(result: &DetectionResult) {
    let icon = if result.detected { "\u{2713}" } else { "\u{2717}" };
    println!("  {} {:<14} {}", icon, result.name, result.detail);
}

fn prompt_line(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok();
    line.trim().to_string()
}

/// Run the init wizard.
pub async fn run_init(output: &Output, yes: bool, force: bool) -> anyhow::Result<()> {
    let config_dir = config::Config::config_dir();
    let config_path = config_dir.join("config.toml");

    // Guard: existing config
    if config_path.exists() && !force {
        if yes {
            println!("Config already exists at {}. Use --force to overwrite.", config_path.display());
            return Ok(());
        }
        println!("Config already exists at {}.", config_path.display());
        let answer = prompt_line("Overwrite? [y/N] ");
        if answer.to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    println!();
    println!("  Welcome to r8r \u{2014} agent-native workflow engine");
    println!();
    println!("  Detecting your environment...");
    println!();

    // Detect services
    let ollama = detect_ollama().await;
    let openai = detect_env_key("OPENAI_API_KEY");
    let anthropic = detect_env_key("ANTHROPIC_API_KEY");
    let docker = detect_docker().await;
    let shell = detect_shell();

    // detect_ollama() already returns name: "Ollama", etc.
    print_detection(&ollama);
    // For env key detections, override the generic name with a descriptive one
    println!("  {} {:<14} {}", if openai.detected { "\u{2713}" } else { "\u{2717}" }, "OpenAI key", openai.detail);
    println!("  {} {:<14} {}", if anthropic.detected { "\u{2713}" } else { "\u{2717}" }, "Anthropic key", anthropic.detail);
    print_detection(&docker);
    print_detection(&shell);

    // Choose LLM provider
    println!();
    println!("  \u{2500}\u{2500}\u{2500} LLM Provider \u{2500}\u{2500}\u{2500}");
    println!();

    let provider: String;
    let model: String;

    if yes {
        // Auto-detect: env key > Ollama > none
        if openai.detected {
            provider = "openai".into();
            model = "gpt-4o".into();
        } else if anthropic.detected {
            provider = "anthropic".into();
            model = "claude-sonnet-4-20250514".into();
        } else if ollama.detected {
            provider = "ollama".into();
            model = "llama3.2".into();
        } else {
            provider = String::new();
            model = String::new();
            println!("  No LLM provider detected. You can configure one later.");
        }
    } else {
        // Interactive selection
        let mut options = Vec::new();
        if ollama.detected {
            options.push(("ollama", "Ollama (detected, free, local)"));
        }
        options.push(("openai", "OpenAI (requires API key)"));
        options.push(("anthropic", "Anthropic (requires API key)"));

        println!("  Which provider do you want to use?");
        for (i, (_, desc)) in options.iter().enumerate() {
            println!("    [{}] {}", i + 1, desc);
        }
        println!();

        let choice = prompt_line("  > ");
        let idx: usize = choice.parse().unwrap_or(1).saturating_sub(1);
        let (chosen_provider, _) = options.get(idx).unwrap_or(&options[0]);

        provider = chosen_provider.to_string();
        model = match provider.as_str() {
            "ollama" => "llama3.2".into(),
            "openai" => "gpt-4o".into(),
            "anthropic" => "claude-sonnet-4-20250514".into(),
            _ => String::new(),
        };

        if !provider.is_empty() {
            println!("  \u{2713} Using {} with {}", provider, model);
        }
    }

    // Write config
    println!();
    println!("  \u{2500}\u{2500}\u{2500} Config \u{2500}\u{2500}\u{2500}");
    println!();

    std::fs::create_dir_all(&config_dir)?;

    // IMPORTANT: The Config struct in src/config/mod.rs has `server`, `storage`,
    // `agent`, and `sandbox` fields — but NO `llm` field. LLM configuration is
    // handled via environment variables (OPENAI_API_KEY, R8R_LLM_BASE_URL, etc.)
    // and the REPL's /model and /apikey slash commands.
    //
    // Write a config.toml with the server section (which Config does parse),
    // plus helpful comments about LLM env vars — NOT a [llm] section that would
    // be silently ignored.
    let llm_env_hint = match provider.as_str() {
        "openai" => "# export OPENAI_API_KEY=<your-key>\n# export R8R_LLM_BASE_URL=https://api.openai.com/v1\n",
        "anthropic" => "# export ANTHROPIC_API_KEY=<your-key>\n",
        "ollama" => "# Ollama detected at localhost:11434 — no key needed\n",
        _ => "# Set OPENAI_API_KEY or ANTHROPIC_API_KEY, or start Ollama\n",
    };
    let config_content = format!(
        r#"# r8r configuration (generated by r8r init)

[server]
port = 8080
host = "127.0.0.1"

# LLM provider: {provider} / {model}
# Configure via environment variables:
{llm_env_hint}
"#,
    );
    std::fs::write(&config_path, &config_content)?;
    println!("  \u{2713} Wrote {}", config_path.display());

    // If the chosen provider needs an API key and it's not set, prompt
    if !yes && !provider.is_empty() && provider != "ollama" {
        let env_key = match provider.as_str() {
            "openai" => "OPENAI_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            _ => "",
        };
        if !env_key.is_empty() && std::env::var(env_key).is_err() {
            println!();
            let key_value = prompt_line(&format!("  Enter your {} (or press Enter to skip): ", env_key));
            if !key_value.is_empty() {
                // Store via credential system rather than env file
                let mut cred_store = crate::credentials::CredentialStore::load().await?;
                cred_store.set(&provider, None, &key_value).await?;
                println!("  \u{2713} API key stored securely via r8r credentials");
            } else {
                println!("  Skipped. Set {} later or use: r8r credentials set {}", env_key, provider);
            }
        }
    }

    // Final suggestions
    println!();
    output.suggest(&[
        ("r8r chat", "interactive AI assistant"),
        ("r8r create \"...\"", "generate a workflow"),
        ("r8r templates list", "browse ready-made templates"),
    ]);

    Ok(())
}
```

- [ ] **Step 5: Replace `cmd_init` stub in `src/main.rs`**

Replace the stub with:
```rust
async fn cmd_init(
    output: &r8r::cli::output::Output,
    yes: bool,
    force: bool,
) -> anyhow::Result<()> {
    r8r::cli::init::run_init(output, yes, force).await
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test cli_init_test 2>&1 | tail -10`
Expected: all 3 tests PASS

- [ ] **Step 7: Verify full compilation**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 8: Commit**

```bash
git add src/cli/init.rs src/cli/mod.rs src/main.rs tests/cli_init_test.rs
git commit -m "feat(cli): implement r8r init wizard with service detection"
```

---

## Chunk 4: Token Usage Tracking

### Task 11: Add `StreamEvent::Usage` variant and parse from providers

**Files:**
- Modify: `src/llm.rs` (`StreamEvent` enum, `build_openai_streaming_request()`, `parse_openai_sse_line()`, `parse_anthropic_sse_line()`, `stream_ndjson_lines()`)

- [ ] **Step 1: Add `Usage` variant to `StreamEvent`**

In `src/llm.rs` at line 153, add before `Done`:

```rust
    /// Token usage data from the final chunk.
    Usage(LlmUsage),
    /// Stream completed.
    Done,
```

- [ ] **Step 2: Add `stream_options` to OpenAI streaming request**

In `build_openai_streaming_request()` (around line 481-494), add `stream_options` to the request body JSON:

```json
"stream_options": {"include_usage": true}
```

Find the line where `"stream": true` is set and add below it.

- [ ] **Step 3: Parse usage from OpenAI streaming final chunk**

**IMPORTANT:** OpenAI's final usage chunk has `"choices": []` (empty array). The current `parse_openai_sse_line` accesses `v["choices"][0]["delta"]` first — on the final chunk, `delta` is null, so the function returns `None` before any usage-parsing code could run.

**Solution:** Add usage parsing at the TOP of `parse_openai_sse_line`, BEFORE the `choices[0].delta` processing:

```rust
pub fn parse_openai_sse_line(line: &str) -> Option<StreamEvent> {
    let data = line.strip_prefix("data: ")?;
    if data.trim() == "[DONE]" {
        return Some(StreamEvent::Done);
    }
    let v: serde_json::Value = serde_json::from_str(data).ok()?;

    // Check for usage in final chunk FIRST (before choices processing)
    if let Some(usage) = v.get("usage") {
        if !usage.is_null() {
            return Some(StreamEvent::Usage(LlmUsage {
                prompt_tokens: usage.get("prompt_tokens").and_then(|v| v.as_u64()),
                completion_tokens: usage.get("completion_tokens").and_then(|v| v.as_u64()),
            }));
        }
    }

    // Existing delta/tool_call processing follows...
    let delta = &v["choices"][0]["delta"];
    // ... rest of existing code unchanged
}
```

This ensures the usage event is captured from OpenAI's final chunk, which has an empty `choices` array.

- [ ] **Step 4: Add `message_delta` match arm to Anthropic parser**

In `parse_anthropic_sse_line()`, add a new match arm for `"message_delta"` (currently falls through to `_ => None`):

```rust
"message_delta" => {
    // NOTE: The local variable is `v` (the parsed JSON), not `data` (the raw string).
    if let Some(usage) = v.get("usage") {
        Some(StreamEvent::Usage(LlmUsage {
            prompt_tokens: usage.get("input_tokens").and_then(|t| t.as_u64()),
            completion_tokens: usage.get("output_tokens").and_then(|t| t.as_u64()),
        }))
    } else {
        None
    }
}
```

- [ ] **Step 5: Parse usage from Ollama final streaming chunk**

**IMPORTANT:** `parse_ollama_ndjson_line()` currently returns `StreamEvent::Done` when `done: true`. The streaming loop in `stream_ndjson_lines` checks `is_done = matches!(event, StreamEvent::Done)` to terminate. If we return `Usage` instead of `Done`, the stream will hang forever.

**Solution:** Do NOT modify `parse_ollama_ndjson_line()`. Instead, modify `stream_ndjson_lines()` (the caller) to extract usage from the final Ollama JSON before sending `Done`. In `stream_ndjson_lines`, when we detect `done: true`, parse usage fields and send BOTH events:

```rust
// In stream_ndjson_lines, when processing the final Ollama line:
if let Some(event) = parse_ollama_ndjson_line(&line) {
    if matches!(event, StreamEvent::Done) {
        // Extract usage from the raw line before sending Done
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
            let prompt_tokens = json.get("prompt_eval_count").and_then(|v| v.as_u64());
            let completion_tokens = json.get("eval_count").and_then(|v| v.as_u64());
            if prompt_tokens.is_some() || completion_tokens.is_some() {
                let _ = tx.send(StreamEvent::Usage(LlmUsage {
                    prompt_tokens,
                    completion_tokens,
                })).await;
            }
        }
    }
    let _ = tx.send(event).await;  // Send Done after Usage
}
```

This way the stream loop sees `Done` and terminates normally, but the consumer also gets `Usage` first.

- [ ] **Step 6: Fix any exhaustive match compile errors**

Search for all `match` on `StreamEvent` and add `StreamEvent::Usage(_) => { /* handled separately */ }` arms.

- [ ] **Step 7: Verify it compiles**

Run: `cargo check 2>&1 | tail -10`
Expected: compiles

- [ ] **Step 8: Commit**

```bash
git add src/llm.rs
git commit -m "feat(llm): add StreamEvent::Usage and parse token counts from all providers"
```

---

### Task 12: Propagate usage through TurnResult and engine

**Files:**
- Modify: `src/repl/engine.rs` (`TurnResult` struct, `process_stream()` function, `run_turn()` function)

- [ ] **Step 1: Add `usage` field to `TurnResult`**

Modify the `TurnResult` struct:

```rust
pub struct TurnResult {
    pub response: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub cancelled: bool,
    pub usage: Option<crate::llm::LlmUsage>,
}
```

- [ ] **Step 2: Add `StreamEvent::Usage` match arm in `process_stream`**

`process_stream` currently returns `StreamResult` (an enum with `TextResponse`, `ToolCall`, `TextToolCall`, `Error` variants). None carry usage. The cleanest fix: change the return type to a tuple `(StreamResult, Option<LlmUsage>)`.

In the `process_stream` function, add `let mut usage: Option<crate::llm::LlmUsage> = None;` before the stream processing loop, then add the match arm:

```rust
StreamEvent::Usage(u) => {
    usage = Some(u);
}
```

Change the function signature and all return points to return `(result, usage)` instead of just `result`.

In `run_turn`, where `process_stream` is called (in a loop for multi-step tool use), accumulate usage across iterations:

```rust
let mut accumulated_usage: Option<LlmUsage> = None;
// ... in the loop:
let (stream_result, turn_usage) = process_stream(...).await;
if let Some(u) = turn_usage {
    let acc = accumulated_usage.get_or_insert(LlmUsage { prompt_tokens: Some(0), completion_tokens: Some(0) });
    acc.prompt_tokens = Some(acc.prompt_tokens.unwrap_or(0) + u.prompt_tokens.unwrap_or(0));
    acc.completion_tokens = Some(acc.completion_tokens.unwrap_or(0) + u.completion_tokens.unwrap_or(0));
}
// ... after loop, set TurnResult.usage = accumulated_usage
```

- [ ] **Step 3: Fix all `TurnResult` construction sites**

Search for places where `TurnResult` is constructed and add `usage: None` (or the captured usage) to each. There will be at least 2-3 sites in engine.rs.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check 2>&1 | tail -10`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/repl/engine.rs
git commit -m "feat(repl): propagate LLM token usage through TurnResult"
```

---

### Task 13: Add session usage tracking to TUI

**Files:**
- Modify: `src/repl/tui.rs` (`ReplApp` struct, rendering logic, `save_repl_message` call sites)

- [ ] **Step 1: Add `SessionUsage` struct and fields to `ReplApp`**

Add near the existing struct definitions in `tui.rs`:

```rust
/// Accumulated token usage for the current REPL session.
#[derive(Debug, Default)]
pub struct SessionUsage {
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub turns_with_usage: u32,
    pub turns_without_usage: u32,
}
```

Add to `ReplApp` struct (before the closing `}`):

```rust
    pub turn_usage: Option<r8r::llm::LlmUsage>,
    pub session_usage: SessionUsage,
```

- [ ] **Step 2: Accumulate usage after each turn**

Find where `TurnResult` is consumed in tui.rs (after a turn completes). Add:

```rust
// After receiving turn_result:
if let Some(ref usage) = turn_result.usage {
    app.turn_usage = Some(usage.clone());
    app.session_usage.total_prompt_tokens += usage.prompt_tokens.unwrap_or(0);
    app.session_usage.total_completion_tokens += usage.completion_tokens.unwrap_or(0);
    app.session_usage.turns_with_usage += 1;
} else {
    app.turn_usage = None;
    app.session_usage.turns_without_usage += 1;
}
```

- [ ] **Step 3: Add inline usage display after assistant messages**

In the rendering section where assistant messages are displayed, after the response content, add a usage line:

```rust
fn format_usage_line(usage: &r8r::llm::LlmUsage, model: &str) -> Option<String> {
    let prompt = usage.prompt_tokens;
    let completion = usage.completion_tokens;
    if prompt.is_none() && completion.is_none() {
        return None;
    }
    let p = prompt.map(|v| format!("{}", v)).unwrap_or("?".into());
    let c = completion.map(|v| format!("{}", v)).unwrap_or("?".into());
    let cost = estimate_cost(model, prompt, completion);
    let cost_str = cost.map(|c| format!(" · ~${:.3}", c)).unwrap_or_default();
    Some(format!("[{} in · {} out{} · {}]", p, c, cost_str, model))
}
```

Render this line as a `DisplayMessage` with `DarkGray` color.

- [ ] **Step 4: Add cost estimation function**

```rust
fn estimate_cost(
    model: &str,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
) -> Option<f64> {
    let p = prompt_tokens? as f64;
    let c = completion_tokens? as f64;
    // Rates per 1M tokens (input, output)
    let (input_rate, output_rate) = match model {
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.00),
        m if m.contains("claude-sonnet") => (3.00, 15.00),
        m if m.contains("claude-haiku") => (0.80, 4.00),
        m if m.contains("claude-opus") => (15.00, 75.00),
        _ => return None, // Unknown model (e.g., Ollama local)
    };
    Some((p * input_rate + c * output_rate) / 1_000_000.0)
}
```

- [ ] **Step 5: Pass token counts to `save_repl_message` calls**

At each `save_repl_message` call site (search for `save_repl_message` in tui.rs — there are ~5 call sites), replace the `None` token_count parameter with the actual count when available:

For assistant message saves:
```rust
// Change token_count from None to:
app.turn_usage.as_ref().map(|u| (u.prompt_tokens.unwrap_or(0) + u.completion_tokens.unwrap_or(0)) as i64)
```

Leave user/tool messages as `None` (token counts apply to LLM responses only).

- [ ] **Step 6: Verify it compiles**

Run: `cargo check 2>&1 | tail -10`
Expected: compiles

- [ ] **Step 7: Commit**

```bash
git add src/repl/tui.rs
git commit -m "feat(repl): add inline token usage display and session accumulator"
```

---

### Task 14: Add `/usage` slash command

**Files:**
- Modify: `src/repl/input.rs` (`InputCommand` enum, `parse_input()`, `slash_commands()`)
- Modify: `src/repl/tui.rs` (handle the `InputCommand::Usage` variant)

- [ ] **Step 1: Add `Usage` variant to `InputCommand`**

In `src/repl/input.rs`, add to the `InputCommand` enum:

```rust
    /// Show session token usage: /usage
    Usage,
```

- [ ] **Step 2: Add match arm in `parse_input`**

In the slash command parsing section of `parse_input()`:

```rust
"/usage" => InputCommand::Usage,
```

- [ ] **Step 3: Add to `slash_commands()` list**

In `slash_commands()` function, add:

```rust
("/usage", "Show session token usage and estimated cost"),
```

- [ ] **Step 4: Handle `InputCommand::Usage` in TUI**

In `src/repl/tui.rs`, find where `InputCommand` variants are matched (the main input handler). Add:

```rust
InputCommand::Usage => {
    let su = &app.session_usage;
    let total = su.total_prompt_tokens + su.total_completion_tokens;
    let turns = su.turns_with_usage;
    let cost = estimate_cost(
        &app.model,
        Some(su.total_prompt_tokens),
        Some(su.total_completion_tokens),
    );

    let mut text = format!("Session usage ({} turns):\n\n", turns);
    text.push_str(&format!("  Prompt tokens:      {:>10}\n", su.total_prompt_tokens));
    text.push_str(&format!("  Completion tokens:  {:>10}\n", su.total_completion_tokens));
    text.push_str(&format!("  Total tokens:       {:>10}\n", total));
    if let Some(c) = cost {
        text.push_str(&format!("  Estimated cost:     ~${:.3}\n", c));
    }
    text.push_str(&format!("\n  Model: {}", app.model));

    // NOTE: DisplayMessage has no ::system() constructor.
    // Use app.push_message(MessageKind::System, text) which is
    // the pattern used throughout tui.rs for adding system messages.
    app.push_message(MessageKind::System, text);
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check 2>&1 | tail -10`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add src/repl/input.rs src/repl/tui.rs
git commit -m "feat(repl): add /usage slash command for session token summary"
```

---

## Chunk 5: Contextual Suggestions

### Task 15: Add contextual suggestions to key commands

**Files:**
- Modify: `src/main.rs` (multiple `cmd_*` functions)

- [ ] **Step 1: Add suggestions to `cmd_workflows_create`**

After the successful creation message, add:

```rust
output.suggest(&[
    (&format!("r8r run {}", name), "execute the workflow"),
    (&format!("r8r workflows dag {}", name), "visualize the DAG"),
]);
```

- [ ] **Step 2: Add suggestions to `cmd_create`**

After successful workflow generation:

```rust
output.suggest(&[
    (&format!("r8r run {}", workflow_name), "execute it now"),
    (&format!("r8r refine {} \"...\"", workflow_name), "improve it"),
]);
```

- [ ] **Step 3: Add suggestions to `cmd_credentials_set`**

After successful credential save:

```rust
output.suggest(&[
    ("r8r doctor", "verify your setup"),
    ("r8r chat", "start building workflows"),
]);
```

- [ ] **Step 4: Add suggestions to `cmd_doctor` on failures**

When `fail > 0`, add specific hints based on which checks failed. After the summary line:

```rust
if fail > 0 {
    output.suggest(&[
        ("r8r init", "re-run setup wizard"),
        ("r8r credentials list", "check stored credentials"),
    ]);
}
```

- [ ] **Step 5: Add first-time nudge**

Create a helper function:

```rust
fn maybe_show_init_hint(output: &r8r::cli::output::Output) {
    if output.is_json() {
        return;
    }
    // Check if any LLM provider is available via env vars or config file.
    // NOTE: We intentionally skip checking Ollama reachability here because
    // this runs on every command and a 2s HTTP timeout would degrade CLI
    // responsiveness. Users with only Ollama (no env vars, no config) will
    // see this hint once, but `r8r init` will detect Ollama properly.
    let has_openai = std::env::var("OPENAI_API_KEY").is_ok();
    let has_anthropic = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let has_config = r8r::config::Config::config_dir().join("config.toml").exists();

    if !has_openai && !has_anthropic && !has_config {
        eprintln!();
        eprintln!("  tip: Run 'r8r init' to set up your environment");
    }
}
```

Call this at the end of `cmd_workflows_list`, `cmd_runs`, and `cmd_doctor` (the most common first-touch commands).

- [ ] **Step 6: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles

- [ ] **Step 7: Run all tests**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): add contextual suggestions and first-time init hint"
```

---

### Task 16: Migrate remaining key commands to use Output

**Files:**
- Modify: `src/main.rs` (remaining `cmd_*` functions and their call sites in `run_command`)

This task migrates the remaining high-traffic commands. Each command follows the same pattern as Task 6 (update call site, add `output` parameter, use `output.list`/`output.item`/`output.success`).

- [ ] **Step 1: Migrate `cmd_runs`**

Update signature to `cmd_runs(output: &Output, ...)`. Replace `println!` table with `output.list()`.

- [ ] **Step 2: Migrate `cmd_runs_show`**

Add `output` parameter. Use `output.item()` for JSON support.

- [ ] **Step 3: Migrate `cmd_templates_list`**

Add `output` parameter. Use `output.list()`.

- [ ] **Step 4: Migrate `cmd_policy_show`**

Add `output` parameter. Use `output.item()`.

- [ ] **Step 5: Update all corresponding call sites in `run_command()`**

Thread `&output` to each migrated function.

- [ ] **Step 6: Verify compilation and tests**

Run: `cargo check && cargo test 2>&1 | tail -20`
Expected: compiles, all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): migrate remaining commands to Output for --json support"
```

---

### Task 17: Final integration test

**Files:**
- Create: `tests/cli_integration_test.rs`

- [ ] **Step 1: Write integration test for --json flag**

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_json_flag_on_workflows_list() {
    let output = Command::cargo_bin("r8r")
        .unwrap()
        .arg("--json")
        .arg("workflows")
        .arg("list")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should be valid JSON (either empty array or array of objects)
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json output should be valid JSON");
    assert!(parsed.is_array());
}

#[test]
fn test_json_flag_on_doctor() {
    let output = Command::cargo_bin("r8r")
        .unwrap()
        .arg("--json")
        .arg("doctor")
        .output()
        .expect("failed to execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json doctor output should be valid JSON");
    assert!(parsed.get("checks").is_some());
}

#[test]
fn test_init_help_shows_flags() {
    Command::cargo_bin("r8r")
        .unwrap()
        .arg("init")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--yes"))
        .stdout(predicate::str::contains("--force"));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --test cli_integration_test 2>&1 | tail -15`
Expected: all 3 tests PASS

- [ ] **Step 3: Run full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass (existing + new)

- [ ] **Step 4: Commit**

```bash
git add tests/cli_integration_test.rs
git commit -m "test: add integration tests for --json flag and init command"
```
