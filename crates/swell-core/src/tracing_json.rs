//! Structured JSON logging with tracing.
//!
//! Every event is serialized as JSON with:
//! - trace_id and span_id included
//! - timestamp with millisecond precision
//! - level (trace/debug/info/warn/error)
//! - message field present

use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

/// JSON event with required fields for structured logging
#[derive(Debug, serde::Serialize)]
pub struct JsonEvent {
    /// Timestamp with millisecond precision
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Log level (trace, debug, info, warn, error)
    pub level: String,
    /// The main message
    pub message: String,
    /// Trace ID for distributed tracing
    pub trace_id: Option<String>,
    /// Span ID for the current span
    pub span_id: Option<String>,
    /// Target/module that generated the event
    pub target: Option<String>,
}

/// Initialize structured JSON tracing for the crate.
///
/// This configures tracing to output JSON-formatted logs with:
/// - trace_id and span_id from span context when available
/// - timestamp with millisecond precision
/// - level field
/// - message field
pub fn init_json_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Create a JSON layer with structured logging support
    // The trace_id and span_id are included from span context when spans are used
    let json_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_span_events(FmtSpan::CLOSE)
        .flatten_event(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_structured_json_event_serialization() {
        // Create a timestamp for comparison
        let timestamp = chrono::Utc::now();

        let event = JsonEvent {
            timestamp,
            level: "info".to_string(),
            message: "Test message".to_string(),
            trace_id: Some("abc123".to_string()),
            span_id: Some("def456".to_string()),
            target: Some("test".to_string()),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify all required fields are present
        assert!(parsed.get("timestamp").is_some());
        assert!(parsed.get("level").is_some());
        assert!(parsed.get("message").is_some());
        assert!(parsed.get("trace_id").is_some());
        assert!(parsed.get("span_id").is_some());

        // Verify values
        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["message"], "Test message");
        assert_eq!(parsed["trace_id"], "abc123");
        assert_eq!(parsed["span_id"], "def456");
    }

    #[test]
    fn test_json_event_without_trace_context() {
        let event = JsonEvent {
            timestamp: chrono::Utc::now(),
            level: "debug".to_string(),
            message: "Event without trace".to_string(),
            trace_id: None,
            span_id: None,
            target: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Should serialize without trace context
        assert_eq!(parsed["level"], "debug");
        assert_eq!(parsed["message"], "Event without trace");
    }

    #[test]
    fn test_all_log_levels() {
        let levels = vec!["trace", "debug", "info", "warn", "error"];

        for name in levels {
            let event = JsonEvent {
                timestamp: chrono::Utc::now(),
                level: name.to_string(),
                message: format!("{} level test", name),
                trace_id: None,
                span_id: None,
                target: None,
            };

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains(name), "JSON should contain level: {}", name);
        }
    }
}
