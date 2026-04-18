# OAuth2 Auth Flows

**Date**: 2026-03-25
**Status**: Approved
**Scope**: `r8r auth login/logout/status` CLI commands, device code flow, local callback flow, runtime token refresh

## Problem

r8r has `OAuth2Credential` data structures and encrypted storage, but no runtime flows to actually acquire tokens. Users must manually paste access tokens into the credential store — which doesn't work for OAuth2 (tokens expire, refresh tokens are single-use on some providers, and initial acquisition requires browser interaction).

This feature provides the missing runtime layer: interactive token acquisition via browser, automatic refresh at execution time, and CLI commands to manage OAuth2 connections.

## Solution

Three CLI commands (`r8r auth login/logout/status`) that use two OAuth2 flows:

1. **Device Code Flow** — headless-friendly, default when supported by provider. User sees a code, enters it at a URL on any device. r8r polls for completion.
2. **Local Callback Flow** — browser-based. r8r spins up a temporary HTTP server on localhost, opens the browser to the provider's authorize URL, catches the redirect callback with the authorization code, exchanges it for tokens.

Plus automatic token refresh at execution time — when `IntegrationNode` resolves an OAuth2 credential and it's expired, refresh it using the stored `refresh_token` before making the API call.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Callback port | Preferred (8400) with random fallback + warning | Predictable default; random fallback works for providers that accept loopback:any-port but warns user to register the URL |
| Refresh timing | Eagerly at execution time, via `RefreshCoordinator` owned by `Executor` | No background processes; coordinator re-reads `CredentialStore` under the per-service lock so in-process refreshes propagate to all nodes |
| Browser behavior | Auto-open by default, `--no-browser` flag, auto-suppress on non-TTY | Best UX for desktop; works over SSH; CI won't hang waiting for `xdg-open` |
| CLI commands | `login`, `logout`, `status` — plus `--flow` escape hatch in v1 | Status gives debugging visibility; `--flow` unblocks users when the auto-selected flow is blocked by their network |
| Flow selection | Device code first if supported, else callback, `--flow` override | Device code is SSH-friendly; override matters for corporate proxies |
| `client_id`/`client_secret` storage | New `oauth2_clients` section of `CredentialStore` — plaintext in 0600 file | Consistent with existing (plaintext) `oauth2_credentials` storage; avoids blocking headless `r8r run` on a password unlock. Encryption-at-rest deferred to a follow-up hardening plan. |
| Refresh failure handling | Soft-delete: `needs_reauth=true`, null `refresh_token`; preserve rest of credential | Prevents concurrent-delete races; gives `r8r auth status` something to display so users know to re-login |
| Mutex scope | Per-service `tokio::sync::Mutex` via `DashMap`, re-read-under-lock | In-process serialization; cross-process races documented as v1 limitation |
| Hash primitive | `ring::digest` (already in tree) | Avoid adding `sha2` dependency; consistent with `credentials.rs` |

---

## 1. Auth Flow Architecture

Both flows share pre/post steps:

**Pre-flow (common):**
1. Load integration YAML definition (`IntegrationLoader`)
2. Read `auth.methods[]` — find entries with `type: oauth2`
3. Prompt for `client_id` + `client_secret` if not already in credential store as `<service>_client_id` / `<service>_client_secret` (interactive prompt uses `rpassword` crate for secret)
4. Determine flow: use device code if `flows` list contains `device_code`, else use `local_callback`

**Post-flow (common):**
1. Build `OAuth2Credential` struct with access_token, refresh_token, expires_at (from `expires_in`), token_type, scopes
2. Save to `CredentialStore` via `set_oauth2()`
3. Print confirmation: `"✓ Connected to github (scopes: repo, read:org)"`

### Device Code Flow

Per RFC 8628. Requires a new `device_authorization_url` field on `AuthMethod::OAuth2` (distinct from `token_url` — they are separate endpoints).

1. **Request device code** — POST to `device_authorization_url`
   - Headers: `Accept: application/json`, `Content-Type: application/x-www-form-urlencoded`
   - Body: `client_id=X&scope=<space-separated>`
   - Response (200): `{device_code, user_code, verification_uri, verification_uri_complete?, expires_in, interval}`
   - `interval` defaults to 5 seconds if the server omits it (RFC 8628 §3.2).

2. **Display instructions to user:**
   ```
   To authorize, open:
     https://github.com/login/device

   And enter code:
     ABCD-1234

   Waiting for authorization... (expires in 15m, polling every 5s)
   Press Ctrl-C to cancel.
   ```
   Use `verification_uri_complete` in the printed URL when provided (it embeds the code).

3. **Poll `token_url`** following RFC 8628 §3.4 / §3.5 rules:
   - **Sleep first**: Wait `interval` seconds BEFORE the first poll, and `interval` seconds between polls.
   - POST `token_url` with:
     - Headers: `Accept: application/json`, `Content-Type: application/x-www-form-urlencoded`
     - Body: `grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=X&client_id=Y` (add `client_secret=Z` for confidential clients)
   - On success (200): parse token response, proceed to credential storage.
   - On 400 `error: "authorization_pending"`: continue polling at the same `interval`.
   - On 400 `error: "slow_down"`: `interval += 5`, continue polling (RFC 8628 §3.5).
   - On 400 `error: "access_denied"`: abort with message `"Authorization denied."`.
   - On 400 `error: "expired_token"`: abort with message `"Authorization code expired. Run r8r auth login again."`.

4. **Timeout** — use `min(expires_in_from_response, 900 seconds)` — the response's `expires_in` is the authoritative device-code lifetime (GitHub sets 900s / 15 min). Capping at 900s prevents a misconfigured provider from holding the CLI hostage, while respecting values shorter than that. On timeout, abort with `"Authorization timed out. Run r8r auth login again."`.

### Local Callback Flow

Authorization code flow with PKCE (RFC 7636).

**Prerequisite (W1):** The user must have registered `http://127.0.0.1:8400/callback` (or whatever port) as an authorized redirect URI in the provider's OAuth app settings. Random-port fallback (step 2) will fail with `redirect_uri_mismatch` at many providers. When falling back to a random port, we warn the user to register that specific URL.

1. **Generate PKCE + CSRF state**:
   - `code_verifier`: 32 random bytes from `ring::rand::SystemRandom`, base64url-encoded without padding → 43 ASCII characters (unreserved subset per RFC 7636 §4.1).
   - `code_challenge`: `BASE64URL_NOPAD(SHA256(code_verifier))` — use `ring::digest::digest(&SHA256, verifier.as_bytes())`.
   - `state`: 32 random bytes, base16-encoded → 64 hex characters. CSRF defense.

2. **Start temporary HTTP server (must precede browser open — W6):**
   - Try `127.0.0.1:8400` first.
   - On `AddrInUse`, fall back to `127.0.0.1:0` (OS-assigned). Log a warning: `"Port 8400 in use, using port <N>. Ensure http://127.0.0.1:<N>/callback is registered in your OAuth app."`.
   - Use `axum 0.8` (already in Cargo.toml).
   - Single route: `GET /callback` → parses `code`, `state`, `error`, `error_description` from query string. On success: returns HTML "Authorization successful. You can close this window." and resolves a `tokio::sync::oneshot::Sender<CallbackResult>`. On `error` query parameter: returns HTML error page and resolves `Err(AuthorizationError { ... })`.
   - Use `axum_server::Handle` for graceful shutdown triggered by the oneshot resolution.
   - Wrap the whole server lifecycle in a `struct CallbackServer { handle, port }` with `Drop` to guarantee shutdown on every code path (including panics).

3. **Build authorize URL**:
   ```
   {authorization_url}?response_type=code
     &client_id={client_id}
     &redirect_uri=http://127.0.0.1:{port}/callback
     &scope={space-separated scopes, URL-encoded}
     &state={state}
     &code_challenge={code_challenge}
     &code_challenge_method=S256
   ```

4. **Open browser** — call `open::that(&url)` unless `--no-browser`. Always print the URL too so users can copy/paste if the auto-open fails. On non-TTY environments (detected via `atty::is(Stderr)` = false), auto-detect and skip browser open with a printed note (W10).

5. **Await callback** — `tokio::time::timeout(Duration::from_secs(300), receiver).await`. Timeout → `"Authorization timed out after 5 minutes."`.

6. **Validate state** — constant-time comparison (`subtle::ConstantTimeEq`) between received `state` and generated `state`. Mismatch → `"Authorization state mismatch (possible CSRF). Abort."`.

7. **Exchange code for tokens** — POST to `token_url`:
   - Headers: `Accept: application/json`, `Content-Type: application/x-www-form-urlencoded`
   - Body fields:
     - `grant_type=authorization_code`
     - `code=<received code>`
     - `redirect_uri=http://127.0.0.1:{port}/callback` (must match the authorize request exactly)
     - `client_id=<client_id>`
     - `client_secret=<client_secret>` — **only for `client_type: confidential`**
     - `code_verifier=<verifier>`
   - Response: `{access_token, refresh_token?, expires_in, scope, token_type}`.

8. **Shutdown server** — `CallbackServer::drop` fires on function return, guaranteeing cleanup.

### Flow Selection Logic

```rust
fn select_flow(method: &AuthMethod) -> Flow {
    match method {
        AuthMethod::OAuth2 { flows, .. } => {
            if flows.contains("device_code") {
                Flow::DeviceCode
            } else {
                Flow::LocalCallback
            }
        }
        _ => unreachable!("non-oauth2 method"),
    }
}
```

If a provider supports both, prefer device code (simpler, works over SSH). The user can force a specific flow via `r8r auth login <service> --flow <device_code|local_callback>`; if the forced flow is not in the provider's `flows` list, we error out with the list of supported flows (see §5 error table). This v1 escape hatch helps users behind corporate proxies where device flow is blocked but the callback flow would work.

---

## 2. Runtime Token Refresh

### State propagation: the coordinator pattern

`NodeContext.oauth2_credentials` is `Arc<HashMap<String, OAuth2Credential>>` — a read-only snapshot loaded at workflow start. Mutating it in place would not propagate to sibling/downstream nodes within the same run, and would not cross workflow boundaries. Instead, all refresh logic goes through a single `RefreshCoordinator` whose job is to own the read-through-write-through path for OAuth2 credentials.

```rust
pub struct RefreshCoordinator {
    /// Per-service mutex — serializes refresh attempts across async tasks in this process.
    /// Does NOT serialize across processes; see W11.
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// HTTP client for token requests.
    http: reqwest::Client,
}

impl RefreshCoordinator {
    /// Resolve a usable access token for this service. Re-reads the credential store
    /// on every call so that (a) refreshes done by earlier nodes are seen, and
    /// (b) refreshes done by other processes are seen.
    ///
    /// Returns the access_token string. On permanent failure, does NOT delete the
    /// stored credential — marks `needs_reauth=true` so `r8r auth status` can surface
    /// the condition, and returns an error prompting the user to re-login.
    pub async fn access_token_for(
        &self,
        service: &str,
        token_url: &str,
        client_id: &str,
        client_secret: Option<&str>,
    ) -> Result<String> { ... }
}
```

The coordinator is constructed once per `Executor` (not per-node), passed into `IntegrationNode.execute()` via an extension to `NodeContext` (new field `oauth2_coordinator: Option<Arc<RefreshCoordinator>>`). For `IntegrationNode`, if `oauth2_coordinator` is `None` (legacy path), fall back to reading `ctx.oauth2_credentials` directly without refresh — preserves the existing non-refresh behavior for callers that haven't wired the coordinator yet.

### Per-request flow

When `IntegrationNode.execute()` needs an OAuth2 access_token:

```
1. Identify the OAuth2 credential name from workflow config (`credential: github_oauth`)
2. Look up AuthMethod::OAuth2 entry from the service's YAML definition to obtain token_url,
   client_type (public | confidential).
3. Fetch client_id and client_secret (if confidential) from CredentialStore
   (see §3 "Client secret storage" — these are stored unencrypted alongside oauth2_credentials
   in credentials.json with 0600 mode; see "Security Posture" in §7).
4. Call coordinator.access_token_for(service, token_url, client_id, client_secret).
5. Use the returned access_token as a bearer token in the HTTP request.
```

### Coordinator internals

```
access_token_for(service, token_url, client_id, client_secret):
  lock = locks.entry(service).or_insert(Mutex::new())
  guard = lock.lock().await

  # Re-read the store under the lock (handles cross-node + cross-process consistency)
  store = CredentialStore::load().await
  cred = store.get_oauth2(service).ok_or(NotConnected)?

  if !cred.is_expired():
    return Ok(cred.access_token.clone())

  if cred.needs_reauth:
    return Err("OAuth2 credential for '{service}' needs re-authorization.
                Run: r8r auth login {service}")

  if cred.refresh_token.is_none():
    # Can't refresh, mark for re-auth
    cred.needs_reauth = true
    store.set_oauth2(cred)
    store.save().await
    return Err("Access token expired; no refresh_token stored.
                Run: r8r auth login {service}")

  # Attempt refresh
  result = http.post(token_url)
    .header("Accept", "application/json")
    .header("Content-Type", "application/x-www-form-urlencoded")
    .form(&[
      ("grant_type", "refresh_token"),
      ("refresh_token", &cred.refresh_token.unwrap()),
      ("client_id", client_id),
      # Include client_secret ONLY for confidential clients:
      ...maybe("client_secret", client_secret),
    ])
    .send()
    .with_retry(network_errors_only: 3, backoff: exponential_1_2_4_secs)
    .await

  match result:
    Ok(200, response):
      new_cred = build_refreshed_credential(cred, response)
      # RFC 6749 §6: response MAY include new refresh_token (rotation) or omit it (reuse).
      # RFC 6749 §6: response MAY include narrower scope.
      # RFC 6749 §6: response MUST include new access_token and SHOULD include expires_in.
      store.set_oauth2(new_cred.clone())
      store.save().await   # NOTE: requires adding save() call to set_oauth2 path
      return Ok(new_cred.access_token)

    Ok(4xx, body) with error == "invalid_grant":
      # Definitively dead refresh_token. Soft-delete: null out refresh_token,
      # mark needs_reauth so the user sees it in `r8r auth status`.
      # Do NOT remove the whole OAuth2Credential — preserves history, avoids
      # concurrent-delete races where task B clobbers task A's just-rotated credential.
      cred.refresh_token = None
      cred.needs_reauth = true
      store.set_oauth2(cred)
      store.save().await
      return Err("Refresh token invalid. Run: r8r auth login {service}")

    Ok(4xx, _) with other error:
      # Could be rate limit, misconfiguration, transient. Do not delete.
      return Err("Refresh failed: {error_description}. Retry the operation.")

    Ok(5xx, _) or network error after retries:
      return Err("Token refresh failed (transient: {status}). Retry the operation.")
```

**Cross-process concurrency (W11):** The `DashMap` lock is in-process only. Two `r8r run` processes sharing the same credentials.json can still race, and one may rotate the refresh_token before the other reads. The re-read-under-lock pattern makes this safe for in-process concurrency but not for cross-process. For v1, we accept this limitation and document it in `r8r auth login` output:

> *"Tip: Running multiple r8r processes against the same credential store concurrently is not recommended — refresh token rotation may race."*

If this becomes a problem, add `fs2::FileExt::lock_exclusive` around the refresh block in a follow-up.

### `needs_reauth` field

Add a new field to `OAuth2Credential`:

```rust
#[serde(default)]
pub needs_reauth: bool,
```

- Set to `true` when refresh fails with `invalid_grant` or when access token expires with no refresh_token.
- `r8r auth login <service>` clears this flag and overwrites the credential.
- `r8r auth status` displays "needs re-login" for any credential where this is set.

### CredentialStore changes

Add to `src/credentials.rs`:

```rust
/// Set an OAuth2 credential AND persist to disk.
pub async fn set_oauth2_and_save(
    &mut self,
    credential: OAuth2Credential,
) -> Result<()> {
    self.set_oauth2(credential);
    self.save().await
}

/// Remove an OAuth2 credential (used by `r8r auth logout`).
pub async fn remove_oauth2(&mut self, service: &str) -> Result<bool> {
    let existed = self.oauth2_credentials.remove(service).is_some();
    if existed {
        self.save().await?;
    }
    Ok(existed)
}
```

The existing `set_oauth2()` method stays (in-memory only) for callers that batch multiple changes before saving. The coordinator uses `set_oauth2_and_save()`.

---

## 3. CLI Commands

### Client secret storage

OAuth2 requires `client_id` (always) and `client_secret` (for confidential clients). The existing `CredentialStore` has two classes of storage:

- `credentials: HashMap<String, Credential>` — encrypted with master key. Requires interactive unlock to read.
- `oauth2_credentials: HashMap<String, OAuth2Credential>` — **stored as plaintext JSON in the same `credentials.json` file** (file-mode 0600). No encryption today.

Since `oauth2_credentials` already lives unencrypted in the 0600-protected file, we store OAuth2 client config in the same tier — as a new field `oauth2_clients: HashMap<String, OAuth2ClientConfig>` on `CredentialStore`, plaintext, `#[serde(default)]` for backward compatibility:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2ClientConfig {
    pub service: String,
    pub client_id: String,
    pub client_secret: Option<String>,  // None for public clients
    pub client_type: ClientType,         // public | confidential
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ClientType {
    Public,
    Confidential,
}

impl CredentialStore {
    pub fn get_oauth2_client(&self, service: &str) -> Option<&OAuth2ClientConfig> { ... }
    pub async fn set_oauth2_client_and_save(&mut self, cfg: OAuth2ClientConfig) -> Result<()> { ... }
    pub async fn remove_oauth2_client(&mut self, service: &str) -> Result<bool> { ... }
}
```

This design resolves **B2**: client secrets can be read during a non-interactive `r8r run` (which uses `CredentialStore::load()` without password) because they live outside the encrypted section.

**Security posture note (see also §7):** This is a deliberate tradeoff matching the existing OAuth2 credential storage — all OAuth2 material lives in one 0600 file, consistent but unencrypted at rest. A follow-up plan can migrate OAuth2 material to the encrypted section with `R8R_MASTER_PASSWORD` env-var unlock for headless runs; that's out of scope for v1 and tracked as a future security hardening task.

### `r8r auth login <service> [--no-browser] [--flow device_code|local_callback]`

1. Load integration definition for `<service>` from `IntegrationLoader`
2. Find first `AuthMethod::OAuth2` entry (error if none exists)
3. Determine client config:
   - Check `store.get_oauth2_client(service)` — if present, use it.
   - If missing, prompt interactively:
     - Prompt for `client_id` using normal readline.
     - Prompt for `client_type` (public | confidential). Default to `confidential` (matches GitHub/Slack/Notion).
     - If confidential, prompt for `client_secret` using `rpassword::prompt_password` (no echo).
     - Save via `store.set_oauth2_client_and_save()`.
4. Select flow:
   - If `--flow` passed, use that (error if unsupported by provider).
   - Else if provider's `flows` includes `device_code` AND client is supported: use device code.
   - Else: use local callback.
5. Run flow (`--no-browser` only affects local callback; device code always prints code + URL).
6. Build `OAuth2Credential` from token response, store via `set_oauth2_and_save()`.
7. Print success: `"✓ Connected to <service> (scopes: <scopes>, expires in <duration>)"`.

### `r8r auth logout <service>`

1. Load `CredentialStore`
2. Zero out local tokens regardless of revocation success (unconditional local deletion — W8):
   - Best-effort revocation: if provider has a `revocation_url` in YAML, POST `token=<access_token>&token_type_hint=refresh_token` to revoke (RFC 7009). Log network/provider errors at warn level, do not fail.
   - Then `store.remove_oauth2(service)` and `store.save()`.
3. Optionally also `store.remove_oauth2_client(service)` if user passes `--remove-client`. Default behavior preserves `client_id`/`client_secret` so a subsequent `login` doesn't re-prompt.
4. Print: `"✓ Disconnected from <service>"` or `"<service> was not connected"`.

### `r8r auth status [--verbose]`

1. Load `CredentialStore`, call `list_oauth2()`
2. For each credential, compute a single STATUS column:
   - `active` — not expired
   - `expired (refreshing)` — expired, has refresh_token, not marked `needs_reauth`
   - `needs re-login` — `needs_reauth` is true, or expired without refresh_token
   - `no expiry` — no expires_at set

   Render a table:
   ```
   SERVICE        PROVIDER    SCOPES              EXPIRES         STATUS
   github_oauth   github      repo, read:org      in 45m          active
   slack_oauth    slack       chat:write          expired         expired (refreshing)
   notion_oauth   notion      (none)              never           no expiry
   stripe_oauth   stripe      read_write          expired         needs re-login
   ```
3. With `--verbose`, additionally show: `client_id`, `client_type`, whether `refresh_token` is present, `created_at`, `updated_at`.
4. If none: `"No OAuth2 connections. Run: r8r auth login <service>"`

### HTTP conventions for all token exchanges

Every POST to a token endpoint (device code, callback code exchange, refresh) uses:

- `Content-Type: application/x-www-form-urlencoded`
- `Accept: application/json` — **mandatory for GitHub compatibility** (see B3)
- Request body built via `reqwest::RequestBuilder::form(&[...])`
- Response parsed as JSON; if parsing fails, emit a descriptive error including a hint about the `Accept` header

---

## 4. File Layout

### Restructure

Current: `src/integrations/oauth2.rs` (single file with `OAuth2Credential`)

New:
```
src/integrations/oauth2/
├── mod.rs               # OAuth2Credential (existing, add `needs_reauth` field) + re-exports
├── device_flow.rs       # acquire tokens via RFC 8628 device code flow
├── callback_flow.rs     # acquire tokens via browser redirect + PKCE
├── refresh.rs           # RefreshCoordinator with per-service locks + re-read-under-lock
├── client_config.rs     # OAuth2ClientConfig + ClientType + CredentialStore extension methods
└── cli.rs               # login/logout/status command handlers
```

The existing `src/integrations/oauth2.rs` is split: `OAuth2Credential` + serde remain in `oauth2/mod.rs`, preserving the serialized format fully (only addition: `#[serde(default)] needs_reauth: bool`, which deserializes as `false` for existing files — zero migration).

### HttpNode integration (W12)

Today `IntegrationNode::build_http_config` (in `src/integrations/node.rs`) resolves string credentials from `ctx.credentials` and emits `{credential, auth_type}` fields for `HttpNode` to consume. For OAuth2, the chain is:

1. `IntegrationNode::execute()` receives `NodeContext` with new `oauth2_coordinator: Option<Arc<RefreshCoordinator>>` field.
2. When the integration's active `AuthMethod` is `OAuth2`, `IntegrationNode` calls `coordinator.access_token_for(service, ...)` and receives a bearer token string.
3. The bearer token is injected into the outgoing HTTP request via a header merge: `Authorization: Bearer <token>` is added to `headers` in the built HTTP config. **Not** via `ctx.credentials` (which would persist the token in the context map, leaking across nodes).
4. `HttpNode` sees this as a regular `Authorization` header and sends it; it does not need to know the header came from OAuth2.

This keeps `HttpNode` OAuth2-unaware. The `auth_type: bearer` path in `HttpNode::apply_authentication` is unchanged.

### Existing Files Modified

- `src/integrations/definition.rs` — extend `AuthMethod::OAuth2` variant with:
  - `device_authorization_url: Option<String>` (required if `flows` contains `"device_code"`; validator should emit an error during YAML loading when missing)
  - `revocation_url: Option<String>` (optional, used by `logout`)
  - `client_type: Option<String>` — when absent, default is `"confidential"` at login time (matches GitHub/Slack/Notion); user is prompted for confirmation
  - `scope_separator: Option<String>` — default `" "` (space); GitHub uses `","` for some scope parsings — leave as future work if needed
- `src/integrations/node.rs` — add OAuth2 branch: call coordinator, inject `Authorization: Bearer` header
- `src/nodes/types.rs` — add `oauth2_coordinator: Option<Arc<RefreshCoordinator>>` field to `NodeContext`; update `Debug` impl to not dump the coordinator; propagate through `for_item()`; add `.with_oauth2_coordinator()` builder method
- `src/engine/executor.rs` — construct `RefreshCoordinator` once per Executor, attach to every `NodeContext` built
- `src/integrations/mod.rs` — `pub mod oauth2;` already in place (dir-based module works with existing syntax)
- `src/credentials.rs`:
  - Add `oauth2_clients: HashMap<String, OAuth2ClientConfig>` field with `#[serde(default)]`
  - Add `get_oauth2_client`, `set_oauth2_client_and_save`, `remove_oauth2_client` methods
  - Add `set_oauth2_and_save`, `remove_oauth2` methods
  - **Update `OAuth2Credential` Debug impl** to redact `access_token` and `refresh_token` (N4) — same pattern as `NodeContext`'s redacted credentials
- `src/main.rs` — add `Auth` subcommand to `Commands` enum, `AuthCommand` enum with `Login { service, no_browser, flow }` / `Logout { service, remove_client }` / `Status { verbose }` variants
- `src/integrations/builtin/github.yaml`:
  - Add `device_authorization_url: https://github.com/login/device/code`
  - Add `client_type: confidential`
- `src/integrations/builtin/slack.yaml`:
  - Keep only `local_callback` in flows (Slack does not support device code for user tokens)
  - Add `client_type: confidential`
- `src/integrations/builtin/notion.yaml`:
  - Keep only `local_callback` in flows
  - Add `client_type: confidential`

### New Dependencies

- `open = "5"` — cross-platform URL opener for browser auto-open
- `rpassword = "7"` — secret input prompts
- `dashmap = "6"` — concurrent HashMap for `RefreshCoordinator.locks`
- `atty = "0.2"` — detect non-TTY environments to suppress auto-browser-open on headless systems
- **Removed from previous draft:** `sha2` — use `ring::digest::digest(&ring::digest::SHA256, ...)` instead, since `ring` is already the hash primitive in `credentials.rs`. No new crypto dep.
- `base64 = "0.22"` — already a dep, for PKCE base64url encoding
- `subtle = "2"` — already a dep, for constant-time state comparison

Dev deps:
- `wiremock = "0.6"` — mock HTTP server for testing token endpoints

---

## 5. Error Handling

### User-Facing Errors

| Condition | Message |
|-----------|---------|
| No OAuth2 method in integration | `"Integration '<service>' does not support OAuth2."` |
| Missing `client_id`/`client_secret` (non-interactive) | `"OAuth2 client credentials missing. Run: r8r credentials set <service>_client_id <value>"` |
| Device code timeout (5 min) | `"Authorization timed out after 5 minutes. Run r8r auth login again."` |
| User denied authorization | `"Authorization denied."` |
| Device code expired before use | `"Authorization code expired. Run r8r auth login again."` |
| Port 8400 in use (informational) | `"Port 8400 in use, using port <random>"` (warning, not error) |
| All ports unavailable | `"Could not bind callback server to any port"` |
| Token refresh: invalid_grant | Soft-delete: set `needs_reauth=true`, null refresh_token, preserve rest. `"Refresh token invalid. Run: r8r auth login <service>"` |
| Token refresh: non-invalid_grant 4xx | Keep credential intact. `"Refresh failed: <error_description>. Retry the operation."` |
| Token refresh: 5xx after retries | Keep credential intact. `"Token refresh failed (transient: <status>). Retry the operation."` |
| Token refresh: network error after retries | Keep credential intact. `"Token refresh failed (network): <error>. Retry the operation."` |
| `--flow <name>` passed but not supported | `"Flow '<name>' not supported by '<service>'. Supported: <list from YAML>."` |
| State mismatch (CSRF) | `"Authorization state mismatch (possible CSRF attack). Abort."` |

### Logging

- **Debug level**: HTTP method + URL of every OAuth request (never log tokens, body with secrets)
- **Info level**: Refresh attempts (`"Refreshing token for '<service>'"`)
- **Warn level**: Failed refreshes (with reason — not the token itself)

Never log: `access_token`, `refresh_token`, `client_secret`, PKCE verifier, state parameter.

### Retry Policy

- Network errors during token exchange: 3 attempts, exponential backoff (1s, 2s, 4s)
- 5xx from token endpoint: same retry policy
- 4xx from token endpoint: no retry (indicates client error)

---

## 6. Testing Strategy

### Unit Tests

**`device_flow.rs`** — `wiremock` mock for device-auth + token endpoints:
- Successful flow: device authorization request → polled token request returns tokens → `OAuth2Credential` built with correct fields
- `interval` is respected: sleep before first poll (assert via `tokio::time::pause()` + elapsed check)
- `authorization_pending` response causes continued polling at same interval
- `slow_down` response increments interval by 5s (assert via advance of mocked clock)
- `access_denied` aborts with `"Authorization denied."`
- `expired_token` aborts with `"Authorization code expired..."`
- Timeout at `min(expires_in, 900s)` — verify with 60s `expires_in` that we give up at 60s not 900s
- `Accept: application/json` header is sent on token endpoint (verify with `wiremock` request matcher)

**`callback_flow.rs`** — split into pure-function tests and TCP-integration tests:

*Pure functions (no I/O):*
- PKCE verifier: 32 random bytes → exactly 43 base64url chars, only unreserved characters
- PKCE challenge: `SHA256(verifier)` base64url-encoded, matches reference vector from RFC 7636 Appendix B
- State: 64 hex chars, two consecutive generations produce different values (weak entropy sanity check)
- Authorize URL builder: all required params present, scopes space-separated and URL-encoded
- Callback query parser: `?code=X&state=Y` → `{code, state}`; missing `state` → error; `?error=access_denied` → error

*TCP + mock integration test:*
- Harness:
  1. Spin up `CallbackServer` on port 0 (random).
  2. Start `wiremock` for the token endpoint.
  3. Build the authorize URL.
  4. In a spawned task, simulate the browser: HTTP GET `http://127.0.0.1:{port}/callback?code=TEST_CODE&state={state}`.
  5. Assert the server receives the callback, resolves the oneshot, token exchange fires against wiremock, resulting `OAuth2Credential` has the mocked access_token.
- State mismatch variant: simulated browser sends wrong `state` → error returned, no token request made.
- User-denial variant: simulated browser sends `?error=access_denied` → error returned.
- Port-in-use variant: bind two servers; second falls back to random port, succeeds.
- Drop-on-error: force a panic mid-flow; assert the server port is free afterwards (cleanup via `Drop`).

**`refresh.rs`** — `wiremock` for token endpoint, tempfile for `CredentialStore`:
- Fresh token (not expired) → `access_token_for` returns without HTTP call (assert with `wiremock` zero-request expectation)
- Expired token with refresh_token → HTTP call → new `OAuth2Credential` persisted (re-read store from disk to assert)
- Expired without refresh_token → `needs_reauth` set, error returned, no HTTP call
- Refresh token rotation: response includes new `refresh_token` → stored one updated
- Refresh token preserved: response omits `refresh_token` → old one retained
- `invalid_grant` response → credential NOT deleted, `needs_reauth` = true, `refresh_token` = None
- Non-invalid_grant 4xx → credential fully preserved, error returned
- 5xx with network retry → eventually succeeds on 3rd attempt if transient
- 5xx persistent → error after retries, credential preserved
- **Concurrent refresh (N6)**: spawn 2 tasks against the same expired credential, assert only 1 HTTP request made (mutex serialization); second task re-reads the refreshed credential and returns immediately
- **Chain-of-refreshes (N6)**: task T1 refreshes at t=0, task T2 at t=0 (serialized, sees T1's rotation); at t=expires+1, T3 triggers and observes T1's new refresh_token in the store (not the original)
- `Accept: application/json` header sent

**`cli.rs`** — integration-light tests using a temp directory and seeded store file:
- `status` with empty store → "No OAuth2 connections..."
- `status` with 3 seeded credentials in varying expiry states → table lines match expected format per state
- `status --verbose` includes `client_id`, `refresh_token` presence
- `logout` removes only `oauth2_credentials[service]` but preserves `oauth2_clients[service]` by default
- `logout --remove-client` also removes client config

### Integration Tests

- `tests/oauth2_cli_tests.rs` — drive `r8r auth status` / `r8r auth logout` via `assert_cmd` with a temp home dir and pre-seeded `credentials.json` file.

### Out of Scope

- Real OAuth2 flows against GitHub / Slack / Notion — manual QA, documented in release notes.
- Browser auto-open behavior across OSes — manual testing required (mocked with `--no-browser` in unit tests).
- Token revocation call against real providers — best-effort, manual QA only.
- Cross-process refresh serialization — documented limitation, not tested.

---

## 7. Security Considerations

### Storage posture

OAuth2 material (access tokens, refresh tokens, client secrets) is stored as plaintext JSON in `credentials.json` with file mode `0600` (owner-only read/write). The exact on-disk path is `Config::data_dir().join("credentials.json")`:

- **Linux**: `~/.local/share/r8r/credentials.json` (per XDG Base Directory)
- **macOS**: `~/Library/Application Support/r8r/credentials.json`
- **Windows**: `%APPDATA%\r8r\credentials.json`

This matches the existing `oauth2_credentials` storage (already plaintext in v0.4). This is a deliberate tradeoff: encryption would require the master password at every workflow run, breaking non-interactive execution (cron, CI, background runners). Future work: migrate to encrypted storage with an `R8R_MASTER_PASSWORD` env-var unlock path for headless use; tracked as a follow-up security hardening task.

### Token handling

- **Never log tokens or secrets.** `OAuth2Credential` has a custom `Debug` impl redacting `access_token` and `refresh_token` (same pattern as `NodeContext`'s redacted `credentials` field). `OAuth2ClientConfig` similarly redacts `client_secret`. CI includes a grep-based smoke check that forbids `println!`, `eprintln!`, `dbg!`, and `format!("{:?}",` on types holding secret fields within `src/integrations/oauth2/**`.
- **Token refresh logs**: log at most service name + provider + HTTP status code. Never log the request/response body of a token endpoint.
- **Client secret prompts** — use `rpassword::prompt_password` to avoid terminal echo and shell history leakage.

### Authorization flow protections

- **PKCE (RFC 7636) required for the callback flow** — S256 challenge method, `code_verifier` generated from 32 random bytes (→ 43 base64url chars). Applied even when the client is confidential; defeats authorization code interception attacks.
- **State parameter (CSRF defense)** — 32 random bytes, base16-encoded (64 hex chars). Compared in constant time (`subtle::ConstantTimeEq`) on callback to avoid timing side channels.
- **Localhost binding only** — callback server binds to `127.0.0.1`, never `0.0.0.0` or a hostname. Prevents remote network exposure.
- **Callback server lifetime** — single request, shuts down after the first callback is received or timeout expires. `CallbackServer { Drop }` guarantees shutdown on panics and error paths.
- **Browser URL printing** — when `--no-browser` is set OR the environment is non-TTY, we print the authorize URL but never auto-open. The URL contains the `state` parameter but no long-lived secrets.

### Refresh race handling

- **Per-service mutex + re-read-under-lock** (see §2) — prevents concurrent refreshes from rotating the same refresh token in parallel (some providers invalidate the old refresh_token immediately on rotation, which would cause the second task's refresh to fail).
- **Soft failure** — on `invalid_grant`, set `needs_reauth=true` and null out `refresh_token`, but do NOT delete the full credential. Preserves history and prevents concurrent-delete races.
- **Cross-process concurrency is NOT serialized** in v1 (no file lock). Documented limitation; callers running multiple `r8r` processes against one credential store should be aware of potential refresh races.

### HTTP hardening

- All token endpoint requests set `Accept: application/json` explicitly (B3 — required for GitHub).
- `reqwest::Client` built with default `rustls-tls` (existing project convention) — no HTTP-plaintext fallback permitted for token endpoints (reject `http://` scheme for `token_url` / `device_authorization_url` / `authorization_url`, except the callback redirect which is unavoidably loopback HTTP).
