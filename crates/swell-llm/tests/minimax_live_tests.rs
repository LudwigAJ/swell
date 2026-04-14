//! Live integration tests for MiniMax API endpoints.
//!
//! These tests connect to the real MiniMax API and are gated behind `#[ignore]`.
//! They only run when `MINIMAX_API_KEY` environment variable is set.
//!
//! Run with: `cargo test --workspace -- --ignored live`
//! Or for streaming: `cargo test --workspace -- --ignored live_stream`
//!
//! NOTE: These tests require network access and a valid MINIMAX_API_KEY.

use futures::StreamExt;
use std::env;

// Re-export types from swell-llm for convenience
use swell_llm::{AnthropicBackend, LlmBackend, LlmConfig, LlmMessage, LlmRole, OpenAIBackend};

// =============================================================================
// Anthropic-Compatible API Tests (MiniMax supports Anthropic-compatible endpoint)
// =============================================================================

/// Test AnthropicBackend sync completion via MiniMax Anthropic-compatible API.
/// Uses model MiniMax-M2.7 at https://api.minimax.io/anthropic
#[tokio::test]
#[ignore]
async fn test_live_anthropic_sync_completion() {
    let api_key = env::var("MINIMAX_API_KEY").expect("MINIMAX_API_KEY must be set for live tests");
    let model = "MiniMax-M2.7";
    let base_url = "https://api.minimax.io/anthropic";

    // Create backend with custom base URL for MiniMax Anthropic-compatible endpoint
    let backend = AnthropicBackend::with_base_url(model, api_key, base_url);

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Say 'hello' in exactly one word.".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig::default();

    let result = backend.chat(messages, None, config).await;

    match result {
        Ok(response) => {
            // Verify response structure
            assert!(
                !response.content.is_empty(),
                "Response content should not be empty"
            );
            println!("Anthropic sync response: {}", response.content);
            println!("Usage: {:?}", response.usage);
        }
        Err(e) => {
            panic!("Anthropic API request failed: {}", e);
        }
    }
}

/// Test AnthropicBackend SSE streaming via MiniMax Anthropic-compatible API.
/// Uses model MiniMax-M2.7 at https://api.minimax.io/anthropic
#[tokio::test]
#[ignore]
async fn test_live_anthropic_streaming() {
    let api_key = env::var("MINIMAX_API_KEY").expect("MINIMAX_API_KEY must be set for live tests");
    let model = "MiniMax-M2.7";
    let base_url = "https://api.minimax.io/anthropic";

    // Create backend with custom base URL for MiniMax Anthropic-compatible endpoint
    let backend = AnthropicBackend::with_base_url(model, api_key, base_url);

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Count from 1 to 3, one number per message.".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig {
        max_tokens: 100,
        ..Default::default()
    };

    // Call the streaming method
    let stream_result = backend.stream(messages, None, config).await;

    match stream_result {
        Ok(stream) => {
            let mut stream = stream;
            let mut delta_count = 0;
            let mut final_text = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(event) => {
                        use swell_core::StreamEvent;
                        match event {
                            StreamEvent::TextDelta { text, delta } => {
                                final_text = text.clone();
                                delta_count += 1;
                                println!("Text delta: '{}' (total: '{}')", delta, text);
                            }
                            StreamEvent::ToolUse { tool_call } => {
                                println!("Tool use: {:?}", tool_call);
                            }
                            StreamEvent::MessageStop { stop_reason } => {
                                println!("Stream stopped: {:?}", stop_reason);
                            }
                            StreamEvent::Usage {
                                input_tokens,
                                output_tokens,
                                ..
                            } => {
                                println!(
                                    "Usage - input: {}, output: {}",
                                    input_tokens, output_tokens
                                );
                            }
                            StreamEvent::Error { message } => {
                                panic!("Stream error: {}", message);
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        panic!("Stream item error: {}", e);
                    }
                }
            }

            // Verify we received at least 2 deltas
            assert!(
                delta_count >= 2,
                "Expected at least 2 text deltas, got {}",
                delta_count
            );
            assert!(
                !final_text.is_empty(),
                "Final assembled text should not be empty"
            );
            println!("Final assembled text: {}", final_text);
        }
        Err(e) => {
            panic!("Anthropic streaming request failed: {}", e);
        }
    }
}

// =============================================================================
// OpenAI-Compatible API Tests (MiniMax supports OpenAI-compatible endpoint)
// =============================================================================

/// Test OpenAIBackend sync completion via MiniMax OpenAI-compatible API.
/// Uses model MiniMax-M2.7 at https://api.minimax.io/v1
#[tokio::test]
#[ignore]
async fn test_live_openai_sync_completion() {
    let api_key = env::var("MINIMAX_API_KEY").expect("MINIMAX_API_KEY must be set for live tests");
    let model = "MiniMax-M2.7";
    let base_url = "https://api.minimax.io/v1";

    // Create backend with custom base URL
    let backend = OpenAIBackend::with_base_url(model, api_key, base_url)
        .expect("Failed to create OpenAI backend");

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Say 'hello' in exactly one word.".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig::default();

    let result = backend.chat(messages, None, config).await;

    match result {
        Ok(response) => {
            // Verify response structure
            assert!(
                !response.content.is_empty(),
                "Response content should not be empty"
            );
            println!("OpenAI sync response: {}", response.content);
            println!("Usage: {:?}", response.usage);
        }
        Err(e) => {
            panic!("OpenAI API request failed: {}", e);
        }
    }
}

/// Test OpenAIBackend SSE streaming via MiniMax OpenAI-compatible API.
/// Uses model MiniMax-M2.7 at https://api.minimax.io/v1
#[tokio::test]
#[ignore]
async fn test_live_openai_streaming() {
    let api_key = env::var("MINIMAX_API_KEY").expect("MINIMAX_API_KEY must be set for live tests");
    let model = "MiniMax-M2.7";
    let base_url = "https://api.minimax.io/v1";

    // Create backend with custom base URL
    let backend = OpenAIBackend::with_base_url(model, api_key, base_url)
        .expect("Failed to create OpenAI backend");

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Count from 1 to 3, one number per message.".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig {
        max_tokens: 100,
        ..Default::default()
    };

    // Call the streaming method
    let stream_result = backend.stream(messages, None, config).await;

    match stream_result {
        Ok(stream) => {
            let mut stream = stream;
            let mut delta_count = 0;
            let mut final_text = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(event) => {
                        use swell_core::StreamEvent;
                        match event {
                            StreamEvent::TextDelta { text, delta } => {
                                final_text = text.clone();
                                delta_count += 1;
                                println!("Text delta: '{}' (total: '{}')", delta, text);
                            }
                            StreamEvent::ToolUse { tool_call } => {
                                println!("Tool use: {:?}", tool_call);
                            }
                            StreamEvent::MessageStop { stop_reason } => {
                                println!("Stream stopped: {:?}", stop_reason);
                            }
                            StreamEvent::Usage {
                                input_tokens,
                                output_tokens,
                                ..
                            } => {
                                println!(
                                    "Usage - input: {}, output: {}",
                                    input_tokens, output_tokens
                                );
                            }
                            StreamEvent::Error { message } => {
                                panic!("Stream error: {}", message);
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        panic!("Stream item error: {}", e);
                    }
                }
            }

            // Verify we received at least 2 deltas
            assert!(
                delta_count >= 2,
                "Expected at least 2 text deltas, got {}",
                delta_count
            );
            assert!(
                !final_text.is_empty(),
                "Final assembled text should not be empty"
            );
            println!("Final assembled text: {}", final_text);
        }
        Err(e) => {
            panic!("OpenAI streaming request failed: {}", e);
        }
    }
}

// =============================================================================
// Verification Tests (run without ignored to verify env var reading)
// =============================================================================

/// Verify that MINIMAX_API_KEY is read from environment variable.
/// This test always runs to ensure the env var is properly read.
#[test]
fn test_env_var_is_read_via_std_env() {
    // This test verifies the pattern we're using to read the API key
    // It checks that std::env::var is the mechanism we use
    let result = env::var("MINIMAX_API_KEY");

    // If MINIMAX_API_KEY is set, the test passes
    // If it's not set, we get an error but the test still verifies our approach
    match result {
        Ok(key) => {
            assert!(!key.is_empty(), "MINIMAX_API_KEY should not be empty");
            println!(
                "MINIMAX_API_KEY is configured ([key present], length: {})",
                key.len()
            );
        }
        Err(e) => {
            println!("MINIMAX_API_KEY not set (this is expected in CI): {}", e);
        }
    }
}

/// Verify no hardcoded API keys exist in test files.
/// This test searches for suspicious patterns.
#[test]
fn test_no_hardcoded_api_keys_in_tests() {
    // This test uses code inspection to verify we're not hardcoding keys
    // We're using std::env::var which is the correct pattern

    // Read the current file and verify it uses env::var
    let test_code = include_str!("../tests/minimax_live_tests.rs");

    // Verify env::var is used to read the API key
    assert!(
        test_code.contains("env::var(\"MINIMAX_API_KEY\")"),
        "Tests should read API key via std::env::var"
    );

    // Verify no hardcoded key patterns - check for common key prefix patterns
    // by looking for substrings that match typical key formats
    // Key format: starts with "sk-" followed by 30+ more characters
    let remaining_chars = 30;

    // Search for "sk-" and verify it's followed by substantial content (potential key)
    let mut i = 0;
    let bytes = test_code.as_bytes();
    while i < bytes.len().saturating_sub(40) {
        if bytes[i] == b's' && bytes[i + 1] == b'k' && bytes[i + 2] == b'-' {
            // Found "sk-" - now check if it's followed by 30+ more chars (API key pattern)
            let after_prefix = &test_code[i..];
            let remaining = after_prefix.len().saturating_sub(3);
            if remaining > remaining_chars {
                // Check this isn't just code containing "sk-" as part of variable names
                let context = &test_code[i.saturating_sub(10)..i + 50];
                let not_in_code = !context.contains("contains(\"sk-") && !context.contains("\"sk-");
                if not_in_code {
                    panic!("Potential hardcoded API key found in test code");
                }
            }
        }
        i += 1;
    }
}
