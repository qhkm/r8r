# n8n Migration Converter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `r8r migrate n8n` CLI command that converts n8n workflow JSON exports to r8r YAML format, plus native JSON workflow support.

**Architecture:** A new `src/migrate/` module with n8n-specific parser (typed Rust structs for n8n JSON), node type mapper (20+ known types → r8r equivalents), JS-to-Rhai expression transpiler (3 tiers: direct/common/fallback), and connection graph inverter (adjacency list → depends_on arrays). The converter always produces runnable output — unsupported nodes become HTTP placeholders with TODO comments.

**Tech Stack:** Rust, serde/serde_json for n8n JSON parsing, serde_yaml for r8r YAML output, regex for expression transpilation, existing r8r Workflow types.

**Spec:** `docs/superpowers/specs/2026-03-24-n8n-migration-converter-design.md`

---

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `src/migrate/mod.rs` | Module root, `MigrateResult`, `MigrateWarning`, `WarningCategory`, `MigrateSource` trait |
| `src/migrate/n8n/mod.rs` | Public API: `migrate_n8n(input: &[u8]) -> Result<MigrateResult>` |
| `src/migrate/n8n/parser.rs` | n8n JSON structs: `N8nWorkflow`, `N8nNode`, `N8nConnection`, `N8nCredentialRef` |
| `src/migrate/n8n/expressions.rs` | JS-to-Rhai transpiler: `transpile_expression(js: &str) -> TranspileResult` |
| `src/migrate/n8n/node_map.rs` | n8n type → r8r type mapping + per-type config transformers |
| `src/migrate/n8n/converter.rs` | Full workflow conversion: connections, triggers, name sanitization, output assembly |
| `tests/fixtures/n8n/simple-http.json` | Fixture: trigger + HTTP request |
| `tests/fixtures/n8n/if-branch.json` | Fixture: IF with true/false branches |
| `tests/fixtures/n8n/expressions.json` | Fixture: various JS expressions |
| `tests/migrate_n8n_tests.rs` | Integration tests |

### Modified Files

| File | Change |
|------|--------|
| `src/lib.rs` | Add `pub mod migrate;` |
| `src/main.rs:24-238` | Add `Migrate` subcommand to `Commands` enum |
| `src/main.rs:736+` | Add match arm for `Migrate` |
| `src/workflow/parser.rs:31-35` | Branch on `.json` extension → use `serde_json` |

---

## Task 1: Core Types (MigrateResult, MigrateWarning, MigrateSource)

**Files:**
- Create: `src/migrate/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create mod.rs with core types**

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Workflow migration from other platforms.

pub mod n8n;

use crate::error::Result;
use crate::workflow::types::Workflow;

/// Result of a migration operation.
#[derive(Debug)]
pub struct MigrateResult {
    /// The converted r8r workflow.
    pub workflow: Workflow,
    /// Warnings generated during conversion.
    pub warnings: Vec<MigrateWarning>,
}

/// A warning emitted during migration.
#[derive(Debug, Clone)]
pub struct MigrateWarning {
    /// Which n8n node this relates to (None for workflow-level warnings).
    pub node_name: Option<String>,
    /// Warning category.
    pub category: WarningCategory,
    /// Human-readable message.
    pub message: String,
}

/// Categories of migration warnings.
#[derive(Debug, Clone, PartialEq)]
pub enum WarningCategory {
    /// Node type has no r8r equivalent.
    UnsupportedNode,
    /// Expression was approximately converted.
    ApproximateExpression,
    /// Expression could not be converted.
    UnconvertedExpression,
    /// Credential referenced but not migrated.
    CredentialReference,
    /// Feature gate required (e.g., sandbox).
    FeatureGate,
}

impl std::fmt::Display for MigrateWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.node_name {
            Some(name) => write!(f, "Node \"{}\": {}", name, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// Trait for migration sources (extensible for future platforms).
pub trait MigrateSource {
    fn name(&self) -> &str;
    fn convert(&self, input: &[u8]) -> Result<MigrateResult>;
}
```

- [ ] **Step 2: Create empty n8n module**

Create `src/migrate/n8n/mod.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! n8n workflow migration.
```

- [ ] **Step 3: Add to lib.rs**

In `src/lib.rs`, add after `pub mod llm;`:

```rust
pub mod migrate;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check`
Expected: Compiles clean

- [ ] **Step 5: Commit**

```bash
git add src/migrate/ src/lib.rs
git commit -m "feat(migrate): add core migration types (MigrateResult, MigrateWarning, MigrateSource)"
```

---

## Task 2: n8n JSON Parser

**Files:**
- Create: `src/migrate/n8n/parser.rs`
- Modify: `src/migrate/n8n/mod.rs`
- Test: inline `#[cfg(test)]` in `parser.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_n8n_workflow() {
        let json = r#"{
            "name": "Test Workflow",
            "nodes": [
                {
                    "id": "1",
                    "name": "Manual Trigger",
                    "type": "n8n-nodes-base.manualTrigger",
                    "typeVersion": 1,
                    "position": [250, 300],
                    "parameters": {}
                },
                {
                    "id": "2",
                    "name": "HTTP Request",
                    "type": "n8n-nodes-base.httpRequest",
                    "typeVersion": 4,
                    "position": [450, 300],
                    "parameters": {
                        "url": "https://api.example.com",
                        "method": "GET"
                    }
                }
            ],
            "connections": {
                "Manual Trigger": {
                    "main": [[{"node": "HTTP Request", "type": "main", "index": 0}]]
                }
            }
        }"#;

        let workflow = parse_n8n_workflow(json.as_bytes()).unwrap();
        assert_eq!(workflow.name, "Test Workflow");
        assert_eq!(workflow.nodes.len(), 2);
        assert_eq!(workflow.nodes[0].name, "Manual Trigger");
        assert_eq!(workflow.nodes[0].node_type(), "manualTrigger");
        assert_eq!(workflow.nodes[1].parameters["url"], "https://api.example.com");
        assert!(workflow.connections.contains_key("Manual Trigger"));
    }

    #[test]
    fn test_parse_node_with_credentials() {
        let json = r#"{
            "name": "Auth Workflow",
            "nodes": [
                {
                    "id": "1",
                    "name": "GitHub",
                    "type": "n8n-nodes-base.github",
                    "typeVersion": 1,
                    "position": [250, 300],
                    "parameters": {"resource": "issue"},
                    "credentials": {
                        "githubApi": {"id": "1", "name": "My GitHub"}
                    }
                }
            ],
            "connections": {}
        }"#;

        let workflow = parse_n8n_workflow(json.as_bytes()).unwrap();
        let node = &workflow.nodes[0];
        assert!(node.credentials.is_some());
        let creds = node.credentials.as_ref().unwrap();
        assert!(creds.contains_key("githubApi"));
        assert_eq!(creds["githubApi"].name, "My GitHub");
    }

    #[test]
    fn test_node_type_extraction() {
        let node = N8nNode {
            id: "1".into(),
            name: "Test".into(),
            type_name: "n8n-nodes-base.httpRequest".into(),
            type_version: 1,
            position: vec![0, 0],
            parameters: serde_json::json!({}),
            credentials: None,
        };
        assert_eq!(node.node_type(), "httpRequest");
    }

    #[test]
    fn test_node_type_community_package() {
        let node = N8nNode {
            id: "1".into(),
            name: "Test".into(),
            type_name: "n8n-nodes-custom.myNode".into(),
            type_version: 1,
            position: vec![0, 0],
            parameters: serde_json::json!({}),
            credentials: None,
        };
        assert_eq!(node.node_type(), "myNode");
    }
}
```

- [ ] **Step 2: Write implementation**

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! n8n JSON format parser.

use std::collections::HashMap;
use serde::Deserialize;
use serde_json::Value;
use crate::error::{Error, Result};

/// Top-level n8n workflow export.
#[derive(Debug, Deserialize)]
pub struct N8nWorkflow {
    pub name: String,
    pub nodes: Vec<N8nNode>,
    #[serde(default)]
    pub connections: HashMap<String, N8nNodeConnections>,
    #[serde(default)]
    pub settings: Option<Value>,
}

/// An n8n node.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(rename = "typeVersion", default)]
    pub type_version: u32,
    #[serde(default)]
    pub position: Vec<i32>,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default)]
    pub credentials: Option<HashMap<String, N8nCredentialRef>>,
}

impl N8nNode {
    /// Extract the short node type from the full type name.
    /// "n8n-nodes-base.httpRequest" → "httpRequest"
    pub fn node_type(&self) -> &str {
        self.type_name
            .rsplit('.')
            .next()
            .unwrap_or(&self.type_name)
    }
}

/// A credential reference in an n8n node.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nCredentialRef {
    pub id: String,
    pub name: String,
}

/// Connections from a single n8n node.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nNodeConnections {
    #[serde(default)]
    pub main: Vec<Vec<N8nConnectionTarget>>,
}

/// A connection target.
#[derive(Debug, Clone, Deserialize)]
pub struct N8nConnectionTarget {
    pub node: String,
    #[serde(rename = "type", default)]
    pub conn_type: String,
    #[serde(default)]
    pub index: usize,
}

/// Parse an n8n workflow from JSON bytes.
pub fn parse_n8n_workflow(input: &[u8]) -> Result<N8nWorkflow> {
    serde_json::from_slice(input)
        .map_err(|e| Error::Parse(format!("Failed to parse n8n workflow JSON: {}", e)))
}
```

Update `src/migrate/n8n/mod.rs`:

```rust
pub mod parser;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::n8n::parser::tests -v`
Expected: 4 tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/migrate/n8n/parser.rs src/migrate/n8n/mod.rs
git commit -m "feat(migrate): add n8n JSON parser with typed structs"
```

---

## Task 3: JS-to-Rhai Expression Transpiler

**Files:**
- Create: `src/migrate/n8n/expressions.rs`
- Modify: `src/migrate/n8n/mod.rs`
- Test: inline `#[cfg(test)]`

This is the largest single task. The transpiler has 3 tiers.

- [ ] **Step 1: Write tests (Tier 1 — direct replacement)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Tier 1 tests
    #[test]
    fn test_json_field_access() {
        assert_eq!(transpile("$json.name").rhai(), "input.name");
        assert_eq!(transpile("$json.nested.path").rhai(), "input.nested.path");
    }

    #[test]
    fn test_json_bracket_access() {
        assert_eq!(transpile("$json[\"field\"]").rhai(), "input.field");
    }

    #[test]
    fn test_node_reference() {
        assert_eq!(
            transpile("$node[\"HTTP Request\"].json.data").rhai(),
            "nodes.http_request.output.data"
        );
    }

    #[test]
    fn test_builtins() {
        assert_eq!(transpile("$now").rhai(), "now()");
        assert_eq!(transpile("$execution.id").rhai(), "execution_id");
        assert_eq!(transpile("$env.API_KEY").rhai(), "env.API_KEY");
    }

    #[test]
    fn test_strict_equality() {
        assert_eq!(transpile("$json.x === \"y\"").rhai(), "input.x == \"y\"");
        assert_eq!(transpile("$json.x !== 0").rhai(), "input.x != 0");
    }

    #[test]
    fn test_input_item() {
        assert_eq!(transpile("$input.item.json.field").rhai(), "input.field");
    }

    // Tier 2 tests
    #[test]
    fn test_ternary() {
        let result = transpile("$json.active ? \"yes\" : \"no\"");
        assert!(result.rhai().contains("if"));
        assert!(result.rhai().contains("input.active"));
    }

    #[test]
    fn test_array_length() {
        assert_eq!(transpile("$json.items.length").rhai(), "len(input.items)");
    }

    #[test]
    fn test_string_methods() {
        assert_eq!(transpile("$json.name.includes(\"test\")").rhai(), "input.name.contains(\"test\")");
        assert_eq!(transpile("$json.name.toLowerCase()").rhai(), "input.name.to_lower()");
        assert_eq!(transpile("$json.name.toUpperCase()").rhai(), "input.name.to_upper()");
    }

    #[test]
    fn test_json_parse_stringify() {
        assert_eq!(transpile("JSON.parse($json.data)").rhai(), "from_json(input.data)");
        assert_eq!(transpile("JSON.stringify($json.obj)").rhai(), "to_json(input.obj)");
    }

    #[test]
    fn test_parseint() {
        assert_eq!(transpile("parseInt($json.count)").rhai(), "parse_int(input.count)");
    }

    // Tier 3 tests
    #[test]
    fn test_unrecognized_fallback() {
        let result = transpile("$json.items.reduce((sum, i) => sum + i.price, 0)");
        assert!(result.is_failed());
    }

    // Helper
    fn transpile(js: &str) -> TranspileResult {
        transpile_expression(js)
    }
}
```

- [ ] **Step 2: Write implementation**

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! JavaScript to Rhai expression transpiler for n8n migration.

use regex_lite::Regex;

/// Result of transpiling a JS expression to Rhai.
#[derive(Debug, Clone)]
pub enum TranspileResult {
    /// Exact conversion — semantically equivalent.
    Exact(String),
    /// Approximate conversion — mostly equivalent, with a warning.
    Approximate(String, String),
    /// Failed — could not convert, original preserved.
    Failed(String, String),
}

impl TranspileResult {
    /// Get the Rhai expression (or original for Failed).
    pub fn rhai(&self) -> &str {
        match self {
            Self::Exact(s) | Self::Approximate(s, _) => s,
            Self::Failed(original, _) => original,
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_, _))
    }

    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Approximate(_, w) | Self::Failed(_, w) => Some(w),
            Self::Exact(_) => None,
        }
    }
}

/// Transpile an n8n JavaScript expression to Rhai.
pub fn transpile_expression(js: &str) -> TranspileResult {
    let mut expr = js.trim().to_string();

    // Strip surrounding {{ }} if present
    if expr.starts_with("{{") && expr.ends_with("}}") {
        expr = expr[2..expr.len() - 2].trim().to_string();
    }

    // Try Tier 1 (direct replacements)
    let result = apply_tier1(&expr);

    // Try Tier 2 (common patterns)
    let result = apply_tier2(&result);

    // Check if any JS-specific syntax remains
    if contains_unconverted_js(&result) {
        TranspileResult::Failed(
            result,
            "Contains unconverted JavaScript syntax".to_string(),
        )
    } else if result != apply_tier1(&expr) {
        // Tier 2 made changes — approximate
        TranspileResult::Approximate(
            result.clone(),
            "Expression approximately converted".to_string(),
        )
    } else if result != expr {
        TranspileResult::Exact(result)
    } else {
        // Nothing changed and no JS detected — pass through
        TranspileResult::Exact(result)
    }
}

/// Tier 1: Direct pattern replacements.
fn apply_tier1(expr: &str) -> String {
    let mut result = expr.to_string();

    // $input.item.json.X → input.X
    result = regex_replace(&result, r"\$input\.item\.json\.(\w+)", "input.$1");

    // $json["field"] → input.field
    result = regex_replace(&result, r#"\$json\["(\w+)"\]"#, "input.$1");

    // $json.X → input.X
    result = result.replace("$json.", "input.");
    result = result.replace("$json", "input");

    // $node["Name"].json.X → nodes.sanitized_name.output.X
    let node_ref_re = Regex::new(r#"\$node\["([^"]+)"\]\.json\.(\w+)"#).unwrap();
    result = node_ref_re.replace_all(&result, |caps: &regex_lite::Captures| {
        let name = sanitize_node_name(&caps[1]);
        let field = &caps[2];
        format!("nodes.{}.output.{}", name, field)
    }).to_string();

    // $node["Name"].json → nodes.sanitized_name.output
    let node_json_re = Regex::new(r#"\$node\["([^"]+)"\]\.json"#).unwrap();
    result = node_json_re.replace_all(&result, |caps: &regex_lite::Captures| {
        let name = sanitize_node_name(&caps[1]);
        format!("nodes.{}.output", name)
    }).to_string();

    // $env.X → env.X
    result = result.replace("$env.", "env.");

    // $now → now()
    result = result.replace("$now", "now()");

    // $execution.id → execution_id
    result = result.replace("$execution.id", "execution_id");

    // === → ==, !== → !=
    result = result.replace("===", "==");
    result = result.replace("!==", "!=");

    result
}

/// Tier 2: Common JS patterns → Rhai equivalents.
fn apply_tier2(expr: &str) -> String {
    let mut result = expr.to_string();

    // .length → len()
    let length_re = Regex::new(r"(\w[\w.]*)\.length\b").unwrap();
    result = length_re.replace_all(&result, "len($1)").to_string();

    // .includes("x") → .contains("x")
    result = result.replace(".includes(", ".contains(");

    // .toLowerCase() → .to_lower()
    result = result.replace(".toLowerCase()", ".to_lower()");

    // .toUpperCase() → .to_upper()
    result = result.replace(".toUpperCase()", ".to_upper()");

    // parseInt(x) → parse_int(x)
    result = result.replace("parseInt(", "parse_int(");

    // JSON.parse(x) → from_json(x)
    result = result.replace("JSON.parse(", "from_json(");

    // JSON.stringify(x) → to_json(x)
    result = result.replace("JSON.stringify(", "to_json(");

    // Math.round/floor/ceil
    result = result.replace("Math.round(", "round(");
    result = result.replace("Math.floor(", "floor(");
    result = result.replace("Math.ceil(", "ceil(");

    // Object.keys(x) → x.keys()
    let obj_keys_re = Regex::new(r"Object\.keys\((\w[\w.]*)\)").unwrap();
    result = obj_keys_re.replace_all(&result, "$1.keys()").to_string();

    // Ternary: expr ? a : b → if expr { a } else { b }
    // Simple ternary only (no nested ternaries)
    let ternary_re = Regex::new(r"^(.+?)\s*\?\s*(.+?)\s*:\s*(.+)$").unwrap();
    if let Some(caps) = ternary_re.captures(&result) {
        let condition = caps[1].trim();
        let if_true = caps[2].trim();
        let if_false = caps[3].trim();
        result = format!("if {} {{ {} }} else {{ {} }}", condition, if_true, if_false);
    }

    result
}

/// Check if the expression still contains JS-specific syntax.
fn contains_unconverted_js(expr: &str) -> bool {
    // Common JS patterns that have no Rhai equivalent
    expr.contains(".reduce(")
        || expr.contains(".forEach(")
        || expr.contains(".indexOf(")
        || expr.contains(".splice(")
        || expr.contains(".slice(")
        || expr.contains("new Date(")
        || expr.contains("typeof ")
        || expr.contains("instanceof ")
        || expr.contains("=> {")
        || (expr.contains("=>") && expr.contains(".map("))
        || (expr.contains("=>") && expr.contains(".filter("))
}

/// Sanitize an n8n node name for use in Rhai expressions.
fn sanitize_node_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "_")
        .replace('-', "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn regex_replace(input: &str, pattern: &str, replacement: &str) -> String {
    let re = Regex::new(pattern).unwrap();
    re.replace_all(input, replacement).to_string()
}

/// Transpile all expressions within a string (handling {{ expr }} delimiters).
/// Returns the string with expressions replaced and a list of warnings.
pub fn transpile_all_expressions(input: &str) -> (String, Vec<(TranspileResult, String)>) {
    let mut warnings = Vec::new();
    let re = Regex::new(r"\{\{(.*?)\}\}").unwrap();

    let result = re.replace_all(input, |caps: &regex_lite::Captures| {
        let js_expr = caps[1].trim();
        let transpiled = transpile_expression(js_expr);
        let rhai = transpiled.rhai().to_string();

        if !transpiled.is_exact() {
            warnings.push((transpiled.clone(), js_expr.to_string()));
        }

        format!("{{{{ {} }}}}", rhai)
    }).to_string();

    (result, warnings)
}
```

Update `src/migrate/n8n/mod.rs`:

```rust
pub mod parser;
pub mod expressions;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::n8n::expressions::tests -v`
Expected: 12 tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/migrate/n8n/expressions.rs src/migrate/n8n/mod.rs
git commit -m "feat(migrate): add JS-to-Rhai expression transpiler (3 tiers)"
```

---

## Task 4: Node Type Mapper

**Files:**
- Create: `src/migrate/n8n/node_map.rs`
- Modify: `src/migrate/n8n/mod.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_http_request_mapping() {
        let params = json!({"url": "https://api.example.com", "method": "POST", "body": "{}"});
        let result = map_node("httpRequest", &params);
        assert_eq!(result.r8r_type, "http");
        assert_eq!(result.config["url"], "https://api.example.com");
        assert_eq!(result.config["method"], "POST");
    }

    #[test]
    fn test_code_node_javascript() {
        let params = json!({"language": "javaScript", "jsCode": "return [{json: {result: 42}}]"});
        let result = map_node("code", &params);
        assert_eq!(result.r8r_type, "sandbox");
        assert_eq!(result.config["runtime"], "node");
        assert!(result.warnings.iter().any(|w| w.category == WarningCategory::FeatureGate));
    }

    #[test]
    fn test_code_node_python() {
        let params = json!({"language": "python", "pythonCode": "return [{'json': {'x': 1}}]"});
        let result = map_node("code", &params);
        assert_eq!(result.config["runtime"], "python3");
    }

    #[test]
    fn test_trigger_nodes() {
        assert!(is_trigger_node("manualTrigger"));
        assert!(is_trigger_node("scheduleTrigger"));
        assert!(is_trigger_node("webhook"));
        assert!(!is_trigger_node("httpRequest"));
    }

    #[test]
    fn test_unsupported_node() {
        let params = json!({"operation": "read", "sheetId": "abc123"});
        let result = map_node("googleSheets", &params);
        assert_eq!(result.r8r_type, "http");
        assert!(result.warnings.iter().any(|w| w.category == WarningCategory::UnsupportedNode));
    }

    #[test]
    fn test_set_node() {
        let params = json!({"assignments": {"assignments": [{"name": "key", "value": "val"}]}});
        let result = map_node("set", &params);
        assert_eq!(result.r8r_type, "set");
    }

    #[test]
    fn test_wait_node() {
        let params = json!({"amount": 5, "unit": "seconds"});
        let result = map_node("wait", &params);
        assert_eq!(result.r8r_type, "wait");
    }
}
```

- [ ] **Step 2: Write implementation**

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! n8n node type → r8r node type mapping.

use serde_json::{json, Value};
use crate::migrate::{MigrateWarning, WarningCategory};

/// Result of mapping a single n8n node.
pub struct NodeMapResult {
    pub r8r_type: String,
    pub config: Value,
    pub warnings: Vec<MigrateWarning>,
}

/// Check if an n8n node type is a trigger.
pub fn is_trigger_node(n8n_type: &str) -> bool {
    matches!(
        n8n_type,
        "manualTrigger"
            | "scheduleTrigger"
            | "cron"
            | "webhook"
            | "emailTrigger"
            | "intervalTrigger"
    )
}

/// Map an n8n node type + parameters to r8r node type + config.
pub fn map_node(n8n_type: &str, params: &Value) -> NodeMapResult {
    match n8n_type {
        "httpRequest" => map_http_request(params),
        "set" => map_set(params),
        "code" => map_code(params),
        "wait" => map_wait(params),
        "merge" => map_simple("merge", params),
        "filter" => map_simple("filter", params),
        "sort" => map_simple("sort", params),
        "limit" => map_simple("limit", params),
        "aggregate" => map_simple("aggregate", params),
        "noOp" => NodeMapResult {
            r8r_type: "debug".into(),
            config: json!({"log_input": true}),
            warnings: vec![],
        },
        "emailSend" | "emailSendV2" => map_simple("email", params),
        "slack" => map_integration("slack", params),
        "openAi" => map_integration("openai", params),
        "crypto" => map_simple("crypto", params),
        "dateTime" => map_simple("datetime", params),
        // IF and Switch are handled separately in converter.rs
        // because they need connection context
        "if" | "switch" => map_simple(n8n_type, params),
        _ => map_unsupported(n8n_type, params),
    }
}

fn map_http_request(params: &Value) -> NodeMapResult {
    let mut config = json!({});
    if let Some(url) = params.get("url") {
        config["url"] = url.clone();
    }
    config["method"] = params
        .get("method")
        .cloned()
        .unwrap_or(json!("GET"));
    if let Some(headers) = params.get("headers") {
        config["headers"] = headers.clone();
    }
    if let Some(body) = params.get("body") {
        config["body"] = body.clone();
    }

    NodeMapResult {
        r8r_type: "http".into(),
        config,
        warnings: vec![],
    }
}

fn map_code(params: &Value) -> NodeMapResult {
    let language = params
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("javaScript");

    let (runtime, code_field) = match language {
        "python" => ("python3", "pythonCode"),
        _ => ("node", "jsCode"),
    };

    let code = params
        .get(code_field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    NodeMapResult {
        r8r_type: "sandbox".into(),
        config: json!({
            "runtime": runtime,
            "code": code,
        }),
        warnings: vec![MigrateWarning {
            node_name: None,
            category: WarningCategory::FeatureGate,
            message: format!(
                "Converted to sandbox node (runtime: {}). Requires --features sandbox at compile time.",
                runtime
            ),
        }],
    }
}

fn map_set(params: &Value) -> NodeMapResult {
    NodeMapResult {
        r8r_type: "set".into(),
        config: params.clone(),
        warnings: vec![],
    }
}

fn map_wait(params: &Value) -> NodeMapResult {
    let amount = params.get("amount").and_then(|v| v.as_u64()).unwrap_or(1);
    let unit = params
        .get("unit")
        .and_then(|v| v.as_str())
        .unwrap_or("seconds");

    let seconds = match unit {
        "minutes" => amount * 60,
        "hours" => amount * 3600,
        _ => amount,
    };

    NodeMapResult {
        r8r_type: "wait".into(),
        config: json!({"seconds": seconds}),
        warnings: vec![],
    }
}

fn map_simple(r8r_type: &str, params: &Value) -> NodeMapResult {
    NodeMapResult {
        r8r_type: r8r_type.into(),
        config: params.clone(),
        warnings: vec![],
    }
}

fn map_integration(service: &str, params: &Value) -> NodeMapResult {
    let operation = params
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    NodeMapResult {
        r8r_type: "integration".into(),
        config: json!({
            "service": service,
            "operation": operation,
            "params": params,
        }),
        warnings: vec![],
    }
}

fn map_unsupported(n8n_type: &str, params: &Value) -> NodeMapResult {
    let params_str = serde_json::to_string(params).unwrap_or_default();
    NodeMapResult {
        r8r_type: "http".into(),
        config: json!({
            "url": format!("# TODO: Convert n8n node '{}' (n8n-nodes-base.{})", n8n_type, n8n_type),
            "method": "GET",
        }),
        warnings: vec![MigrateWarning {
            node_name: None,
            category: WarningCategory::UnsupportedNode,
            message: format!(
                "Unsupported n8n node type '{}', converted to HTTP placeholder. Original params: {}",
                n8n_type,
                if params_str.len() > 200 { &params_str[..200] } else { &params_str }
            ),
        }],
    }
}
```

Update `src/migrate/n8n/mod.rs`:

```rust
pub mod parser;
pub mod expressions;
pub mod node_map;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::n8n::node_map::tests -v`
Expected: 7 tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/migrate/n8n/node_map.rs src/migrate/n8n/mod.rs
git commit -m "feat(migrate): add n8n node type mapper (20+ known types, placeholder for unknown)"
```

---

## Task 5: Workflow Converter (Core)

**Files:**
- Create: `src/migrate/n8n/converter.rs`
- Modify: `src/migrate/n8n/mod.rs`
- Test: inline `#[cfg(test)]`

This is the most complex task — it assembles everything: parses n8n JSON, maps nodes, transpiles expressions, inverts connections, sanitizes names, extracts triggers, and builds the final r8r Workflow.

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_node_name("HTTP Request"), "http-request");
        assert_eq!(sanitize_node_name("My Node #1"), "my-node-1");
        assert_eq!(sanitize_node_name("  spaces  "), "spaces");
    }

    #[test]
    fn test_deduplicate_names() {
        let mut used = std::collections::HashSet::new();
        assert_eq!(deduplicate_name("http-request", &mut used), "http-request");
        assert_eq!(deduplicate_name("http-request", &mut used), "http-request-2");
        assert_eq!(deduplicate_name("http-request", &mut used), "http-request-3");
    }

    #[test]
    fn test_invert_connections() {
        let n8n_json = r#"{
            "name": "Test",
            "nodes": [
                {"id": "1", "name": "Trigger", "type": "n8n-nodes-base.manualTrigger", "position": [0,0], "parameters": {}},
                {"id": "2", "name": "Step A", "type": "n8n-nodes-base.httpRequest", "position": [0,0], "parameters": {}},
                {"id": "3", "name": "Step B", "type": "n8n-nodes-base.set", "position": [0,0], "parameters": {}}
            ],
            "connections": {
                "Trigger": {"main": [[{"node": "Step A", "type": "main", "index": 0}]]},
                "Step A": {"main": [[{"node": "Step B", "type": "main", "index": 0}]]}
            }
        }"#;

        let result = convert_n8n_workflow(n8n_json.as_bytes()).unwrap();
        let workflow = &result.workflow;

        // Trigger should be extracted, not a node
        assert!(workflow.triggers.len() == 1);

        // Step A should have no depends_on (root after trigger)
        let step_a = workflow.nodes.iter().find(|n| n.id == "step-a").unwrap();
        assert!(step_a.depends_on.is_empty());

        // Step B should depend on Step A
        let step_b = workflow.nodes.iter().find(|n| n.id == "step-b").unwrap();
        assert_eq!(step_b.depends_on, vec!["step-a"]);
    }

    #[test]
    fn test_credential_extraction() {
        let n8n_json = r#"{
            "name": "Auth Test",
            "nodes": [
                {"id": "1", "name": "GitHub", "type": "n8n-nodes-base.github", "position": [0,0],
                 "parameters": {},
                 "credentials": {"githubApi": {"id": "1", "name": "My GitHub Token"}}}
            ],
            "connections": {}
        }"#;

        let result = convert_n8n_workflow(n8n_json.as_bytes()).unwrap();
        assert!(result.warnings.iter().any(|w|
            w.category == WarningCategory::CredentialReference
            && w.message.contains("My GitHub Token")
        ));
    }

    #[test]
    fn test_workflow_metadata() {
        let n8n_json = r#"{
            "name": "My Cool Workflow",
            "nodes": [
                {"id": "1", "name": "Trigger", "type": "n8n-nodes-base.manualTrigger", "position": [0,0], "parameters": {}}
            ],
            "connections": {}
        }"#;

        let result = convert_n8n_workflow(n8n_json.as_bytes()).unwrap();
        assert_eq!(result.workflow.name, "my-cool-workflow");
        assert!(result.workflow.description.contains("My Cool Workflow"));
    }
}
```

- [ ] **Step 2: Write implementation**

The converter needs to:
1. Parse n8n JSON via `parser::parse_n8n_workflow`
2. Build name mapping (n8n name → sanitized r8r id)
3. Separate trigger nodes from workflow nodes
4. Invert connection graph to `depends_on`
5. Map each node via `node_map::map_node`
6. Transpile expressions in all string values via `expressions::transpile_all_expressions`
7. Extract credential references as warnings
8. Assemble the r8r `Workflow` struct

The implementer should read:
- `src/workflow/types.rs` for the `Workflow`, `Node`, `Trigger` struct definitions
- `src/migrate/n8n/parser.rs` for the input types
- `src/migrate/n8n/node_map.rs` for `map_node()` and `is_trigger_node()`
- `src/migrate/n8n/expressions.rs` for `transpile_all_expressions()`

Key functions to implement:
- `pub fn convert_n8n_workflow(input: &[u8]) -> Result<MigrateResult>` — main entry point
- `fn sanitize_node_name(name: &str) -> String` — lowercase, spaces→hyphens, strip specials
- `fn deduplicate_name(name: &str, used: &mut HashSet<String>) -> String` — append -2, -3 if collision
- `fn extract_triggers(nodes: &[N8nNode]) -> Vec<Trigger>` — convert trigger nodes to r8r Trigger structs
- `fn build_depends_on(connections: &HashMap, name_map: &HashMap, trigger_names: &HashSet) -> HashMap<String, Vec<String>>` — invert adjacency list
- `fn transpile_value_expressions(value: &Value) -> (Value, Vec<MigrateWarning>)` — recursively transpile expressions in JSON values

Update `src/migrate/n8n/mod.rs` to re-export `convert_n8n_workflow`:

```rust
pub mod parser;
pub mod expressions;
pub mod node_map;
pub mod converter;

pub use converter::convert_n8n_workflow;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib migrate::n8n::converter::tests -v`
Expected: 4+ tests PASS

- [ ] **Step 4: Run full suite**

Run: `cargo test`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/migrate/n8n/converter.rs src/migrate/n8n/mod.rs
git commit -m "feat(migrate): add n8n workflow converter with connection inversion, expression transpilation, credential extraction"
```

---

## Task 6: CLI Subcommand (`r8r migrate n8n`)

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Read `src/main.rs` to find the Commands enum and match arms**

The Commands enum is at lines 24-238. The match is in a large `run_command` function. The Integrations subcommand (added recently) is at line 234 with match arm at line 736.

- [ ] **Step 2: Add Migrate subcommand to Commands enum**

After the `Integrations` variant (~line 238):

```rust
    /// Migrate workflows from other platforms
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },
```

Define the enum:

```rust
#[derive(Subcommand)]
enum MigrateCommand {
    /// Migrate an n8n workflow to r8r format
    N8n {
        /// Path to n8n workflow JSON export
        file: std::path::PathBuf,
        /// Output file (default: stdout)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Output format
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}
```

- [ ] **Step 3: Implement handler**

```rust
fn cmd_migrate_n8n(
    file: &std::path::Path,
    output: Option<&std::path::Path>,
    format: &str,
) -> Result<()> {
    let input = std::fs::read(file)
        .map_err(|e| Error::Parse(format!("Failed to read file {:?}: {}", file, e)))?;

    let result = r8r::migrate::n8n::convert_n8n_workflow(&input)?;

    // Print warnings to stderr
    let total_nodes = result.workflow.nodes.len();
    let warning_count = result.warnings.len();
    let exact_count = total_nodes - result.warnings.iter()
        .filter(|w| matches!(w.category,
            r8r::migrate::WarningCategory::UnsupportedNode |
            r8r::migrate::WarningCategory::ApproximateExpression
        ))
        .count();

    eprintln!("Migrating n8n workflow: \"{}\" ({} nodes)\n", result.workflow.name, total_nodes);

    for warning in &result.warnings {
        eprintln!("  ⚠ {}", warning);
    }

    if warning_count > 0 {
        eprintln!("\nResult: {}/{} nodes converted exactly, {} warnings\n",
            exact_count, total_nodes, warning_count);
    } else {
        eprintln!("Result: all {} nodes converted exactly\n", total_nodes);
    }

    // Generate header comment
    let header = format!(
        "# Migrated from n8n workflow: \"{}\"\n# Generated by: r8r migrate n8n\n# Date: {}\n",
        result.workflow.name,
        chrono::Utc::now().format("%Y-%m-%d")
    );

    // Warning comments
    let warning_comments: String = if result.warnings.is_empty() {
        String::new()
    } else {
        let mut s = "#\n# Migration warnings:\n".to_string();
        for w in &result.warnings {
            s.push_str(&format!("#   - {}\n", w));
        }
        s
    };

    // Serialize workflow
    let body = match format {
        "json" => serde_json::to_string_pretty(&result.workflow)
            .map_err(|e| Error::Parse(format!("JSON serialization failed: {}", e)))?,
        _ => serde_yaml::to_string(&result.workflow)
            .map_err(|e| Error::Parse(format!("YAML serialization failed: {}", e)))?,
    };

    let full_output = format!("{}{}\n{}", header, warning_comments, body);

    // Write output
    match output {
        Some(path) => {
            std::fs::write(path, &full_output)
                .map_err(|e| Error::Parse(format!("Failed to write {:?}: {}", path, e)))?;
            eprintln!("Written to {:?}", path);
        }
        None => print!("{}", full_output),
    }

    Ok(())
}
```

- [ ] **Step 4: Add match arm**

```rust
Some(Commands::Migrate { command }) => match command {
    MigrateCommand::N8n { file, output, format } => {
        cmd_migrate_n8n(&file, output.as_deref(), &format)?;
    }
},
```

- [ ] **Step 5: Build and test manually**

Run: `cargo build`
Expected: Compiles

Create a test fixture and run:
Run: `echo '{"name":"Test","nodes":[{"id":"1","name":"Trigger","type":"n8n-nodes-base.manualTrigger","position":[0,0],"parameters":{}}],"connections":{}}' > /tmp/test-n8n.json && cargo run --bin r8r -- migrate n8n /tmp/test-n8n.json`
Expected: YAML output to stdout

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): add 'r8r migrate n8n' command with YAML/JSON output"
```

---

## Task 7: JSON Workflow Support

**Files:**
- Modify: `src/workflow/parser.rs:31-35`

- [ ] **Step 1: Read the current parser**

`src/workflow/parser.rs` line 31-35:
```rust
pub fn parse_workflow_file(path: &Path) -> Result<Workflow> {
    let content = std::fs::read_to_string(path)?;
    parse_workflow(&content)
}
```

- [ ] **Step 2: Branch on extension**

```rust
pub fn parse_workflow_file(path: &Path) -> Result<Workflow> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Error::Parse(format!("Failed to read {:?}: {}", path, e)))?;

    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            serde_json::from_str(&content)
                .map_err(|e| Error::Parse(format!("Failed to parse JSON workflow {:?}: {}", path, e)))
        }
        _ => parse_workflow(&content),
    }
}
```

- [ ] **Step 3: Add serde_json import if not already present**

Check if `serde_json` is already imported in `parser.rs`. If not, add `use serde_json;`.

- [ ] **Step 4: Test**

Run: `cargo test --lib workflow -v`
Expected: All existing workflow tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/workflow/parser.rs
git commit -m "feat(workflow): accept .json workflow files alongside .yaml/.yml"
```

---

## Task 8: Integration Tests with Fixtures

**Files:**
- Create: `tests/fixtures/n8n/simple-http.json`
- Create: `tests/fixtures/n8n/if-branch.json`
- Create: `tests/fixtures/n8n/expressions.json`
- Create: `tests/migrate_n8n_tests.rs`

- [ ] **Step 1: Create fixtures**

`tests/fixtures/n8n/simple-http.json`:
```json
{
  "name": "Simple HTTP Workflow",
  "nodes": [
    {"id": "1", "name": "Manual Trigger", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [250, 300], "parameters": {}},
    {"id": "2", "name": "Fetch Data", "type": "n8n-nodes-base.httpRequest", "typeVersion": 4, "position": [450, 300], "parameters": {"url": "https://api.example.com/data", "method": "GET"}},
    {"id": "3", "name": "Transform", "type": "n8n-nodes-base.set", "typeVersion": 3, "position": [650, 300], "parameters": {"assignments": {"assignments": [{"name": "result", "value": "={{ $json.data }}"}]}}}
  ],
  "connections": {
    "Manual Trigger": {"main": [[{"node": "Fetch Data", "type": "main", "index": 0}]]},
    "Fetch Data": {"main": [[{"node": "Transform", "type": "main", "index": 0}]]}
  }
}
```

`tests/fixtures/n8n/if-branch.json`:
```json
{
  "name": "IF Branch Workflow",
  "nodes": [
    {"id": "1", "name": "Webhook", "type": "n8n-nodes-base.webhook", "typeVersion": 1, "position": [250, 300], "parameters": {"path": "/test", "method": "POST"}},
    {"id": "2", "name": "Check Status", "type": "n8n-nodes-base.if", "typeVersion": 2, "position": [450, 300], "parameters": {"conditions": {"boolean": [{"value1": "={{ $json.active }}", "value2": true}]}}},
    {"id": "3", "name": "Process Active", "type": "n8n-nodes-base.httpRequest", "typeVersion": 4, "position": [650, 200], "parameters": {"url": "https://api.example.com/active", "method": "POST"}},
    {"id": "4", "name": "Log Inactive", "type": "n8n-nodes-base.noOp", "typeVersion": 1, "position": [650, 400], "parameters": {}}
  ],
  "connections": {
    "Webhook": {"main": [[{"node": "Check Status", "type": "main", "index": 0}]]},
    "Check Status": {"main": [
      [{"node": "Process Active", "type": "main", "index": 0}],
      [{"node": "Log Inactive", "type": "main", "index": 0}]
    ]}
  }
}
```

`tests/fixtures/n8n/expressions.json`:
```json
{
  "name": "Expression Test",
  "nodes": [
    {"id": "1", "name": "Trigger", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
    {"id": "2", "name": "With Expressions", "type": "n8n-nodes-base.httpRequest", "typeVersion": 4, "position": [200, 0], "parameters": {
      "url": "https://api.example.com/users/{{ $json.userId }}",
      "method": "POST",
      "body": "{\"name\": \"{{ $json.name.toLowerCase() }}\"}"
    }},
    {"id": "3", "name": "With Credentials", "type": "n8n-nodes-base.github", "typeVersion": 1, "position": [400, 0], "parameters": {"resource": "issue"},
     "credentials": {"githubApi": {"id": "1", "name": "My GitHub"}}}
  ],
  "connections": {
    "Trigger": {"main": [[{"node": "With Expressions", "type": "main", "index": 0}]]},
    "With Expressions": {"main": [[{"node": "With Credentials", "type": "main", "index": 0}]]}
  }
}
```

- [ ] **Step 2: Write integration tests**

```rust
//! Integration tests for n8n migration.

use r8r::migrate::n8n::convert_n8n_workflow;
use r8r::migrate::WarningCategory;

#[test]
fn test_migrate_simple_http() {
    let input = include_bytes!("fixtures/n8n/simple-http.json");
    let result = convert_n8n_workflow(input).unwrap();

    assert_eq!(result.workflow.name, "simple-http-workflow");
    assert!(result.workflow.triggers.len() >= 1);
    assert!(result.workflow.nodes.len() >= 2); // fetch-data + transform

    // Check HTTP node was mapped
    let http_node = result.workflow.nodes.iter()
        .find(|n| n.node_type == "http")
        .expect("Should have an http node");
    assert!(http_node.config.get("url").is_some());
}

#[test]
fn test_migrate_if_branch() {
    let input = include_bytes!("fixtures/n8n/if-branch.json");
    let result = convert_n8n_workflow(input).unwrap();

    // Should have webhook trigger
    assert!(result.workflow.triggers.iter().any(|t|
        matches!(&t.trigger_type, r8r::workflow::types::TriggerType::Webhook { .. })
    ));

    // Should have nodes with dependencies
    assert!(!result.workflow.nodes.is_empty());
}

#[test]
fn test_migrate_expressions() {
    let input = include_bytes!("fixtures/n8n/expressions.json");
    let result = convert_n8n_workflow(input).unwrap();

    // Should have credential warnings
    assert!(result.warnings.iter().any(|w| w.category == WarningCategory::CredentialReference));

    // Should have nodes
    assert!(!result.workflow.nodes.is_empty());
}

#[test]
fn test_migrate_invalid_json() {
    let result = convert_n8n_workflow(b"not json");
    assert!(result.is_err());
}

#[test]
fn test_warnings_are_structured() {
    let input = include_bytes!("fixtures/n8n/expressions.json");
    let result = convert_n8n_workflow(input).unwrap();

    for warning in &result.warnings {
        // All warnings should have a message
        assert!(!warning.message.is_empty());
        // Display trait should work
        let display = format!("{}", warning);
        assert!(!display.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test migrate_n8n_tests -v`
Expected: 5 tests PASS

- [ ] **Step 4: Run full suite**

Run: `cargo test`
Expected: ALL tests PASS

- [ ] **Step 5: Commit**

```bash
git add tests/fixtures/n8n/ tests/migrate_n8n_tests.rs
git commit -m "test(migrate): add n8n migration integration tests with fixtures"
```

---

## Summary

| Task | Description | New Files | Tests |
|------|-------------|-----------|-------|
| 1 | Core types (MigrateResult, MigrateWarning) | `migrate/mod.rs`, `migrate/n8n/mod.rs` | — |
| 2 | n8n JSON parser | `migrate/n8n/parser.rs` | 4 unit |
| 3 | JS-to-Rhai transpiler | `migrate/n8n/expressions.rs` | 12 unit |
| 4 | Node type mapper | `migrate/n8n/node_map.rs` | 7 unit |
| 5 | Workflow converter | `migrate/n8n/converter.rs` | 4 unit |
| 6 | CLI subcommand | — (main.rs modification) | manual |
| 7 | JSON workflow support | — (parser.rs modification) | existing |
| 8 | Integration tests | `migrate_n8n_tests.rs`, 3 fixtures | 5 integration |

**Total: 8 tasks, ~32 new tests, 6 new Rust files, 3 JSON fixtures.**

**Deferred:** Full JS-to-Rhai transpiler for arrow functions (`.map()`, `.filter()`), advanced ternary nesting, and `from_json` registration in executor condition scope.

Tasks 1→2→5 is the critical path. Tasks 3 and 4 can parallel after Task 1. Task 5 depends on 2+3+4. Tasks 6-8 depend on Task 5.
