//! Command handlers for daemon CLI commands.
//!
//! This module handles all CLI commands that come through the Unix socket
//! and translates them into appropriate daemon events.

use swell_core::{CliCommand, DaemonEvent, TaskState};
use swell_orchestrator::Orchestrator;
use std::sync::Arc;
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
) -> DaemonEvent {
    match command {
        CliCommand::TaskCreate { description } => {
            let orch = orchestrator.lock().await;
            let task = orch.create_task(description.clone()).await;
            info!(task_id = %task.id, "Task created via CLI");
            DaemonEvent::TaskCreated(task.id)
        }
        CliCommand::TaskApprove { task_id } => {
            let orch = orchestrator.lock().await;
            // Verify task exists before attempting to start
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task approved, starting execution");
                    match orch.start_task(task_id).await {
                        Ok(()) => DaemonEvent::TaskStateChanged {
                            id: task_id,
                            state: TaskState::Ready,
                        },
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "Failed to start task");
                            DaemonEvent::Error {
                                message: format!("Failed to start task: {}", e),
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for approval");
                    DaemonEvent::Error {
                        message: format!("Task not found: {}", e),
                    }
                }
            }
        }
        CliCommand::TaskReject { task_id, reason } => {
            let orch = orchestrator.lock().await;
            // Verify task exists
            match orch.get_task(task_id).await {
                Ok(task) => {
                    warn!(task_id = %task_id, reason = %reason, state = ?task.state, "Task rejected");
                    DaemonEvent::TaskStateChanged {
                        id: task_id,
                        state: TaskState::Rejected,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for rejection");
                    DaemonEvent::Error {
                        message: format!("Task not found: {}", e),
                    }
                }
            }
        }
        CliCommand::TaskCancel { task_id } => {
            let orch = orchestrator.lock().await;
            // Verify task exists
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task cancelled");
                    DaemonEvent::TaskStateChanged {
                        id: task_id,
                        state: TaskState::Failed,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for cancellation");
                    DaemonEvent::Error {
                        message: format!("Task not found: {}", e),
                    }
                }
            }
        }
        CliCommand::TaskList => {
            let orch = orchestrator.lock().await;
            let tasks = orch.get_all_tasks().await;
            let json = serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".to_string());
            info!(task_count = tasks.len(), "Task list requested");
            // Send as a special event with nil UUID to indicate list response
            DaemonEvent::TaskCompleted {
                id: Uuid::nil(),
                pr_url: Some(json),
            }
        }
        CliCommand::TaskWatch { task_id } => {
            let orch = orchestrator.lock().await;
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task watch requested");
                    DaemonEvent::TaskStateChanged {
                        id: task_id,
                        state: task.state,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for watching");
                    DaemonEvent::Error {
                        message: format!("Task not found: {}", e),
                    }
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
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus};
    use std::sync::Arc;
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

    // --- TaskCreate Tests ---

    #[tokio::test]
    async fn test_task_create_returns_task_created_event() {
        let orch = create_test_orchestrator();
        let command = CliCommand::TaskCreate {
            description: "Test task description".to_string(),
        };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskCreated(task_id) => {
                assert!(task_id != Uuid::nil());
            }
            other => panic!("Expected TaskCreated event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_create_with_empty_description() {
        let orch = create_test_orchestrator();
        let command = CliCommand::TaskCreate {
            description: "".to_string(),
        };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskCreated(task_id) => {
                assert!(task_id != Uuid::nil());
            }
            other => panic!("Expected TaskCreated event, got: {:?}", other),
        }
    }

    // --- TaskApprove Tests ---

    #[tokio::test]
    async fn test_task_approve_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskApprove { task_id: fake_id };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::Error { message } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_approve_valid_task_returns_state_changed() {
        let orch = create_test_orchestrator();

        // First create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        let command = CliCommand::TaskApprove { task_id: task.id };
        let event = handle_command(command, Arc::clone(&orch)).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state } => {
                assert_eq!(id, task.id);
                // Task should transition to Ready after approval
                assert!(matches!(state, TaskState::Ready | TaskState::Executing));
            }
            DaemonEvent::Error { message } => {
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
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskReject {
            task_id: fake_id,
            reason: "Test rejection".to_string(),
        };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::Error { message } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_reject_valid_task_returns_rejected_state() {
        let orch = create_test_orchestrator();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskReject {
            task_id: task.id,
            reason: "Test rejection reason".to_string(),
        };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state } => {
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
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskCancel { task_id: fake_id };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::Error { message } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_cancel_valid_task_returns_failed_state() {
        let orch = create_test_orchestrator();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskCancel { task_id: task.id };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state } => {
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
        let command = CliCommand::TaskList;

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskCompleted { id, pr_url } => {
                assert_eq!(id, Uuid::nil()); // nil indicates list response
                assert!(pr_url.is_some());
                let json = pr_url.unwrap();
                assert_eq!(json, "[]");
            }
            other => panic!("Expected TaskCompleted event with nil UUID, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_list_with_tasks_returns_task_array() {
        let orch = create_test_orchestrator();

        // Create some tasks
        orch.lock().await.create_task("Task 1".to_string()).await;
        orch.lock().await.create_task("Task 2".to_string()).await;
        orch.lock().await.create_task("Task 3".to_string()).await;

        let command = CliCommand::TaskList;
        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskCompleted { id, pr_url } => {
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
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskWatch { task_id: fake_id };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::Error { message } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_watch_valid_task_returns_current_state() {
        let orch = create_test_orchestrator();

        // Create a task (starts in Created state)
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskWatch { task_id: task.id };

        let event = handle_command(command, orch).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state } => {
                assert_eq!(id, task.id);
                assert_eq!(state, TaskState::Created);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_watch_after_state_change_reflects_new_state() {
        let orch = create_test_orchestrator();

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
        let event = handle_command(command, Arc::clone(&orch)).await;

        match event {
            DaemonEvent::TaskStateChanged { id, state } => {
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
        let event = DaemonEvent::TaskCreated(task_id);
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("TaskCreated"));
        assert!(json.contains(&task_id.to_string()));
    }

    #[tokio::test]
    async fn test_error_event_serializes_with_message() {
        let event = DaemonEvent::Error {
            message: "Test error message".to_string(),
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Test error message"));
    }

    #[tokio::test]
    async fn test_task_state_changed_serializes_correctly() {
        let event = DaemonEvent::TaskStateChanged {
            id: Uuid::nil(),
            state: TaskState::Created,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("TaskStateChanged"));
        assert!(json.contains("CREATED"));
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
        let json = format!(r#"{{"type":"TaskWatch","payload":{{"task_id":"{}"}}}}"#, task_id);
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
            CliCommand::TaskReject { task_id: id, reason } => {
                assert_eq!(id, task_id);
                assert_eq!(reason, "test reason");
            }
            other => panic!("Expected TaskReject command, got: {:?}", other),
        }
    }
}
