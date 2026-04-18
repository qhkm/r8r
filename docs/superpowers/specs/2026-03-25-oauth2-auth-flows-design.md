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
| Callback port | Preferred (8400) with random fallback | Simple predictable default, but doesn't fail if port taken |
| Refresh timing | Eagerly at execution time (in IntegrationNode) | No background processes; 5-min buffer via `is_expired()` |
| Browser behavior | Auto-open by default, `--no-browser` flag | Best UX for desktop, works over SSH |
| CLI commands | `login`, `logout`, `status` | Status provides debugging value; manual refresh unneeded (automatic) |
| Flow selection | Device code first if supported, else callback | Device code works everywhere (SSH, headless) |
| `client_id`/`client_secret` storage | Existing encrypted credential store | Already secure; no new infrastructure |
| Mutex scope | Per-service `tokio::sync::Mutex` | Prevents concurrent refreshes of same token (providers rotate refresh tokens) |

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

RFC 8628. Flow:

1. **Request device code** — POST to provider's device authorization endpoint (NOT the standard `token_url` — providers document a separate `device_authorization_url`; add this to AuthMethod::OAuth2 struct)
   - Body: `client_id=X&scope=...`
   - Response: `{device_code, user_code, verification_uri, verification_uri_complete?, expires_in, interval}`

2. **Display instructions:**
   ```
   To authorize, open:
     https://github.com/login/device

   And enter code:
     ABCD-1234

   Waiting for authorization... (expires in 15 min, polling every 5s)
   ```
   If `verification_uri_complete` provided, use it (includes code pre-filled).

3. **Poll token endpoint** every `interval` seconds:
   - POST `{token_url}` with `grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=X&client_id=Y`
   - Response: `{access_token, refresh_token?, expires_in, scope, token_type}` on success
   - Response: `{error: "authorization_pending"}` while waiting — continue polling
   - Response: `{error: "slow_down"}` — increase interval by 5s and continue
   - Response: `{error: "access_denied"}` — user denied, abort
   - Response: `{error: "expired_token"}` — device code expired, abort with retry message

4. **Timeout** at 5 minutes of wall-clock time (even if `expires_in` is longer — give up).

### Local Callback Flow

Authorization code flow with PKCE (RFC 7636). Flow:

1. **Generate PKCE challenge**:
   - `code_verifier`: 43-128 char random string (base64url unpadded)
   - `code_challenge`: `BASE64URL(SHA256(code_verifier))`
   - `state`: 32 random bytes as hex (CSRF protection)

2. **Start temporary HTTP server**:
   - Try `127.0.0.1:8400`. If port in use, bind to `127.0.0.1:0` (OS-assigned).
   - Server exposes one route: `GET /callback` → parses `code` and `state` from query string, returns HTML ("Authorization successful. You can close this window.") and shuts down the server.

3. **Build authorize URL**:
   ```
   {authorization_url}?response_type=code
     &client_id={id}
     &redirect_uri=http://127.0.0.1:{port}/callback
     &scope={space-separated scopes}
     &state={state}
     &code_challenge={challenge}
     &code_challenge_method=S256
   ```

4. **Open browser** with the URL (unless `--no-browser`). Also always print the URL.

5. **Wait for callback** — server resolves a `oneshot::Receiver<CallbackResult>`. Timeout after 5 minutes.

6. **Validate state** — compare query string `state` to generated state. Mismatch → error (CSRF).

7. **Exchange code for tokens** — POST to `token_url`:
   ```
   grant_type=authorization_code
   code={received code}
   redirect_uri=http://127.0.0.1:{port}/callback
   client_id={id}
   client_secret={secret}  (if confidential client — public clients skip this)
   code_verifier={verifier}
   ```
   Response: same shape as device flow.

8. **Shut down server** (already done by callback handler, but ensure cleanup on error paths via RAII drop).

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

If a provider supports both, prefer device code (simpler, works over SSH). User can override later via `--flow local_callback` flag (future enhancement, not in v1).

---

## 2. Runtime Token Refresh

When `IntegrationNode.execute()` resolves an OAuth2 credential from `NodeContext.oauth2_credentials`:

```
1. Check credential.is_expired() (already implemented — true if within 5-min window)
2. If NOT expired → use access_token as bearer
3. If expired:
   a. If !can_refresh() (no refresh_token):
      → Return Error::Node("OAuth2 token for '<service>' expired and cannot be refreshed.
                            Run: r8r auth login <service>")
   b. If can_refresh():
      → Acquire per-service tokio::sync::Mutex (lazy-init per service name)
      → Re-check expiry (double-checked locking — another request may have refreshed)
      → POST to token_url:
          grant_type=refresh_token
          refresh_token=<stored>
          client_id=<from credential store>
          client_secret=<from credential store>
      → On 200 OK:
          - Build new OAuth2Credential with fresh access_token, expires_at
          - If response includes new refresh_token, use it (some providers rotate)
          - If response omits refresh_token, keep the old one (RFC 6749 §6)
          - Write to CredentialStore via set_oauth2()
          - Return new access_token
      → On 400/401 (invalid_grant, refresh token revoked):
          - Delete OAuth2Credential from store
          - Return Error::Node("OAuth2 refresh failed for '<service>'. Run: r8r auth login <service>")
      → On 5xx: return Error::Node (transient, don't delete credential)
      → Release lock
```

### Mutex Registry

```rust
pub struct RefreshLockRegistry {
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl RefreshLockRegistry {
    pub fn lock_for(&self, service: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .entry(service.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}
```

The registry is a field on `IntegrationNode`, lazy-init on first use. `DashMap` for concurrent access.

---

## 3. CLI Commands

### `r8r auth login <service> [--no-browser]`

1. Load integration definition for `<service>` from `IntegrationLoader`
2. Find first `AuthMethod::OAuth2` entry (error if none)
3. Prompt for `client_id`/`client_secret` if not stored:
   - Check `CredentialStore::get("<service>_client_id")` and `<service>_client_secret`
   - If missing, prompt interactively (use `rpassword` for secret)
   - Save to credential store before proceeding
4. Select flow (device code if supported, else local callback)
5. Run flow (passing `--no-browser` only affects local callback)
6. Store `OAuth2Credential`
7. Print success: `"✓ Connected to <service> (scopes: <scopes>, expires in <duration>)"`

### `r8r auth logout <service>`

1. Load `CredentialStore`
2. If credential exists:
   - Best-effort revocation: if provider has a `revocation_url` in YAML (optional field, future), POST `token=<access_token>` to revoke. Swallow errors.
   - Remove `OAuth2Credential` from store via new `remove_oauth2()` method
3. Print: `"✓ Disconnected from <service>"` or `"<service> was not connected"`

### `r8r auth status`

1. Load `CredentialStore`, call `list_oauth2()`
2. For each credential, print a row:
   ```
   SERVICE        PROVIDER    SCOPES              EXPIRES         REFRESHABLE
   github_oauth   github      repo, read:org      in 45 minutes   yes
   slack_oauth    slack       chat:write          expired         yes
   notion_oauth   notion      (none)              never           no
   ```
3. If none: `"No OAuth2 connections. Run: r8r auth login <service>"`

---

## 4. File Layout

### Restructure

Current: `src/integrations/oauth2.rs` (single file with `OAuth2Credential`)

New:
```
src/integrations/oauth2/
├── mod.rs               # OAuth2Credential moves here (unchanged struct)
├── device_flow.rs       # DeviceCodeFlow: acquire tokens via device code
├── callback_flow.rs     # LocalCallbackFlow: acquire tokens via browser redirect
├── refresh.rs           # refresh_if_needed() + RefreshLockRegistry
└── cli.rs               # login/logout/status command handlers
```

Keep `OAuth2Credential` fully backward-compatible (same struct, same serde format).

### Existing Files Modified

- `src/integrations/definition.rs` — extend `AuthMethod::OAuth2` variant with new fields:
  - `device_authorization_url: Option<String>` (required if `flows` contains `device_code`)
  - `revocation_url: Option<String>` (optional, for logout)
- `src/integrations/node.rs` — integrate `refresh::refresh_if_needed()` into OAuth2 credential resolution
- `src/integrations/mod.rs` — update `pub mod oauth2;` still works (becomes dir-based)
- `src/main.rs` — add `Auth` subcommand to `Commands` enum, `AuthCommand` enum with `Login`/`Logout`/`Status` variants
- `src/credentials.rs` — add `remove_oauth2(service: &str)` method on `CredentialStore`
- `src/integrations/builtin/github.yaml` — add `device_authorization_url: https://github.com/login/device/code`
- `src/integrations/builtin/slack.yaml` — no device code support (remove from `flows`), keep `local_callback`
- `src/integrations/builtin/notion.yaml` — keep `local_callback` only

### New Dependencies

- `open = "5"` — cross-platform URL opener for browser auto-open
- `rpassword = "7"` — secret input prompts
- `dashmap = "6"` — concurrent HashMap for RefreshLockRegistry
- `sha2 = "0.10"` — already in dep tree likely, for PKCE S256
- `base64 = "0.22"` — already a dep, for PKCE

Dev deps:
- `wiremock = "0.6"` — mock HTTP server for testing flows

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
| Token refresh: invalid_grant | Delete credential. `"Refresh token invalid. Run: r8r auth login <service>"` |
| Token refresh: network error | Keep credential. `"Token refresh failed (network): <error>. Retry the operation."` |
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

**`device_flow.rs`** — mock token endpoint via `wiremock`:
- Successful flow: device authorization request → token request returns tokens → credential built
- `authorization_pending` polls until success
- `slow_down` increases interval
- `access_denied` aborts with correct error
- `expired_token` aborts with correct error
- Timeout at 5 minutes (use `tokio::time::pause()` to control clock)

**`callback_flow.rs`** — test independent pieces:
- PKCE generation: verifier length, challenge is SHA256 of verifier base64url-encoded
- State generation: 64 hex chars, entropy check (not sequential)
- URL building: all required params present
- Callback parsing: extract code + state from query string, reject missing state
- State mismatch: return error
- Full round-trip via `wiremock` for token exchange + loopback TCP for callback

**`refresh.rs`** — mock token endpoint:
- Fresh token (not expired) → no HTTP call, same credential returned
- Expired token with refresh_token → HTTP call → new credential stored
- Expired without refresh_token → error
- Refresh token rotation: response includes new refresh_token → stored
- Refresh token preserved: response omits refresh_token → old one kept
- invalid_grant → credential deleted
- Concurrent refresh: spawn 2 tasks, only 1 HTTP call made (mutex works)
- 5xx response → credential kept, error returned

**`cli.rs`** — smoke tests with mock CredentialStore:
- `status` with no credentials
- `status` with seeded credential (verify formatting)
- `logout` removes credential

### Integration Tests

- `tests/oauth2_cli_tests.rs` — drive `r8r auth status` / `logout` via `assert_cmd` with a temp XDG dir and seeded credential file

### Out of Scope

- Real OAuth2 flows with GitHub/Slack (manual QA — document in release notes)
- Browser auto-open behavior (platform-dependent, manual test)
- Token revocation (best-effort, tested manually)

---

## 7. Security Considerations

- **Never log tokens or secrets** — enforce via code review + grep-based test (`grep -r 'access_token.*{' src/` in CI to flag direct `Debug` prints)
- **PKCE required for callback flow** — even with confidential clients, PKCE defeats code interception
- **State parameter** — 32 bytes of CSPRNG, validated on callback
- **Localhost binding only** — callback server binds to `127.0.0.1`, never `0.0.0.0`
- **Callback server lifetime** — bound to single request (shutdown after first callback received or timeout)
- **Token refresh double-checked locking** — prevents race where concurrent refreshes invalidate each other's refresh tokens (providers rotate them on use)
- **Client secret prompts** — use `rpassword::prompt_password` to avoid echo and shell history leakage
- **XDG path for credential store** — unchanged, already `~/.local/share/r8r/credentials.json` with `0600` mode
