# n8n Migration Converter + JSON Workflow Support

**Date**: 2026-03-24
**Status**: Approved
**Scope**: `r8r migrate n8n` CLI command, JS-to-Rhai transpiler, JSON workflow support

## Problem

n8n is the most popular open-source workflow automation tool with 400+ integrations. Users evaluating r8r need a migration path. Currently, converting an n8n workflow to r8r format is entirely manual — requiring knowledge of both formats, expression syntax differences, and node type mappings.

## Solution

A built-in `r8r migrate n8n <file.json>` CLI command that:
1. Parses n8n's JSON export format into typed Rust structs
2. Maps n8n node types to r8r equivalents (exact for ~20 known types, placeholder for unknown)
3. Transpiles n8n JavaScript expressions to Rhai (full transpiler with 3 fidelity tiers)
4. Converts n8n's connection graph to r8r's `depends_on` arrays
5. Outputs valid r8r YAML (or JSON) with migration warnings as comments

Additionally, r8r gains native JSON workflow support (`.json` files accepted alongside `.yaml`/`.yml`).

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| CLI pattern | `r8r migrate n8n <file>` subcommand | Extensible for future sources (windmill, temporal) without breaking changes |
| Unsupported nodes | Convert to HTTP placeholder with TODO + original config as comments | Always produces runnable output, user knows what needs manual work |
| Expression conversion | Full JS-to-Rhai transpiler (3 tiers) | Covers ~90% of real expressions automatically |
| n8n format knowledge | Typed Rust structs via serde | Compile-time safety, serves as documentation |
| Output format | YAML default, `--format json` option, stdout default with `--output` flag | Flexible for piping, inspection, and direct use |
| JSON workflow support | Accept `.json` alongside `.yaml`/`.yml` natively | serde_yaml already parses JSON; just extend file discovery |

---

## 1. n8n JSON Format (Input)

n8n exports workflows as JSON:

```json
{
  "name": "My Workflow",
  "nodes": [
    {
      "id": "uuid-here",
      "name": "HTTP Request",
      "type": "n8n-nodes-base.httpRequest",
      "typeVersion": 4,
      "position": [250, 300],
      "parameters": {
        "url": "https://api.example.com",
        "method": "GET",
        "headers": { "Authorization": "Bearer {{ $json.token }}" }
      },
      "credentials": {
        "httpBasicAuth": { "id": "1", "name": "My Auth" }
      }
    }
  ],
  "connections": {
    "Trigger": {
      "main": [[{ "node": "HTTP Request", "type": "main", "index": 0 }]]
    }
  },
  "settings": {
    "executionOrder": "v1"
  },
  "triggerCount": 1
}
```

Key characteristics:
- Nodes identified by `name` (not id) in connections
- `type` format: `n8n-nodes-base.httpRequest` — the part after the last `.` is the node type
- Connections are adjacency lists keyed by source node name, with `main` outputs (index 0, 1 for true/false branches on IF)
- Expressions use `{{ }}` delimiters with JavaScript inside: `$json`, `$node`, `$now`, `$execution`

The parser deserializes this into typed Rust structs via serde.

**Credential extraction:** The converter scans every node's `credentials` field (node-level, not top-level) and collects all referenced credential names. These are emitted as `CredentialReference` warnings in the output, prompting the user to set them up manually in r8r's credential store.

---

## 2. Node Type Mapping

### Exact mappings (~20 known types)

| n8n Type | r8r Type | Config Translation |
|----------|----------|-------------------|
| `httpRequest` | `http` | url, method, headers, body direct mapping |
| `if` | `if` | See "IF Node Translation" below |
| `switch` | `switch` | See "Switch Node Translation" below |
| `set` | `set` | assignments → r8r set config |
| `code` | `sandbox` | JS → `runtime: node`, Python → `runtime: python3` |
| `merge` | `merge` | mode mapping |
| `splitInBatches` | `split` + `for_each: true` | batch size → chunk_size |
| `wait` | `wait` | duration mapping |
| `noOp` | `debug` | passthrough → debug with log_input |
| `manualTrigger` | trigger `type: manual` | — |
| `cron` / `scheduleTrigger` | trigger `type: cron` | cron expression mapping |
| `webhook` | trigger `type: webhook` | path, method mapping |
| `emailSend` | `email` | to, subject, body mapping |
| `slack` | `integration` (service: slack) | maps to YAML integration layer |
| `openAi` | `integration` (service: openai) or `agent` | maps to agent node or integration |
| `filter` | `filter` | conditions mapping |
| `sort` | `sort` | field + order mapping |
| `limit` | `limit` | limit + offset mapping |
| `aggregate` | `aggregate` | mode mapping |
| `crypto` | `crypto` | operation mapping |
| `dateTime` | `datetime` | operation mapping |

### Unsupported nodes

Nodes without a known mapping get converted to a placeholder HTTP node:

```yaml
- id: google-sheets-1
  type: http
  config:
    url: "# TODO: Convert n8n node 'Google Sheets' (n8n-nodes-base.googleSheets)"
    method: GET
  # WARNING: This node was auto-generated from an unsupported n8n node type.
  # Original n8n config preserved below for reference:
  # n8n_type: n8n-nodes-base.googleSheets
  # n8n_params: {"operation": "read", "sheetId": "abc123"}
```

### IF Node Translation

n8n IF nodes use JavaScript expressions for conditions (e.g., `{{ $json.status === "active" }}`). r8r IF nodes expect `{field, operator, value}` tuples. The converter handles this in two tiers:

**Simple conditions** — pattern-match n8n expressions into r8r tuples (using actual r8r operator names from `if_node.rs`):
- `$json.field === "value"` → `{ field: "input.field", operator: "equals", value: "value" }`
- `$json.field !== "value"` → `{ field: "input.field", operator: "not_equals", value: "value" }`
- `$json.field > 100` → `{ field: "input.field", operator: "gt", value: 100 }`
- `$json.field < 100` → `{ field: "input.field", operator: "lt", value: 100 }`
- `$json.field >= 100` → `{ field: "input.field", operator: "gte", value: 100 }`
- `$json.field <= 100` → `{ field: "input.field", operator: "lte", value: 100 }`
- `$json.field.includes("sub")` → `{ field: "input.field", operator: "contains", value: "sub" }`
- `$json.field.match(/pattern/)` → `{ field: "input.field", operator: "regex", value: "pattern" }`

**Complex conditions** — when the expression cannot be decomposed into tuples (e.g., compound logic, function calls, `array.length > 0`), the converter uses a **transform + if** pattern instead of the IF node's `condition` field (which is a gate, not a branch selector):

```yaml
# Pre-evaluate complex condition into a boolean field
- id: complex-check-eval
  type: transform
  config:
    expression: |
      let data = from_json(input);
      let result = data.amount > 100 && data.status == "active";
      to_json(#{ passed: result, data: data })
  depends_on: [previous-node]

# Branch based on the boolean result
- id: complex-check
  type: if
  config:
    conditions:
      - field: "input.passed"
        operator: "equals"
        value: true
    true_branch: success-node
    false_branch: failure-node
  depends_on: [complex-check-eval]
```

This avoids the issue where r8r's IF node rejects empty conditions arrays. The transform node evaluates the complex Rhai expression and produces a boolean, which the IF node can check with a simple `equals` condition. The converter emits an `Approximate` warning for these cases.

### Switch Node Translation

n8n's Switch node routes to different downstream nodes via output indices (0, 1, 2, ...). r8r's Switch node evaluates an expression and matches against case values, producing output data per case.

Structural mismatch: n8n Switch *routes*, r8r Switch *produces output*. The converter bridges this by:

1. **Extract the switch expression** — n8n Switch typically tests a single field (e.g., `$json.type`). Convert to Rhai: `input.type`.
2. **Map output indices to cases** — n8n output 0 → case value from rule 0, output 1 → case value from rule 1, etc.
3. **Wire downstream nodes** — each downstream node gets a `condition` that parses the switch output (which is a JSON string in Rhai scope) and checks `matched_case`.

**Important runtime detail:** The executor pushes node outputs into Rhai scope as JSON strings, not parsed objects. So `route_by_type` in Rhai is a string like `"{\"matched_case\":\"order\"}"`, not a structured object. Conditions must parse first.

Example conversion:

```yaml
# n8n Switch with 3 outputs: "order", "invoice", "other"
- id: route-by-type
  type: switch
  config:
    expression: "input.type"
    cases:
      - value: "order"
        output: { matched_case: "order" }
      - value: "invoice"
        output: { matched_case: "invoice" }
    default: { matched_case: "other" }

- id: process-order
  type: http
  config: { ... }
  depends_on: [route-by-type]
  condition: |
    let sw = from_json(route_by_type);
    sw.matched_case == "order"
```

Each downstream node parses the switch output with `from_json()` and checks `matched_case`. The converter emits `Approximate` warnings for all switch translations, noting the `from_json` pattern.

**Alternative approach (if `from_json` is not available in condition evaluation):** The converter can instead decompose the switch into multiple IF nodes — one per branch — each checking the original field directly:

```yaml
- id: is-order
  type: if
  config:
    conditions:
      - field: "input.type"
        operator: "equals"
        value: "order"
    true_branch: process-order
    false_branch: check-invoice
```

The converter should attempt the `from_json` pattern first. If testing reveals `from_json` is not available in condition scope, fall back to the IF-chain decomposition.

### Name Sanitization and Deduplication

n8n node names like `"HTTP Request"`, `"HTTP Request1"` are sanitized to r8r IDs:
- Lowercase, spaces to hyphens, strip special chars
- Deduplication: if `http-request` already exists, append `-2`, `-3`, etc.
- Example: `"HTTP Request"` → `http-request`, `"HTTP Request1"` → `http-request-2`

### Sandbox Feature Gate

When converting n8n `code` nodes to r8r `sandbox`, the converter emits a warning noting that the `sandbox` feature must be enabled at compile time (`--features sandbox`). If the target r8r build does not include it, the node will fail at runtime.

---

## 3. JS-to-Rhai Expression Transpiler

Converts n8n JavaScript expressions inside `{{ }}` delimiters to Rhai equivalents. Returns one of three results: `Exact(rhai)`, `Approximate(rhai, warning)`, or `Failed(original, reason)`.

### Tier 1 — Direct pattern replacement (~70% of expressions)

| n8n JS | Rhai |
|--------|------|
| `$json.field` | `input.field` |
| `$json["field"]` | `input.field` |
| `$json.nested.path` | `input.nested.path` |
| `$node["Name"].json.field` | `nodes.name.output.field` |
| `$now` | `now()` |
| `$execution.id` | `execution_id` |
| `$input.item.json.field` | `input.field` |
| `$env.VAR_NAME` | `env.VAR_NAME` |
| `"string" + variable` | `"string" + variable` |
| `=== / !==` | `== / !=` |

### Tier 2 — Common patterns (~20% more)

| n8n JS | Rhai |
|--------|------|
| `condition ? a : b` | `if condition { a } else { b }` |
| `array.length` | `len(array)` |
| `array.map(x => x.field)` | `array.map(\|x\| x.field)` |
| `array.filter(x => x.active)` | `array.filter(\|x\| x.active)` |
| `str.includes("sub")` | `str.contains("sub")` |
| `str.toLowerCase()` | `str.to_lower()` |
| `str.toUpperCase()` | `str.to_upper()` |
| `str.split(",")` | `str.split(",")` |
| `parseInt(x)` | `parse_int(x)` |
| `JSON.parse(x)` | `from_json(x)` |
| `JSON.stringify(x)` | `to_json(x)` |
| `Math.round/floor/ceil` | `round/floor/ceil` |
| `Object.keys(x)` | `x.keys()` |

### Tier 3 — Unrecognized JS (~10%)

Wrapped in a comment with the original preserved:

```yaml
expression: |
  // TODO: Convert n8n expression manually
  // Original: {{ $json.items.reduce((sum, i) => sum + i.price, 0) }}
  0
```

---

## 4. Connection → depends_on Conversion

n8n stores connections as an adjacency list (source → targets). The converter:

1. **Inverts the graph** — for each target node, collect source nodes → `depends_on`
2. **Sanitizes node names** — `"HTTP Request"` → `http-request` (lowercase, spaces to hyphens, strip special chars)
3. **Handles IF/Switch branches** — n8n output index 0 = true, index 1 = false. Sets `true_branch` / `false_branch` on the r8r `if` node using downstream node IDs
4. **Trigger nodes** — become r8r `triggers:` entries, not workflow nodes. Their downstream connections become root nodes (no `depends_on`)
5. **Merge/fan-in** — nodes with multiple incoming connections get `depends_on: [source1, source2]`

---

## 5. CLI Interface + Output

### Command

```
r8r migrate n8n <input-file> [--output <file>] [--format yaml|json]
```

- No `--output` → print to stdout
- `--output path.yaml` → write to file
- `--format yaml` (default) or `--format json`
- Input must be valid JSON (n8n export format)

### Output YAML

```yaml
# Migrated from n8n workflow: "My Workflow"
# Generated by: r8r migrate n8n
# Date: 2026-03-24
#
# Migration warnings:
#   - Node "Google Sheets" (n8n-nodes-base.googleSheets): unsupported, converted to HTTP placeholder
#   - Expression in "Set Data": approximate conversion (ternary operator)
#   - Credential "myGoogleAuth" referenced but not migrated (set up manually)

name: my-workflow
description: "Migrated from n8n: My Workflow"
version: 1

triggers:
  - type: webhook
    path: /webhooks/my-workflow

nodes:
  - id: fetch-data
    type: http
    config:
      url: "https://api.example.com/data"
      method: GET
```

### Console output (stderr)

```
Migrating n8n workflow: "My Workflow" (12 nodes, 11 connections)

  ✓ Manual Trigger → trigger (manual)
  ✓ HTTP Request → http
  ✓ IF → if
  ✓ Set → set
  ✓ Code → sandbox (node)
  ⚠ Google Sheets → http (unsupported node, placeholder)
  ⚠ Expression: approximate conversion in "Set Data"

Result: 10/12 nodes converted exactly, 2 warnings
```

---

## 6. JSON Workflow Support

r8r natively accepts `.json` workflow files alongside `.yaml`/`.yml` for commands that take file paths.

### Parser changes

The workflow parser branches on file extension:
- `.json` → `serde_json::from_str()` for proper JSON parsing and error messages
- `.yaml` / `.yml` → `serde_yaml::from_str()` (existing behavior)

This avoids confusing YAML-flavored error messages when parsing JSON files.

### Which commands are affected

Commands that accept **file paths** gain `.json` support:
- `r8r workflows create my-workflow.json` — import from JSON file
- `r8r dev my-workflow.json` — dev mode with JSON file
- `r8r lint my-workflow.json` — validate JSON workflow file

`r8r run` takes a **workflow name** (from storage), not a file path — unchanged.

### Migration output

```
r8r migrate n8n workflow.json                     # outputs YAML (default)
r8r migrate n8n workflow.json --format json        # outputs JSON
r8r migrate n8n workflow.json --format yaml        # explicit YAML
```

---

## 7. File Layout

### New Files

### Core types

```rust
/// Result of a migration operation.
pub struct MigrateResult {
    /// The converted r8r workflow.
    pub workflow: Workflow,
    /// Warnings generated during conversion (structured for CLI + YAML comments).
    pub warnings: Vec<MigrateWarning>,
}

pub struct MigrateWarning {
    pub node_name: Option<String>,   // which n8n node this relates to
    pub category: WarningCategory,
    pub message: String,
}

pub enum WarningCategory {
    UnsupportedNode,
    ApproximateExpression,
    UnconvertedExpression,
    CredentialReference,
    FeatureGate,         // e.g., sandbox requires --features sandbox
}

/// Trait for migration sources (extensible for future platforms).
pub trait MigrateSource {
    fn name(&self) -> &str;
    fn convert(&self, input: &[u8]) -> Result<MigrateResult>;
}
```

### File layout

```
src/
├── migrate/
│   ├── mod.rs              # Module root, MigrateSource trait, MigrateResult, MigrateWarning
│   └── n8n/
│       ├── mod.rs           # Public API: migrate_n8n(input) -> MigrateResult
│       ├── parser.rs        # n8n JSON structs (N8nWorkflow, N8nNode, N8nConnection)
│       ├── converter.rs     # N8nWorkflow → r8r Workflow conversion logic
│       ├── node_map.rs      # n8n node type → r8r node type mapping + config transformers
│       └── expressions.rs   # JS-to-Rhai transpiler
tests/
├── fixtures/n8n/
│   ├── simple-http.json     # Trigger + HTTP request
│   ├── if-branch.json       # IF with true/false branches
│   └── complex.json         # 10+ nodes, mixed types, expressions
├── migrate_n8n_tests.rs     # Integration tests
```

### Changes to Existing Files

- `src/lib.rs` — add `pub mod migrate;`
- `src/main.rs` — add `Migrate` subcommand with `N8n` variant
- Storage layer — extend file extension checks to include `.json`

---

## 8. Testing Strategy

### Unit Tests

- `parser.rs` — parse real n8n JSON exports (minimal, medium, complex)
- `expressions.rs` — Tier 1 patterns (15+ cases), Tier 2 patterns (12+ cases), Tier 3 fallback
- `node_map.rs` — each supported node type maps correctly
- `converter.rs` — connections → depends_on, trigger extraction, name sanitization, IF branch wiring

### Integration Tests (with fixtures)

- `tests/fixtures/n8n/simple-http.json` — trigger + HTTP request, verify exact YAML output
- `tests/fixtures/n8n/if-branch.json` — IF with true/false branches, verify branch wiring
- `tests/fixtures/n8n/complex.json` — 10+ nodes, mixed types, expressions, verify warnings

### Out of Scope

- Testing every n8n node type (400+)
- Real n8n API exports (fixtures only)
- Round-trip fidelity (converted workflow may need manual tuning)
