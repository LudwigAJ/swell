//! Tests for cache token event emission.
//!
//! These tests verify that cache_creation_tokens and cache_read_tokens are
//! emitted as first-class structured observability events through the
//! StreamEvent::Usage type.

use swell_llm::{AnthropicBackend, LlmConfig, LlmMessage, LlmRole};
use swell_core::StreamEvent;
use futures::StreamExt;
use tokio::test;

/// Test that cache_creation_tokens and cache_read_tokens are emitted as structured events
/// when present in the SSE stream from Anthropic API.
#[test]
async fn test_cache_tokens_emitted_as_usage_event() {
    // Create a mock server that returns SSE with cache tokens
    use mockito::Server;

    let mut server = Server::new_async().await;

    // Mock the messages endpoint with a streaming response that includes cache tokens
    let mock_response = r#"event: content_block_start
data: {"type":"text","index":0}

event: content_block_delta
data: {"type":"text_delta","text":"Hello","index":0}

event: message_delta
data: {"type":"message_delta","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":200,"cache_read_input_tokens":300}}

event: message_stop
data: {"type":"message_stop"}
"#;

    let m = server
        .mock("POST", "/v1/messages")
        .with_body(mock_response)
        .create();

    let backend = AnthropicBackend::with_base_url(
        "claude-sonnet-4-20250514",
        "test-api-key",
        server.url(),
    );

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Say hello".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig::default();

    // Call stream() and collect events
    let stream = backend
        .stream(messages, None, config)
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    // Find the Usage event
    let usage_events: Vec<&StreamEvent> = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Usage { .. }))
        .collect();

    assert!(
        !usage_events.is_empty(),
        "Expected at least one Usage event, got: {:?}",
        events
    );

    // Verify the Usage event contains cache tokens
    for event in usage_events {
        match event {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            } => {
                assert_eq!(*input_tokens, 100, "input_tokens should be 100");
                assert_eq!(*output_tokens, 50, "output_tokens should be 50");
                assert_eq!(
                    *cache_creation_input_tokens,
                    Some(200),
                    "cache_creation_input_tokens should be Some(200)"
                );
                assert_eq!(
                    *cache_read_input_tokens,
                    Some(300),
                    "cache_read_input_tokens should be Some(300)"
                );
            }
            _ => unreachable!("We filtered for Usage events"),
        }
    }
}

/// Test that cache tokens are emitted even when there are no output tokens.
/// This can happen with cached responses where only cache tokens are counted.
#[test]
async fn test_cache_tokens_emitted_without_output_tokens() {
    use mockito::Server;

    let mut server = Server::new_async().await;

    // Mock response with cache tokens but NO output tokens (cache hit only)
    let mock_response = r#"event: message_delta
data: {"type":"message_delta","usage":{"input_tokens":50,"cache_read_input_tokens":500}}

event: message_stop
data: {"type":"message_stop"}
"#;

    let m = server
        .mock("POST", "/v1/messages")
        .with_body(mock_response)
        .create();

    let backend = AnthropicBackend::with_base_url(
        "claude-sonnet-4-20250514",
        "test-api-key",
        server.url(),
    );

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Hello".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig::default();

    let stream = backend
        .stream(messages, None, config)
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    // Find the Usage event - should be emitted even without output tokens
    // since we have cache tokens
    let usage_events: Vec<&StreamEvent> = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Usage { .. }))
        .collect();

    // This test will FAIL until we fix the SSE parser to emit Usage
    // when cache tokens are present (even without output tokens)
    assert!(
        !usage_events.is_empty(),
        "Expected at least one Usage event when cache tokens are present, got: {:?}",
        events
    );

    // Verify the Usage event contains cache tokens
    for event in usage_events {
        match event {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            } => {
                assert_eq!(*input_tokens, 50, "input_tokens should be 50");
                assert_eq!(*output_tokens, 0, "output_tokens should be 0");
                assert_eq!(
                    *cache_creation_input_tokens,
                    None,
                    "cache_creation_input_tokens should be None"
                );
                assert_eq!(
                    *cache_read_input_tokens,
                    Some(500),
                    "cache_read_input_tokens should be Some(500)"
                );
            }
            _ => unreachable!("We filtered for Usage events"),
        }
    }
}

/// Test that Usage events are structured and machine-parseable (JSON serializable).
#[test]
async fn test_usage_event_is_json_serializable() {
    let event = StreamEvent::Usage {
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_input_tokens: Some(200),
        cache_read_input_tokens: Some(300),
    };

    // Serialize to JSON
    let json = serde_json::to_string(&event).expect("Usage event should be serializable");

    // Verify it contains the expected fields (serde tag format)
    assert!(json.contains("\"type\":\"Usage\""), "JSON should have type: Usage");
    assert!(json.contains("\"input_tokens\":100"), "JSON should have input_tokens: 100");
    assert!(json.contains("\"output_tokens\":50"), "JSON should have output_tokens: 50");
    assert!(
        json.contains("\"cache_creation_input_tokens\":200"),
        "JSON should have cache_creation_input_tokens: 200"
    );
    assert!(
        json.contains("\"cache_read_input_tokens\":300"),
        "JSON should have cache_read_input_tokens: 300"
    );

    // Deserialize back
    let deserialized: StreamEvent =
        serde_json::from_str(&json).expect("Usage event should be deserializable");

    match deserialized {
        StreamEvent::Usage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
        } => {
            assert_eq!(input_tokens, 100);
            assert_eq!(output_tokens, 50);
            assert_eq!(cache_creation_input_tokens, Some(200));
            assert_eq!(cache_read_input_tokens, Some(300));
        }
        _ => panic!("Expected Usage event after deserialization"),
    }
}

/// Test that cache_creation_tokens without cache_read_tokens is handled correctly.
#[test]
async fn test_cache_creation_tokens_only() {
    use mockito::Server;

    let mut server = Server::new_async().await;

    // Mock response with only cache_creation_input_tokens (cache miss creating new cache)
    let mock_response = r#"event: content_block_start
data: {"type":"text","index":0}

event: content_block_delta
data: {"type":"text_delta","text":"Response","index":0}

event: message_delta
data: {"type":"message_delta","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":500}}

event: message_stop
data: {"type":"message_stop"}
"#;

    let m = server
        .mock("POST", "/v1/messages")
        .with_body(mock_response)
        .create();

    let backend = AnthropicBackend::with_base_url(
        "claude-sonnet-4-20250514",
        "test-api-key",
        server.url(),
    );

    let messages = vec![LlmMessage {
        role: LlmRole::User,
        content: "Hello".to_string(),
        ..Default::default()
    }];

    let config = LlmConfig::default();

    let stream = backend
        .stream(messages, None, config)
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    // Find the Usage event
    let usage_events: Vec<&StreamEvent> = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Usage { .. }))
        .collect();

    assert!(
        !usage_events.is_empty(),
        "Expected at least one Usage event, got: {:?}",
        events
    );

    for event in usage_events {
        match event {
            StreamEvent::Usage {
                input_tokens: _,
                output_tokens: _,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            } => {
                assert_eq!(
                    *cache_creation_input_tokens,
                    Some(500),
                    "cache_creation_input_tokens should be Some(500)"
                );
                assert_eq!(
                    *cache_read_input_tokens,
                    None,
                    "cache_read_input_tokens should be None"
                );
            }
            _ => unreachable!("We filtered for Usage events"),
        }
    }
}
