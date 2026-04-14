//! Event emitter for task lifecycle events.
//!
//! This module provides:
//! - [`EventEmitter`]: Emits structured events with correlation IDs
//! - [`ImmutableEventLog`]: An append-only log of all emitted events
//! - [`EventLogEntry`]: A single entry in the immutable log
//!
//! # Correlation IDs
//!
//! Every event emitted includes a correlation ID that links related events
//! together. For example, all events in a task's lifecycle share the same
//! correlation ID, allowing you to trace the complete history of an operation.
//!
//! # Immutability
//!
//! The event log is designed to be immutable - once an event is recorded,
//! it cannot be modified or deleted. This provides a reliable audit trail
//! and ensures event ordering is preserved.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use swell_core::{CorrelationId, DaemonEvent, FailureClass, Task, TaskState};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// An event log entry that records an emitted event with metadata.
/// This struct is immutable once created - it provides a complete
/// audit trail of all events in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogEntry {
    /// Unique sequence number for ordering
    pub sequence: u64,
    /// When the event was emitted
    pub timestamp: DateTime<Utc>,
    /// The correlation ID linking related events
    pub correlation_id: CorrelationId,
    /// The task ID this event pertains to (if applicable)
    pub task_id: Option<Uuid>,
    /// The actual event data
    pub event: DaemonEvent,
}

impl EventLogEntry {
    /// Create a new event log entry
    fn new(
        sequence: u64,
        correlation_id: CorrelationId,
        task_id: Option<Uuid>,
        event: DaemonEvent,
    ) -> Self {
        Self {
            sequence,
            timestamp: Utc::now(),
            correlation_id,
            task_id,
            event,
        }
    }
}

/// An append-only, immutable log of all events emitted by the daemon.
/// This provides a reliable audit trail and enables event replay/observation.
#[derive(Debug, Clone, Default)]
pub struct ImmutableEventLog {
    /// Internal sequence counter
    next_sequence: u64,
    /// The actual log entries (appended only, never modified)
    entries: Vec<EventLogEntry>,
}

impl ImmutableEventLog {
    /// Create a new empty event log
    pub fn new() -> Self {
        Self {
            next_sequence: 0,
            entries: Vec::new(),
        }
    }

    /// Record a new event in the log.
    /// This operation is append-only - existing entries cannot be modified.
    fn record(
        &mut self,
        correlation_id: CorrelationId,
        task_id: Option<Uuid>,
        event: DaemonEvent,
    ) -> EventLogEntry {
        let entry = EventLogEntry::new(self.next_sequence, correlation_id, task_id, event);
        self.entries.push(entry.clone());
        self.next_sequence += 1;
        entry
    }

    /// Get all events for a specific correlation ID
    pub fn get_by_correlation_id(&self, correlation_id: CorrelationId) -> Vec<&EventLogEntry> {
        self.entries
            .iter()
            .filter(|e| e.correlation_id == correlation_id)
            .collect()
    }

    /// Get all events for a specific task ID
    pub fn get_by_task_id(&self, task_id: Uuid) -> Vec<&EventLogEntry> {
        self.entries
            .iter()
            .filter(|e| e.task_id == Some(task_id))
            .collect()
    }

    /// Get all events in the log (in order)
    pub fn get_all(&self) -> &[EventLogEntry] {
        &self.entries
    }

    /// Get events for a specific task ID since a given sequence number (exclusive).
    /// Returns events with sequence > given_sequence for the specified task.
    pub fn get_events_since_for_task(
        &self,
        task_id: Uuid,
        since_sequence: u64,
    ) -> Vec<&EventLogEntry> {
        self.entries
            .iter()
            .filter(|e| e.task_id == Some(task_id) && e.sequence > since_sequence)
            .collect()
    }

    /// Get the current sequence number (the next sequence that will be assigned).
    /// This allows callers to track what events are new since they last checked.
    pub fn current_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Get the total number of events recorded
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the log is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Event emitter for the daemon.
/// Handles emitting structured events with correlation IDs and maintains
/// an immutable event log for auditing and replay.
#[derive(Debug, Clone)]
pub struct EventEmitter {
    /// Shared immutable event log
    log: Arc<RwLock<ImmutableEventLog>>,
    /// Broadcast channel for real-time event subscribers
    broadcast_tx: Arc<RwLock<Option<broadcast::Sender<DaemonEvent>>>>,
}

impl EventEmitter {
    /// Create a new event emitter with an empty log
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            log: Arc::new(RwLock::new(ImmutableEventLog::new())),
            broadcast_tx: Arc::new(RwLock::new(Some(tx))),
        }
    }

    /// Subscribe to events. Returns a receiver that will receive all subsequent events.
    pub async fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        let tx = self.broadcast_tx.read().await;
        if let Some(sender) = tx.as_ref() {
            sender.subscribe()
        } else {
            // If no sender, create a channel that will never receive
            let (tx, rx) = broadcast::channel(1);
            tx.send(DaemonEvent::Error {
                message: "EventEmitter shutting down".to_string(),
                failure_class: None,
                correlation_id: Uuid::nil(),
            })
            .ok();
            rx
        }
    }

    /// Generate a new correlation ID for tracking related events
    pub fn new_correlation_id() -> CorrelationId {
        Uuid::new_v4()
    }

    /// Broadcast an event to all subscribers
    async fn broadcast(&self, event: &DaemonEvent) {
        let tx = self.broadcast_tx.read().await;
        if let Some(sender) = tx.as_ref() {
            // Ignore send errors (subscriber lag is expected)
            let _ = sender.send(event.clone());
        }
    }

    /// Emit a TaskCreated event and record it in the log
    pub async fn emit_task_created(&self, task: &Task) -> DaemonEvent {
        let correlation_id = Self::new_correlation_id();
        let event = DaemonEvent::TaskCreated {
            id: task.id,
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, Some(task.id), event.clone());

        tracing::info!(
            task_id = %task.id,
            correlation_id = %correlation_id,
            "Event: TaskCreated"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Emit a TaskStateChanged event and record it in the log
    pub async fn emit_task_state_changed(
        &self,
        task_id: Uuid,
        state: TaskState,
        correlation_id: CorrelationId,
    ) -> DaemonEvent {
        let event = DaemonEvent::TaskStateChanged {
            id: task_id,
            state,
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, Some(task_id), event.clone());

        tracing::info!(
            task_id = %task_id,
            state = %state,
            correlation_id = %correlation_id,
            "Event: TaskStateChanged"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Emit a TaskProgress event and record it in the log
    pub async fn emit_task_progress(
        &self,
        task_id: Uuid,
        message: String,
        correlation_id: CorrelationId,
    ) -> DaemonEvent {
        let event = DaemonEvent::TaskProgress {
            id: task_id,
            message: message.clone(),
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, Some(task_id), event.clone());

        tracing::info!(
            task_id = %task_id,
            message = %message,
            correlation_id = %correlation_id,
            "Event: TaskProgress"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Emit a TaskCompleted event and record it in the log
    pub async fn emit_task_completed(
        &self,
        task_id: Uuid,
        pr_url: Option<String>,
        correlation_id: CorrelationId,
    ) -> DaemonEvent {
        let event = DaemonEvent::TaskCompleted {
            id: task_id,
            pr_url,
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, Some(task_id), event.clone());

        tracing::info!(
            task_id = %task_id,
            correlation_id = %correlation_id,
            "Event: TaskCompleted"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Emit a TaskFailed event and record it in the log
    pub async fn emit_task_failed(
        &self,
        task_id: Uuid,
        error: String,
        failure_class: Option<FailureClass>,
        correlation_id: CorrelationId,
    ) -> DaemonEvent {
        let event = DaemonEvent::TaskFailed {
            id: task_id,
            error: error.clone(),
            failure_class,
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, Some(task_id), event.clone());

        tracing::warn!(
            task_id = %task_id,
            error = %error,
            correlation_id = %correlation_id,
            "Event: TaskFailed"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Emit an error event and record it in the log
    pub async fn emit_error(
        &self,
        message: String,
        failure_class: Option<FailureClass>,
        correlation_id: CorrelationId,
    ) -> DaemonEvent {
        let event = DaemonEvent::Error {
            message: message.clone(),
            failure_class,
            correlation_id,
        };

        let mut log = self.log.write().await;
        log.record(correlation_id, None, event.clone());

        tracing::error!(
            message = %message,
            correlation_id = %correlation_id,
            "Event: Error"
        );

        drop(log);
        self.broadcast(&event).await;
        event
    }

    /// Get the event log for reading
    pub async fn log(&self) -> Arc<RwLock<ImmutableEventLog>> {
        Arc::clone(&self.log)
    }

    /// Get all events for a correlation ID
    pub async fn get_events_by_correlation_id(
        &self,
        correlation_id: CorrelationId,
    ) -> Vec<DaemonEvent> {
        let log = self.log.read().await;
        log.get_by_correlation_id(correlation_id)
            .into_iter()
            .map(|e| e.event.clone())
            .collect()
    }

    /// Get all events for a task ID
    pub async fn get_events_by_task_id(&self, task_id: Uuid) -> Vec<DaemonEvent> {
        let log = self.log.read().await;
        log.get_by_task_id(task_id)
            .into_iter()
            .map(|e| e.event.clone())
            .collect()
    }

    /// Get all events in the log
    pub async fn get_all_events(&self) -> Vec<DaemonEvent> {
        let log = self.log.read().await;
        log.get_all().iter().map(|e| e.event.clone()).collect()
    }

    /// Get the total number of events emitted
    pub async fn event_count(&self) -> usize {
        let log = self.log.read().await;
        log.len()
    }

    /// Get events for a specific task since a given sequence number.
    /// Returns events with sequence > since_sequence for the specified task.
    pub async fn get_events_since_for_task(
        &self,
        task_id: Uuid,
        since_sequence: u64,
    ) -> Vec<DaemonEvent> {
        let log = self.log.read().await;
        log.get_events_since_for_task(task_id, since_sequence)
            .into_iter()
            .map(|e| e.event.clone())
            .collect()
    }

    /// Get the current sequence number.
    /// This allows callers to track what events are new since they last checked.
    pub async fn current_sequence(&self) -> u64 {
        let log = self.log.read().await;
        log.current_sequence()
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::Task;

    #[tokio::test]
    async fn test_event_emitter_records_task_created() {
        let emitter = EventEmitter::new();
        let task = Task::new("Test task".to_string());

        let event = emitter.emit_task_created(&task).await;

        match event {
            DaemonEvent::TaskCreated { id, correlation_id } => {
                assert_eq!(id, task.id);
                assert!(correlation_id != Uuid::nil());
            }
            other => panic!("Expected TaskCreated event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_event_emitter_records_task_state_changed() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let task_id = Uuid::new_v4();

        let event = emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;

        match event {
            DaemonEvent::TaskStateChanged {
                id,
                state,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Executing);
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_event_emitter_records_task_progress() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let task_id = Uuid::new_v4();
        let message = "Processing step 1/3".to_string();

        let event = emitter
            .emit_task_progress(task_id, message.clone(), correlation_id)
            .await;

        match event {
            DaemonEvent::TaskProgress {
                id,
                message: msg,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(msg, message);
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected TaskProgress event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_event_emitter_records_task_completed() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let task_id = Uuid::new_v4();
        let pr_url = Some("https://github.com/example/pr/123".to_string());

        let event = emitter
            .emit_task_completed(task_id, pr_url.clone(), correlation_id)
            .await;

        match event {
            DaemonEvent::TaskCompleted {
                id,
                pr_url: url,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(url, pr_url);
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected TaskCompleted event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_event_emitter_records_task_failed() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let task_id = Uuid::new_v4();
        let error = "Validation failed".to_string();

        let event = emitter
            .emit_task_failed(task_id, error.clone(), None, correlation_id)
            .await;

        match event {
            DaemonEvent::TaskFailed {
                id,
                error: err,
                failure_class,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(err, error);
                assert_eq!(cid, correlation_id);
                assert!(failure_class.is_none());
            }
            other => panic!("Expected TaskFailed event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_event_emitter_records_error() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let message = "Something went wrong".to_string();

        let event = emitter.emit_error(message.clone(), None, correlation_id).await;

        match event {
            DaemonEvent::Error {
                message: msg,
                failure_class,
                correlation_id: cid,
            } => {
                assert_eq!(msg, message);
                assert_eq!(cid, correlation_id);
                assert!(failure_class.is_none());
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }

        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_correlation_id_links_events() {
        let emitter = EventEmitter::new();
        let task_id = Uuid::new_v4();
        let correlation_id = EventEmitter::new_correlation_id();

        // Emit multiple events with the same correlation ID
        emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;
        emitter
            .emit_task_progress(task_id, "Working...".to_string(), correlation_id)
            .await;
        emitter
            .emit_task_completed(task_id, None, correlation_id)
            .await;

        // All events should have the same correlation ID
        let events = emitter.get_events_by_correlation_id(correlation_id).await;
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn test_get_events_by_task_id() {
        let emitter = EventEmitter::new();
        let task_id = Uuid::new_v4();
        let correlation_id = EventEmitter::new_correlation_id();

        // Emit events for a specific task
        emitter
            .emit_task_created(&Task::new("Test".to_string()))
            .await;
        emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;
        emitter
            .emit_task_progress(task_id, "Done".to_string(), correlation_id)
            .await;

        let events = emitter.get_events_by_task_id(task_id).await;
        assert_eq!(events.len(), 2); // Only state_changed and progress (created has different task_id)
    }

    #[tokio::test]
    async fn test_get_all_events() {
        let emitter = EventEmitter::new();

        // Emit some events
        let task = Task::new("Test".to_string());
        emitter.emit_task_created(&task).await;
        emitter
            .emit_error("Test error".to_string(), None, EventEmitter::new_correlation_id())
            .await;

        let all = emitter.get_all_events().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_immutable_event_log_records_in_order() {
        let emitter = EventEmitter::new();
        let correlation_id = EventEmitter::new_correlation_id();
        let task_id = Uuid::new_v4();

        emitter
            .emit_task_created(&Task::new("Test".to_string()))
            .await;
        emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;
        emitter
            .emit_task_progress(task_id, "Step 1".to_string(), correlation_id)
            .await;

        let log = emitter.log().await;
        let log_guard = log.read().await;
        let entries = log_guard.get_all();

        assert_eq!(entries.len(), 3);
        // Check sequence numbers are in order
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[1].sequence, 1);
        assert_eq!(entries[2].sequence, 2);
    }

    #[tokio::test]
    async fn test_event_log_get_by_correlation_id() {
        let emitter = EventEmitter::new();
        let correlation_id1 = EventEmitter::new_correlation_id();
        let correlation_id2 = EventEmitter::new_correlation_id();
        let task_id1 = Uuid::new_v4();
        let task_id2 = Uuid::new_v4();

        // Events with correlation_id1
        emitter
            .emit_task_created(&Task::new("Test1".to_string()))
            .await;
        emitter
            .emit_task_state_changed(task_id1, TaskState::Executing, correlation_id1)
            .await;

        // Events with correlation_id2
        emitter
            .emit_task_state_changed(task_id2, TaskState::Executing, correlation_id2)
            .await;
        emitter
            .emit_task_progress(task_id2, "Done".to_string(), correlation_id2)
            .await;

        let events1 = emitter.get_events_by_correlation_id(correlation_id1).await;
        let events2 = emitter.get_events_by_correlation_id(correlation_id2).await;

        assert_eq!(events1.len(), 1); // Only state_changed (correlation_id1) - created has its own ID
        assert_eq!(events2.len(), 2); // state_changed and progress
    }

    #[tokio::test]
    async fn test_event_count() {
        let emitter = EventEmitter::new();
        assert_eq!(emitter.event_count().await, 0);

        emitter
            .emit_error("Error 1".to_string(), None, EventEmitter::new_correlation_id())
            .await;
        assert_eq!(emitter.event_count().await, 1);

        emitter
            .emit_error("Error 2".to_string(), None, EventEmitter::new_correlation_id())
            .await;
        assert_eq!(emitter.event_count().await, 2);

        emitter
            .emit_error("Error 3".to_string(), None, EventEmitter::new_correlation_id())
            .await;
        assert_eq!(emitter.event_count().await, 3);
    }

    #[tokio::test]
    async fn test_event_timestamps_are_set() {
        let emitter = EventEmitter::new();
        let before = Utc::now();

        emitter
            .emit_error("Test".to_string(), None, EventEmitter::new_correlation_id())
            .await;

        let after = Utc::now();

        let log = emitter.log().await;
        let log_guard = log.read().await;
        let entries = log_guard.get_all();

        assert_eq!(entries.len(), 1);
        assert!(entries[0].timestamp >= before && entries[0].timestamp <= after);
    }

    #[tokio::test]
    async fn test_event_emitter_subscribe() {
        let emitter = EventEmitter::new();
        let task_id = Uuid::new_v4();
        let correlation_id = EventEmitter::new_correlation_id();

        // Subscribe before emitting
        let mut rx = emitter.subscribe().await;

        // Emit an event
        emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;

        // Receive should get the event
        let event = rx.recv().await.unwrap();
        match event {
            DaemonEvent::TaskStateChanged {
                id,
                state,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Executing);
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_event_emitter_subscribe_multiple_receivers() {
        let emitter = EventEmitter::new();
        let task_id = Uuid::new_v4();
        let correlation_id = EventEmitter::new_correlation_id();

        // Subscribe multiple receivers
        let mut rx1 = emitter.subscribe().await;
        let mut rx2 = emitter.subscribe().await;

        // Emit an event
        emitter
            .emit_task_state_changed(task_id, TaskState::Executing, correlation_id)
            .await;

        // Both receivers should get the event
        let event1 = rx1.recv().await.unwrap();
        let event2 = rx2.recv().await.unwrap();

        match (&event1, &event2) {
            (
                DaemonEvent::TaskStateChanged { id: id1, .. },
                DaemonEvent::TaskStateChanged { id: id2, .. },
            ) => {
                assert_eq!(*id1, task_id);
                assert_eq!(*id2, task_id);
            }
            _ => panic!("Expected TaskStateChanged events"),
        }
    }
}
