# MiniMax Integration for Live LLM Testing

MiniMax provides OpenAI-compatible and Anthropic-compatible API endpoints that can be used to test Swell's LLM backends against a real API without needing actual OpenAI/Anthropic keys.

## Endpoints

- **Anthropic-compatible**: `https://api.minimax.io/anthropic`
- **OpenAI-compatible**: `https://api.minimax.io/v1`
- **Model**: `MiniMax-M2.7`

## Authentication

The API key is loaded from the `MINIMAX_API_KEY` environment variable at runtime.

**SECURITY RULES:**
- NEVER hardcode the API key in source code, tests, configs, or any committed file
- NEVER read from `plan/minimax-docs/minimax-api-key.md` in code — use `std::env::var("MINIMAX_API_KEY")`
- The key file is gitignored via `/plan/**` pattern
- Tests requiring the key must be gated with `#[ignore]` so CI works without it

## Anthropic-Compatible API

- Supports: `model`, `messages`, `max_tokens`, `stream`, `system`, `temperature`, `tools`, `tool_choice`, `thinking`
- Supports `content_block_delta` / `message_delta` / `message_stop` SSE events for streaming
- Does NOT support: `top_k`, `stop_sequences`, `service_tier`, image/document inputs
- Temperature range: (0.0, 1.0]

## OpenAI-Compatible API

- Supports: `model`, `messages`, `max_tokens`, `stream`, `temperature`, `tools`, `tool_choice`
- Supports `choices[0].delta.content` SSE streaming
- Does NOT support: `presence_penalty`, `frequency_penalty`, `logit_bias`, `n > 1`, image/audio inputs
- Temperature range: (0.0, 1.0]
- Deprecated `function_call` not supported — use `tools` parameter

## Test Pattern

```rust
#[tokio::test]
#[ignore] // Only runs when MINIMAX_API_KEY is set
async fn test_live_anthropic_request() {
    let api_key = match std::env::var("MINIMAX_API_KEY") {
        Ok(key) => key,
        Err(_) => return, // Skip if no key
    };

    let backend = AnthropicBackend::new(
        &api_key,
        "MiniMax-M2.7",
        Some("https://api.minimax.io/anthropic"),
    );

    // ... test ...
}
```

## Documentation Reference

Full MiniMax API docs are in `plan/minimax-docs/` (gitignored):
- `minimax-anthropic-api.md` — Anthropic-compatible API details
- `minimax-openai-api.md` — OpenAI-compatible API details
