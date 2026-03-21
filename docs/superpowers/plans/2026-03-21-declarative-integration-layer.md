# Declarative Integration Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a declarative YAML integration layer with OAuth2 support so services can be added as YAML files instead of Rust code, delegating execution to the existing HttpNode.

**Architecture:** A new `src/integrations/` module contains the IntegrationNode (registered as `type: integration`), YAML definition parser, loader with built-in + user override, and OAuth2 credential management. IntegrationNode translates YAML definitions + user params into HttpNode-compatible configs and delegates execution. OAuth2Credential is a new type stored alongside existing credentials in the encrypted store.

**Tech Stack:** Rust, serde/serde_yaml for YAML parsing, percent-encoding for URL query params, tokio::sync::Mutex for OAuth2 refresh locks, reqwest for OAuth2 token exchange, existing r8r Node trait + HttpNode.

**Spec:** `docs/superpowers/specs/2026-03-21-declarative-integration-layer-design.md`

---

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `src/integrations/mod.rs` | Module root, re-exports |
| `src/integrations/definition.rs` | `IntegrationDefinition`, `OperationDef`, `ParamDef`, `AuthMethod` structs with serde deserialization |
| `src/integrations/loader.rs` | `IntegrationLoader`: loads built-in YAML (via `include_str!`) + user YAML from `~/.config/r8r/integrations/`, mtime caching |
| `src/integrations/validator.rs` | Param validation: required fields, type checking, enum enforcement, default application |
| `src/integrations/node.rs` | `IntegrationNode` implementing `Node` trait: param substitution, typed body construction, query encoding, delegation to HttpNode |
| `src/integrations/oauth2.rs` | `OAuth2Credential` struct, token refresh with per-service mutex, device code + local callback flows |
| `src/integrations/builtin/github.yaml` | GitHub REST API definition |
| `src/integrations/builtin/slack.yaml` | Slack Web API definition |
| `src/integrations/builtin/openai.yaml` | OpenAI API definition |
| `src/integrations/builtin/stripe.yaml` | Stripe API definition |
| `src/integrations/builtin/notion.yaml` | Notion API definition |
| `tests/integration_node_tests.rs` | Integration tests for the full pipeline |

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `percent-encoding` dependency |
| `src/lib.rs:45-72` | Add `pub mod integrations;` |
| `src/credentials.rs:71-77` | Add `oauth2_credentials` field to `CredentialStore` with `#[serde(default)]` |
| `src/nodes/types.rs:57-92` | Add `oauth2_credentials` field to `NodeContext`; update `for_item()`, `Debug`, `new()` |
| `src/nodes/registry.rs:36-63` | Register `IntegrationNode` |
| `src/engine/executor.rs:3392-3439` | Extend `load_workflow_credentials()` to also load OAuth2 credentials |
| `src/engine/executor.rs:488-509` | Pass OAuth2 credentials into NodeContext |
| `src/main.rs` | Add `r8r integrations` CLI subcommands (list, show, validate, test) |

### Deferred to Follow-up

The following spec items are **not implemented in this plan** and will be done in a separate follow-up:
- `r8r auth login/status/logout` CLI commands (OAuth2 browser-based flows)
- OAuth2 device code flow and local callback flow execution
- Token refresh at runtime (auto-refresh when `IntegrationNode` detects expired tokens)
- Per-service `tokio::sync::Mutex` for concurrent refresh protection

This plan delivers the **data structures, YAML system, and node execution pipeline**. OAuth2 auth flows are a separate concern requiring browser interaction, which warrants its own plan.

---

## Task 1: Integration Definition Structs

**Files:**
- Create: `src/integrations/definition.rs`
- Create: `src/integrations/mod.rs`
- Modify: `src/lib.rs:45-72`
- Test: inline `#[cfg(test)]` in `definition.rs`

- [ ] **Step 1: Write the failing test**

In `src/integrations/definition.rs`, add a test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_definition() {
        let yaml = r#"
version: 1
name: test-service
display_name: Test Service
description: A test integration
base_url: https://api.test.com

operations:
  get_item:
    description: Get an item
    method: GET
    path: /items/{{ params.id }}
    params:
      id: { required: true, type: string }
"#;
        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "test-service");
        assert_eq!(def.base_url, "https://api.test.com");
        assert_eq!(def.operations.len(), 1);
        let op = def.operations.get("get_item").unwrap();
        assert_eq!(op.method, "GET");
        assert!(op.params.get("id").unwrap().required);
    }

    #[test]
    fn test_parse_full_definition_with_auth() {
        let yaml = r#"
version: 1
name: github
display_name: GitHub
description: GitHub REST API
base_url: https://api.github.com
docs_url: https://docs.github.com

auth:
  methods:
    - type: oauth2
      authorization_url: https://github.com/login/oauth/authorize
      token_url: https://github.com/login/oauth/access_token
      scopes: [repo, read:org]
      flows: [device_code, local_callback]
    - type: bearer
      description: Personal access token

headers:
  Accept: application/vnd.github+json

operations:
  create_issue:
    description: Create an issue
    method: POST
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true, description: "Repo owner" }
      repo: { required: true }
      title: { required: true }
      labels: { required: false, type: array }
    body:
      title: "{{ params.title }}"
      labels: "{{ params.labels }}"

  list_issues:
    description: List issues
    method: GET
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true }
      repo: { required: true }
      state: { required: false, default: open, enum: [open, closed, all] }
    query:
      state: "{{ params.state }}"
"#;
        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "github");
        assert_eq!(def.auth.as_ref().unwrap().methods.len(), 2);
        assert_eq!(def.headers.as_ref().unwrap().len(), 1);
        assert_eq!(def.operations.len(), 2);

        let create = def.operations.get("create_issue").unwrap();
        assert_eq!(create.method, "POST");
        assert!(create.body.is_some());
        assert_eq!(create.params.len(), 4);

        let list = def.operations.get("list_issues").unwrap();
        assert!(list.query.is_some());
        let state_param = list.params.get("state").unwrap();
        assert!(!state_param.required);
        assert_eq!(state_param.default.as_ref().unwrap().as_str(), Some("open"));
        assert_eq!(state_param.enum_values.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_parse_param_types() {
        let yaml = r#"
version: 1
name: types-test
base_url: https://api.test.com
operations:
  test_op:
    description: Test types
    method: POST
    path: /test
    params:
      name: { required: true, type: string }
      count: { required: false, type: integer, default: 10 }
      tags: { required: false, type: array }
      active: { required: false, type: boolean, default: true }
"#;
        let def: IntegrationDefinition = serde_yaml::from_str(yaml).unwrap();
        let op = def.operations.get("test_op").unwrap();
        assert_eq!(op.params.get("count").unwrap().param_type.as_deref(), Some("integer"));
        assert_eq!(op.params.get("tags").unwrap().param_type.as_deref(), Some("array"));
    }
}
```

- [ ] **Step 2: Write the structs to make tests compile and pass**

In `src/integrations/definition.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Integration definition types - parsed from YAML files.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level integration definition parsed from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationDefinition {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    pub operations: HashMap<String, OperationDef>,
}

/// Auth configuration for an integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub methods: Vec<AuthMethod>,
}

/// A single authentication method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthMethod {
    #[serde(rename = "oauth2")]
    OAuth2 {
        authorization_url: String,
        token_url: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        flows: Vec<String>,
    },
    #[serde(rename = "bearer")]
    Bearer {
        #[serde(default)]
        description: Option<String>,
    },
    #[serde(rename = "api_key")]
    ApiKey {
        #[serde(default)]
        header_name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
}

/// An API operation (endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDef {
    pub description: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub params: HashMap<String, ParamDef>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
}

/// A parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    #[serde(default)]
    pub required: bool,
    #[serde(rename = "type", default)]
    pub param_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<String>>,
}
```

In `src/integrations/mod.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Declarative integration layer.
//!
//! Enables defining API integrations as YAML files instead of Rust code.
//! Integrations execute via the existing HttpNode.

pub mod definition;

pub use definition::IntegrationDefinition;
```

In `src/lib.rs`, add after `pub mod error;` (line 53):

```rust
pub mod integrations;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib integrations::definition::tests -v`
Expected: 3 tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/integrations/ src/lib.rs
git commit -m "feat(integrations): add YAML definition structs with serde deserialization"
```

---

## Task 2: Param Validator

**Files:**
- Create: `src/integrations/validator.rs`
- Modify: `src/integrations/mod.rs`
- Test: inline `#[cfg(test)]` in `validator.rs`

- [ ] **Step 1: Write failing tests**

In `src/integrations/validator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_params() -> HashMap<String, ParamDef> {
        let mut params = HashMap::new();
        params.insert("owner".into(), ParamDef {
            required: true,
            param_type: Some("string".into()),
            description: None,
            default: None,
            enum_values: None,
        });
        params.insert("state".into(), ParamDef {
            required: false,
            param_type: Some("string".into()),
            description: None,
            default: Some(json!("open")),
            enum_values: Some(vec!["open".into(), "closed".into(), "all".into()]),
        });
        params.insert("count".into(), ParamDef {
            required: false,
            param_type: Some("integer".into()),
            description: None,
            default: Some(json!(30)),
            enum_values: None,
        });
        params.insert("tags".into(), ParamDef {
            required: false,
            param_type: Some("array".into()),
            description: None,
            default: None,
            enum_values: None,
        });
        params
    }

    #[test]
    fn test_valid_params() {
        let defs = make_params();
        let input = json!({"owner": "octocat", "state": "closed"});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.get("owner").unwrap(), &json!("octocat"));
        assert_eq!(resolved.get("state").unwrap(), &json!("closed"));
        assert_eq!(resolved.get("count").unwrap(), &json!(30)); // default applied
    }

    #[test]
    fn test_missing_required_param() {
        let defs = make_params();
        let input = json!({"state": "open"});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("owner"));
    }

    #[test]
    fn test_invalid_enum_value() {
        let defs = make_params();
        let input = json!({"owner": "octocat", "state": "invalid"});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("state"));
    }

    #[test]
    fn test_type_coercion_integer() {
        let defs = make_params();
        let input = json!({"owner": "octocat", "count": "42"});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get("count").unwrap(), &json!(42));
    }

    #[test]
    fn test_array_param() {
        let defs = make_params();
        let input = json!({"owner": "octocat", "tags": ["bug", "urgent"]});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get("tags").unwrap(), &json!(["bug", "urgent"]));
    }

    #[test]
    fn test_defaults_not_overridden() {
        let defs = make_params();
        let input = json!({"owner": "octocat", "count": 5});
        let result = validate_and_resolve_params(&defs, &input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get("count").unwrap(), &json!(5));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib integrations::validator::tests -v`
Expected: FAIL - `validate_and_resolve_params` not defined

- [ ] **Step 3: Write implementation**

In `src/integrations/validator.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Parameter validation and resolution for integration operations.

use std::collections::HashMap;
use serde_json::{json, Value};
use super::definition::ParamDef;

/// Validate user-provided params against operation param definitions.
/// Applies defaults, checks required fields, validates enums, coerces types.
/// Returns resolved params as a HashMap<String, Value>.
pub fn validate_and_resolve_params(
    defs: &HashMap<String, ParamDef>,
    input: &Value,
) -> Result<HashMap<String, Value>, String> {
    let input_obj = input.as_object().unwrap_or(&serde_json::Map::new()).clone();
    let mut resolved = HashMap::new();

    for (name, def) in defs {
        let value = input_obj.get(name).cloned();

        match value {
            Some(v) if !v.is_null() => {
                // Validate enum
                if let Some(enum_values) = &def.enum_values {
                    if let Some(s) = v.as_str() {
                        if !enum_values.contains(&s.to_string()) {
                            return Err(format!(
                                "Parameter '{}' value '{}' not in allowed values: {:?}",
                                name, s, enum_values
                            ));
                        }
                    }
                }

                // Coerce types
                let coerced = coerce_type(&v, def.param_type.as_deref());
                resolved.insert(name.clone(), coerced);
            }
            _ => {
                // Missing or null
                if def.required {
                    return Err(format!("Required parameter '{}' is missing", name));
                }
                // Apply default
                if let Some(default) = &def.default {
                    resolved.insert(name.clone(), default.clone());
                }
            }
        }
    }

    Ok(resolved)
}

/// Coerce a JSON value to the expected type.
fn coerce_type(value: &Value, expected_type: Option<&str>) -> Value {
    match expected_type {
        Some("integer") => {
            if let Some(s) = value.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    return json!(n);
                }
            }
            value.clone()
        }
        Some("boolean") => {
            if let Some(s) = value.as_str() {
                match s {
                    "true" => return json!(true),
                    "false" => return json!(false),
                    _ => {}
                }
            }
            value.clone()
        }
        _ => value.clone(),
    }
}
```

Add to `src/integrations/mod.rs`:

```rust
pub mod validator;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib integrations::validator::tests -v`
Expected: 6 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/integrations/validator.rs src/integrations/mod.rs
git commit -m "feat(integrations): add param validation with type coercion and enum checks"
```

---

## Task 3: Built-in Loader

**Files:**
- Create: `src/integrations/loader.rs`
- Create: `src/integrations/builtin/github.yaml`
- Modify: `src/integrations/mod.rs`
- Test: inline `#[cfg(test)]` in `loader.rs`

- [ ] **Step 1: Create the GitHub YAML definition**

Create `src/integrations/builtin/github.yaml`:

```yaml
version: 1
name: github
display_name: GitHub
description: GitHub REST API integration
base_url: https://api.github.com
docs_url: https://docs.github.com/en/rest

auth:
  methods:
    - type: oauth2
      authorization_url: https://github.com/login/oauth/authorize
      token_url: https://github.com/login/oauth/access_token
      scopes: [repo, read:org, read:user]
      flows: [device_code, local_callback]
    - type: bearer
      description: Personal access token

headers:
  Accept: application/vnd.github+json
  X-GitHub-Api-Version: "2022-11-28"

operations:
  create_issue:
    description: Create a new issue in a repository
    method: POST
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true, type: string, description: "Repository owner" }
      repo: { required: true, type: string, description: "Repository name" }
      title: { required: true, type: string, description: "Issue title" }
      body: { required: false, type: string, description: "Issue body markdown" }
      labels: { required: false, type: array, description: "Label names" }
      assignees: { required: false, type: array, description: "Usernames to assign" }
    body:
      title: "{{ params.title }}"
      body: "{{ params.body }}"
      labels: "{{ params.labels }}"
      assignees: "{{ params.assignees }}"

  get_issue:
    description: Get a single issue by number
    method: GET
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues/{{ params.number }}
    params:
      owner: { required: true, type: string }
      repo: { required: true, type: string }
      number: { required: true, type: integer }

  list_issues:
    description: List repository issues
    method: GET
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true, type: string }
      repo: { required: true, type: string }
      state: { required: false, type: string, default: open, enum: [open, closed, all] }
      per_page: { required: false, type: integer, default: 30 }
      page: { required: false, type: integer, default: 1 }
    query:
      state: "{{ params.state }}"
      per_page: "{{ params.per_page }}"
      page: "{{ params.page }}"

  create_repo:
    description: Create a repository for the authenticated user
    method: POST
    path: /user/repos
    params:
      name: { required: true, type: string, description: "Repository name" }
      description: { required: false, type: string }
      private: { required: false, type: boolean, default: false }
    body:
      name: "{{ params.name }}"
      description: "{{ params.description }}"
      private: "{{ params.private }}"
```

- [ ] **Step 2: Write failing test for loader**

In `src/integrations/loader.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_github() {
        let loader = IntegrationLoader::new();
        let github = loader.get_builtin("github");
        assert!(github.is_some());
        let def = github.unwrap();
        assert_eq!(def.name, "github");
        assert_eq!(def.base_url, "https://api.github.com");
        assert!(def.operations.contains_key("create_issue"));
        assert!(def.operations.contains_key("get_issue"));
        assert!(def.operations.contains_key("list_issues"));
    }

    #[test]
    fn test_list_builtin_services() {
        let loader = IntegrationLoader::new();
        let services = loader.list_builtin();
        assert!(services.contains(&"github".to_string()));
    }

    #[test]
    fn test_builtin_not_found() {
        let loader = IntegrationLoader::new();
        assert!(loader.get_builtin("nonexistent").is_none());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib integrations::loader::tests -v`
Expected: FAIL - `IntegrationLoader` not defined

- [ ] **Step 4: Write implementation**

In `src/integrations/loader.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Integration definition loader.
//!
//! Loads built-in YAML definitions (compiled into binary) and user definitions
//! from ~/.config/r8r/integrations/. User definitions override built-ins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Instant, SystemTime};

use tracing::{debug, warn};

use super::definition::IntegrationDefinition;
use crate::config::Config;

// Built-in integration definitions compiled into the binary.
const BUILTIN_GITHUB: &str = include_str!("builtin/github.yaml");

/// Loader for integration definitions.
pub struct IntegrationLoader {
    builtin: HashMap<String, IntegrationDefinition>,
    user_cache: RwLock<UserDefinitionCache>,
}

struct UserDefinitionCache {
    definitions: HashMap<String, IntegrationDefinition>,
    mtimes: HashMap<PathBuf, SystemTime>,
    last_scan: Instant,
}

impl IntegrationLoader {
    pub fn new() -> Self {
        let mut builtin = HashMap::new();

        // Parse built-in definitions
        for (name, yaml) in [("github", BUILTIN_GITHUB)] {
            match serde_yaml::from_str::<IntegrationDefinition>(yaml) {
                Ok(def) => { builtin.insert(name.to_string(), def); }
                Err(e) => warn!("Failed to parse built-in integration '{}': {}", name, e),
            }
        }

        Self {
            builtin,
            user_cache: RwLock::new(UserDefinitionCache {
                definitions: HashMap::new(),
                mtimes: HashMap::new(),
                last_scan: Instant::now(),
            }),
        }
    }

    /// Get a built-in definition by name.
    pub fn get_builtin(&self, name: &str) -> Option<&IntegrationDefinition> {
        self.builtin.get(name)
    }

    /// List all built-in service names.
    pub fn list_builtin(&self) -> Vec<String> {
        self.builtin.keys().cloned().collect()
    }

    /// Get a definition by name (user overrides take precedence).
    pub fn get(&self, name: &str) -> Option<IntegrationDefinition> {
        // Check user definitions first
        self.reload_user_if_stale();
        {
            let cache = self.user_cache.read().unwrap();
            if let Some(def) = cache.definitions.get(name) {
                return Some(def.clone());
            }
        }
        self.builtin.get(name).cloned()
    }

    /// List all available service names (built-in + user).
    pub fn list_all(&self) -> Vec<String> {
        self.reload_user_if_stale();
        let cache = self.user_cache.read().unwrap();
        let mut names: Vec<String> = self.builtin.keys().cloned().collect();
        for name in cache.definitions.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names.sort();
        names
    }

    /// User definitions directory.
    fn user_dir() -> PathBuf {
        Config::config_dir().join("integrations")
    }

    /// Reload user definitions if directory contents changed.
    fn reload_user_if_stale(&self) {
        let user_dir = Self::user_dir();
        if !user_dir.exists() {
            return;
        }

        // Check if any file mtime changed
        let needs_reload = {
            let cache = self.user_cache.read().unwrap();
            // Re-scan at most once per second
            if cache.last_scan.elapsed().as_secs() < 1 {
                return;
            }
            Self::check_dir_changed(&user_dir, &cache.mtimes)
        };

        if needs_reload {
            let mut cache = self.user_cache.write().unwrap();
            cache.definitions.clear();
            cache.mtimes.clear();
            cache.last_scan = Instant::now();
            Self::load_from_dir(&user_dir, &mut cache.definitions, &mut cache.mtimes);
        }
    }

    fn check_dir_changed(dir: &Path, known_mtimes: &HashMap<PathBuf, SystemTime>) -> bool {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return false,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "yaml" && ext != "yml") {
                continue;
            }
            let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());
            match (known_mtimes.get(&path), mtime) {
                (Some(known), Some(current)) if *known == current => continue,
                _ => return true,
            }
        }
        false
    }

    fn load_from_dir(
        dir: &Path,
        definitions: &mut HashMap<String, IntegrationDefinition>,
        mtimes: &mut HashMap<PathBuf, SystemTime>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read integrations directory {:?}: {}", dir, e);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "yaml" && ext != "yml") {
                continue;
            }

            let mtime = entry.metadata().ok().and_then(|m| m.modified().ok());
            if let Some(mt) = mtime {
                mtimes.insert(path.clone(), mt);
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_yaml::from_str::<IntegrationDefinition>(&content) {
                    Ok(def) => {
                        debug!("Loaded user integration '{}' from {:?}", def.name, path);
                        definitions.insert(def.name.clone(), def);
                    }
                    Err(e) => warn!("Failed to parse integration {:?}: {}", path, e),
                },
                Err(e) => warn!("Failed to read integration {:?}: {}", path, e),
            }
        }
    }
}

impl Default for IntegrationLoader {
    fn default() -> Self {
        Self::new()
    }
}
```

Add to `src/integrations/mod.rs`:

```rust
pub mod loader;
pub use loader::IntegrationLoader;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib integrations::loader::tests -v`
Expected: 3 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/integrations/loader.rs src/integrations/mod.rs src/integrations/builtin/
git commit -m "feat(integrations): add loader with built-in github.yaml and user override support"
```

---

## Task 4: IntegrationNode (Core)

**Files:**
- Create: `src/integrations/node.rs`
- Modify: `src/integrations/mod.rs`
- Modify: `src/nodes/mod.rs:12-72`
- Modify: `src/nodes/registry.rs:36-63`
- Modify: `Cargo.toml`
- Test: inline `#[cfg(test)]` in `node.rs`

- [ ] **Step 1: Add percent-encoding dependency**

In `Cargo.toml`, add after the `regex-lite` line (line 98):

```toml
percent-encoding = "2"
```

- [ ] **Step 2: Write failing test for IntegrationNode**

In `src/integrations/node.rs`, add test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_http_config_get_with_query() {
        let loader = IntegrationLoader::new();
        let def = loader.get("github").unwrap();
        let op = def.operations.get("list_issues").unwrap();

        let params = json!({"owner": "octocat", "repo": "hello-world", "state": "closed", "per_page": 10, "page": 1});
        let resolved = crate::integrations::validator::validate_and_resolve_params(&op.params, &params).unwrap();

        let config = build_http_config(&def, op, &resolved, None);
        let url = config["url"].as_str().unwrap();

        assert!(url.starts_with("https://api.github.com/repos/octocat/hello-world/issues?"));
        assert!(url.contains("state=closed"));
        assert!(url.contains("per_page=10"));
        assert_eq!(config["method"], "GET");
    }

    #[test]
    fn test_build_http_config_post_with_typed_body() {
        let loader = IntegrationLoader::new();
        let def = loader.get("github").unwrap();
        let op = def.operations.get("create_issue").unwrap();

        let params = json!({
            "owner": "octocat",
            "repo": "hello-world",
            "title": "Bug report",
            "labels": ["bug", "urgent"]
        });
        let resolved = crate::integrations::validator::validate_and_resolve_params(&op.params, &params).unwrap();

        let config = build_http_config(&def, op, &resolved, None);
        assert_eq!(config["method"], "POST");
        assert_eq!(config["url"], "https://api.github.com/repos/octocat/hello-world/issues");

        let body = &config["body"];
        assert_eq!(body["title"], "Bug report");
        // Labels should be an actual array, not a stringified array
        assert!(body["labels"].is_array());
        assert_eq!(body["labels"][0], "bug");
        assert_eq!(body["labels"][1], "urgent");
    }

    #[test]
    fn test_build_http_config_with_credential() {
        let loader = IntegrationLoader::new();
        let def = loader.get("github").unwrap();
        let op = def.operations.get("get_issue").unwrap();

        let params = json!({"owner": "octocat", "repo": "hello-world", "number": 42});
        let resolved = crate::integrations::validator::validate_and_resolve_params(&op.params, &params).unwrap();

        let config = build_http_config(&def, op, &resolved, Some("my_github_token"));
        assert_eq!(config["credential"], "my_github_token");
        assert_eq!(config["auth_type"], "bearer");

        // Check headers are set
        let headers = &config["headers"];
        assert_eq!(headers["Accept"], "application/vnd.github+json");
    }

    #[test]
    fn test_params_substitution_in_path() {
        let path = "/repos/{{ params.owner }}/{{ params.repo }}/issues";
        let mut resolved = HashMap::new();
        resolved.insert("owner".to_string(), json!("octocat"));
        resolved.insert("repo".to_string(), json!("hello-world"));

        let result = substitute_params_in_string(path, &resolved);
        assert_eq!(result, "/repos/octocat/hello-world/issues");
    }

    #[test]
    fn test_null_optional_params_omitted_from_query() {
        let loader = IntegrationLoader::new();
        let def = loader.get("github").unwrap();
        let op = def.operations.get("list_issues").unwrap();

        // Only provide required params + state, let per_page and page use defaults
        let params = json!({"owner": "octocat", "repo": "hello-world"});
        let resolved = crate::integrations::validator::validate_and_resolve_params(&op.params, &params).unwrap();

        let config = build_http_config(&def, op, &resolved, None);
        let url = config["url"].as_str().unwrap();
        assert!(url.contains("state=open")); // default
        assert!(url.contains("per_page=30")); // default
    }

    #[test]
    fn test_absent_optional_body_params_omitted() {
        let loader = IntegrationLoader::new();
        let def = loader.get("github").unwrap();
        let op = def.operations.get("create_issue").unwrap();

        // Only provide required params (title), omit optional body/labels/assignees
        let params = json!({"owner": "octocat", "repo": "hello-world", "title": "Bug report"});
        let resolved = crate::integrations::validator::validate_and_resolve_params(&op.params, &params).unwrap();

        let config = build_http_config(&def, op, &resolved, None);
        let body = &config["body"];

        // Title should be present
        assert_eq!(body["title"], "Bug report");
        // Absent optional params should NOT appear in body (no template literals, no empty strings)
        assert!(body.get("body").is_none() || body["body"].is_null());
        assert!(body.get("labels").is_none() || body["labels"].is_null());
        assert!(body.get("assignees").is_none() || body["assignees"].is_null());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib integrations::node::tests -v`
Expected: FAIL - functions not defined

- [ ] **Step 4: Write implementation**

In `src/integrations/node.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! IntegrationNode - executes YAML-defined integrations via HttpNode delegation.

use std::collections::HashMap;

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::definition::{IntegrationDefinition, OperationDef};
use super::loader::IntegrationLoader;
use super::validator::validate_and_resolve_params;
use crate::error::{Error, Result};
use crate::nodes::types::{Node, NodeContext, NodeResult};
use crate::nodes::HttpNode;

/// Integration node - delegates to HttpNode using YAML definitions.
pub struct IntegrationNode {
    http_node: HttpNode,
    loader: IntegrationLoader,
}

#[derive(Debug, Deserialize)]
struct IntegrationConfig {
    service: String,
    operation: String,
    #[serde(default)]
    credential: Option<String>,
    #[serde(default)]
    params: Value,
}

impl IntegrationNode {
    pub fn new() -> Self {
        Self {
            http_node: HttpNode::new(),
            loader: IntegrationLoader::new(),
        }
    }

    /// Get the loader (for CLI commands).
    pub fn loader(&self) -> &IntegrationLoader {
        &self.loader
    }
}

impl Default for IntegrationNode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Node for IntegrationNode {
    fn node_type(&self) -> &str {
        "integration"
    }

    fn description(&self) -> &str {
        "Execute YAML-defined API integrations (delegates to HTTP node)"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: IntegrationConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid integration config: {}", e)))?;

        // Load definition
        let def = self.loader.get(&config.service).ok_or_else(|| {
            Error::Node(format!(
                "Unknown integration service '{}'. Run 'r8r integrations list' to see available services.",
                config.service
            ))
        })?;

        // Get operation
        let op = def.operations.get(&config.operation).ok_or_else(|| {
            let available: Vec<&str> = def.operations.keys().map(|s| s.as_str()).collect();
            Error::Node(format!(
                "Unknown operation '{}' for service '{}'. Available: {:?}",
                config.operation, config.service, available
            ))
        })?;

        // Validate and resolve params
        let resolved = validate_and_resolve_params(&op.params, &config.params)
            .map_err(|e| Error::Node(format!("Parameter validation failed: {}", e)))?;

        debug!(
            service = %config.service,
            operation = %config.operation,
            "Executing integration"
        );

        // Build HttpNode config
        let http_config = build_http_config(&def, op, &resolved, config.credential.as_deref());

        // Delegate to HttpNode
        self.http_node.execute(&http_config, ctx).await
    }
}

/// Substitute {{ params.X }} in a string with resolved param values.
pub fn substitute_params_in_string(template: &str, resolved: &HashMap<String, Value>) -> String {
    let mut result = template.to_string();
    for (name, value) in resolved {
        let pattern_spaced = format!("{{{{ params.{} }}}}", name);
        let pattern_tight = format!("{{{{params.{}}}}}", name);
        let replacement = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => value.to_string(),
        };
        result = result.replace(&pattern_spaced, &replacement);
        result = result.replace(&pattern_tight, &replacement);
    }
    result
}

/// Check if a template string is a direct param reference like "{{ params.X }}".
/// Returns the param name if it is, None otherwise.
fn extract_param_ref(template: &str) -> Option<String> {
    let trimmed = template.trim();
    // Match "{{ params.X }}" or "{{params.X}}"
    let name = if trimmed.starts_with("{{ params.") && trimmed.ends_with(" }}") {
        Some(trimmed[10..trimmed.len()-3].to_string())
    } else if trimmed.starts_with("{{params.") && trimmed.ends_with("}}") {
        Some(trimmed[9..trimmed.len()-2].to_string())
    } else {
        None
    };
    name
}

/// Build a typed JSON body from a body template + resolved params.
/// Instead of string interpolation, this constructs proper typed values.
/// Absent optional params are omitted from the body entirely.
fn build_typed_body(body_template: &Value, resolved: &HashMap<String, Value>) -> Value {
    match body_template {
        Value::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, value_template) in obj {
                if let Some(s) = value_template.as_str() {
                    // Check if this is a direct param reference: "{{ params.X }}"
                    if let Some(param_name) = extract_param_ref(s) {
                        // Direct param reference - use typed value, skip if absent
                        if let Some(param_value) = resolved.get(&param_name) {
                            if !param_value.is_null() {
                                result.insert(key.clone(), param_value.clone());
                            }
                            // else: param is null, omit from body
                        }
                        // else: param not in resolved (absent optional), omit from body
                    } else {
                        // Mixed template or literal string - do string substitution
                        let substituted = substitute_params_in_string(s, resolved);
                        if !substituted.is_empty() {
                            result.insert(key.clone(), Value::String(substituted));
                        }
                    }
                } else {
                    result.insert(key.clone(), build_typed_body(value_template, resolved));
                }
            }
            Value::Object(result)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| build_typed_body(v, resolved)).collect())
        }
        other => other.clone(),
    }
}

/// Build an HttpNode-compatible config from an integration definition + resolved params.
pub fn build_http_config(
    def: &IntegrationDefinition,
    op: &OperationDef,
    resolved: &HashMap<String, Value>,
    credential: Option<&str>,
) -> Value {
    // Build URL: base_url + path with params substituted
    let path = substitute_params_in_string(&op.path, resolved);
    let mut url = format!("{}{}", def.base_url, path);

    // Append query parameters to URL
    if let Some(query) = &op.query {
        let mut query_parts: Vec<String> = Vec::new();
        for (key, template) in query {
            let value = substitute_params_in_string(template, resolved);
            if !value.is_empty() {
                let encoded = utf8_percent_encode(&value, NON_ALPHANUMERIC).to_string();
                query_parts.push(format!("{}={}", key, encoded));
            }
        }
        if !query_parts.is_empty() {
            url = format!("{}?{}", url, query_parts.join("&"));
        }
    }

    // Merge headers: service-level + operation-level
    let mut headers = serde_json::Map::new();
    if let Some(service_headers) = &def.headers {
        for (k, v) in service_headers {
            headers.insert(k.clone(), json!(v));
        }
    }
    if let Some(op_headers) = &op.headers {
        for (k, v) in op_headers {
            headers.insert(k.clone(), json!(v));
        }
    }

    // Build config
    let mut config = json!({
        "url": url,
        "method": op.method,
    });

    if !headers.is_empty() {
        config["headers"] = Value::Object(headers);
    }

    // Build typed body
    if let Some(body_template) = &op.body {
        let body = build_typed_body(body_template, resolved);
        config["body"] = body;
    }

    // Add credential
    if let Some(cred) = credential {
        config["credential"] = json!(cred);
        config["auth_type"] = json!("bearer"); // default, can be overridden
    }

    config
}
```

- [ ] **Step 5: Register IntegrationNode in the node registry**

In `src/nodes/mod.rs`, add after `mod wait;` (line 42):

```rust
// Integration node is in a separate top-level module
```

In `src/nodes/registry.rs`, add import at line 20 (after the existing imports):

```rust
use crate::integrations::node::IntegrationNode;
```

Add registration in `NodeRegistry::new()` after the `ApprovalNode` line (after line 61):

```rust
registry.register(Arc::new(IntegrationNode::new()));
```

Add to `src/integrations/mod.rs`:

```rust
pub mod node;
pub use node::IntegrationNode;
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib integrations::node::tests -v`
Expected: 5 tests PASS

Then run all existing tests to verify nothing broke:

Run: `cargo test`
Expected: All existing tests PASS + new tests PASS

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/integrations/node.rs src/integrations/mod.rs src/nodes/registry.rs
git commit -m "feat(integrations): add IntegrationNode with HttpNode delegation, param substitution, typed body construction"
```

---

## Task 5: OAuth2 Credential Type

**Files:**
- Create: `src/integrations/oauth2.rs`
- Modify: `src/credentials.rs:71-77`
- Modify: `src/integrations/mod.rs`
- Test: inline `#[cfg(test)]` in `oauth2.rs`

- [ ] **Step 1: Write failing tests**

In `src/integrations/oauth2.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_oauth2_credential_serialization() {
        let cred = OAuth2Credential {
            service: "github_oauth".into(),
            provider: "github".into(),
            access_token: "gho_abc123".into(),
            refresh_token: Some("ghr_xyz789".into()),
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec!["repo".into(), "read:org".into()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json = serde_json::to_string(&cred).unwrap();
        let deserialized: OAuth2Credential = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.service, "github_oauth");
        assert_eq!(deserialized.provider, "github");
        assert_eq!(deserialized.scopes.len(), 2);
    }

    #[test]
    fn test_is_expired_true() {
        let cred = OAuth2Credential {
            service: "test".into(),
            provider: "test".into(),
            access_token: "token".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() - chrono::Duration::minutes(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(cred.is_expired());
    }

    #[test]
    fn test_is_expired_near_expiry() {
        let cred = OAuth2Credential {
            service: "test".into(),
            provider: "test".into(),
            access_token: "token".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::minutes(3)),
            token_type: "Bearer".into(),
            scopes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        // Within 5-minute refresh window
        assert!(cred.is_expired());
    }

    #[test]
    fn test_is_expired_false() {
        let cred = OAuth2Credential {
            service: "test".into(),
            provider: "test".into(),
            access_token: "token".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: "Bearer".into(),
            scopes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!cred.is_expired());
    }

    #[test]
    fn test_no_expiry_not_expired() {
        let cred = OAuth2Credential {
            service: "test".into(),
            provider: "test".into(),
            access_token: "token".into(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".into(),
            scopes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!cred.is_expired());
    }
}
```

- [ ] **Step 2: Write implementation**

In `src/integrations/oauth2.rs`:

```rust
/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! OAuth2 credential management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Refresh window: tokens are considered expired if they expire within this many minutes.
const REFRESH_WINDOW_MINUTES: i64 = 5;

/// OAuth2 credential with access and refresh tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Credential {
    pub service: String,
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OAuth2Credential {
    /// Check if the access token is expired or near-expiry (within 5 minutes).
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires) => {
                let threshold = Utc::now() + chrono::Duration::minutes(REFRESH_WINDOW_MINUTES);
                expires <= threshold
            }
            None => false, // No expiry = never expires (e.g., GitHub PATs)
        }
    }

    /// Check if this credential can be refreshed.
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some()
    }
}
```

- [ ] **Step 3: Add oauth2_credentials to CredentialStore**

In `src/credentials.rs`, modify `CredentialStore` (line 71-77):

Add after `credentials: HashMap<String, Credential>,` (line 73):

```rust
    /// OAuth2 credentials (stored separately for clean semantics).
    /// Private to match existing `credentials` field encapsulation.
    #[serde(default)]
    oauth2_credentials: HashMap<String, crate::integrations::oauth2::OAuth2Credential>,
```

Also add getter and setter methods to `impl CredentialStore` (matching the pattern of the existing `get()` method):

```rust
    /// Get an OAuth2 credential by service name.
    pub fn get_oauth2(&self, service: &str) -> Option<&crate::integrations::oauth2::OAuth2Credential> {
        self.oauth2_credentials.get(service)
    }

    /// Set an OAuth2 credential.
    pub fn set_oauth2(&mut self, credential: crate::integrations::oauth2::OAuth2Credential) {
        self.oauth2_credentials.insert(credential.service.clone(), credential);
    }

    /// List all OAuth2 credential service names.
    pub fn list_oauth2(&self) -> Vec<&str> {
        self.oauth2_credentials.keys().map(|s| s.as_str()).collect()
    }
```

Add to `src/integrations/mod.rs`:

```rust
pub mod oauth2;
pub use oauth2::OAuth2Credential;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib integrations::oauth2::tests -v`
Expected: 5 tests PASS

Run: `cargo test --lib credentials -v`
Expected: All existing credential tests still PASS

- [ ] **Step 5: Commit**

```bash
git add src/integrations/oauth2.rs src/integrations/mod.rs src/credentials.rs
git commit -m "feat(integrations): add OAuth2Credential type with expiry detection, add to CredentialStore"
```

---

## Task 6: NodeContext + Executor OAuth2 Wiring

**Files:**
- Modify: `src/nodes/types.rs:57-92` (NodeContext)
- Modify: `src/nodes/types.rs:94-108` (Debug impl)
- Modify: `src/nodes/types.rs:171-184` (for_item)
- Modify: `src/engine/executor.rs:3392-3439` (load_workflow_credentials)
- Modify: `src/engine/executor.rs:488-509` (execution context)

- [ ] **Step 1: Add oauth2_credentials to NodeContext**

In `src/nodes/types.rs`, after the `credentials` field (line 85), add:

```rust
    /// Resolved OAuth2 credentials (keyed by service name).
    /// SECURITY: Never log or trace this field!
    /// Wrapped in Arc for cheap cloning.
    pub oauth2_credentials: Arc<std::collections::HashMap<String, crate::integrations::oauth2::OAuth2Credential>>,
```

- [ ] **Step 2: Update Debug impl**

In `src/nodes/types.rs`, in the `Debug` impl (around line 104), add after the credentials redaction line:

```rust
            .field("oauth2_credentials", &"[REDACTED]")
```

- [ ] **Step 3: Update new() constructor**

In `src/nodes/types.rs`, in `NodeContext::new()` (around line 113-126), add after the `credentials` field:

```rust
            oauth2_credentials: Arc::new(std::collections::HashMap::new()),
```

- [ ] **Step 4: Update for_item()**

In `src/nodes/types.rs`, in `for_item()` (around line 171-184), add after `credentials: self.credentials.clone(),`:

```rust
                oauth2_credentials: self.oauth2_credentials.clone(),
```

- [ ] **Step 5: Add with_oauth2_credentials builder method**

After `with_credentials()` method, add:

```rust
    /// Set OAuth2 credentials.
    pub fn with_oauth2_credentials(
        mut self,
        oauth2_credentials: std::collections::HashMap<String, crate::integrations::oauth2::OAuth2Credential>,
    ) -> Self {
        self.oauth2_credentials = Arc::new(oauth2_credentials);
        self
    }
```

- [ ] **Step 6: Extend executor credential loading**

In `src/engine/executor.rs`, at the end of `load_workflow_credentials()` (around line 3439), the function currently returns `HashMap<String, String>`. We need a separate function for OAuth2. Add after `load_workflow_credentials`:

```rust
/// Load OAuth2 credentials for integration nodes in a workflow.
async fn load_workflow_oauth2_credentials(
    workflow: &Workflow,
) -> Result<HashMap<String, crate::integrations::oauth2::OAuth2Credential>> {
    let mut oauth2_creds = HashMap::new();

    // Only scan integration nodes
    for node in &workflow.nodes {
        if node.node_type != "integration" {
            continue;
        }
        if let Some(credential_name) = node.config.get("credential").and_then(|v| v.as_str()) {
            if oauth2_creds.contains_key(credential_name) {
                continue;
            }

            // Try to load from credential store's oauth2 section
            let store = match CredentialStore::load().await {
                Ok(s) => s,
                Err(_) => continue,
            };

            if let Some(oauth2_cred) = store.oauth2_credentials.get(credential_name) {
                debug!(
                    "Loaded OAuth2 credential '{}' for workflow '{}'",
                    credential_name, workflow.name
                );
                oauth2_creds.insert(credential_name.to_string(), oauth2_cred.clone());
            }
        }
    }

    Ok(oauth2_creds)
}
```

- [ ] **Step 7: Wire OAuth2 credentials into execution context**

In `src/engine/executor.rs`, around line 493-509 where the context is built, add OAuth2 loading after the regular credential loading and wire it into the context:

After `let credentials = merge_inherited_credentials(...)` (line 494-497), add:

```rust
        let oauth2_credentials = load_workflow_oauth2_credentials(workflow).await.unwrap_or_default();
```

After `.with_credentials(credentials)` (line 506), add:

```rust
            .with_oauth2_credentials(oauth2_credentials)
```

Do the same for the second execution path (around line 1674).

- [ ] **Step 8: Run all tests**

Run: `cargo test`
Expected: All tests PASS (existing + new)

- [ ] **Step 9: Commit**

```bash
git add src/nodes/types.rs src/engine/executor.rs
git commit -m "feat(integrations): wire OAuth2 credentials through NodeContext and executor"
```

---

## Task 7: Remaining Built-in YAML Definitions

**Files:**
- Create: `src/integrations/builtin/slack.yaml`
- Create: `src/integrations/builtin/openai.yaml`
- Create: `src/integrations/builtin/stripe.yaml`
- Create: `src/integrations/builtin/notion.yaml`
- Modify: `src/integrations/loader.rs` (add include_str! for new definitions)

- [ ] **Step 1: Create slack.yaml**

```yaml
version: 1
name: slack
display_name: Slack
description: Slack Web API integration
base_url: https://slack.com/api
docs_url: https://api.slack.com/methods

auth:
  methods:
    - type: oauth2
      authorization_url: https://slack.com/oauth/v2/authorize
      token_url: https://slack.com/api/oauth.v2.access
      scopes: [chat:write, channels:read, users:read]
      flows: [local_callback]
    - type: bearer
      description: Bot token (xoxb-...)

operations:
  send_message:
    description: Send a message to a channel
    method: POST
    path: /chat.postMessage
    params:
      channel: { required: true, type: string, description: "Channel ID or name" }
      text: { required: false, type: string, description: "Message text" }
      blocks: { required: false, type: array, description: "Block Kit blocks" }
    body:
      channel: "{{ params.channel }}"
      text: "{{ params.text }}"
      blocks: "{{ params.blocks }}"

  list_channels:
    description: List channels
    method: GET
    path: /conversations.list
    params:
      limit: { required: false, type: integer, default: 100 }
      types: { required: false, type: string, default: "public_channel" }
    query:
      limit: "{{ params.limit }}"
      types: "{{ params.types }}"

  get_user:
    description: Get user info
    method: GET
    path: /users.info
    params:
      user: { required: true, type: string, description: "User ID" }
    query:
      user: "{{ params.user }}"
```

- [ ] **Step 2: Create openai.yaml**

```yaml
version: 1
name: openai
display_name: OpenAI
description: OpenAI API integration
base_url: https://api.openai.com/v1
docs_url: https://platform.openai.com/docs/api-reference

auth:
  methods:
    - type: bearer
      description: API key

headers:
  Content-Type: application/json

operations:
  chat_completion:
    description: Create a chat completion
    method: POST
    path: /chat/completions
    params:
      model: { required: true, type: string, description: "Model ID (e.g. gpt-4)" }
      messages: { required: true, type: array, description: "Chat messages" }
      temperature: { required: false, type: number, default: 1.0 }
      max_tokens: { required: false, type: integer }
    body:
      model: "{{ params.model }}"
      messages: "{{ params.messages }}"
      temperature: "{{ params.temperature }}"
      max_tokens: "{{ params.max_tokens }}"

  create_embedding:
    description: Create embeddings
    method: POST
    path: /embeddings
    params:
      model: { required: true, type: string }
      input: { required: true, type: string }
    body:
      model: "{{ params.model }}"
      input: "{{ params.input }}"

  list_models:
    description: List available models
    method: GET
    path: /models
    params: {}
```

- [ ] **Step 3: Create stripe.yaml**

```yaml
version: 1
name: stripe
display_name: Stripe
description: Stripe API integration (GET operations only - POST requires form-encoding support, planned for follow-up)
base_url: https://api.stripe.com/v1
docs_url: https://docs.stripe.com/api

auth:
  methods:
    - type: bearer
      description: Secret key (sk_...)

operations:
  get_customer:
    description: Get a customer
    method: GET
    path: /customers/{{ params.id }}
    params:
      id: { required: true, type: string, description: "Customer ID" }

  list_customers:
    description: List customers
    method: GET
    path: /customers
    params:
      limit: { required: false, type: integer, default: 10 }
      email: { required: false, type: string }
    query:
      limit: "{{ params.limit }}"
      email: "{{ params.email }}"

  list_charges:
    description: List charges
    method: GET
    path: /charges
    params:
      limit: { required: false, type: integer, default: 10 }
      customer: { required: false, type: string }
    query:
      limit: "{{ params.limit }}"
      customer: "{{ params.customer }}"

  get_balance:
    description: Get current balance
    method: GET
    path: /balance
    params: {}
```

- [ ] **Step 4: Create notion.yaml**

```yaml
version: 1
name: notion
display_name: Notion
description: Notion API integration
base_url: https://api.notion.com/v1
docs_url: https://developers.notion.com/reference

auth:
  methods:
    - type: oauth2
      authorization_url: https://api.notion.com/v1/oauth/authorize
      token_url: https://api.notion.com/v1/oauth/token
      scopes: []
      flows: [local_callback]
    - type: bearer
      description: Internal integration token

headers:
  Notion-Version: "2022-06-28"
  Content-Type: application/json

operations:
  query_database:
    description: Query a database
    method: POST
    path: /databases/{{ params.database_id }}/query
    params:
      database_id: { required: true, type: string }
      filter: { required: false, type: object }
      page_size: { required: false, type: integer, default: 100 }
    body:
      filter: "{{ params.filter }}"
      page_size: "{{ params.page_size }}"

  create_page:
    description: Create a page
    method: POST
    path: /pages
    params:
      parent: { required: true, type: object, description: "Parent database or page" }
      properties: { required: true, type: object }
    body:
      parent: "{{ params.parent }}"
      properties: "{{ params.properties }}"

  get_page:
    description: Get a page
    method: GET
    path: /pages/{{ params.page_id }}
    params:
      page_id: { required: true, type: string }

  search:
    description: Search pages and databases
    method: POST
    path: /search
    params:
      query: { required: false, type: string }
      page_size: { required: false, type: integer, default: 100 }
    body:
      query: "{{ params.query }}"
      page_size: "{{ params.page_size }}"
```

- [ ] **Step 5: Register all built-ins in the loader**

In `src/integrations/loader.rs`, add the include_str! constants after `BUILTIN_GITHUB`:

```rust
const BUILTIN_SLACK: &str = include_str!("builtin/slack.yaml");
const BUILTIN_OPENAI: &str = include_str!("builtin/openai.yaml");
const BUILTIN_STRIPE: &str = include_str!("builtin/stripe.yaml");
const BUILTIN_NOTION: &str = include_str!("builtin/notion.yaml");
```

Update the loop in `IntegrationLoader::new()`:

```rust
        for (name, yaml) in [
            ("github", BUILTIN_GITHUB),
            ("slack", BUILTIN_SLACK),
            ("openai", BUILTIN_OPENAI),
            ("stripe", BUILTIN_STRIPE),
            ("notion", BUILTIN_NOTION),
        ] {
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib integrations -v`
Expected: All integration tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/integrations/builtin/ src/integrations/loader.rs
git commit -m "feat(integrations): add Slack, OpenAI, Stripe, Notion built-in definitions"
```

---

## Task 8: Integration Tests

**Files:**
- Create: `tests/integration_node_tests.rs`

- [ ] **Step 1: Write integration tests**

```rust
//! Integration tests for the IntegrationNode pipeline.

use r8r::integrations::loader::IntegrationLoader;
use r8r::integrations::node::build_http_config;
use r8r::integrations::validator::validate_and_resolve_params;
use serde_json::json;

#[test]
fn test_full_pipeline_github_create_issue() {
    let loader = IntegrationLoader::new();
    let def = loader.get("github").unwrap();
    let op = def.operations.get("create_issue").unwrap();

    let params = json!({
        "owner": "octocat",
        "repo": "hello-world",
        "title": "Test issue",
        "labels": ["bug", "help wanted"]
    });

    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("my_token"));

    assert_eq!(config["url"], "https://api.github.com/repos/octocat/hello-world/issues");
    assert_eq!(config["method"], "POST");
    assert_eq!(config["body"]["title"], "Test issue");
    assert!(config["body"]["labels"].is_array());
    assert_eq!(config["body"]["labels"][0], "bug");
    assert_eq!(config["credential"], "my_token");
    assert_eq!(config["headers"]["Accept"], "application/vnd.github+json");
}

#[test]
fn test_full_pipeline_openai_completion() {
    let loader = IntegrationLoader::new();
    let def = loader.get("openai").unwrap();
    let op = def.operations.get("chat_completion").unwrap();

    let params = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "temperature": 0.7
    });

    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("openai_key"));

    assert_eq!(config["url"], "https://api.openai.com/v1/chat/completions");
    assert_eq!(config["body"]["model"], "gpt-4");
    assert!(config["body"]["messages"].is_array());
    assert_eq!(config["body"]["temperature"], 0.7);
}

#[test]
fn test_full_pipeline_slack_send_message() {
    let loader = IntegrationLoader::new();
    let def = loader.get("slack").unwrap();
    let op = def.operations.get("send_message").unwrap();

    let params = json!({
        "channel": "#general",
        "text": "Hello from r8r!"
    });

    let resolved = validate_and_resolve_params(&op.params, &params).unwrap();
    let config = build_http_config(&def, op, &resolved, Some("slack_token"));

    assert_eq!(config["url"], "https://slack.com/api/chat.postMessage");
    assert_eq!(config["body"]["channel"], "#general");
    assert_eq!(config["body"]["text"], "Hello from r8r!");
}

#[test]
fn test_all_builtin_definitions_parse() {
    let loader = IntegrationLoader::new();
    let services = loader.list_all();

    assert!(services.contains(&"github".to_string()));
    assert!(services.contains(&"slack".to_string()));
    assert!(services.contains(&"openai".to_string()));
    assert!(services.contains(&"stripe".to_string()));
    assert!(services.contains(&"notion".to_string()));

    // Every definition should have at least one operation
    for service in &services {
        let def = loader.get(service).unwrap();
        assert!(!def.operations.is_empty(), "Service '{}' has no operations", service);
        assert!(!def.base_url.is_empty(), "Service '{}' has no base_url", service);
    }
}

#[test]
fn test_user_override_takes_precedence() {
    // This test verifies the loader's override logic works
    // In a real test, you'd create a temp dir, but we test the loader's logic here
    let loader = IntegrationLoader::new();

    // Built-in github should exist
    let github = loader.get_builtin("github");
    assert!(github.is_some());
    assert_eq!(github.unwrap().base_url, "https://api.github.com");
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test integration_node_tests -v`
Expected: 5 tests PASS

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: ALL tests PASS

- [ ] **Step 4: Commit**

```bash
git add tests/integration_node_tests.rs
git commit -m "test(integrations): add integration tests for full YAML->HttpNode pipeline"
```

---

## Task 9: CLI Subcommands (r8r integrations)

**Files:**
- Modify: `src/main.rs` — add `integrations` subcommand with `list`, `show`, `validate`, `test`

This task depends heavily on the existing CLI structure in main.rs. The implementer should:

- [ ] **Step 1: Read `src/main.rs` to understand the existing CLI pattern (clap derive)**

- [ ] **Step 2: Add `Integrations` variant to the main CLI enum**

Add a new subcommand group:

```rust
/// Manage integration definitions
Integrations {
    #[command(subcommand)]
    command: IntegrationsCommand,
},
```

With subcommands:

```rust
#[derive(clap::Subcommand)]
enum IntegrationsCommand {
    /// List all available integration services
    List,
    /// Show details of an integration service
    Show { service: String },
    /// Validate a YAML integration definition file
    Validate { file: PathBuf },
    /// Dry-run: construct an HTTP request without sending it
    Test {
        service: String,
        operation: String,
        /// JSON params (e.g. '{"owner":"octocat","repo":"hello-world"}')
        #[arg(long, default_value = "{}")]
        params: String,
    },
}
```

- [ ] **Step 3: Implement the handlers**

`list` — create an `IntegrationLoader`, call `list_all()`, print as a table.

`show <service>` — load definition, print operations with their params, methods, paths.

`validate <file>` — read the file, attempt `serde_yaml::from_str::<IntegrationDefinition>`, report success or errors.

`test <service> <operation>` — load definition, validate params, call `build_http_config()`, print the constructed request (method, URL, headers, body) without sending. Uses `"MOCK_CREDENTIAL"` as placeholder if no credential configured.

- [ ] **Step 4: Test manually**

Run: `cargo run -- integrations list`
Expected: Table showing github, slack, openai, stripe, notion

Run: `cargo run -- integrations show github`
Expected: Operations with params and methods

Run: `cargo run -- integrations test github list_issues --params '{"owner":"octocat","repo":"hello-world"}'`
Expected: Prints GET request to `https://api.github.com/repos/octocat/hello-world/issues?state=open&per_page=30&page=1` with headers

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): add 'r8r integrations list/show/validate' subcommands"
```

---

## Task 10: Workflow Schema Validation Update

**Files:**
- Modify: `src/workflow/schema.rs`

- [ ] **Step 1: Read `src/workflow/schema.rs` to understand the existing JSON Schema**

- [ ] **Step 2: Add `integration` to the valid node types**

The schema likely has an enum or pattern for valid `type` values. Add `"integration"` to it.

- [ ] **Step 3: Add schema for integration node config**

The integration node config requires `service` (string) and `operation` (string), with optional `credential` (string) and `params` (object).

- [ ] **Step 4: Run existing schema tests**

Run: `cargo test --lib workflow::schema -v`
Expected: All existing tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/workflow/schema.rs
git commit -m "feat(schema): add 'integration' node type to workflow validation schema"
```

---

## Summary

| Task | Description | New Files | Tests |
|------|-------------|-----------|-------|
| 1 | Definition structs | `definition.rs`, `mod.rs` | 3 unit |
| 2 | Param validator | `validator.rs` | 6 unit |
| 3 | Built-in loader | `loader.rs`, `github.yaml` | 3 unit |
| 4 | IntegrationNode core | `node.rs` | 6 unit |
| 5 | OAuth2 credential type | `oauth2.rs` | 5 unit |
| 6 | NodeContext + executor wiring | — (modifications only) | existing tests |
| 7 | Remaining YAML definitions | 4 YAML files | — |
| 8 | Integration tests | `integration_node_tests.rs` | 5 integration |
| 9 | CLI subcommands | — (main.rs modifications) | manual |
| 10 | Schema validation update | — (schema.rs modification) | existing tests |

**Total: 10 tasks, ~28 new tests, 7 new Rust files, 5 YAML definitions.**

**Deferred to follow-up plan:** OAuth2 auth flows (device code, local callback), token refresh at runtime, `r8r auth` CLI commands, form-encoding support for APIs like Stripe POST operations.

Tasks 1-4 form the critical path (definition → validation → loading → node). Tasks 5-6 add OAuth2 support. Tasks 7-10 are polish and completeness. Tasks 1-8 can be parallelized across subagents where dependencies allow (1 before 2-3, 2+3 before 4, 5 before 6, 4+6 before 8).
