# World-Class CLI Improvements — Design Spec

**Date:** 2026-03-13
**Status:** Approved
**Scope:** 5 high-ROI improvements to make r8r's CLI world-class for both humans and agents

## Overview

r8r's CLI is functionally mature (Clap 4, ratatui TUI, streaming progress, shell completions, credential redaction) but lacks the polish that separates good CLIs from great ones. This spec covers 5 targeted improvements:

1. **Elm-style error diagnostics** — actionable errors with recovery hints and fuzzy suggestions
2. **Global `--json` output** — machine-readable output on every command
3. **`r8r init` wizard** — first-run setup with service detection
4. **Token usage tracking** — inline cost awareness and session summaries
5. **Contextual suggestions** — teach the CLI organically after actions

**Target users:** Both AI agents (programmatic `--json` usage) and developers (interactive REPL/CLI).

**Guiding principle:** Layer improvements into the existing architecture. No rewrites, no new frameworks, minimal new dependencies.

---

## 1. Elm-Style Error Diagnostics

### New Files
- `src/cli/diagnostics.rs` — error enrichment and rendering
- `src/cli/mod.rs` — module declarations

### New Dependencies
- `strsim` — Jaro-Winkler string similarity for fuzzy matching

### Core Type

```rust
pub struct Diagnostic {
    pub code: &'static str,        // from Error::code() e.g. "WORKFLOW_ERROR"
    pub kind: DiagnosticKind,      // specific sub-type for enrichment
    pub message: String,           // the error message
    pub hints: Vec<String>,        // actionable recovery commands
    pub suggestions: Vec<String>,  // fuzzy-matched "did you mean?" items
}

/// Specific error scenarios for enrichment (not tied to Error enum variants)
pub enum DiagnosticKind {
    WorkflowNotFound { name: String },
    NoLlmConfigured,
    YamlParseError { line: Option<usize>, column: Option<usize> },
    CredentialMissing { service: String },
    ConnectionRefused { url: String },
    DbNotInitialized,
    Generic,
}
```

### Error Pattern Matching

The existing `Error` enum uses broad categories (`Error::Workflow(String)`, `Error::Config(String)`, etc.) with generic codes like `"WORKFLOW_ERROR"`. Since adding fine-grained variants to `Error` would be a large refactor, the diagnostic layer uses **message pattern matching** to detect specific scenarios:

```rust
impl Diagnostic {
    pub fn from_error(err: &r8r::Error) -> Self {
        let code = err.code();
        let message = err.to_string();
        let kind = classify_error(code, &message);
        let mut diag = Diagnostic { code, kind, message, hints: vec![], suggestions: vec![] };
        diag.enrich();  // populates hints and suggestions based on kind
        diag
    }
}

fn classify_error(code: &str, message: &str) -> DiagnosticKind {
    match code {
        "WORKFLOW_ERROR" if message.contains("not found") => {
            // Extract workflow name from message pattern
            DiagnosticKind::WorkflowNotFound { name: extract_name(message) }
        }
        "CONFIG_ERROR" if message.contains("LLM") || message.contains("provider") => {
            DiagnosticKind::NoLlmConfigured
        }
        // ... other patterns
        _ => DiagnosticKind::Generic,
    }
}
```

This is intentionally fragile-by-design: if error messages change, classification falls back to `Generic` (which still shows the error clearly, just without enriched hints). As the codebase matures, specific `Error` variants can replace message-based matching.

### Human Rendering (stderr)

```
error[WORKFLOW_ERROR]: workflow 'my-workflo' not found

  Similar workflows:
    • my-workflow
    • my-other-workflow

  hint: r8r workflows list    — show all workflows
        r8r create "..."      — generate a new one
```

No box-drawing borders. Color + spacing only (rustc-style). Degrades gracefully without Unicode.

### JSON Rendering (stderr, when `--json` active)

```json
{"error": {"code": "WORKFLOW_ERROR", "message": "...", "hints": [...], "suggestions": [...]}}
```

### Fuzzy Matching

- Algorithm: Jaro-Winkler (better for short strings like workflow names)
- Threshold: only suggest matches with similarity > 0.7
- Applied to: workflow names, credential service names, template names, policy profile names

### Phase 1 Error Coverage (6 errors)

| Error Scenario | Hints |
|---|---|
| Workflow not found | fuzzy suggestions + `r8r workflows list` |
| No LLM configured | `r8r credentials set openai --key ...` or `ollama serve` |
| YAML parse error | line/column pointer + `r8r lint <file>` |
| Credential missing | `r8r credentials set <service>` |
| Connection refused (server) | `r8r server` to start, check port |
| DB not initialized | `r8r doctor` to diagnose |

### Integration Point

In `main()`, wrap the top-level `anyhow::Result` error path with a diagnostic formatter that:
1. Downcasts to `r8r::Error` if possible
2. Matches known error patterns
3. Enriches with hints and fuzzy suggestions
4. Renders via `Diagnostic::print()` (human) or `Diagnostic::print_json()` (json mode)
5. Falls through to default `anyhow` display for unrecognized errors

---

## 2. Global `--json` Output

### Modified Files
- `src/main.rs` — add global `--json` flag, thread `Output` through commands
- New `src/cli/output.rs` — output abstraction

### CLI Change

Add `json` field to the existing `Cli` struct (which currently has only `command`):

```rust
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Output results as JSON (added — new field)
    #[arg(long, global = true)]
    json: bool,
}
```

### Output Helper

```rust
pub enum OutputMode { Human, Json }

pub struct Output {
    mode: OutputMode,
}

impl Output {
    /// Print a list of items as table (human) or JSON array (json)
    pub fn list<T: Serialize>(
        &self,
        headers: &[&str],
        rows: &[T],
        format_row: fn(&T) -> Vec<String>,
    );

    /// Print a single item as formatted text (human) or JSON object (json)
    pub fn item<T: Serialize>(&self, item: &T, format_human: fn(&T) -> String);

    /// Print a success message (human) or {"ok": true, "message": "..."} (json)
    pub fn success(&self, msg: &str);

    /// Print an error diagnostic (human) or structured JSON error (json)
    pub fn error(&self, diagnostic: &Diagnostic);
}
```

### Migration Strategy

- Thread `Output` as first parameter to each `cmd_*` function
- Migrate incrementally — existing per-command `--json` flags on `run`, `lint`, `prompt` become redundant (still work, global flag takes precedence)
- In JSON mode: all data to stdout as valid JSON, all diagnostics/suggestions to stderr as JSON, no colors, no spinners

### JSON Output per Command

| Command | JSON Structure |
|---|---|
| `workflows list` | `[{"name": "...", "enabled": true, "updated_at": "..."}]` |
| `workflows show` | Full workflow object |
| `runs` | Array of execution summaries |
| `runs show` | Full execution with optional trace |
| `credentials list` | `[{"service": "...", "key": "...", "updated_at": "..."}]` (values redacted) |
| `doctor` | `{"checks": [{"name": "Database", "pass": true, "message": "..."}]}` |
| `templates list` | Array of template objects |
| `policy show` | Policy config object |
| `create` / `refine` | `{"workflow": "...", "yaml": "..."}` |
| `run` (success) | Full execution result |
| `run` (failure) | Error diagnostic JSON |

---

## 3. `r8r init` Wizard

### New Files
- `src/cli/init.rs` — wizard logic and service detection

### Commands Enum Addition

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

### Interactive Flow

```
$ r8r init

  Welcome to r8r — agent-native workflow engine

  Detecting your environment...

  ✓ Ollama         running (localhost:11434, llama3.2)
  ✗ OpenAI key     not found
  ✗ Anthropic key  not found
  ✓ Docker         running (for sandboxed execution)
  ✓ Shell          zsh detected

  ─── LLM Provider ───

  Which provider do you want to use?
    [1] Ollama (detected, free, local)
    [2] OpenAI (requires API key)
    [3] Anthropic (requires API key)

  > 1

  ✓ Using Ollama with llama3.2

  ─── Shell Completions ───

  Install zsh completions? [Y/n] y
  ✓ Completions installed. Restart your shell or run: source ~/.zshrc

  ─── Config ───

  ✓ Wrote ~/.config/r8r/config.toml

  ─── Quick Start ───

  Running a demo workflow to verify everything works...

  ✓ Created workflow: hello-world
  ✓ Executed successfully (247ms)

  You're all set! Try these next:

    r8r chat                    — interactive AI assistant
    r8r create "check weather"  — generate a workflow
    r8r templates list          — browse ready-made templates
```

### Service Detection

Uses `Config::config_dir()` from `src/config/mod.rs` for config path resolution (respects `dirs::config_dir()` with fallback to `.r8r/`), not hardcoded `~/.config/r8r/`.

| Check | Method | Timeout |
|---|---|---|
| Ollama | HTTP GET `localhost:11434/api/tags`, parse model list | 2s |
| OpenAI key | Check `OPENAI_API_KEY` env var | instant |
| Anthropic key | Check `ANTHROPIC_API_KEY` env var | instant |
| Docker | `docker info` subprocess | 2s |
| Shell | `SHELL` env var | instant |
| Existing config | Check via `Config::config_dir()` / `config.toml` | instant |

### `--yes` Mode (Agent-Friendly)

Auto-detect best provider (priority: existing env key > running Ollama > prompt for key), skip confirmations, write config, output JSON summary. No interactive prompts.

### Demo Workflow

Built-in "hello-world" that doesn't need any external API — verifies engine runs. If LLM provider is configured, runs a one-step LLM workflow ("write a haiku about automation") to prove the full pipeline.

### Guard Rails

- If `config.toml` already exists: warn and confirm overwrite (unless `--force`)
- If no LLM provider detected: complete init but warn with clear next steps
- Never overwrite existing credentials

---

## 4. Token Usage Tracking

### Modified Files
- `src/llm.rs` — add `StreamEvent::Usage(LlmUsage)` variant, parse from final stream chunks
- `src/repl/engine.rs` — add `usage: Option<LlmUsage>` to `TurnResult`
- `src/repl/tui.rs` — add session accumulator, inline display, `/usage` command
- `src/repl/input.rs` — add `/usage` to slash command list

### The Gap Today

`LlmUsage` is parsed from all 3 providers (OpenAI, Anthropic, Ollama) in `src/llm.rs` but is never propagated beyond `LlmResponse`. `TurnResult` doesn't carry it. The DB column `repl_messages.token_count` exists but is always `NULL`.

### Stream Event Addition

```rust
pub enum StreamEvent {
    TextDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, arguments: String },
    Usage(LlmUsage),  // NEW — emitted from final chunk before Done
    Done,
}
```

For streaming responses:
- **OpenAI**: Modify `build_openai_streaming_request()` to include `stream_options: {"include_usage": true}` in the request body. Parse usage from the final SSE chunk.
- **Anthropic**: Add a new `"message_delta"` match arm in `parse_anthropic_sse_line()` (currently unhandled — falls through to `_ => None`). Parse `usage.output_tokens` from this event type.
- **Ollama**: Parse from final streaming chunk (`prompt_eval_count`, `eval_count`)

### TUI State Additions

```rust
pub struct ReplApp {
    // ... existing fields ...

    /// Current turn usage (reset each turn)
    pub turn_usage: Option<LlmUsage>,

    /// Session-level accumulator
    pub session_usage: SessionUsage,
}

pub struct SessionUsage {
    pub total_prompt_tokens: u64,    // accumulated from Option<u64>, defaulting to 0 when None
    pub total_completion_tokens: u64, // accumulated from Option<u64>, defaulting to 0 when None
    pub turns_with_usage: u32,       // turns where usage data was available
    pub turns_without_usage: u32,    // turns where provider returned no usage data
}
```

### Inline Display

After each assistant response, append a line in `DarkGray`:

```
[1,240 in · 380 out · ~$0.003 · gpt-4o]
```

- Only shown when `LlmUsage` has at least one `Some` token field
- When `prompt_tokens` is `None` but `completion_tokens` is `Some`: show `[? in · 380 out · gpt-4o]`
- Ollama local models show tokens but no cost (cost estimation returns `None`)
- Unknown models show tokens only, no cost estimate
- When both fields are `None`: line is not displayed

### Cost Estimation

```rust
fn estimate_cost(model: &str, prompt_tokens: Option<u64>, completion_tokens: Option<u64>) -> Option<f64>
```

Returns `None` if model is unknown OR if both token fields are `None`.

Hardcoded lookup table for common models:
- gpt-4o, gpt-4o-mini
- claude-sonnet-4-20250514, claude-haiku-4-5-20251001
- Returns `None` for unknown/local models

### `/usage` Slash Command

```
Session usage (12 turns):

  Prompt tokens:      14,820
  Completion tokens:   4,210
  Total tokens:       19,030
  Estimated cost:     ~$0.037

  Model: gpt-4o via openai
```

### DB Persistence

Pass actual token counts to existing `save_repl_message()` calls. Currently all call sites pass `None` — change to pass `turn_usage.and_then(|u| Some(u.prompt_tokens.unwrap_or(0) + u.completion_tokens.unwrap_or(0)))`. No schema migration needed.

### `/usage` Slash Command Registration

Add `InputCommand::Usage` variant to the command enum in `src/repl/input.rs`. Add `/usage` match arm in `parse_input()`. Add entry to `slash_commands()` function for autocomplete.

---

## 5. Contextual Suggestions

### Modified Files
- `src/main.rs` — add `suggest()` calls at the end of relevant `cmd_*` functions
- `src/cli/output.rs` — `Output::suggest()` method

### Helper

```rust
impl Output {
    /// Print contextual next-step hints (stderr, suppressed in JSON mode)
    pub fn suggest(&self, hints: &[(&str, &str)]) {
        if matches!(self.mode, OutputMode::Json) { return; }
        eprintln!();
        eprintln!("  Next steps:");
        for (cmd, desc) in hints {
            eprintln!("    {}  — {}", cmd, desc);
        }
    }
}
```

Uses `eprintln!` so suggestions don't pollute stdout. Suppressed entirely in `--json` mode.

### Suggestion Map

| After command... | Hints |
|---|---|
| `workflows create <file>` | `r8r run <name>`, `r8r workflows dag <name>` |
| `create "..."` | `r8r run <name>`, `r8r refine <name> "..."` |
| `run` (success) | `r8r runs show <id> --trace`, `r8r workflows logs <name>` |
| `run` (failure) | `r8r workflows trace <id>`, `r8r refine <name> "fix..."` |
| `credentials set` | `r8r doctor`, `r8r chat` |
| `init` | `r8r chat`, `r8r create "..."`, `r8r templates list` |
| `doctor` (failures) | specific fix command per failing check |
| `templates use <name>` | `r8r run <name>`, `r8r workflows show <name>` |
| `lint` (warnings) | `r8r workflows validate <file>` |

### What Doesn't Get Suggestions

- List/show commands (read-only browsing)
- Commands in `--json` mode
- The REPL (has its own contextual quick-actions already)

### First-Time Nudge

If any command runs and no LLM provider is configured (no env vars `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` set AND no config file with LLM settings AND Ollama not reachable), append:

```
  tip: Run 'r8r init' to set up your environment
```

Note: checks provider availability rather than just config file existence, since users may configure r8r entirely via environment variables without ever running `init`.

---

## Implementation Order

Features should be implemented in this order due to dependencies:

1. **`src/cli/mod.rs` + `src/cli/output.rs`** — Output abstraction (needed by all features)
2. **`src/cli/diagnostics.rs`** — Error diagnostics (needed by `--json` error path)
3. **Global `--json` in `src/main.rs`** — Thread `Output` through commands
4. **`src/cli/init.rs`** — Init wizard (standalone, uses Output)
5. **Token tracking** — `src/llm.rs` (StreamEvent + build_openai_streaming_request + parse_anthropic_sse_line) → `src/repl/engine.rs` (TurnResult + process_stream match arm) → `src/repl/tui.rs` → `src/repl/input.rs`
6. **Contextual suggestions** — Lightest change, added last to existing commands

## New Dependencies

| Crate | Purpose | Size Impact |
|---|---|---|
| `strsim` | Jaro-Winkler fuzzy matching | ~15KB, no transitive deps |

## Files Changed Summary

| File | Changes |
|---|---|
| `Cargo.toml` | Add `strsim` |
| `src/main.rs` | Global `--json` flag, `Init` command, thread `Output`, diagnostic wrapper, contextual suggestions |
| `src/cli/mod.rs` | New module declarations |
| `src/cli/output.rs` | New `Output` helper |
| `src/cli/diagnostics.rs` | New `Diagnostic` type, enrichment logic, fuzzy matching |
| `src/cli/init.rs` | New init wizard |
| `src/llm.rs` | `StreamEvent::Usage` variant, modify `build_openai_streaming_request()` to add `stream_options`, add `"message_delta"` match arm in `parse_anthropic_sse_line()` |
| `src/repl/engine.rs` | `usage` field on `TurnResult`, new `StreamEvent::Usage` match arm in `process_stream()` |
| `src/repl/tui.rs` | `SessionUsage` state, inline display, `/usage` rendering |
| `src/repl/input.rs` | `InputCommand::Usage` variant, `/usage` parser match arm, `slash_commands()` entry |
| `src/error.rs` | No structural changes — diagnostic enrichment uses message pattern matching in `src/cli/diagnostics.rs` rather than adding new variants |
