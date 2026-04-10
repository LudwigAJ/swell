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
//! - `root_trace_id`: Propagated trace ID linking all related spans across tasks/agents
//! - `cross_task_correlation_id`: Links events across multiple related tasks
//! - `agent_session_id`: Unique session ID for each agent instance
//! - `request_id`: Unique ID for each API call
//! - `agent_id`: Identifier of the agent that generated this event
//! - `session_id`: Identifier of the session grouping related events
//! - `task_id`: Identifier of the task this event relates to
//! - `tool_invocation`: Information about tool usage (if applicable)
//! - `timestamp`: When the event occurred
//! - `outcome`: Result of the operation (success/failure/error)
//!
//! # Correlation IDs
//!
//! The system uses several correlation IDs for different purposes:
//! - `root_trace_id`: Propagated through the entire trace hierarchy
//! - `cross_task_correlation_id`: Groups related tasks (e.g., a feature split into sub-tasks)
//! - `agent_session_id`: Unique per-agent-instance session for tracking agent lifecycle
//! - `request_id`: Unique per external API call for tracing HTTP requests

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
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

/// Cross-task correlation ID for linking multiple related tasks.
///
/// When a parent task spawns multiple sub-tasks (e.g., feature decomposition),
/// all related tasks share the same cross_task_correlation_id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrossTaskCorrelationId(pub Uuid);

impl CrossTaskCorrelationId {
    /// Create a new cross-task correlation ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Get the underlying UUID
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Nil UUID for unset
    pub fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Check if this is a nil (unset) correlation ID
    pub fn is_nil(&self) -> bool {
        self.0 == Uuid::nil()
    }
}

impl std::fmt::Display for CrossTaskCorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Agent session ID for tracking individual agent instances.
///
/// Each agent instance gets a unique session ID when created,
/// allowing correlation of all events from a specific agent instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentSessionId(pub Uuid);

impl AgentSessionId {
    /// Create a new agent session ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Get the underlying UUID
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Nil UUID for unset
    pub fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Check if this is a nil (unset) session ID
    pub fn is_nil(&self) -> bool {
        self.0 == Uuid::nil()
    }
}

impl std::fmt::Display for AgentSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Request ID for tracking individual API calls.
///
/// Each external API call (LLM request, HTTP request, etc.) gets a unique
/// request ID for tracing that specific operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestId(pub Uuid);

impl RequestId {
    /// Create a new request ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Get the underlying UUID
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Nil UUID for unset
    pub fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Check if this is a nil (unset) request ID
    pub fn is_nil(&self) -> bool {
        self.0 == Uuid::nil()
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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
    pub fn new(
        tool_name: String,
        arguments: serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Self {
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
    /// Root trace ID propagated through the entire trace hierarchy
    /// This allows tracing across all related spans even when trace_id changes
    pub root_trace_id: TraceId,
    /// Cross-task correlation ID linking events across multiple related tasks
    pub cross_task_correlation_id: CrossTaskCorrelationId,
    /// Agent session ID unique to each agent instance
    pub agent_session_id: AgentSessionId,
    /// Request ID for this specific API call
    pub request_id: RequestId,
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
    /// Create a new observable event with all correlation IDs
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trace_id: TraceId,
        span_id: SpanId,
        root_trace_id: TraceId,
        cross_task_correlation_id: CrossTaskCorrelationId,
        agent_session_id: AgentSessionId,
        request_id: RequestId,
        agent_id: Uuid,
        session_id: Uuid,
        task_id: Uuid,
        outcome: Outcome,
    ) -> Self {
        Self {
            trace_id,
            span_id,
            parent_span_id: None,
            root_trace_id,
            cross_task_correlation_id,
            agent_session_id,
            request_id,
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
        let root_trace_id = trace_id.clone();
        Self::new(
            trace_id,
            span_id,
            root_trace_id,
            CrossTaskCorrelationId::new(),
            AgentSessionId::new(),
            RequestId::new(),
            agent_id,
            session_id,
            task_id,
            outcome,
        )
    }

    /// Create an event with a specific root_trace_id for propagation
    pub fn with_root_trace(
        root_trace_id: TraceId,
        agent_id: Uuid,
        session_id: Uuid,
        task_id: Uuid,
        outcome: Outcome,
    ) -> Self {
        let trace_id = root_trace_id.clone();
        let span_id = SpanId::generate();
        Self::new(
            trace_id,
            span_id,
            root_trace_id,
            CrossTaskCorrelationId::new(),
            AgentSessionId::new(),
            RequestId::new(),
            agent_id,
            session_id,
            task_id,
            outcome,
        )
    }

    /// Start a new child span under this event's span, propagating root_trace_id
    pub fn start_child_span(&self) -> (SpanId, ObservableEvent) {
        let child_span_id = SpanId::generate();
        let child_event = ObservableEvent {
            trace_id: self.trace_id.clone(),
            span_id: child_span_id.clone(),
            parent_span_id: Some(self.span_id.clone()),
            root_trace_id: self.root_trace_id.clone(),
            cross_task_correlation_id: self.cross_task_correlation_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            request_id: RequestId::new(), // New request ID for child span
            agent_id: self.agent_id,
            session_id: self.session_id,
            task_id: self.task_id,
            tool_invocation: None,
            timestamp: Utc::now(),
            outcome: Outcome::Success,
        };
        (child_span_id, child_event)
    }

    /// Propagate this event's root_trace_id to a new event (for cross-agent/cross-task correlation)
    pub fn propagate_root_trace(&self, task_id: Uuid) -> ObservableEvent {
        let span_id = SpanId::generate();
        ObservableEvent {
            trace_id: self.root_trace_id.clone(),
            span_id,
            parent_span_id: None,
            root_trace_id: self.root_trace_id.clone(),
            cross_task_correlation_id: self.cross_task_correlation_id.clone(),
            agent_session_id: AgentSessionId::new(), // New agent session for new context
            request_id: RequestId::new(),
            agent_id: self.agent_id,
            session_id: self.session_id,
            task_id,
            tool_invocation: None,
            timestamp: Utc::now(),
            outcome: Outcome::Success,
        }
    }
}

// ============================================================================
// Event Store for Correlation ID Querying
// ============================================================================

/// In-memory event store for storing and querying events by correlation IDs.
///
/// This store maintains indexes for efficient querying by:
/// - root_trace_id: Find all events in a trace hierarchy
/// - cross_task_correlation_id: Find all events across related tasks
/// - agent_session_id: Find all events for a specific agent session
/// - request_id: Find a specific event by its request ID
/// - task_id: Find all events for a specific task
#[derive(Debug, Default)]
pub struct EventStore {
    events: Vec<ObservableEvent>,
    by_root_trace_id: HashMap<String, Vec<usize>>,
    by_cross_task_correlation_id: HashMap<String, Vec<usize>>,
    by_agent_session_id: HashMap<String, Vec<usize>>,
    by_request_id: HashMap<String, Vec<usize>>,
    by_task_id: HashMap<String, Vec<usize>>,
}

impl EventStore {
    /// Create a new empty event store
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an event to the store
    pub fn add(&mut self, event: ObservableEvent) {
        let idx = self.events.len();
        self.events.push(event.clone());

        // Index by root_trace_id
        self.by_root_trace_id
            .entry(event.root_trace_id.as_str().to_string())
            .or_default()
            .push(idx);

        // Index by cross_task_correlation_id
        self.by_cross_task_correlation_id
            .entry(event.cross_task_correlation_id.as_uuid().to_string())
            .or_default()
            .push(idx);

        // Index by agent_session_id
        self.by_agent_session_id
            .entry(event.agent_session_id.as_uuid().to_string())
            .or_default()
            .push(idx);

        // Index by request_id
        self.by_request_id
            .entry(event.request_id.as_uuid().to_string())
            .or_default()
            .push(idx);

        // Index by task_id
        self.by_task_id
            .entry(event.task_id.to_string())
            .or_default()
            .push(idx);
    }

    /// Get all events
    pub fn all(&self) -> &[ObservableEvent] {
        &self.events
    }

    /// Get events by root_trace_id (trace hierarchy)
    pub fn by_root_trace_id(&self, root_trace_id: &TraceId) -> Vec<&ObservableEvent> {
        self.by_root_trace_id
            .get(root_trace_id.as_str())
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Get events by cross_task_correlation_id (multi-task flows)
    pub fn by_cross_task_correlation_id(
        &self,
        correlation_id: &CrossTaskCorrelationId,
    ) -> Vec<&ObservableEvent> {
        self.by_cross_task_correlation_id
            .get(&correlation_id.as_uuid().to_string())
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Get events by agent_session_id
    pub fn by_agent_session_id(&self, session_id: &AgentSessionId) -> Vec<&ObservableEvent> {
        self.by_agent_session_id
            .get(&session_id.as_uuid().to_string())
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Get event by request_id
    pub fn by_request_id(&self, request_id: &RequestId) -> Option<&ObservableEvent> {
        self.by_request_id
            .get(&request_id.as_uuid().to_string())
            .and_then(|indices| indices.first().map(|&i| &self.events[i]))
    }

    /// Get events by task_id
    pub fn by_task_id(&self, task_id: &Uuid) -> Vec<&ObservableEvent> {
        self.by_task_id
            .get(&task_id.to_string())
            .map(|indices| indices.iter().map(|&i| &self.events[i]).collect())
            .unwrap_or_default()
    }

    /// Get the number of events in the store
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
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

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);

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

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success)
                .with_parent_span(parent_span_id.clone());

        assert!(event.parent_span_id.is_some());
        assert_eq!(
            event.parent_span_id.unwrap().as_str(),
            parent_span_id.as_str()
        );
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

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success)
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

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);

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
        assert_eq!(
            child_event.parent_span_id.unwrap().as_str(),
            event.span_id.as_str()
        );
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

    // =========================================================================
    // Correlation ID Tests
    // =========================================================================

    #[test]
    fn test_cross_task_correlation_id() {
        let id = CrossTaskCorrelationId::new();
        assert!(!id.is_nil());
        assert_ne!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_cross_task_correlation_id_nil() {
        let id = CrossTaskCorrelationId::nil();
        assert!(id.is_nil());
        assert_eq!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_cross_task_correlation_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let id = CrossTaskCorrelationId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn test_cross_task_correlation_id_display() {
        let id = CrossTaskCorrelationId::new();
        let display = format!("{}", id);
        assert_eq!(display.len(), 36); // UUID string length
    }

    #[test]
    fn test_agent_session_id() {
        let id = AgentSessionId::new();
        assert!(!id.is_nil());
        assert_ne!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_agent_session_id_nil() {
        let id = AgentSessionId::nil();
        assert!(id.is_nil());
        assert_eq!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_request_id() {
        let id = RequestId::new();
        assert!(!id.is_nil());
        assert_ne!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_request_id_nil() {
        let id = RequestId::nil();
        assert!(id.is_nil());
        assert_eq!(id.as_uuid(), Uuid::nil());
    }

    #[test]
    fn test_root_trace_id_propagation() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);

        // root_trace_id should equal trace_id for initial event
        assert_eq!(event.root_trace_id.as_str(), event.trace_id.as_str());

        // Child span should propagate root_trace_id
        let (_child_span_id, child_event) = event.start_child_span();
        assert_eq!(
            child_event.root_trace_id.as_str(),
            event.root_trace_id.as_str()
        );
    }

    #[test]
    fn test_propagate_root_trace_for_new_task() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();

        let event1 = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id_1,
            Outcome::Success,
        );

        // Propagate to a new task (simulating cross-task flow)
        let event2 = event1.propagate_root_trace(task_id_2);

        // root_trace_id should be the same
        assert_eq!(event2.root_trace_id.as_str(), event1.root_trace_id.as_str());

        // cross_task_correlation_id should be the same
        assert_eq!(
            event2.cross_task_correlation_id.as_uuid(),
            event1.cross_task_correlation_id.as_uuid()
        );

        // But task_id should be different
        assert_eq!(event2.task_id, task_id_2);
        assert_ne!(event2.task_id, task_id_1);

        // Agent session should be NEW (different)
        assert_ne!(
            event2.agent_session_id.as_uuid(),
            event1.agent_session_id.as_uuid()
        );
    }

    #[test]
    fn test_request_id_unique_per_span() {
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);

        let (_child_span_id, child_event) = event.start_child_span();

        // Each span should have its own request_id
        assert_ne!(child_event.request_id.as_uuid(), event.request_id.as_uuid());
    }

    #[test]
    fn test_correlation_ids_in_event_serialization() {
        let event = ObservableEvent::with_generated_trace(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Outcome::Success,
        );

        let json = serde_json::to_string(&event).unwrap();

        // Verify correlation ID fields are present
        assert!(json.contains("root_trace_id"));
        assert!(json.contains("cross_task_correlation_id"));
        assert!(json.contains("agent_session_id"));
        assert!(json.contains("request_id"));
    }

    #[test]
    fn test_cross_task_correlation_id_serialization() {
        let id = CrossTaskCorrelationId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        // UUID stored as string
        assert!(parsed.as_str().is_some() || parsed.get("0").is_some());
    }

    // =========================================================================
    // EventStore Tests
    // =========================================================================

    #[test]
    fn test_event_store_empty() {
        let store = EventStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_event_store_add_and_query_by_root_trace_id() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);
        let root_trace_id = event.root_trace_id.clone();

        let (_child_span_id, child_event) = event.start_child_span();

        store.add(event);
        store.add(child_event);

        let events = store.by_root_trace_id(&root_trace_id);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_store_query_by_cross_task_correlation_id() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();

        let event1 = ObservableEvent::with_generated_trace(
            agent_id,
            session_id,
            task_id_1,
            Outcome::Success,
        );
        let correlation_id = event1.cross_task_correlation_id.clone();

        // Create another event in the same cross-task flow
        let event2 = event1.propagate_root_trace(task_id_2);

        store.add(event1);
        store.add(event2);

        let events = store.by_cross_task_correlation_id(&correlation_id);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_store_query_by_agent_session_id() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);
        let agent_session_id = event.agent_session_id.clone();

        let (_child_span_id, child_event) = event.start_child_span();

        store.add(event);
        store.add(child_event);

        let events = store.by_agent_session_id(&agent_session_id);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_store_query_by_request_id() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);
        let request_id = event.request_id.clone();

        store.add(event);

        let found = store.by_request_id(&request_id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().request_id.as_uuid(), request_id.as_uuid());
    }

    #[test]
    fn test_event_store_query_by_task_id() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let event =
            ObservableEvent::with_generated_trace(agent_id, session_id, task_id, Outcome::Success);

        let (_child_span_id, child_event) = event.start_child_span();

        store.add(event);
        store.add(child_event);

        let events = store.by_task_id(&task_id);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_event_store_all() {
        let mut store = EventStore::new();
        let agent_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        for _ in 0..5 {
            let event = ObservableEvent::with_generated_trace(
                agent_id,
                session_id,
                task_id,
                Outcome::Success,
            );
            store.add(event);
        }

        assert_eq!(store.len(), 5);
        assert_eq!(store.all().len(), 5);
    }

    #[test]
    fn test_event_store_query_non_existent() {
        let store = EventStore::new();

        let events = store.by_root_trace_id(&TraceId::generate());
        assert!(events.is_empty());

        let events = store.by_task_id(&Uuid::new_v4());
        assert!(events.is_empty());
    }
}
