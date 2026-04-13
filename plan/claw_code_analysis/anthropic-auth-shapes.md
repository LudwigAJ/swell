# Anthropic Auth Shapes: API Key vs Bearer Token

## Overview

The `claw` runtime accepts two distinct Anthropic credential environment variables that are **not interchangeable**. Each maps to a different HTTP header, a different token shape, and a different expected source. Using them incorrectly produces opaque 401 errors; understanding the distinction unlocks correct setup for direct API access, OAuth flows, and third-party proxy integrations.

## The Two Credential Forms

### `ANTHROPIC_API_KEY` → `x-api-key` header

**Shape:** `sk-ant-*` API key string
**HTTP header:** `x-api-key: sk-ant-...`
**Source:** [console.anthropic.com](https://console.anthropic.com) API key page

This is the standard direct-access credential. `sk-ant-*` keys are the classic Anthropic API keys issued from the console. They are sent over the `x-api-key` header — a custom header, not a standard Bearer scheme. This is the shape that `AnthropicClient::new(api_key: impl Into<String>)` and `AuthSource::ApiKey(...)` wrap.

### `ANTHROPIC_AUTH_TOKEN` → `Authorization: Bearer` header

**Shape:** Opaque bearer token string (often from OAuth flows or proxies)
**HTTP header:** `Authorization: Bearer ...`
**Typical source:** Anthropic-compatible proxy, OpenRouter, or an OAuth-to-Anthropic integration that mints short-lived bearer tokens

This variable is for credentials that travel as standard OAuth/Bearer tokens rather than Anthropic-specific `sk-ant-*` keys. The `AuthSource::BearerToken(...)` variant produces a `Authorization: Bearer` header and is the landing point for `OAuthTokenSet` tokens produced by the OAuth exchange flow in `anthropic.rs`.

## The `sk-ant-*` Bearer Token Pitfall

**The problem:** A `sk-ant-*` API key placed in `ANTHROPIC_AUTH_TOKEN` instead of `ANTHROPIC_API_KEY` produces a 401 "Invalid bearer token" response from Anthropic's API. This is one of the most common support contacts in the `claw` ecosystem.

**Why it fails:** Anthropic's API explicitly rejects `sk-ant-*` keys when they arrive over the `Authorization: Bearer` header. The `sk-ant-*` key shape is only accepted on the `x-api-key` custom header. There is no overlap — a `sk-ant-*` key sent as a Bearer token is treated as invalid.

**The fix is a one-line env var swap:** move the key from `ANTHROPIC_AUTH_TOKEN` to `ANTHROPIC_API_KEY`.

**Automated detection:** The `claw` runtime detects this exact failure mode in `enrich_bearer_auth_error` (`references/claw-code/rust/crates/api/src/providers/anthropic.rs`). When a raw 401 is received and the `AuthSource` is pure `BearerToken` carrying a `sk-ant-*` prefix, the error message is enriched with:

```
hint: sk-ant-* keys go in ANTHROPIC_API_KEY (x-api-key header),
not ANTHROPIC_AUTH_TOKEN (Bearer header). Move your key to ANTHROPIC_API_KEY.
```

The enrichment only fires when the `AuthSource` has **no** `api_key` component — meaning the `x-api-key` header is not already being sent. When both are present (`ApiKeyAndBearer`), the 401 is forwarded unchanged because the `x-api-key` header should be valid on its own and the 401 has a different cause.

```rust
// Only append the hint when the AuthSource is pure BearerToken.
// If both api_key and bearer_token are present, the x-api-key header
// is already being sent and the 401 comes from a different cause.
if auth.api_key().is_some() {
    return ApiError::Api { ... }; // pass through, do not add misleading hint
}
```

This behavior is verified by the test `enrich_bearer_auth_error_skips_hint_when_api_key_header_is_also_present`.

## The `AuthSource` Enum and Credential Combination

`AuthSource` in `anthropic.rs` models all four valid credential states:

```rust
pub enum AuthSource {
    None,
    ApiKey(String),                      // Only x-api-key
    BearerToken(String),                 // Only Authorization: Bearer
    ApiKeyAndBearer { api_key: String, bearer_token: String }, // Both
}
```

The `apply` method fans these out to separate HTTP headers:

```rust
pub fn apply(&self, mut request_builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(api_key) = self.api_key() {
        request_builder = request_builder.header("x-api-key", api_key);
    }
    if let Some(token) = self.bearer_token() {
        request_builder = request_builder.bearer_auth(token);
    }
    request_builder
}
```

When both credentials are present (`ApiKeyAndBearer`), the request carries **both** headers. The `x-api-key` is Anthropic's primary auth signal; the Bearer token is consumed by a proxy or middleware in front of Anthropic.

The credential resolution order during startup is:

1. If `ANTHROPIC_API_KEY` is set → start with `ApiKey`
2. If `ANTHROPIC_AUTH_TOKEN` is also set → upgrade to `ApiKeyAndBearer`
3. If `ANTHROPIC_API_KEY` is absent but `ANTHROPIC_AUTH_TOKEN` is set → `BearerToken`
4. If neither is set → `MissingCredentials` error (with foreign-provider hint if `OPENAI_API_KEY`, `XAI_API_KEY`, or `DASHSCOPE_API_KEY` is detected)

## OAuth Token Flow

The OAuth flow in `anthropic.rs` is separate from the direct API key path. When a user completes an OAuth authorization flow, the resulting `OAuthTokenSet` is converted into `AuthSource::BearerToken(access_token)`:

```rust
impl From<OAuthTokenSet> for AuthSource {
    fn from(value: OAuthTokenSet) -> Self {
        Self::BearerToken(value.access_token)
    }
}
```

OAuth tokens are stored and refreshed using `runtime::load_oauth_credentials` / `runtime::save_oauth_credentials`, with automatic refresh when `expires_at` passes the current Unix timestamp. The `resolve_saved_oauth_token` function handles expiry detection and token refresh, then persists the new token set to disk.

The `resolve_startup_auth_source` function reads credentials in the following priority order for the running process, without triggering OAuth config loading until env vars are exhausted.

## Provider Routing Interaction

The auth shape also interacts with provider detection in `providers/mod.rs`. The `has_auth_from_env_or_saved()` check at the bottom of the `detect_provider_kind` chain detects whether any `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` is present to route toward the `Anthropic` provider kind as a fallback. However, explicit model-prefix routing (e.g., `openai/`, `grok`, `qwen/`) takes precedence over the credential-sniffer order.

When `OPENAI_BASE_URL` is set without a recognized model prefix, the runtime routes to OpenAI-compatible even if Anthropic credentials are present — a deliberate design to support local model backends with ambiguous model names.

## Builder Lessons

1. **Custom headers vs Bearer headers are not equivalent.** `x-api-key` and `Authorization: Bearer` are distinct protocols. An API key that works on one will fail on the other. This is an upstream API constraint, not a `claw` invention.

2. **Error enrichment requires specific detection.** A bare 401 "Invalid bearer token" is ambiguous — it could mean a bad token, a revoked token, or a token routed to the wrong header. The `sk-ant-*` prefix detection transforms a generic failure into an actionable one-liner.

3. **Credential enum states are composable.** Modeling all four combinations explicitly (`None`, `ApiKey`, `BearerToken`, `ApiKeyAndBearer`) eliminates ambiguous fallback paths. Each combination is handled by `apply` and `enrich_bearer_auth_error` with different behavior.

4. **OAuth tokens are just Bearer tokens with a lifecycle.** The `OAuthTokenSet` → `AuthSource::BearerToken` conversion means the Bearer-token path already handles the OAuth case — the only difference is that the token has an expiry and a refresh mechanism managed by the runtime.

## Repo Evidence

| Claim | Evidence |
|---|---|
| `ANTHROPIC_API_KEY` → `x-api-key` header | `AuthSource::apply` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| `ANTHROPIC_AUTH_TOKEN` → `Authorization: Bearer` header | `AuthSource::apply` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| `sk-ant-*` key rejected over Bearer header | `SK_ANT_BEARER_HINT` constant and `enrich_bearer_auth_error` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| `OAuthTokenSet` maps to `BearerToken` | `impl From<OAuthTokenSet> for AuthSource` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| Credential resolution order | `AuthSource::from_env_or_saved`, `resolve_startup_auth_source` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| 4-state `AuthSource` enum | `enum AuthSource` definition in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| Hint suppressed when `x-api-key` also present | Test `enrich_bearer_auth_error_skips_hint_when_api_key_header_is_also_present` in `references/claw-code/rust/crates/api/src/providers/anthropic.rs` |
| Provider routing vs credential sniffing | `detect_provider_kind` in `references/claw-code/rust/crates/api/src/providers/mod.rs` |
| Auth env var mapping table | `references/claw-code/USAGE.md` ("Which env var goes where" table) |