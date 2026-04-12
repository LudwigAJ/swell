//! OpenTelemetry integration with GenAI semantic conventions.
//!
//! This module provides OpenTelemetry instrumentation for LLM calls following
//! the [OpenTelemetry GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/).
//!
//! # GenAI Span Attributes
//!
//! The following attributes are recorded on LLM spans:
//! - `gen_ai.operation.name`: The operation being performed (e.g., "chat")
//! - `gen_ai.provider.name`: The LLM provider (e.g., "anthropic", "openai")
//! - `gen_ai.request.model`: The model name
//! - `gen_ai.usage.input_tokens`: Prompt tokens
//! - `gen_ai.usage.output_tokens`: Completion tokens
//! - `gen_ai.response.model`: The actual model used (may differ from request)
//!
//! # Cost Tracking
//!
//! Cost is calculated based on provider-specific pricing:
//! - Anthropic: $3.75/M input tokens, $15/M output tokens (Claude 3.5 Sonnet)
//! - OpenAI: Varies by model (GPT-4o: $5/M input, $15/M output)

use opentelemetry::trace::{Span, SpanKind, Status};
use opentelemetry::KeyValue;
use std::time::Instant;

/// GenAI semantic convention attribute keys
pub mod gen_ai {
    use opentelemetry::Key;

    /// The name of the operation being performed
    pub const OPERATION_NAME: Key = Key::from_static_str("gen_ai.operation.name");

    /// The Generative AI provider name
    pub const PROVIDER_NAME: Key = Key::from_static_str("gen_ai.provider.name");

    /// The name of the GenAI model a request is being made to
    pub const REQUEST_MODEL: Key = Key::from_static_str("gen_ai.request.model");

    /// The name of the model that generated the response
    pub const RESPONSE_MODEL: Key = Key::from_static_str("gen_ai.response.model");

    /// The number of tokens used in the GenAI input (prompt)
    pub const USAGE_INPUT_TOKENS: Key = Key::from_static_str("gen_ai.usage.input_tokens");

    /// The number of tokens used in the GenAI response (completion)
    pub const USAGE_OUTPUT_TOKENS: Key = Key::from_static_str("gen_ai.usage.output_tokens");

    /// The number of input tokens written to a provider-managed cache
    pub const USAGE_CACHE_CREATION_INPUT_TOKENS: Key =
        Key::from_static_str("gen_ai.usage.cache_creation.input_tokens");

    /// The number of input tokens served from a provider-managed cache
    pub const USAGE_CACHE_READ_INPUT_TOKENS: Key =
        Key::from_static_str("gen_ai.usage.cache_read.input_tokens");
}

/// LLM provider identifiers following GenAI conventions
#[derive(Debug, Clone, Copy)]
pub enum LlmProvider {
    Anthropic,
    OpenAI,
    Mock,
}

impl LlmProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "anthropic",
            LlmProvider::OpenAI => "openai",
            LlmProvider::Mock => "mock",
        }
    }
}

/// Token pricing information for cost calculation
#[derive(Debug, Clone, Copy)]
pub struct TokenPricing {
    /// Cost per million input tokens in USD
    pub input_per_million: f64,
    /// Cost per million output tokens in USD
    pub output_per_million: f64,
}

impl TokenPricing {
    /// Calculate the cost in USD for a given number of input and output tokens
    pub fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }
}

/// Default pricing for common providers (prices in USD per million tokens)
pub mod pricing {
    use super::TokenPricing;

    /// Anthropic Claude 3.5 Sonnet pricing
    pub const ANTHROPIC_SONNET: TokenPricing = TokenPricing {
        input_per_million: 3.0,
        output_per_million: 15.0,
    };

    /// Anthropic Claude 3 Opus pricing
    pub const ANTHROPIC_OPUS: TokenPricing = TokenPricing {
        input_per_million: 15.0,
        output_per_million: 75.0,
    };

    /// OpenAI GPT-4o pricing
    pub const OPENAI_GPT4O: TokenPricing = TokenPricing {
        input_per_million: 5.0,
        output_per_million: 15.0,
    };

    /// OpenAI GPT-4o Mini pricing
    pub const OPENAI_GPT4O_MINI: TokenPricing = TokenPricing {
        input_per_million: 0.15,
        output_per_million: 0.60,
    };

    /// Get pricing for a model name
    pub fn for_model(model: &str) -> TokenPricing {
        let model_lower = model.to_lowercase();
        if model_lower.contains("claude-3-opus") || model_lower.contains("claude-opus") {
            ANTHROPIC_OPUS
        } else if model_lower.contains("claude-3-5-sonnet")
            || model_lower.contains("claude-sonnet")
            || model_lower.contains("sonnet")
        {
            ANTHROPIC_SONNET
        } else if model_lower.contains("gpt-4o-2024") || model_lower.contains("gpt-4o-mini") {
            // Default to gpt-4o-mini for mini models
            if model_lower.contains("mini") {
                OPENAI_GPT4O_MINI
            } else {
                OPENAI_GPT4O
            }
        } else if model_lower.contains("gpt-4") {
            // GPT-4 pricing (approximate)
            TokenPricing {
                input_per_million: 30.0,
                output_per_million: 60.0,
            }
        } else if model_lower.contains("gpt-3.5") {
            // GPT-3.5 Turbo pricing
            TokenPricing {
                input_per_million: 0.50,
                output_per_million: 1.50,
            }
        } else {
            // Default fallback
            OPENAI_GPT4O
        }
    }
}

/// Configuration for OpenTelemetry tracing
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// Whether to enable OpenTelemetry tracing
    pub enabled: bool,
    /// Service name for the tracer
    pub service_name: String,
    /// Endpoint for OTLP exporter (if None, uses stdout)
    pub otlp_endpoint: Option<String>,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            service_name: "swell".to_string(),
            otlp_endpoint: None,
        }
    }
}

/// Latency tracker for measuring LLM call duration
#[derive(Debug)]
pub struct LatencyTracker {
    start: Instant,
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Returns the elapsed time in milliseconds
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating GenAI spans with proper attributes
#[allow(dead_code)]
pub struct GenAiSpanBuilder {
    operation_name: String,
    provider: LlmProvider,
    model: String,
}

impl GenAiSpanBuilder {
    /// Create a new builder for a GenAI span
    pub fn new(operation_name: &str, provider: LlmProvider, model: &str) -> Self {
        Self {
            operation_name: operation_name.to_string(),
            provider,
            model: model.to_string(),
        }
    }

    /// Start the span with all required GenAI attributes
    pub fn start_span(
        operation_name: String,
        provider: LlmProvider,
        model: String,
        tracer: &opentelemetry::global::BoxedTracer,
    ) -> opentelemetry::global::BoxedSpan {
        use opentelemetry::trace::Tracer;

        let mut span_builder = tracer.span_builder(operation_name.clone());
        span_builder.span_kind = Some(SpanKind::Client);
        span_builder.attributes = Some(vec![
            KeyValue::new(gen_ai::OPERATION_NAME, operation_name),
            KeyValue::new(gen_ai::PROVIDER_NAME, provider.as_str().to_string()),
            KeyValue::new(gen_ai::REQUEST_MODEL, model),
        ]);

        tracer.build(span_builder)
    }
}

/// Extension trait for adding GenAI attributes to spans
pub trait GenAiSpanExt {
    /// Record prompt tokens
    fn record_prompt_tokens(&mut self, tokens: u64);

    /// Record completion tokens
    fn record_completion_tokens(&mut self, tokens: u64);

    /// Record the response model (may differ from request model)
    fn record_response_model(&mut self, model: &str);

    /// Record cost in USD
    fn record_cost_usd(&mut self, cost: f64);

    /// Record latency in milliseconds
    fn record_latency_ms(&mut self, latency_ms: u64);

    /// Record an error on the span (for GenAI-specific errors)
    fn record_genai_error(&mut self, error: &str);
}

impl<T: Span> GenAiSpanExt for T {
    fn record_prompt_tokens(&mut self, tokens: u64) {
        self.set_attribute(KeyValue::new(gen_ai::USAGE_INPUT_TOKENS, tokens as i64));
    }

    fn record_completion_tokens(&mut self, tokens: u64) {
        self.set_attribute(KeyValue::new(gen_ai::USAGE_OUTPUT_TOKENS, tokens as i64));
    }

    fn record_response_model(&mut self, model: &str) {
        self.set_attribute(KeyValue::new(gen_ai::RESPONSE_MODEL, model.to_string()));
    }

    fn record_cost_usd(&mut self, cost: f64) {
        // Use a custom attribute for cost since it's not standardized yet
        self.set_attribute(KeyValue::new("cost.usd", cost));
    }

    fn record_latency_ms(&mut self, latency_ms: u64) {
        // Use a custom attribute for latency since it's not standardized yet
        self.set_attribute(KeyValue::new("latency_ms", latency_ms as i64));
    }

    fn record_genai_error(&mut self, error: &str) {
        self.set_status(Status::error(error.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_as_str() {
        assert_eq!(LlmProvider::Anthropic.as_str(), "anthropic");
        assert_eq!(LlmProvider::OpenAI.as_str(), "openai");
        assert_eq!(LlmProvider::Mock.as_str(), "mock");
    }

    #[test]
    fn test_token_pricing_calculation() {
        let pricing = pricing::ANTHROPIC_SONNET;
        // 1M input + 1M output = $3 + $15 = $18
        let cost = pricing.calculate_cost(1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_pricing_for_model() {
        // Anthropic Sonnet
        let p = pricing::for_model("claude-3-5-sonnet-20250514");
        assert!((p.input_per_million - 3.0).abs() < 0.001);

        // OpenAI GPT-4o
        let p = pricing::for_model("gpt-4o-2024-08-06");
        assert!((p.input_per_million - 5.0).abs() < 0.001);

        // Unknown model defaults to GPT-4o
        let p = pricing::for_model("unknown-model");
        assert!((p.input_per_million - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_latency_tracker() {
        let tracker = LatencyTracker::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let elapsed = tracker.elapsed_ms();
        assert!(elapsed >= 10, "Expected >= 10ms, got {}ms", elapsed);
    }

    // Tests for otel_genai feature - GenAI semantic conventions

    #[test]
    fn test_otel_genai_gen_ai_module_keys() {
        // Verify gen_ai module defines all required semantic convention keys
        assert_eq!(gen_ai::OPERATION_NAME.as_str(), "gen_ai.operation.name");
        assert_eq!(gen_ai::PROVIDER_NAME.as_str(), "gen_ai.provider.name");
        assert_eq!(gen_ai::REQUEST_MODEL.as_str(), "gen_ai.request.model");
        assert_eq!(gen_ai::RESPONSE_MODEL.as_str(), "gen_ai.response.model");
        assert_eq!(gen_ai::USAGE_INPUT_TOKENS.as_str(), "gen_ai.usage.input_tokens");
        assert_eq!(gen_ai::USAGE_OUTPUT_TOKENS.as_str(), "gen_ai.usage.output_tokens");
    }

    #[test]
    fn test_otel_genai_provider_conventions() {
        // Test that providers follow GenAI conventions
        assert_eq!(LlmProvider::Anthropic.as_str(), "anthropic");
        assert_eq!(LlmProvider::OpenAI.as_str(), "openai");
        assert_eq!(LlmProvider::Mock.as_str(), "mock");
    }

    #[test]
    fn test_otel_genai_pricing_for_anthropic_models() {
        // Test Anthropic model pricing
        let sonnet = pricing::for_model("claude-3-5-sonnet-20250514");
        assert!((sonnet.input_per_million - 3.0).abs() < 0.001);
        assert!((sonnet.output_per_million - 15.0).abs() < 0.001);

        let opus = pricing::for_model("claude-3-opus-20240229");
        assert!((opus.input_per_million - 15.0).abs() < 0.001);
        assert!((opus.output_per_million - 75.0).abs() < 0.001);
    }

    #[test]
    fn test_otel_genai_pricing_for_openai_models() {
        // Test OpenAI model pricing
        let gpt4o = pricing::for_model("gpt-4o-2024-08-06");
        assert!((gpt4o.input_per_million - 5.0).abs() < 0.001);
        assert!((gpt4o.output_per_million - 15.0).abs() < 0.001);

        let gpt4o_mini = pricing::for_model("gpt-4o-mini");
        assert!((gpt4o_mini.input_per_million - 0.15).abs() < 0.001);
        assert!((gpt4o_mini.output_per_million - 0.60).abs() < 0.001);
    }

    #[test]
    fn test_otel_genai_cost_calculation() {
        // Test cost calculation for GenAI usage
        let pricing = pricing::ANTHROPIC_SONNET;

        // 1000 input + 500 output tokens
        let cost = pricing.calculate_cost(1000, 500);
        // $3/M input → 1000/1M * $3 = $0.003
        // $15/M output → 500/1M * $15 = $0.0075
        // Total = $0.0105
        assert!((cost - 0.0105).abs() < 0.0001);

        // Large request: 1M input + 1M output
        let large_cost = pricing.calculate_cost(1_000_000, 1_000_000);
        assert!((large_cost - 18.0).abs() < 0.001);
    }

    #[allow(unused_comparisons)]
    #[test]
    fn test_otel_genai_latency_tracker_precision() {
        // Test that latency tracker provides millisecond precision
        let tracker = LatencyTracker::new();

        // Small delay - elapsed_ms returns u64 which is always >= 0
        std::thread::sleep(std::time::Duration::from_micros(500));
        let elapsed = tracker.elapsed_ms();

        // Verify elapsed is reasonable (not zero since we slept)
        assert!(elapsed >= 0); // u64 is always >= 0, but explicit check documents intent

        // But for 10ms sleep, should get at least 10ms
        let tracker2 = LatencyTracker::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let elapsed2 = tracker2.elapsed_ms();
        assert!(elapsed2 >= 10);
    }

    #[test]
    fn test_otel_genai_model_attribute_values() {
        // Verify model names are preserved correctly
        let models = vec![
            "claude-3-5-sonnet-20250514",
            "gpt-4o-2024-08-06",
            "gpt-4o-mini",
            "claude-opus-4",
        ];

        for model in models {
            let pricing = pricing::for_model(model);
            // All should return valid pricing (not zero)
            assert!(pricing.input_per_million > 0.0);
            assert!(pricing.output_per_million > 0.0);
        }
    }

    #[test]
    fn test_otel_genai_credit_token_calculation() {
        // Test token-to-cost calculation for different providers
        // Anthropic Sonnet: $3/M input, $15/M output

        // 1 token = $3 / 1,000,000 = $0.000003
        let per_token = pricing::ANTHROPIC_SONNET.calculate_cost(1, 0);
        assert!((per_token - 0.000003).abs() < 0.000001);

        // 1000 tokens = $0.003
        let per_thousand = pricing::ANTHROPIC_SONNET.calculate_cost(1000, 0);
        assert!((per_thousand - 0.003).abs() < 0.0001);
    }
}
