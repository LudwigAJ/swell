//! Capability profiles for Anthropic-compatible providers.
//!
//! The Anthropic message API has become a *de facto* wire format adopted
//! by several non-Anthropic vendors (MiniMax, Moonshot/Kimi, Z.ai/GLM,
//! DeepSeek, Qwen/DashScope, OpenRouter, …). Each gateway accepts the
//! same JSON shape but supports a subset of the request fields and
//! constrains some sampling ranges.
//!
//! `AnthropicBackend` consults a [`ProviderCaps`] profile when building
//! requests so we don't send fields the upstream silently drops or
//! rejects. The profile is selected by [`AnthropicProvider`], which is
//! either pinned explicitly via `.swell` settings (`llm.models.<alias>.provider`)
//! or auto-detected from `base_url` as a fallback.
//!
//! ## Adding a new provider
//!
//! 1. Add a variant to [`AnthropicProvider`].
//! 2. Add a row to [`AnthropicProvider::caps`] with the upstream's known
//!    constraints. Mark anything you haven't verified with a `TODO` and
//!    pick conservative defaults — it's better to drop a feature than to
//!    have the upstream reject the whole request.
//! 3. Add a substring to [`AnthropicProvider::detect`] so URL-only
//!    configs route correctly.
//! 4. Add a token to [`AnthropicProvider::from_settings_name`] so users
//!    can pin it explicitly (this is the durable knob — URL detection is
//!    only a fallback).

use serde::{Deserialize, Serialize};

/// Which Anthropic-shaped endpoint we're talking to.
///
/// The serialized name (lowercase variant) is what users put in
/// `llm.models.<alias>.provider` in `.swell` settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnthropicProvider {
    /// Anthropic's own API. Full feature surface.
    #[default]
    Anthropic,
    /// MiniMax's Anthropic-compatible endpoint. Verified: ignores
    /// `top_k`, `stop_sequences`, `cache_control`, `mcp_servers`,
    /// `container`, `context_management`. Clamps `temperature` to
    /// (0, 1]. Image/document content blocks are unsupported.
    MiniMax,
    /// Moonshot's `api.moonshot.ai/anthropic` endpoint (Kimi K2 etc.).
    /// TODO: verify exact capability set; conservative defaults applied.
    Moonshot,
    /// Z.ai / Zhipu's GLM Anthropic-compatible endpoint
    /// (`api.z.ai/api/anthropic`). TODO: verify.
    Zai,
    /// DeepSeek's Anthropic-compatible endpoint. TODO: verify.
    DeepSeek,
    /// Alibaba DashScope / Qwen Anthropic-compatible endpoint. TODO: verify.
    Qwen,
    /// OpenRouter's Anthropic-format passthrough. Behaviour depends on the
    /// underlying model; OpenRouter itself imposes no extra constraints.
    /// TODO: confirm for tool-use + cache_control routing.
    OpenRouter,
    /// Unknown / self-hosted / future gateway. Defaults to the same
    /// surface as `Anthropic` (i.e. send everything) with a startup log
    /// nudging the user to pin a real provider. Override the caps by
    /// pinning an explicit provider in `.swell`.
    Custom,
}

impl AnthropicProvider {
    /// Capability flags for this provider. Used by `build_params` to
    /// filter out fields the upstream would silently drop or reject.
    pub fn caps(&self) -> ProviderCaps {
        match self {
            // Verified profiles ----------------------------------------
            Self::Anthropic => ProviderCaps::FULL,
            Self::MiniMax => ProviderCaps {
                supports_top_k: false,
                supports_stop_sequences: false,
                supports_cache_control: false,
                supports_thinking: true,
                supports_tool_use: true,
                clamp_temperature_unit: true,
                // MiniMax's `/v1/models` returns `{"data": null}` which the
                // SDK can't deserialize. Skip the listing entirely.
                supports_models_listing: false,
            },

            // Stub profiles — conservative defaults until verified. ----
            // The shape is "what we're confident about, plus a TODO for
            // anything that needs a docs read or a live probe."
            Self::Moonshot => ProviderCaps {
                // TODO(provider:moonshot): verify cache_control,
                // thinking, top_k. Conservative defaults below.
                supports_top_k: false,
                supports_stop_sequences: true,
                supports_cache_control: true,
                supports_thinking: false,
                supports_tool_use: true,
                clamp_temperature_unit: false,
                supports_models_listing: true,
            },
            Self::Zai => ProviderCaps {
                // TODO(provider:zai): verify against api.z.ai/api/anthropic.
                supports_top_k: false,
                supports_stop_sequences: true,
                supports_cache_control: false,
                supports_thinking: true, // GLM-4.6 advertises thinking
                supports_tool_use: true,
                clamp_temperature_unit: false,
                supports_models_listing: true,
            },
            Self::DeepSeek => ProviderCaps {
                // TODO(provider:deepseek): verify.
                supports_top_k: false,
                supports_stop_sequences: true,
                supports_cache_control: false,
                supports_thinking: true,
                supports_tool_use: true,
                clamp_temperature_unit: false,
                supports_models_listing: true,
            },
            Self::Qwen => ProviderCaps {
                // TODO(provider:qwen): verify against DashScope.
                supports_top_k: true,
                supports_stop_sequences: true,
                supports_cache_control: false,
                supports_thinking: true,
                supports_tool_use: true,
                clamp_temperature_unit: false,
                supports_models_listing: true,
            },
            Self::OpenRouter => ProviderCaps {
                // TODO(provider:openrouter): caps depend on the routed
                // model; assume tool_use yes, cache_control no.
                supports_top_k: true,
                supports_stop_sequences: true,
                supports_cache_control: false,
                supports_thinking: true,
                supports_tool_use: true,
                clamp_temperature_unit: false,
                supports_models_listing: true,
            },

            // Unknown gateway — full surface, log a nudge at startup.
            Self::Custom => ProviderCaps::FULL,
        }
    }

    /// Best-effort detection from a base URL. Used only as a fallback
    /// when settings don't pin a provider explicitly.
    pub fn detect(base_url: Option<&str>) -> Self {
        let Some(url) = base_url else {
            return Self::Anthropic;
        };
        let u = url.to_ascii_lowercase();
        if u.contains("minimax") {
            Self::MiniMax
        } else if u.contains("moonshot") {
            Self::Moonshot
        } else if u.contains("z.ai") || u.contains("bigmodel") {
            Self::Zai
        } else if u.contains("deepseek") {
            Self::DeepSeek
        } else if u.contains("dashscope") || u.contains("aliyuncs") {
            Self::Qwen
        } else if u.contains("openrouter") {
            Self::OpenRouter
        } else if u.contains("anthropic") {
            Self::Anthropic
        } else {
            Self::Custom
        }
    }

    /// Parse an explicit provider name as written in `.swell` settings.
    /// Case-insensitive, matches the serde lowercase form. Returns
    /// `None` for unknown tokens so the caller can decide whether to
    /// fall back to URL detection or surface an error.
    pub fn from_settings_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "anthropic" => Some(Self::Anthropic),
            "minimax" => Some(Self::MiniMax),
            "moonshot" | "kimi" => Some(Self::Moonshot),
            "zai" | "z.ai" | "zhipu" | "glm" => Some(Self::Zai),
            "deepseek" => Some(Self::DeepSeek),
            "qwen" | "dashscope" => Some(Self::Qwen),
            "openrouter" => Some(Self::OpenRouter),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    /// Human-readable name for logs. Stable; safe to grep.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::MiniMax => "minimax",
            Self::Moonshot => "moonshot",
            Self::Zai => "zai",
            Self::DeepSeek => "deepseek",
            Self::Qwen => "qwen",
            Self::OpenRouter => "openrouter",
            Self::Custom => "custom",
        }
    }
}

/// Per-provider capability flags consulted when building a request.
///
/// Add a field here when you discover a request-shape difference that
/// the SDK happily serializes but the upstream silently drops (or
/// rejects). Each field defaults to the Anthropic-native answer; new
/// providers opt out.
#[derive(Debug, Clone, Copy)]
pub struct ProviderCaps {
    /// Forward `top_k` in the request.
    pub supports_top_k: bool,
    /// Forward `stop_sequences` in the request.
    pub supports_stop_sequences: bool,
    /// Wrap the system prompt in `cache_control` so the upstream caches
    /// it across calls. False ⇒ send a plain text block instead.
    pub supports_cache_control: bool,
    /// Forward `thinking_enabled(budget)` (extended thinking).
    pub supports_thinking: bool,
    /// Forward tool definitions and process tool_use / tool_result
    /// content blocks. Currently every known gateway supports this; the
    /// flag exists for future read-only completion-style endpoints.
    pub supports_tool_use: bool,
    /// Clamp `temperature` to (0, 1] before sending. MiniMax rejects
    /// values outside this range.
    pub clamp_temperature_unit: bool,
    /// Whether `GET /v1/models` returns a usable list. MiniMax currently
    /// returns `{"data": null}` which the SDK can't deserialize; we skip
    /// the call entirely and log a clearer reason.
    pub supports_models_listing: bool,
}

impl ProviderCaps {
    /// The Anthropic-native baseline: every feature on, no clamps.
    /// Other profiles spread from this with `..ProviderCaps::FULL` and
    /// turn fields off.
    pub const FULL: Self = Self {
        supports_top_k: true,
        supports_stop_sequences: true,
        supports_cache_control: true,
        supports_thinking: true,
        supports_tool_use: true,
        clamp_temperature_unit: false,
        supports_models_listing: true,
    };

    /// Apply per-field overrides from `.swell` settings on top of this
    /// caps profile. `Some(v)` replaces the field; `None` leaves it
    /// alone. This is how users adjust an unknown gateway without
    /// shipping a new variant — see
    /// [`swell_core::llm_config::ProviderCapsOverride`].
    pub fn with_override(mut self, ov: &swell_core::llm_config::ProviderCapsOverride) -> Self {
        if let Some(v) = ov.supports_top_k {
            self.supports_top_k = v;
        }
        if let Some(v) = ov.supports_stop_sequences {
            self.supports_stop_sequences = v;
        }
        if let Some(v) = ov.supports_cache_control {
            self.supports_cache_control = v;
        }
        if let Some(v) = ov.supports_thinking {
            self.supports_thinking = v;
        }
        if let Some(v) = ov.supports_tool_use {
            self.supports_tool_use = v;
        }
        if let Some(v) = ov.clamp_temperature_unit {
            self.clamp_temperature_unit = v;
        }
        if let Some(v) = ov.supports_models_listing {
            self.supports_models_listing = v;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_minimax_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://api.minimaxi.com/anthropic")),
            AnthropicProvider::MiniMax
        );
    }

    #[test]
    fn detects_moonshot_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://api.moonshot.ai/anthropic")),
            AnthropicProvider::Moonshot
        );
    }

    #[test]
    fn detects_zai_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://api.z.ai/api/anthropic")),
            AnthropicProvider::Zai
        );
        assert_eq!(
            AnthropicProvider::detect(Some("https://open.bigmodel.cn/api/paas/v4/anthropic")),
            AnthropicProvider::Zai
        );
    }

    #[test]
    fn detects_deepseek_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://api.deepseek.com/anthropic")),
            AnthropicProvider::DeepSeek
        );
    }

    #[test]
    fn detects_qwen_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some(
                "https://dashscope-intl.aliyuncs.com/api/v2/apps/x/anthropic"
            )),
            AnthropicProvider::Qwen
        );
    }

    #[test]
    fn detects_openrouter_from_url() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://openrouter.ai/api/v1")),
            AnthropicProvider::OpenRouter
        );
    }

    #[test]
    fn unknown_url_falls_back_to_custom() {
        assert_eq!(
            AnthropicProvider::detect(Some("https://example.com/v1")),
            AnthropicProvider::Custom
        );
    }

    #[test]
    fn no_url_defaults_to_anthropic() {
        assert_eq!(
            AnthropicProvider::detect(None),
            AnthropicProvider::Anthropic
        );
    }

    #[test]
    fn settings_name_aliases() {
        assert_eq!(
            AnthropicProvider::from_settings_name("kimi"),
            Some(AnthropicProvider::Moonshot)
        );
        assert_eq!(
            AnthropicProvider::from_settings_name("GLM"),
            Some(AnthropicProvider::Zai)
        );
        assert_eq!(
            AnthropicProvider::from_settings_name("Anthropic"),
            Some(AnthropicProvider::Anthropic)
        );
        assert_eq!(AnthropicProvider::from_settings_name("nope"), None);
    }

    #[test]
    fn anthropic_caps_are_full() {
        let c = AnthropicProvider::Anthropic.caps();
        assert!(c.supports_top_k);
        assert!(c.supports_stop_sequences);
        assert!(c.supports_cache_control);
        assert!(c.supports_thinking);
        assert!(!c.clamp_temperature_unit);
    }

    #[test]
    fn override_replaces_only_set_fields() {
        let base = AnthropicProvider::Custom.caps();
        assert!(base.supports_top_k);
        let ov = swell_core::llm_config::ProviderCapsOverride {
            supports_top_k: Some(false),
            supports_thinking: Some(false),
            ..Default::default()
        };
        let merged = base.with_override(&ov);
        assert!(!merged.supports_top_k);
        assert!(!merged.supports_thinking);
        // Untouched fields keep their base value.
        assert_eq!(merged.supports_cache_control, base.supports_cache_control);
        assert_eq!(merged.supports_tool_use, base.supports_tool_use);
    }

    #[test]
    fn empty_override_is_a_noop() {
        let ov = swell_core::llm_config::ProviderCapsOverride::default();
        assert!(ov.is_empty());
        let base = AnthropicProvider::MiniMax.caps();
        let merged = base.with_override(&ov);
        assert_eq!(merged.supports_top_k, base.supports_top_k);
        assert_eq!(merged.clamp_temperature_unit, base.clamp_temperature_unit);
    }

    #[test]
    fn minimax_caps_drop_unsupported() {
        let c = AnthropicProvider::MiniMax.caps();
        assert!(!c.supports_top_k);
        assert!(!c.supports_stop_sequences);
        assert!(!c.supports_cache_control);
        assert!(c.clamp_temperature_unit);
    }
}
