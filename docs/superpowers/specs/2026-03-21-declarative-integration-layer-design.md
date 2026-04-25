# Declarative Integration Layer + OAuth2

**Date**: 2026-03-21
**Status**: Approved
**Scope**: OAuth2 credential system, YAML integration definitions, IntegrationNode, distribution model

## Problem

r8r has 25 built-in node types, including purpose-built nodes for Slack, S3, Email, and Database. Each is a Rust struct requiring compilation. The Slack node is 299 lines for a single operation (chat.postMessage). Adding more operations or services means more Rust code, more compilation, more releases.

Users can already call any REST API via the generic `http` node + templates, but this gives no validation, no discoverability, no reusable auth configuration, and no OAuth2 support.

## Solution

A declarative YAML integration layer that:
1. Defines services and their operations as YAML files
2. Executes via a single `IntegrationNode` that delegates to `HttpNode`
3. Supports OAuth2 credentials alongside existing auth types
4. Ships built-in definitions with user overrides

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Distribution | Built-in + user overrides at `~/.config/r8r/integrations/` | Out-of-box experience + community extensibility without recompilation |
| OAuth2 storage | Separate `OAuth2Credential` type in existing encrypted store | Clean semantics, no migration needed |
| HTTP execution | Delegate to `HttpNode` | Inherits SSRF protection, circuit breakers, timeouts |
| Node namespace | `type: integration` with `service` + `operation` in config | No collision with existing nodes, clean separation |
| Existing nodes | Keep Rust nodes (slack, s3, email, database) as-is | No breaking changes |
| Initial integrations | GitHub, Slack, OpenAI, Stripe, Notion | Covers OAuth2, webhook/token, API key, bearer auth patterns (all REST-based) |
| OAuth2 flows | Device code (default) + local callback (fallback) | Device flow works headless/SSH; local callback for providers that require it |

---

## 1. OAuth2 Credential System

### New Struct

```rust
pub struct OAuth2Credential {
    pub service: String,               // e.g. "github_oauth"
    pub provider: String,              // e.g. "github"
    pub access_token: String,          // encrypted
    pub refresh_token: Option<String>, // encrypted
    pub expires_at: Option<DateTime<Utc>>,
    pub token_type: String,            // "Bearer"
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Storage

Stored in the existing `credentials.json` under a new `oauth2_credentials` key. Same AES-256-GCM encryption, same master key. The existing `credentials` field is untouched — no migration needed.

### Token Refresh

When `IntegrationNode` resolves an OAuth2 credential and `expires_at` is past or within 5 minutes, it auto-refreshes using the `refresh_token` + the provider's `token_url` from the YAML integration definition. Updated tokens are written back to the store.

Token refresh uses an in-process `tokio::sync::Mutex` per service name to prevent concurrent refresh attempts. The first caller refreshes; others wait and receive the updated token. This prevents refresh token invalidation on providers that rotate refresh tokens on use.

### Provider Configuration

OAuth2 flows require `client_id` and `client_secret` for token exchange. These are stored as regular credentials in the existing encrypted credential store:
- `<service>_client_id` — e.g. `github_oauth_client_id`
- `<service>_client_secret` — e.g. `github_oauth_client_secret`

The `r8r auth login` command prompts for these if not already configured. They can also be pre-set via `r8r credentials set github_oauth_client_id <value>`.

### Device Code Flow Details

- Polling interval: use server-provided `interval` from device authorization response, default 5 seconds
- Maximum wait: 5 minutes before timeout
- User messaging: `"Open https://... and enter code: XXXX-XXXX. Waiting for approval..."`
- On device code expiry: prompt user to retry

### Credential Storage Path

OAuth2 credentials are stored alongside existing credentials at `Config::data_dir()/credentials.json` (typically `~/.local/share/r8r/credentials.json`). This is the data directory, distinct from `~/.config/r8r/` where integration definitions live.

### CLI Commands

- `r8r auth login <service>` — initiates device flow or local callback based on provider config
- `r8r auth status` — shows connected services + token expiry
- `r8r auth logout <service>` — revokes + removes tokens

### Auth Flow Selection

The YAML integration definition specifies supported flows:
- `device_code` — default, works over SSH and headless servers
- `local_callback` — spins up temporary localhost server, opens browser, catches redirect

The CLI tries device code first. If the provider doesn't support it, falls back to local callback.

---

## 2. YAML Integration Definition Format

Each integration is a YAML file defining a service's base config, auth requirements, and operations.

### Example: GitHub

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
    description: Create a new issue
    method: POST
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true, description: "Repository owner" }
      repo: { required: true, description: "Repository name" }
      title: { required: true, description: "Issue title" }
      body: { required: false, description: "Issue body" }
      labels: { required: false, type: array, description: "Label names" }
    body:
      title: "{{ params.title }}"
      body: "{{ params.body }}"
      labels: "{{ params.labels }}"

  get_issue:
    description: Get an issue by number
    method: GET
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues/{{ params.number }}
    params:
      owner: { required: true }
      repo: { required: true }
      number: { required: true, type: integer }

  list_issues:
    description: List repository issues
    method: GET
    path: /repos/{{ params.owner }}/{{ params.repo }}/issues
    params:
      owner: { required: true }
      repo: { required: true }
      state: { required: false, default: open, enum: [open, closed, all] }
      per_page: { required: false, type: integer, default: 30 }
    query:
      state: "{{ params.state }}"
      per_page: "{{ params.per_page }}"
```

### Design Points

- `params` defines inputs with validation: `required`, `type`, `enum`, `default`
- `body` vs `query` placement is explicit per operation — no n8n-style routing abstraction
- `headers` at service level merge with per-operation headers; per-operation headers can be added inside any operation block
- `path` uses `{{ params.X }}` template syntax — see "Param Substitution" below for how this differs from the main template engine
- `auth.methods` lists supported auth types; the credential's actual type determines which is used at runtime
- Definition files should include `version: 1` for future schema evolution

### Param Substitution (IntegrationNode-specific)

The `{{ params.X }}` syntax in YAML definitions is **not** handled by the main r8r template engine (`render_template()`). It is a separate substitution pass performed by `IntegrationNode` before building the `HttpNode` config:

1. IntegrationNode resolves `{{ params.X }}` in `path`, `query`, and `headers` using simple string replacement
2. For `body`, IntegrationNode constructs a typed JSON object programmatically — it does **not** use string interpolation. Array params become JSON arrays, integers become JSON numbers, strings become JSON strings. This prevents the problem where `"{{ params.labels }}"` would stringify an array into `"[\"bug\"]"` instead of `["bug"]`
3. After IntegrationNode builds the final URL (with query params encoded into the URL string), `HttpNode` receives the config and applies the main template engine's `{{ input.X }}` / `{{ nodes.X.output }}` substitution on the URL field as usual

### Query Parameter Handling

`HttpNode` does not have a native `query` field. IntegrationNode handles this by URL-encoding query parameters and appending them to the URL string before delegating:

- `base_url` + `path` + `?key1=value1&key2=value2`
- Values are URL-encoded using `percent-encoding`
- Empty/null optional params are omitted from the query string
- This avoids any modification to `HttpNode`

---

## 3. IntegrationNode Implementation

### Workflow Usage

```yaml
- id: create-issue
  type: integration
  config:
    service: github
    operation: create_issue
    credential: my_github_token
    params:
      owner: "{{ input.owner }}"
      repo: "{{ input.repo }}"
      title: "Bug: {{ input.error_message }}"
      body: "Reported by automation"
```

### Execution Flow

1. Parse `service` + `operation` from config
2. Load integration definition (built-in definitions are cached; user definitions are loaded from `~/.config/r8r/integrations/` with mtime check for cache invalidation)
3. Validate `params` against the operation's param definitions (required, type, enum)
4. Resolve `credential`:
   - First check `ctx.oauth2_credentials` for an `OAuth2Credential` match
   - If found and expired/near-expiry, acquire per-service refresh lock and auto-refresh
   - If not found, fall back to `ctx.credentials` (plain credential, used as-is by `HttpNode`)
5. Perform `{{ params.X }}` substitution on `path` and `query` (string replacement)
6. Build typed JSON `body` programmatically (arrays stay arrays, integers stay integers — no string interpolation)
7. Encode `query` params into the URL string (`base_url` + rendered `path` + `?key=value&...`)
8. Build `HttpNode`-compatible config:
   - `url` = fully constructed URL with query params
   - `method` from operation
   - `headers` = service-level merged with operation-level + auth header from credential
   - `body` = typed JSON object from step 6
   - `credential` + `auth_type` passed through if using plain credential
9. Delegate to `HttpNode.execute()`
10. Return result as-is (status, headers, body)

### Struct Design

```rust
pub struct IntegrationNode {
    http_node: HttpNode,
    builtin: HashMap<String, IntegrationDefinition>,  // parsed once at startup
    user_cache: RwLock<UserDefinitionCache>,            // lazy-loaded, mtime-checked
}

struct UserDefinitionCache {
    definitions: HashMap<String, IntegrationDefinition>,
    last_scan: Instant,
    mtimes: HashMap<PathBuf, SystemTime>,
}

impl IntegrationNode {
    /// Resolve a service definition: user override takes precedence over built-in.
    /// User definitions are cached and re-scanned if any file mtime changed.
    pub fn get_definition(&self, service: &str) -> Option<IntegrationDefinition> {
        let user = self.user_cache.read().unwrap();
        if let Some(def) = user.definitions.get(service) {
            return Some(def.clone());
        }
        self.builtin.get(service).cloned()
    }
}
```

### CLI Discoverability

- `r8r integrations list` — show all available services
- `r8r integrations show github` — show operations + params
- `r8r integrations test github create_issue` — dry-run: validates params, constructs the HTTP request, and prints it (method, URL, headers, body) without sending. Uses mock credentials if none configured.

---

## 4. Distribution Model

### Built-in Definitions

- Stored in `src/integrations/builtin/*.yaml` in the source tree
- Compiled into the binary via `include_str!()` macro
- Parsed once at `IntegrationRegistry` initialization
- Initial set: GitHub, Slack, OpenAI, Stripe, Notion

### User Definitions

- Directory: `~/.config/r8r/integrations/`
- Created automatically on first run if missing
- User drops YAML files here, available immediately (cached with mtime check for invalidation)
- User definitions with the same `name` as a built-in override it completely

### Validation

- `r8r integrations validate <file.yaml>` — checks definition against a JSON schema
- Schema validates: required fields (name, base_url, operations), param types, valid HTTP methods, template syntax
- Invalid definitions logged as warnings and skipped, don't crash the system

### Future (Not In Scope)

- `r8r integrations install <url>` — fetch from a community registry
- Versioning of definitions
- Community registry (curated GitHub repo of YAML definitions)

---

## 5. File Layout

### New Files

```
src/
├── integrations/
│   ├── mod.rs              # IntegrationNode, IntegrationRegistry
│   ├── definition.rs       # IntegrationDefinition, OperationDef, ParamDef structs
│   ├── loader.rs           # Load built-in + user YAML, merge, validate
│   ├── validator.rs        # Schema validation for definition files
│   ├── oauth2.rs           # OAuth2Credential, token refresh, auth flows
│   └── builtin/
│       ├── github.yaml
│       ├── slack.yaml
│       ├── openai.yaml
│       ├── stripe.yaml
│       └── notion.yaml
```

### Changes to Existing Files

- `credentials.rs` — add `OAuth2Credential` struct, `oauth2_credentials: HashMap<String, OAuth2Credential>` to `CredentialStore` (with `#[serde(default)]` for backward-compatible deserialization of existing credential files), encryption/decryption for OAuth2 tokens
- `nodes/types.rs` — add `oauth2_credentials: Arc<HashMap<String, OAuth2Credential>>` to `NodeContext`; update `for_item()` to propagate the new field; update `Debug` impl to redact it
- `engine/executor.rs` — extend `load_workflow_credentials()` to also load OAuth2 credentials into the new `NodeContext` field
- `nodes/registry.rs` — register `IntegrationNode`
- `main.rs` — add `r8r auth` and `r8r integrations` CLI subcommands
- `workflow/schema.rs` — extend JSON schema to validate `type: integration` node configs

### No Changes To

- `nodes/http.rs` — used as-is via delegation
- `nodes/slack.rs`, `nodes/s3.rs` — kept intact

---

## 6. Testing Strategy

### Unit Tests

- `definition.rs` — parse valid/invalid YAML definitions, param validation (required, type, enum, defaults)
- `loader.rs` — built-in loading, user override precedence, invalid files skipped gracefully
- `validator.rs` — schema validation catches missing fields, bad HTTP methods, invalid template syntax
- `oauth2.rs` — token refresh logic, expiry detection, credential serialization/deserialization
- `IntegrationNode.execute()` — builds correct `HttpNode` config from definition + params (mock `HttpNode`)

### Integration Tests

- Round-trip: YAML definition -> IntegrationNode config -> HttpNode delegation -> response
- OAuth2 credential flow with mock token endpoint
- User definition overriding built-in definition
- CLI commands: `r8r integrations list`, `r8r integrations show`, `r8r integrations validate`

### Existing Test Preservation

- All 25 existing node tests untouched
- Existing credential store tests still pass
- Existing workflow schema validation still passes

### Out of Scope for Automated Tests

- Actual OAuth2 browser flows (manual QA)
- Real API calls to GitHub/Slack/etc (mock servers only)
- Community registry fetching (future feature)
