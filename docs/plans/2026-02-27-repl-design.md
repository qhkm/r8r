# Interactive REPL (Phase 2) Design

## Overview

Add an interactive REPL to r8r where users type natural language or slash commands to create, manage, and debug workflows. The LLM autonomously executes r8r tools (Claude Code-style) with full grounding against hallucination. Rich TUI with panels for conversation, DAG visualization, and YAML/trace views.

**Phase 1 (shipped)**: `r8r create` and `r8r refine` CLI commands + `r8r_generate` MCP tool.
**Phase 2 (this design)**: Interactive REPL built on Phase 1 foundation.

## Architecture

**Single-process (Approach A)**: One binary handles TUI rendering, LLM conversation loop, and tool execution. Tokio async keeps the UI responsive. No IPC complexity.

**Engine mode**: Embedded by default (loads storage, executor, credentials directly). Auto-detects running server on localhost and connects for richer features (live executions, webhooks). Graceful fallback to embedded if server disconnects.

**Entry points**: `r8r` with no arguments or `r8r chat` both open the REPL. `r8r chat --resume [id]` continues a previous session.

## Input Routing

User input is classified into two categories:

### Slash commands (deterministic, no LLM)

- `/run <workflow>` — execute a workflow
- `/list` — list workflows
- `/logs <workflow>` — show execution history
- `/trace <exec-id>` — node-level trace
- `/show <workflow>` — show workflow YAML
- `/dag <workflow>` — render visual node DAG in context panel
- `/sessions` — list recent sessions
- `/resume <id>` — switch to previous session
- `/clear` — clear conversation context
- `/reconnect` — retry LLM connection
- `/help` — show available commands
- `/exit` or Ctrl+C (x2) — quit

### Natural language (everything else goes to LLM)

If input doesn't start with `/`, it's sent to the LLM with full tool access.

## Agentic Loop

```
User prompt
    ↓
Build messages (system prompt + history + user message)
    ↓
Call LLM (streaming) with tool definitions ────┐
    ↓                                           │
Response contains tool_use?                     │
    ├── yes → Execute tool, append result ──────┘
    │         Show ⚙ tool call + result in TUI
    └── no  → Display final text response
              Save to conversation history
```

The loop continues until the LLM responds with text only (no more tool calls). Max 20 tool calls per turn to prevent infinite loops.

### Available tools (15)

The LLM has access to all existing MCP tools, executed directly via embedded engine:

- `r8r_execute`, `r8r_run_and_wait` — run workflows
- `r8r_list_workflows`, `r8r_get_workflow`, `r8r_discover` — inspect
- `r8r_get_execution`, `r8r_list_executions`, `r8r_get_trace` — check results
- `r8r_validate`, `r8r_lint`, `r8r_create_workflow` — validate/manage
- `r8r_generate` — LLM-based workflow generation (Phase 1)
- `r8r_test`, `r8r_list_approvals`, `r8r_approve` — testing and approval

## Anti-Hallucination & Grounding

Three layers prevent the LLM from going off-spec:

### Layer 1: System prompt auto-generated from source of truth

The system prompt is assembled at runtime from actual r8r internals:

- **Tool definitions** — extracted from the same Rust structs that define MCP tools
- **Node catalog** — generated from NodeRegistry at runtime
- **Credential names** — queried from CredentialStore (names only, never values)
- **Workflow names** — queried from storage

The LLM cannot reference tools, node types, credentials, or workflows that don't exist.

### Layer 2: Tool execution is the only action path

The LLM cannot do anything except call defined tools. If it outputs:
- **Unknown tool name** → rejected, error fed back
- **Invalid parameters** → rejected, schema validation error fed back
- **Non-existent workflow** → tool returns "not found", LLM sees real error

No hallucinated action can execute. Max 3 invalid tool attempts before showing error to user.

### Layer 3: All generated YAML goes through validation

Generated workflow YAML passes through:
1. YAML parser (syntax check)
2. Schema validator (node types exist, depends_on valid, required fields present)
3. Auto-fix retry (errors fed back to LLM for one correction attempt)

Same validation pipeline as `r8r workflows validate`.

### System prompt structure

```
You are the r8r workflow assistant. You help users create,
manage, and debug workflows.

RULES:
- Only use the tools provided. Never suggest manual actions.
- Only reference node types from the catalog below.
- Only reference credentials listed below.
- When creating workflows, always validate before saving.
- If unsure, use r8r_discover to inspect a workflow first.

AVAILABLE TOOLS:
[auto-generated from tool registry]

NODE CATALOG:
[auto-generated from NodeRegistry]

CONFIGURED CREDENTIALS:
[auto-generated from CredentialStore]

EXISTING WORKFLOWS:
[auto-generated from storage]
```

Everything after RULES is generated from code. Nothing is hand-maintained.

## Diagram Viewing

Workflows are visualized as node-level DAGs in the TUI, not just raw YAML.

### Access methods

- `/dag <workflow>` slash command
- Automatic after generation — when LLM creates a workflow, the context panel shows the DAG

### Rendering

Reuses existing `dag_view.rs` renderer from the monitor TUI:

```
  ┌──────────────┐
  │ fetch-price  │ ○ http
  └──────────────┘
      │
  ┌──────────────┐
  │ check-price  │ ○ transform
  └──────────────┘
      │
  ┌──────────────┐
  │ alert-slack  │ ○ slack
  └──────────────┘
```

With execution status when running:

```
  ┌──────────────┐
  │ fetch-price  │ ✓ http 120ms
  └──────────────┘
      │
  ┌──────────────┐
  │ check-price  │ ⟳ transform 45ms
  └──────────────┘
      │
  ┌──────────────┐
  │ alert-slack  │ ○ slack
  └──────────────┘
```

## TUI Layout

```
┌─────────────────────────────────────────────────────┐
│  r8r chat │ model: gpt-4o │ session: #3 │ 3 workflows│
├───────────────────────────────┬─────────────────────┤
│                               │                     │
│  Conversation Panel (~65%)    │  Context Panel (~35%)│
│                               │                     │
│  > create a workflow that     │  ┌──────────────┐   │
│    checks BTC every hour      │  │ fetch-price  │   │
│                               │  └──────┬───────┘   │
│  ⚙ r8r_generate(...)         │         │           │
│  ⚙ r8r_validate(...)         │  ┌──────────────┐   │
│  ✓ Created "btc-alert"       │  │ check-price  │   │
│    3 nodes: http→transform→  │  └──────┬───────┘   │
│    slack                      │         │           │
│                               │  ┌──────────────┐   │
│  > now run it                 │  │ alert-slack  │   │
│                               │  └──────────────┘   │
│  ⚙ r8r_execute("btc-alert") │                     │
│  ✓ Execution #42 completed   │  --- or YAML ---    │
│    in 340ms                   │  name: btc-alert    │
│                               │  nodes:             │
│                               │    - id: fetch...   │
├───────────────────────────────┴─────────────────────┤
│ > type a message or /command                    [⏎] │
└─────────────────────────────────────────────────────┘
```

**Top bar**: Model name, session ID, workflow count, server connection status.

**Conversation panel (left)**: Scrollable chat history with user messages (`>`), tool calls (`⚙`, dimmed), and LLM responses (streamed).

**Context panel (right)**: Auto-switches based on context:
- DAG view when a workflow is generated/referenced
- YAML view when showing workflow definition
- Trace view when an execution completes
- Toggle with Tab key

**Input bar (bottom)**: Text input with slash command autocomplete.

### Key bindings

- `Enter` — send message
- `Tab` — toggle context panel (DAG / YAML / trace)
- `Ctrl+C` — cancel current LLM request or exit (double-press)
- `Ctrl+L` — clear conversation
- `↑/↓` — scroll conversation / input history

### Responsive

- Width < 80: hide context panel (conversation only)
- Height < 10: hide status bar
- Minimum: 40x10

## Streaming & Provider Support

### Streaming architecture

```
LLM API (SSE stream)
    ↓
Stream decoder (provider-specific)
    ↓
Token channel (tokio::mpsc)
    ↓
Two consumers:
├── TUI renderer (displays tokens as they arrive)
└── Message accumulator (builds full response for history)
```

When the stream contains a tool call: accumulate silently, show `⚙ calling tool(...)`, execute, feed result back, start new stream.

### Provider-specific streaming

| Provider | Format |
|----------|--------|
| OpenAI | SSE `data: {"choices":[{"delta":...}]}` with `stream: true` |
| Anthropic | SSE `content_block_delta`, `message_delta` with `stream: true` |
| Ollama | NDJSON `{"message":{"content":"..."}}` with `stream: true` |
| Custom | OpenAI-compatible SSE |

### Tool calling per provider

| Provider | Native tool_use | Fallback |
|----------|----------------|----------|
| OpenAI | `tools` param + `tool_calls` in response | — |
| Anthropic | `tools` param + `tool_use` content blocks | — |
| Ollama | Not supported by most models | Text-based parsing |
| Custom | Try native first | Fall back to text parsing |

### Text fallback (Ollama/unsupported)

Added to system prompt:
```
When you need to call a tool, output EXACTLY:
<tool_call>{"name": "tool_name", "args": {"param": "value"}}</tool_call>
Do not add any text before or after the tag. Wait for the result before continuing.
You may call one tool at a time.
```

REPL detects `<tool_call>` in stream and switches from rendering to parsing.

## Session Persistence

### SQLite tables

```sql
repl_sessions
├── id (TEXT PRIMARY KEY)
├── model (TEXT)
├── summary (TEXT)
├── created_at (TEXT)
└── updated_at (TEXT)

repl_messages
├── id (TEXT PRIMARY KEY)
├── session_id (TEXT FK → repl_sessions)
├── role (TEXT)           -- "user", "assistant", "tool_call", "tool_result"
├── content (TEXT)
├── token_count (INTEGER)
└── created_at (TEXT)
```

### Session lifecycle

1. `r8r` / `r8r chat` — creates new session
2. `r8r chat --resume` — lists recent sessions to pick from
3. `r8r chat --resume <id>` — resumes specific session
4. During chat — every message saved immediately (crash-safe)
5. `/exit` — session saved, can be resumed later

### Context window management

Full conversation history sent each turn. When too long:
- System prompt always kept
- Last N messages kept (default ~50)
- Older messages summarized into single context message
- Tool call/result pairs compressed first (summary, not raw JSON)

## Error Handling

| Scenario | Behavior |
|----------|----------|
| No LLM configured | Startup warning. REPL launches in slash-only mode. |
| API key invalid | Show error on first call, stay in REPL. `/reconnect` to retry. |
| Rate limited | "Retrying in Xs..." with 1 retry + backoff. |
| Network timeout | "Timed out after 30s." No auto-retry. |
| Stream interrupted | Show partial response + "(interrupted)". |
| Tool execution fails | Error fed back to LLM as tool result. LLM decides next step. |
| Unknown tool called | Error fed back. Max 3 invalid attempts, then show error. |
| Tool loop (>20 calls) | Force text response: "Reached limit. Here's what I have..." |
| Server not running | Embedded mode works. Server features gracefully disabled. |
| Server connection lost | Auto-fallback to embedded mode. |
| Ctrl+C during stream | Cancel request. Show "(cancelled)". Stay in REPL. |
| Ctrl+C at prompt | First: "Press again to exit." Second: clean exit, session saved. |
| Terminal too small | Hide panels progressively. Min: 40x10. |

## Component Architecture

### New modules

- `src/repl/mod.rs` — REPL entry point, session management, main loop
- `src/repl/input.rs` — input parsing, slash command dispatch
- `src/repl/conversation.rs` — message history, context window management
- `src/repl/tools.rs` — tool bridge (LLM tool calls → r8r tool execution)
- `src/repl/tui.rs` — TUI layout, panels, rendering
- `src/repl/engine.rs` — agentic loop (LLM streaming + tool execution)
- `src/repl/prompt.rs` — system prompt builder

### Modified modules

- `src/llm.rs` — add `call_llm_streaming()` returning token stream via mpsc channel
- `src/main.rs` — add `Commands::Chat` variant, wire to repl module
- `src/storage/sqlite.rs` — add `repl_sessions` and `repl_messages` table operations

### Reused as-is

- `src/generator/` — Phase 1 generator (called via `r8r_generate` tool)
- `src/tui/widgets/dag_view.rs` — DAG rendering
- `src/mcp/tools.rs` — tool definitions (schemas reused for LLM function definitions)
- `src/workflow/` — parser, validator
- `src/engine/` — executor
- `src/credentials/` — credential store

## Future Considerations (not in scope)

- Multi-model switching mid-session
- Plugin system for custom tools
- Shared sessions (collaborative)
- Web-based REPL frontend
