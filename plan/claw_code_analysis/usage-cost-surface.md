# Usage Cost Surface

The runtime's usage reporting has two distinct concerns tracked in separate documents: `usage-tracker-lifecycle.md` covers per-turn recording and session reconstruction, while this document focuses on the **cost estimation surface** — model-sensitive pricing, the CLI-facing token and cost summary format, and the parity evidence chain that validates usage reporting claims.

## Model-Sensitive Pricing

`pricing_for_model()` in `crates/runtime/src/usage.rs` maps model names to `ModelPricing` structs using case-insensitive substring matching against three known families:

```rust
pub fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    let normalized = model.to_ascii_lowercase();
    if normalized.contains("haiku") {
        return Some(ModelPricing { input_cost_per_million: 1.0, output_cost_per_million: 5.0, ... });
    }
    if normalized.contains("opus") {
        return Some(ModelPricing { input_cost_per_million: 15.0, output_cost_per_million: 75.0, ... });
    }
    if normalized.contains("sonnet") {
        return Some(ModelPricing::default_sonnet_tier());
    }
    None  // unknown model → fallback
}
```

The returned `ModelPricing` carries per-million-token rates for four independent billing dimensions:

| Dimension | Sonnet default | Haiku | Opus |
|---|---|---|---|
| `input_cost_per_million` | $15.00 | $1.00 | $15.00 |
| `output_cost_per_million` | $75.00 | $5.00 | $75.00 |
| `cache_creation_cost_per_million` | $18.75 | $1.25 | $18.75 |
| `cache_read_cost_per_million` | $1.50 | $0.10 | $1.50 |

When a model is unrecognized, `pricing_for_model` returns `None` and the caller falls back to `ModelPricing::default_sonnet_tier()`. The `summary_lines_for_model` method surfaces this fallback explicitly with a `pricing=estimated-default` suffix so operators can distinguish known-model estimates from generic ones.

## Token-to-Cost Conversion

`TokenUsage::estimate_cost_usd_with_pricing()` in `usage.rs` applies the per-million rates to four token counters:

```rust
pub fn estimate_cost_usd_with_pricing(self, pricing: ModelPricing) -> UsageCostEstimate {
    UsageCostEstimate {
        input_cost_usd: cost_for_tokens(self.input_tokens, pricing.input_cost_per_million),
        output_cost_usd: cost_for_tokens(self.output_tokens, pricing.output_cost_per_million),
        cache_creation_cost_usd: cost_for_tokens(self.cache_creation_input_tokens, pricing.cache_creation_cost_per_million),
        cache_read_cost_usd: cost_for_tokens(self.cache_read_input_tokens, pricing.cache_read_cost_per_million),
    }
}
```

`cost_for_tokens` is: `f64::from(tokens) / 1_000_000.0 * usd_per_million_tokens`.

The `UsageCostEstimate::total_cost_usd()` method sums all four dimensions. The four-dimensional breakdown is not an implementation artifact — it maps directly to the Anthropic API's own metering surface, where cache-creation and cache-read tokens are billed at different rates from raw input/output tokens.

## CLI-Facing Summary Format

`TokenUsage::summary_lines_for_model()` in `usage.rs` produces two lines of text designed for human-readable CLI output:

```
usage: total_tokens=1800000 input=1000000 output=500000 cache_write=100000 cache_read=200000 estimated_cost=$54.6750 model=claude-sonnet-4-20250514
  cost breakdown: input=$15.0000 output=$37.5000 cache_write=$1.8750 cache_read=$0.3000
```

The `format_usd()` helper formats dollar amounts as `$X.XXXX` (always four decimal places).

This format appears in two places:
- Turn summary output in `crates/rusty-claude-cli/src/main.rs` — the `TurnOutput` JSON structure includes `estimated_cost` as a dollar-prefixed string alongside token breakdowns.
- `/cost` slash command surface — renders cumulative session usage with the same format.

The JSON turn output structure in `main.rs` (referenced via `crates/api/src/types.rs`) is:

```rust
"usage": {
    "input_tokens": summary.usage.input_tokens,
    "output_tokens": summary.usage.output_tokens,
    "cache_creation_input_tokens": summary.usage.cache_creation_input_tokens,
    "cache_read_input_tokens": summary.usage.cache_read_input_tokens,
},
"estimated_cost": format_usd(
    summary.usage.estimate_cost_usd_with_pricing(
        pricing_for_model(&self.model)
            .unwrap_or_else(runtime::ModelPricing::default_sonnet_tier)
    ).total_cost_usd()
)
```

The `Usage::estimated_cost_usd(model)` method in `crates/api/src/types.rs` bridges the API types back to the runtime pricing surface, allowing the API client to emit `estimated_cost_usd` telemetry events during request/response cycles.

## Validation Evidence

### Unit tests in `usage.rs`

`crates/runtime/src/usage.rs` contains three direct tests of the cost surface:

- `computes_cost_summary_lines` — verifies 1M input + 500K output + 100K cache_write + 200K cache_read at sonnet pricing yields `$54.6750` total, and that the summary line contains `model=claude-sonnet-4-20250514`.
- `supports_model_specific_pricing` — verifies haiku (1M input + 500K output) costs `$3.50` while opus costs `$52.50`, confirming the pricing tier difference.
- `marks_unknown_model_pricing_as_fallback` — verifies that calling `summary_lines_for_model` with a custom model name produces a `pricing=estimated-default` suffix.

### Parity harness: `token_cost_reporting`

`mock_parity_scenarios.json` defines the `token_cost_reporting` scenario in the `token-usage` category:

```json
{
  "name": "token_cost_reporting",
  "category": "token-usage",
  "description": "Confirms usage token counts and estimated_cost appear in JSON output.",
  "parity_refs": ["Token counting / cost tracking accuracy"]
}
```

The harness in `crates/rusty-claude-cli/tests/mock_parity_harness.rs` wires this scenario through `assert_token_cost_reporting`, which asserts:

```rust
assert!(run.response["usage"]["output_tokens"].as_u64().unwrap_or(0) > 0, "output_tokens should be non-zero");
assert!(
    run.response["estimated_cost"].as_str().is_some_and(|cost| cost.starts_with('$')),
    "estimated_cost should be a dollar-prefixed string"
);
```

The mock service (`crates/mock-anthropic-service/src/lib.rs`) responds to `token_cost_reporting` with `text_message_response_with_usage` delivering 1,000 input tokens and 500 output tokens, allowing the harness to verify the counter and cost fields are populated in the JSON response.

The API client integration test in `crates/api/tests/client_integration.rs` additionally verifies that a `TelemetryEvent::Analytics` with action `message_usage` carries `estimated_cost_usd` as a `$0.0001`-formatted string alongside `total_tokens`.

### PARITY.md status

`PARITY.md` lists "Token counting / cost tracking accuracy" under "Still open", with the `token_cost_reporting` scenario noted as the validation target. The scenario is implemented in the harness and the unit tests verify the pricing math directly. The PARITY.md flag reflects that the broader parity documentation has not been formally closed, not that the mechanism is absent.

## Builder Lessons

1. **Four billing dimensions, not three.** Cache-creation and cache-read tokens are billed at different rates from input/output tokens. A three-field usage struct that omits the cache dimensions will produce incorrect cost estimates for cache-heavy workloads. The four-dimensional `TokenUsage` in `usage.rs` is the correct shape.

2. **Substring model matching is stable for current models.** `pricing_for_model` uses `contains` on the lowercased model name, so `claude-sonnet-4-20250514`, `claude-haiku-4-5-20251001`, and `claude-opus-4-6` all route correctly. New model names must contain one of the known family substrings to receive tier pricing; unknown models fall back silently with `pricing=estimated-default` in output.

3. **Format dollars consistently.** `format_usd("$0.0001")` always produces four decimal places, which keeps cost columns aligned in tabular output and avoids `$0` display for sub-mill dollar amounts.

4. **Separate the cost estimation surface from the usage tracking surface.** `UsageTracker` owns recording and accumulation; `TokenUsage::estimate_cost_usd` and `summary_lines_for_model` own rendering. This separation means cost formatting can be exercised independently of a live session tracker, which is why the unit tests can verify pricing math without a runtime session.

5. **Parity scenarios and unit tests are complementary evidence.** The unit tests in `usage.rs` verify pricing arithmetic directly. The parity harness verifies that `estimated_cost` and token counters appear in the end-to-end JSON turn output. Both are needed for a complete validation story — arithmetic correctness alone does not guarantee the field appears in serialized output.
