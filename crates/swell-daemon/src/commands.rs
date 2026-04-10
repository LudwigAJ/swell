//! Command handlers for daemon CLI commands.
//!
//! This module handles all CLI commands that come through the Unix socket
//! and translates them into appropriate daemon events.

use crate::events::EventEmitter;
use std::sync::Arc;
use swell_core::{CliCommand, DaemonEvent, TaskState};
use swell_orchestrator::Orchestrator;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

/// Handle a parsed CLI command and return an appropriate daemon event.
///
/// # Command Support
/// - `TaskCreate` - Creates a new task with the given description
/// - `TaskApprove` - Approves and starts a task (transitions to Ready)
/// - `TaskReject` - Rejects a task with a reason
/// - `TaskCancel` - Cancels a task (transitions to Failed)
/// - `TaskList` - Returns all tasks as JSON
/// - `TaskWatch` - Returns current state of a specific task
///
/// # Error Handling
/// Returns `DaemonEvent::Error` with a message for:
/// - Task not found (invalid task_id)
/// - Invalid state transitions
/// - Orchestrator errors
pub async fn handle_command(
    command: CliCommand,
    orchestrator: Arc<Mutex<Orchestrator>>,
    event_emitter: Arc<EventEmitter>,
) -> DaemonEvent {
    match command {
        CliCommand::TaskCreate { description } => {
            let orch = orchestrator.lock().await;
            let task = orch.create_task(description.clone()).await;
            info!(task_id = %task.id, "Task created via CLI");
            // Emit event with the emitter (records to log)
            let event = event_emitter.emit_task_created(&task).await;
            // Also return the event for immediate response
            event
        }
        CliCommand::TaskApprove { task_id } => {
            let orch = orchestrator.lock().await;
            // Verify task exists before attempting to start
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task approved, starting execution");
                    match orch.start_task(task_id).await {
                        Ok(()) => {
                            let correlation_id = EventEmitter::new_correlation_id();
                            let event = event_emitter
                                .emit_task_state_changed(task_id, TaskState::Ready, correlation_id)
                                .await;
                            event
                        }
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "Failed to start task");
                            let correlation_id = EventEmitter::new_correlation_id();
                            event_emitter
                                .emit_error(format!("Failed to start task: {}", e), correlation_id)
                                .await
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for approval");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskReject { task_id, reason } => {
            let orch = orchestrator.lock().await;
            // Verify task exists
            match orch.get_task(task_id).await {
                Ok(task) => {
                    warn!(task_id = %task_id, reason = %reason, state = ?task.state, "Task rejected");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_task_state_changed(task_id, TaskState::Rejected, correlation_id)
                        .await
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for rejection");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskCancel { task_id } => {
            let orch = orchestrator.lock().await;
            // Verify task exists
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task cancelled");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_task_state_changed(task_id, TaskState::Failed, correlation_id)
                        .await
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for cancellation");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskList => {
            let orch = orchestrator.lock().await;
            let tasks = orch.get_all_tasks().await;
            let json = serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".to_string());
            info!(task_count = tasks.len(), "Task list requested");
            // Send as a special event with nil UUID to indicate list response
            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::TaskCompleted {
                id: Uuid::nil(),
                pr_url: Some(json),
                correlation_id,
            }
        }
        CliCommand::TaskWatch { task_id } => {
            let orch = orchestrator.lock().await;
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task watch requested");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_task_state_changed(task_id, task.state, correlation_id)
                        .await
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for watching");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), correlation_id)
                        .await
                }
            }
        }
    }
}

/// Parse a JSON string into a CliCommand.
///
/// Returns `Err` if the JSON is invalid or doesn't represent a valid command.
pub fn parse_command(json: &str) -> Result<CliCommand, String> {
    serde_json::from_str(json).map_err(|e| format!("Invalid command JSON: {}", e))
}

/// Convert a DaemonEvent to JSON string.
///
/// Returns `Err` if serialization fails (should rarely happen).
pub fn event_to_json(event: &DaemonEvent) -> Result<String, String> {
    serde_json::to_string(event).map_err(|e| format!("Failed to serialize event: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventEmitter;
    use std::sync::Arc;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus};
    use tokio::sync::Mutex;

    fn create_test_plan(task_id: Uuid) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![PlanStep {
                id: Uuid::new_v4(),
                description: "Test step".to_string(),
                affected_files: vec!["test.rs".to_string()],
                expected_tests: vec!["test_foo".to_string()],
                risk_level: RiskLevel::Low,
                dependencies: vec![],
                status: StepStatus::Pending,
            }],
            total_estimated_tokens: 1000,
            risk_assessment: "Low risk".to_string(),
        }
    }

    fn create_test_orchestrator() -> Arc<Mutex<Orchestrator>> {
        Arc::new(Mutex::new(Orchestrator::new()))
    }

    fn create_test_event_emitter() -> Arc<EventEmitter> {
        Arc::new(EventEmitter::new())
    }

    // --- TaskCreate Tests ---

    #[tokio::test]
    async fn test_task_create_returns_task_created_event() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let command = CliCommand::TaskCreate {
            description: "Test task description".to_string(),
        };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskCreated { id, correlation_id } => {
                assert!(id != Uuid::nil());
                assert!(correlation_id != Uuid::nil());
            }
            other => panic!("Expected TaskCreated event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_create_with_empty_description() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let command = CliCommand::TaskCreate {
            description: "".to_string(),
        };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskCreated { id, correlation_id } => {
                assert!(id != Uuid::nil());
                assert!(correlation_id != Uuid::nil());
            }
            other => panic!("Expected TaskCreated event, got: {:?}", other),
        }
    }

    // --- TaskApprove Tests ---

    #[tokio::test]
    async fn test_task_approve_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskApprove { task_id: fake_id };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_approve_valid_task_returns_state_changed() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // First create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        let command = CliCommand::TaskApprove { task_id: task.id };
        let event = handle_command(command, Arc::clone(&orch), Arc::clone(&emitter)).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                // Task should transition to Ready after approval
                assert!(matches!(state, TaskState::Ready | TaskState::Executing));
            }
            DaemonEvent::Error { message, .. } => {
                // If there's no plan set properly, this might error
                panic!("Unexpected error: {}", message);
            }
            other => panic!("Expected TaskStateChanged or Error event, got: {:?}", other),
        }
    }

    // --- TaskReject Tests ---

    #[tokio::test]
    async fn test_task_reject_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskReject {
            task_id: fake_id,
            reason: "Test rejection".to_string(),
        };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_reject_valid_task_returns_rejected_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskReject {
            task_id: task.id,
            reason: "Test rejection reason".to_string(),
        };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                assert_eq!(state, TaskState::Rejected);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    // --- TaskCancel Tests ---

    #[tokio::test]
    async fn test_task_cancel_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskCancel { task_id: fake_id };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_cancel_valid_task_returns_failed_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskCancel { task_id: task.id };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                assert_eq!(state, TaskState::Failed);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    // --- TaskList Tests ---

    #[tokio::test]
    async fn test_task_list_empty_returns_empty_array() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let command = CliCommand::TaskList;

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskCompleted { id, pr_url, .. } => {
                assert_eq!(id, Uuid::nil()); // nil indicates list response
                assert!(pr_url.is_some());
                let json = pr_url.unwrap();
                assert_eq!(json, "[]");
            }
            other => panic!(
                "Expected TaskCompleted event with nil UUID, got: {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_task_list_with_tasks_returns_task_array() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create some tasks
        orch.lock().await.create_task("Task 1".to_string()).await;
        orch.lock().await.create_task("Task 2".to_string()).await;
        orch.lock().await.create_task("Task 3".to_string()).await;

        let command = CliCommand::TaskList;
        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskCompleted { id, pr_url, .. } => {
                assert_eq!(id, Uuid::nil());
                assert!(pr_url.is_some());
                let json = pr_url.unwrap();
                // Parse the JSON and check we have 3 tasks
                let tasks: Vec<swell_core::Task> =
                    serde_json::from_str(&json).expect("Should be valid JSON");
                assert_eq!(tasks.len(), 3);
            }
            other => panic!("Expected TaskCompleted event, got: {:?}", other),
        }
    }

    // --- TaskWatch Tests ---

    #[tokio::test]
    async fn test_task_watch_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskWatch { task_id: fake_id };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_watch_valid_task_returns_current_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task (starts in Created state)
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskWatch { task_id: task.id };

        let event = handle_command(command, orch, emitter).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                assert_eq!(state, TaskState::Created);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_watch_after_state_change_reflects_new_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        // Transition to Enriched
        {
            let sm = orch.lock().await.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task.id).unwrap();
        }

        let command = CliCommand::TaskWatch { task_id: task.id };
        let event = handle_command(command, Arc::clone(&orch), Arc::clone(&emitter)).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                assert_eq!(state, TaskState::Enriched);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    // --- Error Handling Tests (VAL-DAEMON-003) ---

    #[tokio::test]
    async fn test_invalid_command_json_returns_error_via_parse() {
        let invalid_json = r#"{"type": "InvalidCommand", "payload": {}}"#;
        let result = parse_command(invalid_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid command JSON"));
    }

    #[tokio::test]
    async fn test_malformed_json_returns_error_via_parse() {
        let malformed_json = "not valid json at all";
        let result = parse_command(malformed_json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_partial_json_returns_error() {
        let partial_json = r#"{"type": "TaskCreate"#;
        let result = parse_command(partial_json);
        assert!(result.is_err());
    }

    // --- Event Serialization Tests ---

    #[tokio::test]
    async fn test_task_created_event_serializes_correctly() {
        let task_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::TaskCreated {
            id: task_id,
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("TaskCreated"));
        assert!(json.contains(&task_id.to_string()));
        assert!(json.contains(&correlation_id.to_string()));
    }

    #[tokio::test]
    async fn test_error_event_serializes_with_message_and_correlation_id() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::Error {
            message: "Test error message".to_string(),
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Test error message"));
        assert!(json.contains(&correlation_id.to_string()));
    }

    #[tokio::test]
    async fn test_task_state_changed_serializes_correctly() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::TaskStateChanged {
            id: Uuid::nil(),
            state: TaskState::Created,
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("TaskStateChanged"));
        assert!(json.contains("CREATED"));
        assert!(json.contains(&correlation_id.to_string()));
    }

    // --- Event Emitter Integration Tests ---

    #[tokio::test]
    async fn test_event_emitter_records_events_from_commands() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task
        let command = CliCommand::TaskCreate {
            description: "Test".to_string(),
        };
        handle_command(command, orch, Arc::clone(&emitter)).await;

        // Event should be recorded
        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_correlation_ids_link_related_events_within_single_operation() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();

        // Create a task - get the correlation_id
        let task = {
            let command = CliCommand::TaskCreate {
                description: "Test".to_string(),
            };
            let event = handle_command(command, Arc::clone(&orch), Arc::clone(&emitter)).await;
            match event {
                DaemonEvent::TaskCreated { id, correlation_id } => {
                    // Use the correlation_id for subsequent events
                    (id, correlation_id)
                }
                other => panic!("Expected TaskCreated event, got: {:?}", other),
            }
        };

        // Now do an approve operation - this is a NEW operation with its own correlation_id
        // but we can still verify that the task creation has its own correlation_id
        let command = CliCommand::TaskApprove { task_id: task.0 };
        let _ = handle_command(command, Arc::clone(&orch), Arc::clone(&emitter)).await;

        // Task creation event should be linkable by its correlation_id
        let events = emitter.get_events_by_correlation_id(task.1).await;
        assert_eq!(events.len(), 1);

        // The approve event has a different correlation_id
        // We can verify the emitter has more than 1 event total
        assert!(emitter.event_count().await >= 2);
    }

    #[tokio::test]
    async fn test_error_events_have_correlation_id() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let fake_id = Uuid::new_v4();

        // Try to approve a non-existent task - should return an error with a correlation_id
        let command = CliCommand::TaskApprove { task_id: fake_id };
        let event = handle_command(command, orch, Arc::clone(&emitter)).await;

        match event {
            DaemonEvent::Error {
                message,
                correlation_id,
            } => {
                assert!(!message.is_empty());
                assert!(correlation_id != Uuid::nil());
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    // --- parse_command Tests ---

    #[tokio::test]
    async fn test_parse_task_create_command() {
        let json = r#"{"type":"TaskCreate","payload":{"description":"test"}}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::TaskCreate { description } => {
                assert_eq!(description, "test");
            }
            other => panic!("Expected TaskCreate command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_list_command() {
        let json = r#"{"type":"TaskList"}"#;
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskList => {}
            other => panic!("Expected TaskList command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_watch_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskWatch","payload":{{"task_id":"{}"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskWatch { task_id: id } => {
                assert_eq!(id, task_id);
            }
            other => panic!("Expected TaskWatch command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_reject_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskReject","payload":{{"task_id":"{}","reason":"test reason"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskReject {
                task_id: id,
                reason,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(reason, "test reason");
            }
            other => panic!("Expected TaskReject command, got: {:?}", other),
        }
    }
}
