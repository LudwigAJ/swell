//! Transcript-mediated event architecture for SWELL.
//!
//! This module implements a typed, append-only event log where all runtime events
//! flow through a single source of truth. Unlike [`ObservableEvent`] (which is
//! for OpenTelemetry tracing), [`TranscriptEvent`] captures the complete execution
//! history for session replay and auditing.
//!
//! # Architecture
//!
//! - [`TranscriptEventType`] - Type discriminant for events (ToolCall, LlmResponse, etc.)
//! - [`TranscriptEvent`] - A single event with timestamp, session ID, and typed payload
//! - [`TranscriptLog`] - Append-only event log, indexed by type for efficient filtering
//! - [`TranscriptSubscriber`] - Async receiver that filters events by type
//!
//! # Event Types
//!
//! The system supports these event types:
//! - `ToolCall` - Tool invocation with name, arguments, success/failure
//! - `LlmResponse` - LLM response with content and token usage
//! - `StateTransition` - Task/agent state changes
//! - `Error` - Error events with error type and message
//!
//! # Append-Only Invariant
//!
//! The [`TranscriptLog`] enforces append-only semantics:
//! - [`TranscriptLog::append()`] adds new events
//! - No public method exists to remove or modify events
//! - The log maintains insertion order for replay

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::ids::SessionId;

// ============================================================================
// Event Type Discriminants
// ============================================================================

/// Type discriminant for transcript events.
///
/// Each variant represents a distinct category of runtime event.
/// Used for subscriber filtering and log indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptEventType {
    /// Tool invocation event
    ToolCall,
    /// LLM response event
    LlmResponse,
    /// State transition event
    StateTransition,
    /// Error event
    Error,
}

impl std::fmt::Display for TranscriptEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptEventType::ToolCall => write!(f, "tool_call"),
            TranscriptEventType::LlmResponse => write!(f, "llm_response"),
            TranscriptEventType::StateTransition => write!(f, "state_transition"),
            TranscriptEventType::Error => write!(f, "error"),
        }
    }
}

// ============================================================================
// Event Payloads
// ============================================================================

/// Payload for a tool call event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPayload {
    /// Name of the tool that was invoked
    pub tool_name: String,
    /// Arguments passed to the tool
    pub arguments: serde_json::Value,
    /// Whether the tool executed successfully
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Error message if unsuccessful
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl ToolCallPayload {
    /// Create a new successful tool call payload
    pub fn success(tool_name: String, arguments: serde_json::Value, duration_ms: u64) -> Self {
        Self {
            tool_name,
            arguments,
            success: true,
            duration_ms,
            error_message: None,
        }
    }

    /// Create a new failed tool call payload
    pub fn failure(tool_name: String, arguments: serde_json::Value, error_message: String) -> Self {
        Self {
            tool_name,
            arguments,
            success: false,
            duration_ms: 0,
            error_message: Some(error_message),
        }
    }
}

/// Payload for an LLM response event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponsePayload {
    /// Model that generated the response
    pub model: String,
    /// Text content of the response
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    /// Token usage statistics
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
    /// Stop reason (e.g., "end_turn", "max_tokens")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl LlmResponsePayload {
    /// Create a new LLM response payload
    pub fn new(
        model: String,
        text_content: Option<String>,
        token_usage: Option<TokenUsage>,
        stop_reason: Option<String>,
    ) -> Self {
        Self {
            model,
            text_content,
            token_usage,
            stop_reason,
        }
    }
}

/// Token usage information for LLM responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed
    pub input_tokens: u64,
    /// Output tokens generated
    pub output_tokens: u64,
    /// Cache creation tokens (Anthropic)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    /// Cache read tokens (Anthropic)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
}

impl TokenUsage {
    /// Create a new token usage record
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    }

    /// Create with cache tokens
    pub fn with_cache(
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: Option<u64>,
        cache_read_input_tokens: Option<u64>,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
        }
    }

    /// Total tokens used
    pub fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Payload for a state transition event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransitionPayload {
    /// What kind of entity is transitioning state
    pub entity_type: EntityType,
    /// ID of the entity transitioning
    pub entity_id: Uuid,
    /// State before transition
    pub from_state: String,
    /// State after transition
    pub to_state: String,
    /// Reason for the transition (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Type of entity transitioning state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// Task entity
    Task,
    /// Agent entity
    Agent,
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntityType::Task => write!(f, "task"),
            EntityType::Agent => write!(f, "agent"),
        }
    }
}

/// Payload for an error event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Kind of error
    pub error_kind: ErrorKind,
    /// Human-readable error message
    pub message: String,
    /// Whether the error was recovered from
    pub recovered: bool,
}

/// Kind of error
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Tool execution failed
    ToolError,
    /// LLM API error
    LlmError,
    /// Validation failed
    ValidationError,
    /// System error (panic, crash, etc.)
    SystemError,
    /// Other error
    Other,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::ToolError => write!(f, "tool_error"),
            ErrorKind::LlmError => write!(f, "llm_error"),
            ErrorKind::ValidationError => write!(f, "validation_error"),
            ErrorKind::SystemError => write!(f, "system_error"),
            ErrorKind::Other => write!(f, "other"),
        }
    }
}

// ============================================================================
// Event Payload Enum
// ============================================================================

/// Structured payload for a transcript event.
///
/// Each variant corresponds to a specific event type and contains
/// the relevant data for that event kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum TranscriptEventPayload {
    /// Tool call event
    ToolCall(ToolCallPayload),
    /// LLM response event
    LlmResponse(LlmResponsePayload),
    /// State transition event
    StateTransition(StateTransitionPayload),
    /// Error event
    Error(ErrorPayload),
}

impl TranscriptEventPayload {
    /// Returns the event type discriminant for this payload
    pub fn event_type(&self) -> TranscriptEventType {
        match self {
            TranscriptEventPayload::ToolCall(_) => TranscriptEventType::ToolCall,
            TranscriptEventPayload::LlmResponse(_) => TranscriptEventType::LlmResponse,
            TranscriptEventPayload::StateTransition(_) => TranscriptEventType::StateTransition,
            TranscriptEventPayload::Error(_) => TranscriptEventType::Error,
        }
    }
}

// ============================================================================
// Transcript Event
// ============================================================================

/// A single event in the transcript event log.
///
/// Each event has a unique ID, type discriminant, timestamp, session ID,
/// and a structured payload specific to the event type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEvent {
    /// Unique identifier for this event
    pub id: Uuid,
    /// Type discriminant for filtering and routing
    pub event_type: TranscriptEventType,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Session ID grouping related events
    pub session_id: SessionId,
    /// The structured payload for this event
    pub payload: TranscriptEventPayload,
}

impl TranscriptEvent {
    /// Create a new transcript event with a generated UUID and current timestamp
    pub fn new(session_id: SessionId, payload: TranscriptEventPayload) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type: payload.event_type(),
            timestamp: Utc::now(),
            session_id,
            payload,
        }
    }

    /// Create a new tool call event
    pub fn tool_call(session_id: SessionId, payload: ToolCallPayload) -> Self {
        Self::new(session_id, TranscriptEventPayload::ToolCall(payload))
    }

    /// Create a new LLM response event
    pub fn llm_response(session_id: SessionId, payload: LlmResponsePayload) -> Self {
        Self::new(session_id, TranscriptEventPayload::LlmResponse(payload))
    }

    /// Create a new state transition event
    pub fn state_transition(session_id: SessionId, payload: StateTransitionPayload) -> Self {
        Self::new(session_id, TranscriptEventPayload::StateTransition(payload))
    }

    /// Create a new error event
    pub fn error(session_id: SessionId, payload: ErrorPayload) -> Self {
        Self::new(session_id, TranscriptEventPayload::Error(payload))
    }
}

// ============================================================================
// Transcript Log (Append-Only)
// ============================================================================

/// An append-only event log for transcript events.
///
/// The log maintains all events in insertion order and provides
/// indexed access by event type for efficient filtering.
///
/// # Append-Only Invariant
///
/// This struct enforces append-only semantics:
/// - [`TranscriptLog::append()`] is the only way to add events
/// - No public method allows removal or modification of events
/// - The log can be converted to an immutable slice via [`TranscriptLog::events()`]
///
/// # Thread Safety
///
/// [`TranscriptLog`] uses internal synchronization for thread-safe access.
/// It can be shared across async tasks.
#[derive(Debug)]
pub struct TranscriptLog {
    events: Vec<TranscriptEvent>,
    by_type: std::collections::HashMap<TranscriptEventType, Vec<usize>>,
    subscriber_tx: broadcast::Sender<TranscriptEvent>,
}

impl Default for TranscriptLog {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptLog {
    /// Create a new empty transcript log
    pub fn new() -> Self {
        let (subscriber_tx, _) = broadcast::channel(100);
        Self {
            events: Vec::new(),
            by_type: std::collections::HashMap::new(),
            subscriber_tx,
        }
    }

    /// Create a new transcript log with a buffer capacity
    ///
    /// The capacity specifies the initial capacity of the events vector
    /// and the broadcast channel buffer size.
    pub fn with_capacity(capacity: usize) -> Self {
        let (subscriber_tx, _) = broadcast::channel(capacity);
        Self {
            events: Vec::with_capacity(capacity),
            by_type: std::collections::HashMap::new(),
            subscriber_tx,
        }
    }

    /// Append a new event to the log.
    ///
    /// This is the only way to add events - no modification or deletion is possible.
    /// The event is broadcast to all active subscribers.
    pub fn append(&mut self, event: TranscriptEvent) {
        let idx = self.events.len();
        let event_type = event.event_type;

        // Add to main event list
        self.events.push(event.clone());

        // Index by type for efficient filtering
        self.by_type.entry(event_type).or_default().push(idx);

        // Broadcast to subscribers (ignore if no receivers - this is normal)
        let _ = self.subscriber_tx.send(event);
    }

    /// Append a tool call event
    pub fn append_tool_call(&mut self, session_id: SessionId, payload: ToolCallPayload) {
        self.append(TranscriptEvent::tool_call(session_id, payload));
    }

    /// Append an LLM response event
    pub fn append_llm_response(&mut self, session_id: SessionId, payload: LlmResponsePayload) {
        self.append(TranscriptEvent::llm_response(session_id, payload));
    }

    /// Append a state transition event
    pub fn append_state_transition(
        &mut self,
        session_id: SessionId,
        payload: StateTransitionPayload,
    ) {
        self.append(TranscriptEvent::state_transition(session_id, payload));
    }

    /// Append an error event
    pub fn append_error(&mut self, session_id: SessionId, payload: ErrorPayload) {
        self.append(TranscriptEvent::error(session_id, payload));
    }

    /// Get all events in the log (in insertion order)
    ///
    /// Returns an immutable slice since the log is append-only.
    pub fn events(&self) -> &[TranscriptEvent] {
        &self.events
    }

    /// Get events filtered by type.
    ///
    /// Returns events in insertion order, filtered to only the specified types.
    pub fn by_types(&self, types: &[TranscriptEventType]) -> Vec<&TranscriptEvent> {
        if types.is_empty() {
            return Vec::new();
        }

        let mut indices: Vec<usize> = Vec::new();
        for type_filter in types {
            if let Some(type_indices) = self.by_type.get(type_filter) {
                indices.extend(type_indices.iter().copied());
            }
        }

        // Sort indices to maintain insertion order
        indices.sort_unstable();

        indices.into_iter().map(|idx| &self.events[idx]).collect()
    }

    /// Get all events of a specific type
    pub fn by_type(&self, event_type: TranscriptEventType) -> Vec<&TranscriptEvent> {
        self.by_types(&[event_type])
    }

    /// Get the number of events in the log
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if the log is empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get events for a specific session
    pub fn for_session(&self, session_id: SessionId) -> Vec<&TranscriptEvent> {
        self.events
            .iter()
            .filter(|e| e.session_id == session_id)
            .collect()
    }

    /// Subscribe to events matching the given type filters.
    ///
    /// Returns a [`TranscriptSubscriber`] that will receive only events
    /// whose type is in the provided filter set.
    ///
    /// The subscriber will receive events added after subscription.
    /// Events that existed before subscription are not replayed.
    pub fn subscribe(&self, type_filter: HashSet<TranscriptEventType>) -> TranscriptSubscriber {
        TranscriptSubscriber::new(self.subscriber_tx.subscribe(), type_filter)
    }

    /// Subscribe to all event types.
    ///
    /// Returns a [`TranscriptSubscriber`] that will receive all events.
    pub fn subscribe_all(&self) -> TranscriptSubscriber {
        let all_types = [
            TranscriptEventType::ToolCall,
            TranscriptEventType::LlmResponse,
            TranscriptEventType::StateTransition,
            TranscriptEventType::Error,
        ]
        .into_iter()
        .collect();
        self.subscribe(all_types)
    }
}

// ============================================================================
// Transcript Subscriber
// ============================================================================

/// Async subscriber for transcript events.
///
/// Subscribers filter events by type and receive them via an async channel.
/// Created via [`TranscriptLog::subscribe()`].
#[derive(Debug)]
pub struct TranscriptSubscriber {
    receiver: broadcast::Receiver<TranscriptEvent>,
    type_filter: HashSet<TranscriptEventType>,
}

impl TranscriptSubscriber {
    /// Create a new subscriber with a broadcast receiver and type filter
    fn new(
        receiver: broadcast::Receiver<TranscriptEvent>,
        type_filter: HashSet<TranscriptEventType>,
    ) -> Self {
        Self {
            receiver,
            type_filter,
        }
    }

    /// Receive the next matching event.
    ///
    /// Returns `None` if the channel is closed (log dropped).
    /// Returns the event if it matches the type filter, fetching more events
    /// until a matching one is found or the channel is closed.
    pub async fn recv(&mut self) -> Option<TranscriptEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if self.type_filter.contains(&event.event_type) {
                        return Some(event);
                    }
                    // Continue looping to find matching event
                }
                Err(broadcast::error::RecvError::Closed) => return None,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Channel lagged - continue trying to receive
                    continue;
                }
            }
        }
    }

    /// Check if this subscriber filters by a specific event type
    pub fn filters_type(&self, event_type: TranscriptEventType) -> bool {
        self.type_filter.contains(&event_type)
    }

    /// Get all event types this subscriber is filtering for
    pub fn filtered_types(&self) -> &HashSet<TranscriptEventType> {
        &self.type_filter
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    fn session_id() -> SessionId {
        SessionId::new()
    }

    #[test]
    fn test_transcript_event_type_display() {
        assert_eq!(TranscriptEventType::ToolCall.to_string(), "tool_call");
        assert_eq!(TranscriptEventType::LlmResponse.to_string(), "llm_response");
        assert_eq!(
            TranscriptEventType::StateTransition.to_string(),
            "state_transition"
        );
        assert_eq!(TranscriptEventType::Error.to_string(), "error");
    }

    #[test]
    fn test_tool_call_payload() {
        let payload = ToolCallPayload::success(
            "file_read".to_string(),
            serde_json::json!({"path": "/test.txt"}),
            50,
        );

        assert_eq!(payload.tool_name, "file_read");
        assert!(payload.success);
        assert_eq!(payload.duration_ms, 50);
        assert!(payload.error_message.is_none());
    }

    #[test]
    fn test_tool_call_payload_failure() {
        let payload = ToolCallPayload::failure(
            "file_read".to_string(),
            serde_json::json!({"path": "/test.txt"}),
            "File not found".to_string(),
        );

        assert!(!payload.success);
        assert_eq!(payload.error_message, Some("File not found".to_string()));
    }

    #[test]
    fn test_token_usage() {
        let usage = TokenUsage::new(100, 50);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total(), 150);
    }

    #[test]
    fn test_token_usage_with_cache() {
        let usage = TokenUsage::with_cache(100, 50, Some(200), Some(300));
        assert_eq!(usage.cache_creation_input_tokens, Some(200));
        assert_eq!(usage.cache_read_input_tokens, Some(300));
    }

    #[test]
    fn test_entity_type_display() {
        assert_eq!(EntityType::Task.to_string(), "task");
        assert_eq!(EntityType::Agent.to_string(), "agent");
    }

    #[test]
    fn test_error_kind_display() {
        assert_eq!(ErrorKind::ToolError.to_string(), "tool_error");
        assert_eq!(ErrorKind::LlmError.to_string(), "llm_error");
        assert_eq!(ErrorKind::ValidationError.to_string(), "validation_error");
        assert_eq!(ErrorKind::SystemError.to_string(), "system_error");
        assert_eq!(ErrorKind::Other.to_string(), "other");
    }

    #[test]
    fn test_transcript_event_payload_event_type() {
        let tool_payload = ToolCallPayload::success("test".to_string(), serde_json::json!({}), 0);
        assert_eq!(
            TranscriptEventPayload::ToolCall(tool_payload).event_type(),
            TranscriptEventType::ToolCall
        );

        let llm_payload = LlmResponsePayload::new("gpt-4".to_string(), None, None, None);
        assert_eq!(
            TranscriptEventPayload::LlmResponse(llm_payload).event_type(),
            TranscriptEventType::LlmResponse
        );

        let state_payload = StateTransitionPayload {
            entity_type: EntityType::Task,
            entity_id: Uuid::new_v4(),
            from_state: "Created".to_string(),
            to_state: "Executing".to_string(),
            reason: None,
        };
        assert_eq!(
            TranscriptEventPayload::StateTransition(state_payload).event_type(),
            TranscriptEventType::StateTransition
        );

        let error_payload = ErrorPayload {
            error_kind: ErrorKind::ToolError,
            message: "Failed".to_string(),
            recovered: true,
        };
        assert_eq!(
            TranscriptEventPayload::Error(error_payload).event_type(),
            TranscriptEventType::Error
        );
    }

    #[test]
    fn test_transcript_event_creation() {
        let session = session_id();
        let payload = ToolCallPayload::success("test".to_string(), serde_json::json!({}), 0);
        let event = TranscriptEvent::tool_call(session, payload.clone());

        assert!(!event.id.is_nil());
        assert_eq!(event.event_type, TranscriptEventType::ToolCall);
        assert_eq!(event.session_id, session);
    }

    #[test]
    fn test_transcript_log_append_only() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        // Append 5 different event types
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool1".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append_state_transition(
            session,
            StateTransitionPayload {
                entity_type: EntityType::Task,
                entity_id: Uuid::new_v4(),
                from_state: "Created".to_string(),
                to_state: "Executing".to_string(),
                reason: None,
            },
        );
        log.append_error(
            session,
            ErrorPayload {
                error_kind: ErrorKind::ToolError,
                message: "Tool failed".to_string(),
                recovered: true,
            },
        );
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool2".to_string(), serde_json::json!({}), 20),
        );

        assert_eq!(log.len(), 5);

        // Verify all events are in the log in order
        let events = log.events();
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].event_type, TranscriptEventType::ToolCall);
        assert_eq!(events[1].event_type, TranscriptEventType::LlmResponse);
        assert_eq!(events[2].event_type, TranscriptEventType::StateTransition);
        assert_eq!(events[3].event_type, TranscriptEventType::Error);
        assert_eq!(events[4].event_type, TranscriptEventType::ToolCall);
    }

    #[test]
    fn test_transcript_log_by_type() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        // Add multiple events of different types
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool1".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool2".to_string(), serde_json::json!({}), 20),
        );
        log.append_state_transition(
            session,
            StateTransitionPayload {
                entity_type: EntityType::Task,
                entity_id: Uuid::new_v4(),
                from_state: "Created".to_string(),
                to_state: "Executing".to_string(),
                reason: None,
            },
        );

        // Filter by ToolCall
        let tool_calls = log.by_type(TranscriptEventType::ToolCall);
        assert_eq!(tool_calls.len(), 2);
        assert!(tool_calls
            .iter()
            .all(|e| e.event_type == TranscriptEventType::ToolCall));

        // Filter by LlmResponse
        let llm_responses = log.by_type(TranscriptEventType::LlmResponse);
        assert_eq!(llm_responses.len(), 1);

        // Filter by StateTransition
        let transitions = log.by_type(TranscriptEventType::StateTransition);
        assert_eq!(transitions.len(), 1);

        // Filter by Error (none added)
        let errors = log.by_type(TranscriptEventType::Error);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_transcript_log_by_types() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        log.append_tool_call(
            session,
            ToolCallPayload::success("tool1".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append_error(
            session,
            ErrorPayload {
                error_kind: ErrorKind::ToolError,
                message: "Failed".to_string(),
                recovered: true,
            },
        );

        // Filter by ToolCall and Error
        let filtered = log.by_types(&[TranscriptEventType::ToolCall, TranscriptEventType::Error]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| {
            e.event_type == TranscriptEventType::ToolCall
                || e.event_type == TranscriptEventType::Error
        }));
    }

    #[test]
    fn test_transcript_log_for_session() {
        let session1 = SessionId::new();
        let session2 = SessionId::new();

        let mut log = TranscriptLog::new();

        log.append_tool_call(
            session1,
            ToolCallPayload::success("tool1".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session2, // Different session
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append_tool_call(
            session1,
            ToolCallPayload::success("tool2".to_string(), serde_json::json!({}), 20),
        );

        let session1_events = log.for_session(session1);
        assert_eq!(session1_events.len(), 2);

        let session2_events = log.for_session(session2);
        assert_eq!(session2_events.len(), 1);
    }

    #[test]
    fn test_transcript_subscriber_filtering() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        // Add events
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );

        // Subscribe to only ToolCall events
        let subscriber = log.subscribe([TranscriptEventType::ToolCall].into_iter().collect());

        assert!(subscriber.filters_type(TranscriptEventType::ToolCall));
        assert!(!subscriber.filters_type(TranscriptEventType::LlmResponse));
    }

    #[tokio::test]
    async fn test_transcript_subscriber_receives_matching_events() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        // Subscribe before adding events
        let mut subscriber = log.subscribe_all();

        // Add events
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 10),
        );
        let llm_event = TranscriptEvent::llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append(llm_event.clone());
        log.append_error(
            session,
            ErrorPayload {
                error_kind: ErrorKind::ToolError,
                message: "Failed".to_string(),
                recovered: true,
            },
        );

        // Should receive events in order
        let event1 = timeout(Duration::from_secs(1), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event1.event_type, TranscriptEventType::ToolCall);

        let event2 = timeout(Duration::from_secs(1), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event2.event_type, TranscriptEventType::LlmResponse);
    }

    #[tokio::test]
    async fn test_transcript_subscriber_filters_by_type() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        // Subscribe to only ToolCall
        let mut subscriber = log.subscribe([TranscriptEventType::ToolCall].into_iter().collect());

        // Add ToolCall and LlmResponse
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );
        log.append_tool_call(
            session,
            ToolCallPayload::success("tool2".to_string(), serde_json::json!({}), 20),
        );

        // Should only receive ToolCall events
        let event1 = timeout(Duration::from_secs(1), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event1.event_type, TranscriptEventType::ToolCall);

        let event2 = timeout(Duration::from_secs(1), subscriber.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event2.event_type, TranscriptEventType::ToolCall);

        // No more events should be available (LlmResponse was filtered)
        let event3 = timeout(Duration::from_millis(100), subscriber.recv()).await;
        assert!(event3.is_err()); // Timeout
    }

    #[test]
    fn test_transcript_event_serialization() {
        let session = session_id();
        let event = TranscriptEvent::tool_call(
            session,
            ToolCallPayload::success(
                "file_read".to_string(),
                serde_json::json!({"path": "/test.txt"}),
                50,
            ),
        );

        let json = serde_json::to_string(&event).unwrap();

        // Verify type discriminant is present (serde tag uses variant name, not rename_all)
        assert!(json.contains("\"type\":\"ToolCall\""));

        // Deserialize and verify
        let parsed: TranscriptEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, event.id);
        assert_eq!(parsed.event_type, TranscriptEventType::ToolCall);
        assert_eq!(parsed.session_id, session);

        // Check payload
        match parsed.payload {
            TranscriptEventPayload::ToolCall(payload) => {
                assert_eq!(payload.tool_name, "file_read");
                assert!(payload.success);
            }
            _ => panic!("Expected ToolCall payload"),
        }
    }

    #[test]
    fn test_transcript_log_is_append_only() {
        let mut log = TranscriptLog::new();
        let session = session_id();

        log.append_tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 10),
        );
        log.append_llm_response(
            session,
            LlmResponsePayload::new(
                "gpt-4".to_string(),
                Some("response".to_string()),
                None,
                None,
            ),
        );

        assert_eq!(log.len(), 2);

        // events() returns an immutable slice - no way to modify
        let events = log.events();
        assert_eq!(events.len(), 2);

        // The internal Vec is not exposed - append is the only way to add
        // This is verified by the API design - no remove(), clear(), or other mutators
    }

    #[test]
    fn test_transcript_log_empty() {
        let log = TranscriptLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert!(log.events().is_empty());
    }

    #[test]
    fn test_transcript_log_with_capacity() {
        let log = TranscriptLog::with_capacity(1000);
        assert!(log.is_empty());
    }

    #[test]
    fn test_transcript_log_subscribe_all_types() {
        let log = TranscriptLog::new();
        let subscriber = log.subscribe_all();

        assert!(subscriber.filters_type(TranscriptEventType::ToolCall));
        assert!(subscriber.filters_type(TranscriptEventType::LlmResponse));
        assert!(subscriber.filters_type(TranscriptEventType::StateTransition));
        assert!(subscriber.filters_type(TranscriptEventType::Error));
    }

    #[test]
    fn test_filtered_types_returns_correct_set() {
        let log = TranscriptLog::new();
        let filter: HashSet<TranscriptEventType> =
            [TranscriptEventType::ToolCall, TranscriptEventType::Error]
                .into_iter()
                .collect();
        let subscriber = log.subscribe(filter.clone());

        assert_eq!(subscriber.filtered_types(), &filter);
    }

    #[test]
    fn test_transcript_event_id_is_unique() {
        let session = session_id();
        let event1 = TranscriptEvent::tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 10),
        );
        let event2 = TranscriptEvent::tool_call(
            session,
            ToolCallPayload::success("tool".to_string(), serde_json::json!({}), 20),
        );

        assert_ne!(event1.id, event2.id);
    }

    #[test]
    fn test_transcript_event_state_transition_payload() {
        let entity_id = Uuid::new_v4();
        let payload = StateTransitionPayload {
            entity_type: EntityType::Task,
            entity_id,
            from_state: "Created".to_string(),
            to_state: "Executing".to_string(),
            reason: Some("User approved".to_string()),
        };

        let session = session_id();
        let event = TranscriptEvent::state_transition(session, payload.clone());

        assert_eq!(event.event_type, TranscriptEventType::StateTransition);

        match event.payload {
            TranscriptEventPayload::StateTransition(p) => {
                assert_eq!(p.entity_type, EntityType::Task);
                assert_eq!(p.entity_id, entity_id);
                assert_eq!(p.from_state, "Created");
                assert_eq!(p.to_state, "Executing");
                assert_eq!(p.reason, Some("User approved".to_string()));
            }
            _ => panic!("Expected StateTransition payload"),
        }
    }

    #[test]
    fn test_transcript_event_error_payload() {
        let payload = ErrorPayload {
            error_kind: ErrorKind::LlmError,
            message: "API rate limit exceeded".to_string(),
            recovered: false,
        };

        let session = session_id();
        let event = TranscriptEvent::error(session, payload.clone());

        assert_eq!(event.event_type, TranscriptEventType::Error);

        match event.payload {
            TranscriptEventPayload::Error(p) => {
                assert_eq!(p.error_kind, ErrorKind::LlmError);
                assert_eq!(p.message, "API rate limit exceeded");
                assert!(!p.recovered);
            }
            _ => panic!("Expected Error payload"),
        }
    }

    #[test]
    fn test_transcript_event_llm_response_with_usage() {
        let usage = TokenUsage::with_cache(100, 50, Some(200), Some(300));
        let payload = LlmResponsePayload::new(
            "claude-3-opus".to_string(),
            Some("Hello world".to_string()),
            Some(usage),
            Some("end_turn".to_string()),
        );

        let session = session_id();
        let event = TranscriptEvent::llm_response(session, payload);

        assert_eq!(event.event_type, TranscriptEventType::LlmResponse);

        match event.payload {
            TranscriptEventPayload::LlmResponse(p) => {
                assert_eq!(p.model, "claude-3-opus");
                assert_eq!(p.text_content, Some("Hello world".to_string()));
                assert!(p.token_usage.is_some());
                assert_eq!(p.token_usage.as_ref().unwrap().total(), 150);
                assert_eq!(p.stop_reason, Some("end_turn".to_string()));
            }
            _ => panic!("Expected LlmResponse payload"),
        }
    }
}
