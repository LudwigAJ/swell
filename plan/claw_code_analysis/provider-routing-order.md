# Provider Routing Order

ClawCode routes incoming model names to one of four provider backends through a strict three-stage detection chain. The routing is deterministic and priority-ordered: explicit model prefixes take precedence over environment-variable-based fallback, which takes precedence over ambient credential sniffing. Understanding this order explains why adding a credential for one provider does not automatically redirect traffic away from another.

## Canonical evidence

- `references/claw-code/rust/crates/api/src/providers/mod.rs` — `metadata_for_model`, `detect_provider_kind`, `FOREIGN_PROVIDER_ENV_VARS`
- `references/claw-code/rust/crates/api/src/providers/openai_compat.rs` — `is_reasoning_model`, `OpenAiCompatConfig::dashscope`
- `references/claw-code/USAGE.md` — provider matrix and user-facing routing table

## The three-stage detection chain

### Stage 1 — Explicit model-prefix routing (highest priority)

`metadata_for_model()` in `providers/mod.rs` is checked first. If the resolved model name matches a known routing prefix, the correct provider and credential env vars are returned immediately, regardless of what other environment variables are present.

| Prefix | Resolved provider | Auth env var | Base URL env var | Default endpoint |
|---|---|---|---|---|
| `claude*` | `Anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` |
| `grok*` | `Xai` | `XAI_API_KEY` | `XAI_BASE_URL` | `https://api.x.ai/v1` |
| `openai/` or `gpt-*` | `OpenAi` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` |
| `qwen/` or `qwen-*` | `OpenAi` (DashScope) | `DASHSCOPE_API_KEY` | `DASHSCOPE_BASE_URL` | `https://dashscope.aliyuncs.com/compatible-mode/v1` |

The `MODEL_REGISTRY` table in `providers/mod.rs` covers bare aliases like `opus` → `claude-opus-4-6` and `grok` → `grok-3`, plus the canonical xAI family (`grok-3-mini`, `grok-2`, etc.). Model names that do not match any registered alias are passed through verbatim to the next stage.

**Builder lesson:** Prefix routing is the escape hatch for multi-provider environments. If you have `ANTHROPIC_API_KEY` set but want to use a non-Anthropic model, prefix the model name — `--model openai/gpt-4.1-mini` or `--model qwen-plus` — and the prefix wins over the ambient credential sniffer. This is intentional: credential presence is a weak signal compared to an explicit provider selector.

### Stage 2 — `OPENAI_BASE_URL` forcing for unrecognized model names

If `metadata_for_model()` returns `None` (the model name has no recognized prefix), `detect_provider_kind()` in `providers/mod.rs` checks whether `OPENAI_BASE_URL` is set. If it is AND `OPENAI_API_KEY` is present, the provider is set to `OpenAi` immediately — even for model names like `qwen2.5-coder:7b` that would otherwise default to Anthropic.

This covers the common case of local model servers (Ollama, LM Studio, vLLM) where model names are not namespaced but the base URL unambiguously identifies the transport.

**Builder lesson:** `OPENAI_BASE_URL` is a stronger signal than ambient credential sniffing. Setting it redirects all unknown model names to the OpenAI-compatible transport, which is correct for local servers that do not use provider-prefixed model names.

### Stage 3 — Ambient credential sniffing

When neither prefix routing nor `OPENAI_BASE_URL` applies, `detect_provider_kind()` scans the environment for credentials in this order:

1. `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` → `Anthropic`
2. `OPENAI_API_KEY` → `OpenAi`
3. `XAI_API_KEY` → `Xai`
4. `OPENAI_BASE_URL` alone (no API key, some local servers like Ollama accept unauthenticated requests) → `OpenAi`
5. Nothing found → `Anthropic` (hard fallback)

This order means the presence of `ANTHROPIC_API_KEY` in the environment is sufficient to route all unrecognized model names to Anthropic — which is why users with multiple credentials in the same environment frequently misroute to Anthropic when they intended to use OpenAI or DashScope.

**Builder lesson:** Sniffing-based routing is a convenience for single-provider setups. The moment you have credentials for multiple providers, you must use explicit model prefixes to override the sniffer.

## DashScope and Qwen routing nuance

Models starting with `qwen/` or `qwen-` (e.g., `qwen/qwen-max`, `qwen-plus`, `qwen-turbo`) are routed to the `OpenAi` provider kind pointed at Alibaba's DashScope compatible-mode endpoint. The routing is wired in `metadata_for_model()`:

```rust
if canonical.starts_with("qwen/") || canonical.starts_with("qwen-") {
    return Some(ProviderMetadata {
        provider: ProviderKind::OpenAi,
        auth_env: "DASHSCOPE_API_KEY",
        base_url_env: "DASHSCOPE_BASE_URL",
        default_base_url: openai_compat::DEFAULT_DASHSCOPE_BASE_URL,
    });
}
```

The `default_base_url` is `https://dashscope.aliyuncs.com/compatible-mode/v1` — DashScope speaks the OpenAI `/v1/chat/completions` REST shape, so it uses the `OpenAi` provider kind with DashScope-specific env vars.

### Reasoning model parameter stripping

DashScope-backed Qwen reasoning variants (`qwen-qwq-*`, `qwq-*`, models with `thinking` in the name) reject tuning parameters that are valid on non-reasoning models. The `is_reasoning_model()` function in `openai_compat.rs` detects these:

```rust
fn is_reasoning_model(model: &str) -> bool {
    let canonical = lowered.rsplit('/').next().unwrap_or(lowered.as_str());
    canonical.starts_with("o1") || canonical.starts_with("o3") || canonical.starts_with("o4")
        || canonical == "grok-3-mini"
        || canonical.starts_with("qwen-qwq")
        || canonical.starts_with("qwq")
        || canonical.contains("thinking")
}
```

When a reasoning model is detected, `build_chat_completion_request()` strips `temperature`, `top_p`, `frequency_penalty`, and `presence_penalty` from the outbound request before it hits the wire. This prevents DashScope (and OpenAI o-series) from rejecting the request with a parameter-validation error.

**Builder lesson:** Reasoning model detection is model-name-based, not provider-based. The same `qwen-qwq-32b` name works with and without a `/qwen/` prefix; the detection looks at the suffix. The stripping guard fires for any model whose canonical name contains `qwq` or `thinking`, regardless of transport.

## The foreign-provider hint system

When an Anthropic API call fails with missing credentials, `anthropic_missing_credentials_hint()` in `providers/mod.rs` checks whether any non-Anthropic credential env vars are present in the environment:

```rust
const FOREIGN_PROVIDER_ENV_VARS: &[(&str, &str, &str)] = &[
    (
        "OPENAI_API_KEY",
        "OpenAI-compat",
        "prefix your model name with `openai/` ... and set `OPENAI_BASE_URL` ...",
    ),
    (
        "XAI_API_KEY",
        "xAI",
        "use an xAI model alias (e.g. `--model grok`) ...",
    ),
    (
        "DASHSCOPE_API_KEY",
        "Alibaba DashScope",
        "prefix your model name with `qwen/` or `qwen-` ...",
    ),
];
```

If a foreign credential is detected, the error message for the Anthropic failure is augmented with a hint naming the detected env var and the exact prefix fix required. This eliminates the most common misrouting pattern (users with `OPENAI_API_KEY` set who forgot the `openai/` prefix).

**Builder lesson:** Error messages that describe the detected environment state and name the one-line fix are far more actionable than generic "missing credentials" walls. The hint system is the runtime expression of the same routing priority logic: it tells the user that their ambient credentials did not drive routing because a prefix was missing.

## Routing decision summary

```
model name
  │
  ├─ "claude*" ──────────────────────────────→ Anthropic (ANTHROPIC_API_KEY)
  ├─ "grok*" ─────────────────────────────────→ xAI (XAI_API_KEY)
  ├─ "openai/" or "gpt-*" ────────────────────→ OpenAI-compat (OPENAI_API_KEY)
  ├─ "qwen/" or "qwen-*" ────────────────────→ DashScope (DASHSCOPE_API_KEY)
  │
  ├─ metadata_for_model() returned None?
  │   └─ OPENAI_BASE_URL set + OPENAI_API_KEY → OpenAI-compat (OPENAI_BASE_URL)
  │
  └─ ambient credential scan:
        ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN → Anthropic
        OPENAI_API_KEY ────────────────────────→ OpenAI-compat
        XAI_API_KEY ──────────────────────────→ xAI
        OPENAI_BASE_URL alone (no key) ───────→ OpenAI-compat
        (nothing) ────────────────────────────→ Anthropic (hard fallback)
```

The prefix stage never falls through to ambient sniffing. The `OPENAI_BASE_URL` stage only activates when the model name has no recognized prefix and the variable is set. Ambient sniffing is the last resort, not a parallel option.
