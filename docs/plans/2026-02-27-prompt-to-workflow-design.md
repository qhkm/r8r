# Prompt-to-Workflow Generation

## Overview

Add natural language workflow generation to r8r. Users describe what they want in plain English, r8r calls an LLM to generate valid workflow YAML, validates it, and lets the user confirm or refine before saving.

**Phase 1 (this design)**: CLI commands + MCP tool, designed for agent consumption.
**Phase 2 (future)**: Interactive REPL mode (Claude Code-style) built on top of Phase 1.

## CLI Commands

### `r8r create <prompt>`

1. Gathers context (node catalog, credentials, env vars, example workflows)
2. Sends prompt + context to LLM
3. Receives YAML, validates through existing parser + validator pipeline
4. If validation fails → feeds errors back to LLM (1 auto-fix retry)
5. Prints YAML with summary: `Generated workflow "hn-digest" with 4 nodes: http → transform → agent → slack`
6. Prompts: `Save? [yes/no/feedback]`
   - `yes` → saves to workflows directory, registers it
   - `no` → discards
   - Anything else → treated as refinement feedback, loops back to step 2 with current YAML + feedback

### `r8r refine <workflow-name> <prompt>`

1. Loads existing workflow YAML
2. Sends current YAML + refinement prompt + context to LLM
3. Shows diff (before/after) instead of full YAML
4. Same `Save? [yes/no/feedback]` loop

## MCP Tool

### `r8r_generate`

Generates or refines a workflow from natural language.

**Input:**
```json
{
  "prompt": "string (required) - natural language description",
  "workflow_name": "string (optional) - if provided, loads and refines existing workflow"
}
```

**Output:**
```json
{
  "yaml": "string - the generated workflow YAML",
  "summary": "string - human-readable summary of what was generated",
  "validation": {
    "valid": true,
    "errors": []
  }
}
```

Agent receives YAML + validation status and can call `r8r_create_workflow` to save it. Generation is separate from saving — the agent decides what to do with the result.

## System Prompt Architecture

The system prompt sent to the LLM has four parts, assembled at runtime:

### Part 1 — Role & rules

```
You are a workflow generator for r8r. Output valid YAML only.
Follow the schema exactly. Use {{ env.VAR }} for secrets, never hardcode them.
Use {{ nodes.id.output.field }} for node references.
```

### Part 2 — Node catalog (auto-generated from registry)

```
Available node types:
- http: Make HTTP requests. Config: url, method, headers, body, timeout_seconds
- transform: Rhai expression. Config: expression
- agent: LLM call. Config: provider, model, prompt, response_format, json_schema
- slack: Send Slack message. Config: channel, text, webhook_url
...
```

Generated dynamically from the node registry at runtime. Stays in sync automatically when new node types are added. No manual prompt maintenance.

### Part 3 — Available context

```
Credentials configured: slack-webhook, openai-key, github-token
Environment variables: ORDERS_API_URL, WHATSAPP_TOKEN, ...
```

Credential names only — never values. Lets the LLM reference real credentials in generated YAML instead of placeholders.

### Part 4 — Examples

2-3 curated workflows from `examples/` directory, selected based on keyword similarity to the user's prompt.

### Refinement prompt addition

```
Current workflow YAML:
<yaml>...</yaml>

User wants to change: "<refinement prompt>"
Output the complete updated YAML.
```

## Component Architecture

Three new modules:

### `src/generator/mod.rs` — Orchestrator

- `generate(prompt, context) -> Result<String>` — builds system prompt, calls LLM, returns YAML
- `refine(yaml, prompt, context) -> Result<String>` — same but with existing YAML
- `build_context() -> GeneratorContext` — collects node catalog, credentials, env vars, selects example workflows

### `src/generator/prompt.rs` — Prompt assembly

- `build_system_prompt(context: &GeneratorContext) -> String`
- `build_create_prompt(user_prompt: &str) -> String`
- `build_refine_prompt(yaml: &str, user_prompt: &str) -> String`

### `src/generator/llm.rs` — LLM client

Thin wrapper reusing existing provider logic from `nodes/agent.rs`. Extract the HTTP call + provider config into a shared module.

- `call_llm(system: &str, user: &str, config: &AgentConfig) -> Result<String>`

## Data Flow

```
User prompt
    |
generator::build_context()  ->  reads node registry, credentials, env vars, examples/
    |
generator::generate()  ->  assembles prompt, calls LLM
    |
workflow::parser::parse()  ->  validates YAML structure
    |
workflow::validator::validate()  ->  checks node types, dependencies, etc.
    |
  Valid?  --no-->  feed errors back to LLM (1 retry)
    |
   yes
    |
Print YAML + summary to terminal
    |
Prompt: Save? [yes / no / feedback]
    |         |        |
   save    discard   loop back with feedback
```

No new database tables. No new API endpoints. CLI commands -> generator -> existing validation pipeline -> existing `workflows create` path to save.

## LLM Configuration

Reuses existing agent node config. No new configuration needed:

- `R8R_AGENT_PROVIDER` (openai, anthropic, ollama, custom)
- `R8R_AGENT_MODEL` (gpt-4o, claude-sonnet-4-20250514, etc.)
- `R8R_AGENT_API_KEY`

Or via `r8r credentials` for stored keys.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| LLM returns invalid YAML syntax | Feed parse error back to LLM, 1 retry. If still invalid, show raw output + error to user. |
| Valid YAML but invalid workflow | Feed validation errors back to LLM, 1 retry. If still invalid, show YAML + errors. |
| LLM hallucinates node types | Node catalog in system prompt is main defense. Validator catches anything that slips through. |
| API key not configured | Check before calling. Error: `No LLM provider configured. Set R8R_AGENT_PROVIDER and R8R_AGENT_API_KEY.` |
| LLM rate limit / network error | 1 retry with backoff. On failure: `Failed to reach LLM: <error>.` |
| Vague user prompt | Let LLM do its best. Preview+confirm step is the safety net. |
| System prompt exceeds token budget | Cap at ~4k tokens. Truncate examples first. Node catalog always stays complete. |

## Testing Strategy

### Unit tests
- Prompt assembly includes all node types from registry
- Prompt includes credential names (never values)
- Example selection picks relevant workflows
- Validate-retry loop: mock LLM returning invalid then valid YAML

### Integration tests (mocked LLM)
- Full flow: generate() -> parse -> validate -> valid workflow
- Refinement: existing YAML + change prompt -> updated YAML
- Error paths: garbage response -> retry -> still garbage -> returns error

### No live LLM tests in CI

### Manual test suite
- `r8r create "every hour, check if Bitcoin > 100k and send Slack alert"`
- `r8r create "webhook receives GitHub PR, run linter in sandbox, post comment"`
- `r8r refine <name> "add retry with exponential backoff on the HTTP calls"`
- Edge: `r8r create "do stuff"` (vague prompt)
- Edge: no API key configured

## Future: Phase 2 — Interactive REPL

Build on top of Phase 1 foundation:
- `r8r` with no arguments opens interactive session
- Slash commands for deterministic ops (`/run`, `/logs`, `/list`)
- Natural language for everything else (routed to LLM)
- LLM has full tool access to r8r operations (run, logs, credentials, etc.)
- Persistent conversation context within session
