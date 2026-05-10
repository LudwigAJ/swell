//! Typed schema for the `llm` section of `settings.json` and the
//! `${VAR}` environment-variable substitution that turns a stored
//! reference like `"${MINIMAX_API_KEY}"` into the real key at
//! daemon-startup time.
//!
//! The shape, per repo convention:
//!
//! ```json
//! {
//!   "llm": {
//!     "default": "minimax-m2.7",
//!     "models": {
//!       "minimax-m2.7": {
//!         "backend": "anthropic",
//!         "model": "MiniMax-M2.7",
//!         "base_url": "https://api.minimax.io/anthropic",
//!         "env": { "API_KEY": "${MINIMAX_API_KEY}" }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! `backend` selects the wire protocol Swell speaks (we only speak
//! `anthropic` and `openai` today). `env.API_KEY` is required; future
//! per-backend keys (`ORG_ID`, custom headers) live in the same bag
//! without needing a schema bump.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Wire protocol used to talk to a model.
///
/// `Anthropic` is the only fully-supported backend today; it covers
/// every Anthropic-compatible gateway via the per-provider capability
/// framework (see `swell-llm::providers`).
///
/// `Openai` is **parked**: the legacy hand-rolled OpenAI client in
/// `swell-llm/src/openai.rs` is no longer wired into the daemon. We are
/// waiting on a community OpenAI Rust SDK before re-enabling it.
/// Configurations that set `backend = "openai"` are accepted by the
/// parser (so `.swell` files don't break) but the daemon refuses to
/// construct an OpenAI backend and falls back to `MockLlm` with a
/// loud warning. To use OpenAI-shaped models today, route them through
/// an Anthropic-compatible gateway and configure them as `anthropic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmBackendKind {
    Anthropic,
    Openai,
}

/// Sampling overrides expressed as a bag under `params`. Every field is
/// optional; unset fields fall through to the agent's effort-based
/// defaults at request time. `top_k` is Anthropic-only; the OpenAI
/// backend ignores it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// Default context window applied when a profile omits `context_window`.
/// Compaction triggers compare running token usage to this value.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;

/// Per-field capability override for an Anthropic-compatible provider.
///
/// Each field is `Option<bool>`. `Some(v)` overrides the built-in
/// provider profile; `None` leaves the built-in default in place. This
/// lets users opt a self-hosted or unrecognised gateway into (or out
/// of) features without us shipping code for every new endpoint.
///
/// Mapped onto `swell_llm::ProviderCaps` at backend construction time
/// — see `ProviderCaps::with_override`. Field names mirror
/// `ProviderCaps` so the mapping stays mechanical.
///
/// Example `.swell`:
/// ```toml
/// [llm.models.my-gateway]
/// backend = "anthropic"
/// model   = "some-model"
/// base_url = "https://my-gateway.example.com/v1"
/// provider = "custom"            # or omit to inherit URL detection
///
/// [llm.models.my-gateway.caps]
/// supports_top_k         = true
/// supports_cache_control = false
/// supports_thinking      = true
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_top_k: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_stop_sequences: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_cache_control: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tool_use: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp_temperature_unit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_models_listing: Option<bool>,
}

impl ProviderCapsOverride {
    /// True when no override is set — the backend will use the
    /// provider's built-in caps unchanged.
    pub fn is_empty(&self) -> bool {
        self.supports_top_k.is_none()
            && self.supports_stop_sequences.is_none()
            && self.supports_cache_control.is_none()
            && self.supports_thinking.is_none()
            && self.supports_tool_use.is_none()
            && self.clamp_temperature_unit.is_none()
            && self.supports_models_listing.is_none()
    }
}

/// One entry under `llm.models`. The map key is the user-chosen alias.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub backend: LlmBackendKind,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Optional explicit Anthropic-compatible provider profile name
    /// (e.g. `"minimax"`, `"moonshot"`, `"kimi"`, `"zai"`, `"deepseek"`,
    /// `"qwen"`, `"openrouter"`). Drives capability gating in the
    /// Anthropic backend. When unset, the backend falls back to URL
    /// substring detection. Ignored by non-Anthropic backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Per-field capability overrides. Applied on top of the built-in
    /// provider profile so users can adjust an unknown gateway without
    /// shipping code. See [`ProviderCapsOverride`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<ProviderCapsOverride>,
    /// String/string bag. Values may be literal or `${VAR}` references
    /// resolved against process env. `API_KEY` is required.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Per-model sampling overrides. MiniMax constrains `temperature`
    /// to (0.0, 1.0]; values outside that range are rejected by the API.
    #[serde(default)]
    pub params: SamplingParams,
    /// Total context window in tokens. Used to trigger compaction
    /// before the model rejects a request. Falls back to
    /// `DEFAULT_CONTEXT_WINDOW` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

/// The whole `llm` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Alias of the default profile to use when none is specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, ModelProfile>,
}

#[derive(Debug, Error)]
pub enum LlmConfigError {
    #[error("llm.models is empty; no profile to resolve")]
    NoModels,
    #[error("llm.default = {0:?} is not present in llm.models")]
    DefaultNotFound(String),
    #[error("llm.default is unset and llm.models has more than one entry; pick one explicitly")]
    AmbiguousDefault,
    #[error("model profile {alias:?} references env var {var:?} which is not set in the process environment")]
    UnresolvedVar { alias: String, var: String },
    #[error("model profile {alias:?} is missing required env entry {key:?}")]
    MissingEnvKey { alias: String, key: String },
    #[error("malformed ${{VAR}} reference in profile {alias:?} key {key:?}: {raw:?}")]
    MalformedVar {
        alias: String,
        key: String,
        raw: String,
    },
}

/// A `ModelProfile` with all `${VAR}` references resolved against the
/// process environment. This is what backends actually consume.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub alias: String,
    pub backend: LlmBackendKind,
    pub model: String,
    pub base_url: Option<String>,
    /// Explicit provider name as written in settings; passed to the
    /// Anthropic backend for capability gating. `None` means "infer".
    pub provider: Option<String>,
    /// Per-field capability overrides as written in settings. Empty
    /// when the user provided none.
    pub caps: ProviderCapsOverride,
    pub api_key: String,
    /// Remaining `env` entries beyond `API_KEY`, post-substitution.
    /// Reserved for backend-specific knobs (`ORG_ID`, custom headers).
    pub extra_env: HashMap<String, String>,
    pub params: SamplingParams,
    /// Always resolved (defaults to `DEFAULT_CONTEXT_WINDOW`), so
    /// downstream compaction logic never has to handle `None`.
    pub context_window: u32,
}

impl LlmConfig {
    /// Pick the named profile (or `default` if `name` is None), then
    /// resolve every `env` value through process-env substitution.
    pub fn resolve(&self, name: Option<&str>) -> Result<ResolvedProfile, LlmConfigError> {
        if self.models.is_empty() {
            return Err(LlmConfigError::NoModels);
        }

        let alias = match name {
            Some(n) => n.to_string(),
            None => match self.default.clone() {
                Some(d) => d,
                None if self.models.len() == 1 => self.models.keys().next().cloned().unwrap(),
                None => return Err(LlmConfigError::AmbiguousDefault),
            },
        };

        let profile = self
            .models
            .get(&alias)
            .ok_or_else(|| LlmConfigError::DefaultNotFound(alias.clone()))?;

        let mut resolved_env = HashMap::with_capacity(profile.env.len());
        for (k, v) in &profile.env {
            let resolved = substitute(&alias, k, v)?;
            resolved_env.insert(k.clone(), resolved);
        }

        let api_key =
            resolved_env
                .remove("API_KEY")
                .ok_or_else(|| LlmConfigError::MissingEnvKey {
                    alias: alias.clone(),
                    key: "API_KEY".to_string(),
                })?;

        Ok(ResolvedProfile {
            alias,
            backend: profile.backend,
            model: profile.model.clone(),
            base_url: profile.base_url.clone(),
            provider: profile.provider.clone(),
            caps: profile.caps.clone().unwrap_or_default(),
            api_key,
            extra_env: resolved_env,
            params: profile.params.clone(),
            context_window: profile.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
        })
    }
}

/// Resolve a single value: `${VAR}` looks up `VAR` in process env;
/// any other string is taken literally. We deliberately do NOT support
/// partial substitution like `"prefix-${VAR}-suffix"` — secrets are
/// either env-referenced or literal, never spliced.
fn substitute(alias: &str, key: &str, raw: &str) -> Result<String, LlmConfigError> {
    let trimmed = raw.trim();
    if !trimmed.starts_with("${") {
        return Ok(raw.to_string());
    }
    if !trimmed.ends_with('}') {
        return Err(LlmConfigError::MalformedVar {
            alias: alias.to_string(),
            key: key.to_string(),
            raw: raw.to_string(),
        });
    }
    let var_name = &trimmed[2..trimmed.len() - 1];
    if var_name.is_empty() || var_name.contains(char::is_whitespace) {
        return Err(LlmConfigError::MalformedVar {
            alias: alias.to_string(),
            key: key.to_string(),
            raw: raw.to_string(),
        });
    }
    std::env::var(var_name).map_err(|_| LlmConfigError::UnresolvedVar {
        alias: alias.to_string(),
        var: var_name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: &str) -> LlmConfig {
        serde_json::from_str(json).expect("parse")
    }

    #[test]
    fn resolves_default_with_literal_api_key() {
        let c = cfg(r#"{
            "default": "x",
            "models": {
                "x": {
                    "backend": "anthropic",
                    "model": "Claude",
                    "env": { "API_KEY": "literal-key" }
                }
            }
        }"#);
        let r = c.resolve(None).unwrap();
        assert_eq!(r.api_key, "literal-key");
        assert_eq!(r.backend, LlmBackendKind::Anthropic);
        assert_eq!(r.alias, "x");
    }

    #[test]
    fn resolves_var_reference() {
        std::env::set_var("LLM_CONFIG_TEST_KEY", "from-env");
        let c = cfg(r#"{
            "default": "x",
            "models": {
                "x": {
                    "backend": "openai",
                    "model": "gpt",
                    "base_url": "https://example.com",
                    "env": { "API_KEY": "${LLM_CONFIG_TEST_KEY}" }
                }
            }
        }"#);
        let r = c.resolve(None).unwrap();
        assert_eq!(r.api_key, "from-env");
        assert_eq!(r.base_url.as_deref(), Some("https://example.com"));
        std::env::remove_var("LLM_CONFIG_TEST_KEY");
    }

    #[test]
    fn unresolved_var_is_an_error() {
        std::env::remove_var("DEFINITELY_NOT_SET_LLM_CFG");
        let c = cfg(r#"{
            "default": "x",
            "models": {
                "x": {
                    "backend": "anthropic",
                    "model": "m",
                    "env": { "API_KEY": "${DEFINITELY_NOT_SET_LLM_CFG}" }
                }
            }
        }"#);
        match c.resolve(None) {
            Err(LlmConfigError::UnresolvedVar { var, .. }) => {
                assert_eq!(var, "DEFINITELY_NOT_SET_LLM_CFG")
            }
            other => panic!("expected UnresolvedVar, got {other:?}"),
        }
    }

    #[test]
    fn missing_api_key_is_an_error() {
        let c = cfg(r#"{
            "default": "x",
            "models": {
                "x": { "backend": "anthropic", "model": "m", "env": {} }
            }
        }"#);
        match c.resolve(None) {
            Err(LlmConfigError::MissingEnvKey { key, .. }) => assert_eq!(key, "API_KEY"),
            other => panic!("expected MissingEnvKey, got {other:?}"),
        }
    }

    #[test]
    fn ambiguous_default_when_multiple_and_no_default() {
        let c = cfg(r#"{
            "models": {
                "a": { "backend": "anthropic", "model": "m", "env": { "API_KEY": "k" } },
                "b": { "backend": "openai",    "model": "m", "env": { "API_KEY": "k" } }
            }
        }"#);
        assert!(matches!(
            c.resolve(None),
            Err(LlmConfigError::AmbiguousDefault)
        ));
    }

    #[test]
    fn explicit_name_overrides_default() {
        std::env::set_var("LLM_CFG_K", "k");
        let c = cfg(r#"{
            "default": "a",
            "models": {
                "a": { "backend": "anthropic", "model": "ma", "env": { "API_KEY": "${LLM_CFG_K}" } },
                "b": { "backend": "openai",    "model": "mb", "env": { "API_KEY": "${LLM_CFG_K}" } }
            }
        }"#);
        assert_eq!(c.resolve(Some("b")).unwrap().alias, "b");
        std::env::remove_var("LLM_CFG_K");
    }

    #[test]
    fn malformed_var_reference_is_an_error() {
        let c = cfg(r#"{
            "default": "x",
            "models": {
                "x": { "backend": "anthropic", "model": "m", "env": { "API_KEY": "${ }" } }
            }
        }"#);
        assert!(matches!(
            c.resolve(None),
            Err(LlmConfigError::MalformedVar { .. })
        ));
    }
}
