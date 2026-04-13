# swell-llm AGENTS.md

## Purpose

`swell-llm` provides LLM backend implementations for the SWELL autonomous coding engine. It offers a unified `LlmBackend` trait and concrete implementations for Anthropic Claude and OpenAI GPT APIs, plus a `MockLlm` for testing.

This crate handles:
- **AnthropicBackend** — API calls to Anthropic Claude models (SSE streaming support)
- **OpenAIBackend** — API calls to OpenAI GPT models (SSE streaming support)
- **MockLlm** — In-memory mock for reproducible testing
- **ModelRouter** — Routes requests to appropriate models based on task type and cost

**Depends on:** `swell-core` (for `LlmBackend` trait and `SwellError`)

## Public API

### Backend Types (`anthropic.rs`, `openai.rs`, `mock.rs`)

```rust
// Main backend implementations
pub struct AnthropicBackend {
    pub model: String,
    pub api_key: String,
    pub base_url: String,
}

pub struct OpenAIBackend {
    pub model: String,
    pub api_key: String,
    pub base_url: String,
}

pub struct MockLlm {
    responses: VecDeque<String>,
    // ...
}
```

### Model Router (`router.rs`)

```rust
pub struct ModelRouter {
    backends: HashMap<String, Arc<dyn LlmBackend>>,
    route_config: RouteConfig,
}

pub struct ModelRouterBuilder {
    // Builder for constructing ModelRouter
}

pub enum TaskType { Planning, Coding, Review, Research }

pub struct RouteConfig {
    pub planning_model: String,
    pub coding_model: String,
    pub review_model: String,
    pub research_model: String,
}

// Cost optimization
pub struct CostOptimizer {
    // Selects least expensive model that meets task requirements
}
```

### Traits (`traits.rs`)

```rust
// Re-exports from swell-core
pub use swell_core::LlmBackend;
pub use swell_core::{LlmConfig, LlmMessage, LlmRole, LlmResponse, LlmToolCall, LlmToolDefinition, LlmUsage};
```

### Helper Functions

```rust
// Create a backend from URL scheme
pub fn create_backend(url: &str, model: &str, api_key: &str) -> Result<BoxLlmBackend, SwellError>
```

### Usage Example

```rust,ignore
use swell_llm::{AnthropicBackend, LlmConfig, LlmMessage, LlmRole};

let backend = AnthropicBackend::new("claude-sonnet-4-20250514", "sk-ant-api03-...");
let config = LlmConfig::default();
let messages = vec![
    LlmMessage { role: LlmRole::User, content: "Hello".to_string() }
];
let response = backend.chat(messages, None, config).await?;
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        swell-llm                           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │ Anthropic   │  │   OpenAI    │  │    Mock     │         │
│  │  Backend    │  │   Backend   │  │    Backend  │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
│         │                │                │                 │
│         └────────────────┼────────────────┘                 │
│                          │                                  │
│                    ┌─────▼─────┐                            │
│                    │   trait   │  (from swell-core)         │
│                    │ LlmBackend│                            │
│                    └───────────┘                            │
│                          │                                  │
│                    ┌─────▼─────┐                            │
│                    │   Model    │                            │
│                    │   Router   │                            │
│                    │ (route.rs) │                            │
│                    └───────────┘                            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │    swell-orchestrator   │
              │    swell-validation    │
              └────────────────────────┘
```

**Key modules:**
- `anthropic.rs` — Anthropic Messages API client with SSE streaming
- `openai.rs` — OpenAI Chat Completions API client with SSE streaming
- `mock.rs` — In-memory mock for testing
- `router.rs` — Model routing and cost optimization
- `traits.rs` — Re-exports of LLM types from swell-core

**Concurrency:** All backends are `Send + Sync`. Uses `reqwest` for HTTP with async/await.

## Testing

```bash
# Run tests for swell-llm
cargo test -p swell-llm -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-llm

# Run specific test
cargo test -p swell-llm -- test_mock_backend --nocapture

# Run ignored live tests (requires MINIMAX_API_KEY)
cargo test -p swell-llm -- --ignored live
cargo test -p swell-llm -- --ignored live_stream
```

**Test patterns:**
- Unit tests for each backend (mock HTTP responses)
- `MockLlm` for integration testing without API calls
- SSE streaming tests with synthetic event streams
- Token tracking tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_mock_backend() {
    let mock = MockLlm::new("gpt-4");
    let config = LlmConfig { temperature: 0.7, max_tokens: 4096, stop_sequences: None };
    let messages = vec![LlmMessage { role: LlmRole::User, content: "Say hello".to_string() }];
    let response = mock.chat(messages, None, config).await.unwrap();
    assert!(!response.content.is_empty());
}
```

## Dependencies

```toml
# swell-llm/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
reqwest.workspace = true
async-trait.workspace = true
futures.workspace = true
tracing.workspace = true
chrono.workspace = true
uuid.workspace = true

# OpenTelemetry for GenAI observability
opentelemetry = { version = "0.26", features = ["trace"] }
opentelemetry_sdk = { version = "0.26", features = ["rt-tokio"] }

[dev-dependencies]
tokio-test.workspace = true
mockall.workspace = true
```
