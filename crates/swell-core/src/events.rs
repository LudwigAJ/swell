//! Observable event schema for OpenTelemetry-compatible distributed tracing.
//!
//! This module defines the event schema used across SWELL for observability,
//! following OpenTelemetry semantic conventions for traces and spans.
//!
//! # Event Schema
//!
//! Every observable event includes:
//! - `trace_id`: Unique identifier for the distributed trace
//! - `span_id`: Unique identifier for this span
//! - `parent_span_id`: Identifier of the parent span (if any)
//! - `agent_id`: Identifier of the agent that generated this event
//! - `session_id`: Identifier of the session grouping related events
//! - `task_id`: Identifier of the task this event relates to
//! - `tool_invocation`: Information about tool usage (if applicable)
//! - `timestamp`: When the event occurred
//! - `outcome`: Result of the operation (success/failure/error)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Core Event Schema Types
// ============================================================================

/// A unique trace identifier (32 hex characters for OpenTelemetry compatibility)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TraceId(pub String);

impl TraceId {
    /// Create a new trace ID from a 32-character hex string
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(Self(hex.to_uppercase()))
        } else {
            None
        }
    }

    /// Generate a new random trace ID
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string().replace("-", "").to_uppercase())
    }

    /// Returns the hex string representation
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TraceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A unique span identifier (16 hex characters for OpenTelemetry compatibility)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpanId(pub String);

impl SpanId {
    /// Create a new span ID from a 16-character hex string
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() == 16 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(Self(hex.to_uppercase()))
        } else {
            None
        }
    }

    /// Generate a new random span ID
    pub fn generate() -> Self {
        // Span ID is 8 bytes = 16 hex chars
        let bytes: [u8; 8] = rand_u64().to_be_bytes();
        Self(
            bytes
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<String>(),
        )
    }

    /// Returns the hex string representation
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

impl std::fmt::Display for SpanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The outcome of an observable event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The operation completed successfully
    Success,
    /// The operation failed
    Failure,
    /// An error occurred during execution
    Error,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Success => write!(f, "success"),
            Outcome::Failure => write!(f, "failure"),
            Outcome::Error => write!(f, "error"),
        }
    }
}

/// Information about a tool invocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    /// Name of the tool that was invoked
    pub tool_name: String,
    /// Arguments passed to the tool
    pub arguments: serde_json::Value,
    /// Whether the tool executed successfully
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

impl ToolInvocation {
    /// Create a new tool invocation record
    pub fn new(tool_name: String, arguments: serde_json::Value, success: bool, duration_ms: u64) -> Self {
        Self {
            tool_name,
            arguments,
            success,
            duration_ms,
        }
    }
}

/// The observable event schema for SWELL.
///
/// This struct defines the standard event format used across all
/// components for observability and distributed tracing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableEvent {
    /// Unique identifier for the distributed trace (32 hex chars)
    pub trace_id: TraceId,
    /// Unique identifier for this span (16 hex chars)
    pub span_id: SpanId,
    /// Identifier of the parent span (if any)
    pub parent_span_id: Option<SpanId>,
    /// Identifier of the agent that generated this event
    pub agent_id: Uuid,
    /// Identifier of the session grouping related events
    pub session_id: Uuid,
    /// Identifier of the task this event relates to
    pub task_id: Uuid,
    /// Information about tool usage (if applicable)
    pub tool_invocation: Option<ToolInvocation>,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Result of the operation
    pub outcome: Outcome,
}

impl ObservableEvent {
    /// Create a new observable event
    pub fn new(
        trace_id: TraceId,
        span_id: SpanId,
        agent_id: Uuid,
        session_id: Uuid,
        task_id: Uuid,
        outcome: Outcome,
    ) -> Self {
        Self {
            trace_id,
            span_id,
            parent_span_id: None,
            agent_id,
            session_id,
            task_id,
            tool_invocation: None,
            timestamp: Utc::now(),
            outcome,
        }
    }

    /// Create a new event with a parent span
    pub fn with_parent_span(mut self, parent_span_id: SpanId) -> Self {
        self.parent_span_id = Some(parent_span_id);
        self
    }

    /// Create a new event with tool invocation details
    pub fn with_tool_invocation(mut self, tool_invocation: ToolInvocation) -> Self {
        self.tool_invocation = Some(tool_invocation);
        self
    }

    /// Generate a new trace ID and create an event with it
    pub fn with_generated_trace(
        agent_id: Uuid,
        session_id: Uuid,
        task_id: Uuid,
        outcome: Outcome,
    ) -> Self {
        let trace_id = TraceId::generate();
        let span_id = SpanId::generate();
        Self::new(trace_id, span_id, agent_id, session_id, task_id, outcome)
    }

    /// Start a new child span under this event's span
    pub fn start_child_span(&self) -> (SpanId, ObservableEvent) {
        let child_span_id = SpanId::generate();
        let child_event = ObservableEvent {
            trace_id: self.trace_id.clone(),
            span_id: child_span_id.clone(),
            parent_span_id: Some(self.span_id.clone()),
            agent_id: self.agent_id,
            session_id: self.session_id,
            task_id: self.task_id,
            tool_invocation: None,
            timestamp: Utc::now(),
            outcome: Outcome::Success,
        };
        (child_span_id, child_event)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_id_generation() {
        let trace_id = TraceId::generate();
        assert_eq!(trace_id.as_str().len(), 32);
        assert!(trace_id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_trace_id_from_hex_valid() {
        let trace_id = TraceId::from_hex("A1B2C3D4E5F60718293A0B1C2D3E4F50");
        assert!(trace_id.is_some());
        let trace_id = trace_id.unwrap();
        assert_eq!(trace_id.as_str(), "A1B2C3D4E5F60718293A0B1C2D3E4F50");
    }

    #[test]
    fn test_trace_id_from_hex_invalid() {
        // Too short
        assert!(TraceId::from_hex("A1B2").is_none());
        // Too long
        assert!(TraceId::from_hex(&"A".repeat(33)).is_none());
        // Invalid characters
        assert!(TraceId::from_hex("G1B2C3D4E5F60718293A0B1C2D3E4F50").is_none());
    }

    #[test]
    fn test_span_id_generation() {
        let span_id = SpanId::generate();
        assert_eq!(span_id.as_str().len(), 16);
        assert!(span_id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_span_id_from_hex_valid() {
        let span_id = SpanId::from_hex("A1B2C3D4E5F60718");
        assert!(span_id.is_some());
        let span_id = span_id.unwrap();
        assert_eq!(span_id.as_str(), "A1B2C3D4E5F60718");
    }

    #[test]
    fn test_span_id_from_hex_invalid() {
        // Too short
        assert!(SpanId::from_hex("A1B2").is_none());
        // Too long
        assert!(SpanId::from_hex(&"A".repeat(17)).is_none());
    }

    #[test]
    fn test_observable_event_creation() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id,
            Outcome::Success,
        );

        assert_eq!(event.agent_id, agent_id);
        assert_eq!(event.session_id, session_id);
        assert_eq!(event.task_id, task_id);
        assert_eq!(event.outcome, Outcome::Success);
        assert!(event.parent_span_id.is_none());
        assert!(event.tool_invocation.is_none());
    }

    #[test]
    fn test_observable_event_with_parent() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let parent_span_id = SpanId::generate();

        let event = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id,
            Outcome::Success,
        )
        .with_parent_span(parent_span_id.clone());

        assert!(event.parent_span_id.is_some());
        assert_eq!(event.parent_span_id.unwrap().as_str(), parent_span_id.as_str());
    }

    #[test]
    fn test_observable_event_with_tool_invocation() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let tool_invocation = ToolInvocation::new(
            "file_read".to_string(),
            serde_json::json!({"path": "/test/file.txt"}),
            true,
            50,
        );

        let event = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id,
            Outcome::Success,
        )
        .with_tool_invocation(tool_invocation.clone());

        assert!(event.tool_invocation.is_some());
        let stored_tool = event.tool_invocation.unwrap();
        assert_eq!(stored_tool.tool_name, "file_read");
        assert!(stored_tool.success);
        assert_eq!(stored_tool.duration_ms, 50);
    }

    #[test]
    fn test_child_span_creation() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id,
            Outcome::Success,
        );

        let (child_span_id, child_event) = event.start_child_span();

        // Child should have different span ID
        assert_ne!(child_span_id.as_str(), event.span_id.as_str());

        // Child should inherit trace, agent, session, task IDs
        assert_eq!(child_event.trace_id.as_str(), event.trace_id.as_str());
        assert_eq!(child_event.agent_id, event.agent_id);
        assert_eq!(child_event.session_id, event.session_id);
        assert_eq!(child_event.task_id, event.task_id);

        // Child should have parent set to original span
        assert!(child_event.parent_span_id.is_some());
        assert_eq!(child_event.parent_span_id.unwrap().as_str(), event.span_id.as_str());
    }

    #[test]
    fn test_outcome_serialization() {
        let event = ObservableEvent::with_generated_trace(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Outcome::Success,
        );

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"outcome\":\"success\""));
    }

    #[test]
    fn test_observable_event_serialization() {
        let event = ObservableEvent::with_generated_trace(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Outcome::Success,
        );

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify all required fields are present
        assert!(parsed.get("trace_id").is_some());
        assert!(parsed.get("span_id").is_some());
        assert!(parsed.get("agent_id").is_some());
        assert!(parsed.get("session_id").is_some());
        assert!(parsed.get("task_id").is_some());
        assert!(parsed.get("timestamp").is_some());
        assert!(parsed.get("outcome").is_some());
    }

    #[test]
    fn test_outcome_display() {
        assert_eq!(Outcome::Success.to_string(), "success");
        assert_eq!(Outcome::Failure.to_string(), "failure");
        assert_eq!(Outcome::Error.to_string(), "error");
    }
}
