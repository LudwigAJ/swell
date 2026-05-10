//! Tests for cache token event emission.
//!
//! These tests verify that cache_creation_tokens and cache_read_tokens are
//! emitted as first-class structured observability events through the
//! StreamEvent::Usage type.

use futures::StreamExt;
use swell_core::traits::LlmBackend;
use swell_core::StreamEvent;
use swell_llm::{AnthropicBackend, LlmConfig, LlmMessage, LlmRole};
use tokio::test;

const SSE_CONTENT_TYPE: &str = "text/event-stream";

fn make_messages() -> Vec<LlmMessage> {
    vec![LlmMessage {
        role: LlmRole::User,
        content: "Hello".to_string(),
        ..Default::default()
    }]
}

/// Test that cache_creation_tokens and cache_read_tokens are emitted as structured events
/// when present in the SSE stream from Anthropic API.
#[test]
async fn test_cache_tokens_emitted_as_usage_event() {
    use mockito::Server;

    let mut server = Server::new_async().await;

    let mock_response = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":100,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":100,\"output_tokens\":50,\"cache_creation_input_tokens\":200,\"cache_read_input_tokens\":300}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let m = server
        .mock("POST", "/v1/messages")
        .with_header("content-type", SSE_CONTENT_TYPE)
        .with_body(mock_response)
        .create();

    let backend =
        AnthropicBackend::with_base_url("claude-sonnet-4-20250514", "test-api-key", server.url());

    let stream = backend
        .stream(make_messages(), None, LlmConfig::default())
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    let usage = events
        .iter()
        .find_map(|e| match e {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            } if cache_creation_input_tokens.is_some() => Some((
                *input_tokens,
                *output_tokens,
                *cache_creation_input_tokens,
                *cache_read_input_tokens,
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Expected Usage event with cache tokens, got: {events:?}"));

    assert_eq!(usage.0, 100);
    assert_eq!(usage.1, 50);
    assert_eq!(usage.2, Some(200));
    assert_eq!(usage.3, Some(300));
}

/// Test that cache tokens are emitted even when there are no output tokens.
/// This can happen with cached responses where only cache tokens are counted.
#[test]
async fn test_cache_tokens_emitted_without_output_tokens() {
    use mockito::Server;

    let mut server = Server::new_async().await;

    let mock_response = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":50,\"output_tokens\":0,\"cache_read_input_tokens\":500}}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":50,\"output_tokens\":0,\"cache_read_input_tokens\":500}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let m = server
        .mock("POST", "/v1/messages")
        .with_header("content-type", SSE_CONTENT_TYPE)
        .with_body(mock_response)
        .create();

    let backend =
        AnthropicBackend::with_base_url("claude-sonnet-4-20250514", "test-api-key", server.url());

    let stream = backend
        .stream(make_messages(), None, LlmConfig::default())
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    let usage = events
        .iter()
        .find_map(|e| match e {
            StreamEvent::Usage {
                cache_read_input_tokens: Some(_),
                ..
            } => Some(e),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Expected Usage event with cache_read tokens, got: {events:?}"));

    if let StreamEvent::Usage {
        input_tokens,
        output_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    } = usage
    {
        assert_eq!(*input_tokens, 50);
        assert_eq!(*output_tokens, 0);
        assert_eq!(*cache_creation_input_tokens, None);
        assert_eq!(*cache_read_input_tokens, Some(500));
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

    let json = serde_json::to_string(&event).expect("Usage event should be serializable");

    assert!(json.contains("\"type\":\"Usage\""));
    assert!(json.contains("\"input_tokens\":100"));
    assert!(json.contains("\"output_tokens\":50"));
    assert!(json.contains("\"cache_creation_input_tokens\":200"));
    assert!(json.contains("\"cache_read_input_tokens\":300"));

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

    let mock_response = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":100,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Response\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":100,\"output_tokens\":20,\"cache_creation_input_tokens\":500}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

    let m = server
        .mock("POST", "/v1/messages")
        .with_header("content-type", SSE_CONTENT_TYPE)
        .with_body(mock_response)
        .create();

    let backend =
        AnthropicBackend::with_base_url("claude-sonnet-4-20250514", "test-api-key", server.url());

    let stream = backend
        .stream(make_messages(), None, LlmConfig::default())
        .await
        .expect("stream should succeed");

    let events: Vec<StreamEvent> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    m.assert();

    let usage = events
        .iter()
        .find_map(|e| match e {
            StreamEvent::Usage {
                cache_creation_input_tokens: Some(_),
                ..
            } => Some(e),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!("Expected Usage event with cache_creation tokens, got: {events:?}")
        });

    if let StreamEvent::Usage {
        cache_creation_input_tokens,
        cache_read_input_tokens,
        ..
    } = usage
    {
        assert_eq!(*cache_creation_input_tokens, Some(500));
        assert_eq!(*cache_read_input_tokens, None);
    }
}
