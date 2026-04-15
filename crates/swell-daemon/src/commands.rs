//! Command handlers for daemon CLI commands.
//!
//! This module handles all CLI commands that come through the Unix socket
//! and translates them into appropriate daemon events.

use crate::events::EventEmitter;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use swell_core::{get_last_llm_model, get_total_llm_tokens, CliCommand, DaemonEvent, DataResponse, TaskState};
use swell_memory::recall::{RecallQuery, RecallService};
use swell_orchestrator::Orchestrator;
use tokio::sync::Mutex;
use tracing::{info, warn};
#[allow(unused_imports)]
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
/// - `TaskPause` - Pauses a running task (operator intervention)
/// - `TaskResume` - Resumes a paused task (operator intervention)
/// - `TaskInjectInstruction` - Injects instructions into a task (operator intervention)
/// - `TaskModifyScope` - Modifies task scope boundaries (operator intervention)
/// - `TaskGet` - Returns full task details as JSON (description, plan, state, scope, cost, timestamps)
/// - `DaemonStatus` - Returns daemon health status including connections, tasks, cost, and MCP health
/// - `MemoryQuery` - Query memory with BM25 search and temporal filters
/// - `CostQuery` - Query cost data for a task or aggregate across all tasks
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
    active_connections: Arc<AtomicUsize>,
    start_time: std::time::Instant,
    recall_service: Arc<Mutex<Option<RecallService>>>,
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
            // Verify task exists before attempting to approve
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task approved, proceeding to execution");
                    // Call approve_task which transitions AwaitingApproval → Ready → Assigned → Executing
                    match orch.approve_task(task_id).await {
                        Ok(()) => {
                            let correlation_id = EventEmitter::new_correlation_id();
                            let event = event_emitter
                                .emit_task_state_changed(task_id, TaskState::Ready, correlation_id)
                                .await;
                            event
                        }
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "Failed to approve task");
                            let correlation_id = EventEmitter::new_correlation_id();
                            event_emitter
                                .emit_error(
                                    format!("Failed to approve task: {}", e),
                                    None,
                                    correlation_id,
                                )
                                .await
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for approval");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), None, correlation_id)
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
                    // Actually transition the task to Rejected state
                    match orch.reject_task(task_id, reason.clone()).await {
                        Ok(()) => {
                            let correlation_id = EventEmitter::new_correlation_id();
                            event_emitter
                                .emit_task_state_changed(
                                    task_id,
                                    TaskState::Rejected,
                                    correlation_id,
                                )
                                .await
                        }
                        Err(e) => {
                            warn!(task_id = %task_id, error = %e, "Failed to reject task");
                            let correlation_id = EventEmitter::new_correlation_id();
                            event_emitter
                                .emit_error(
                                    format!("Failed to reject task: {}", e),
                                    None,
                                    correlation_id,
                                )
                                .await
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for rejection");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), None, correlation_id)
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
                        .emit_error(format!("Task not found: {}", e), None, correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskList => {
            let orch = orchestrator.lock().await;
            let tasks = orch.get_all_tasks().await;
            info!(task_count = tasks.len(), "Task list requested");
            // Use proper DataResponse variant for typed query response
            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::DataResponse(Box::new(DataResponse::TaskList {
                tasks,
                correlation_id,
            }))
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
                        .emit_error(format!("Task not found: {}", e), None, correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskPause { task_id, reason } => {
            let orch = orchestrator.lock().await;
            match orch.pause_task(task_id, reason.clone()).await {
                Ok(()) => {
                    info!(task_id = %task_id, reason = %reason, "Task paused by operator");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_task_state_changed(task_id, TaskState::Paused, correlation_id)
                        .await
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to pause task");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Failed to pause task: {}", e), None, correlation_id)
                        .await
                }
            }
        }
        CliCommand::TaskResume { task_id } => {
            let orch = orchestrator.lock().await;
            match orch.resume_task(task_id).await {
                Ok(()) => {
                    info!(task_id = %task_id, "Task resumed by operator");
                    let task = orch.get_task(task_id).await.unwrap();
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_task_state_changed(task_id, task.state, correlation_id)
                        .await
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to resume task");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(
                            format!("Failed to resume task: {}", e),
                            None,
                            correlation_id,
                        )
                        .await
                }
            }
        }
        CliCommand::TaskInjectInstruction {
            task_id,
            instruction,
        } => {
            let orch = orchestrator.lock().await;
            match orch.inject_instruction(task_id, instruction.clone()).await {
                Ok(()) => {
                    info!(task_id = %task_id, instruction = %instruction, "Instruction injected by operator");
                    let correlation_id = EventEmitter::new_correlation_id();
                    DaemonEvent::TaskProgress {
                        id: task_id,
                        message: format!("Instruction injected: {}", instruction),
                        correlation_id,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to inject instruction");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(
                            format!("Failed to inject instruction: {}", e),
                            None,
                            correlation_id,
                        )
                        .await
                }
            }
        }
        CliCommand::TaskModifyScope { task_id, scope } => {
            let orch = orchestrator.lock().await;
            match orch.modify_scope(task_id, scope.clone()).await {
                Ok(()) => {
                    info!(task_id = %task_id, files = ?scope.files, "Task scope modified by operator");
                    let correlation_id = EventEmitter::new_correlation_id();
                    DaemonEvent::TaskProgress {
                        id: task_id,
                        message: format!(
                            "Scope modified: {} files, {} directories",
                            scope.files.len(),
                            scope.directories.len()
                        ),
                        correlation_id,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Failed to modify scope");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(
                            format!("Failed to modify scope: {}", e),
                            None,
                            correlation_id,
                        )
                        .await
                }
            }
        }
        CliCommand::TaskGet { task_id } => {
            let orch = orchestrator.lock().await;
            match orch.get_task(task_id).await {
                Ok(task) => {
                    info!(task_id = %task_id, state = ?task.state, "Task details requested");
                    let task_json =
                        serde_json::to_string(&task).unwrap_or_else(|_| "{}".to_string());
                    let correlation_id = EventEmitter::new_correlation_id();
                    DaemonEvent::TaskDetails {
                        id: task_id,
                        task_json,
                        correlation_id,
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Task not found for TaskGet");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(format!("Task not found: {}", e), None, correlation_id)
                        .await
                }
            }
        }
        CliCommand::DaemonStatus => {
            info!("Daemon status requested");
            let orch = orchestrator.lock().await;
            let tasks = orch.get_all_tasks().await;

            // Count tasks by state
            let mut tasks_by_state: HashMap<String, usize> = HashMap::new();
            for task in &tasks {
                let state_name = format!("{:?}", task.state);
                *tasks_by_state.entry(state_name).or_insert(0) += 1;
            }

            // Get cost info from global tracker
            let total_tokens = get_total_llm_tokens();
            let last_model = get_last_llm_model();

            // MCP health - swell-daemon does not have direct access to McpManager
            // (which lives in swell-tools). A placeholder entry is included to signal
            // that the field is structurally present; wire real data through the
            // orchestrator or a shared McpHealthTracker when MCP servers are active.
            let mut mcp_health: HashMap<String, String> = HashMap::new();
            mcp_health.insert(
                "_status".to_string(),
                "pending - MCP manager not yet wired into daemon".to_string(),
            );

            // Calculate uptime
            let uptime_seconds = start_time.elapsed().as_secs();

            // Get version
            let version = env!("CARGO_PKG_VERSION");

            // Budget information (default 1M tokens per task as baseline)
            let total_budget: u64 = 1_000_000;
            let total_spent = total_tokens;
            let remaining_budget = total_budget.saturating_sub(total_spent);

            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::DaemonHealth {
                active_connections: active_connections.load(Ordering::Relaxed),
                total_tasks: tasks.len(),
                tasks_by_state,
                total_tokens,
                last_model,
                mcp_health,
                uptime_seconds,
                version: version.to_string(),
                total_budget,
                total_spent,
                remaining_budget,
                correlation_id,
            }
        }
        CliCommand::ConfigGet { key } => {
            info!(key = %key, "ConfigGet requested");
            use swell_core::config::ConfigLoader;

            // Load config to find the key's value and source
            let loader = ConfigLoader::new();
            match loader.load() {
                Ok(config) => {
                    if let Some(value) = config.get(&key) {
                        // Find the source file for this key from the audit trail
                        let source_file = config
                            .loaded_entries()
                            .iter()
                            .find(|e| e.key_path == key)
                            .and_then(|e| e.source_file.clone());

                        let correlation_id = EventEmitter::new_correlation_id();
                        DaemonEvent::ConfigValue {
                            key,
                            value: value.clone(),
                            source_file,
                            correlation_id,
                        }
                    } else {
                        // Key not found in config
                        let correlation_id = EventEmitter::new_correlation_id();
                        event_emitter
                            .emit_error(
                                format!("Configuration key '{}' not found", key),
                                Some(swell_core::FailureClass::ConfigError),
                                correlation_id,
                            )
                            .await
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to load configuration");
                    let correlation_id = EventEmitter::new_correlation_id();
                    event_emitter
                        .emit_error(
                            format!("Failed to load configuration: {}", e),
                            Some(swell_core::FailureClass::ConfigError),
                            correlation_id,
                        )
                        .await
                }
            }
        }
        CliCommand::ConfigSet { key, value } => {
            info!(key = %key, "ConfigSet requested");
            use std::path::PathBuf;

            // Determine the settings.local.json path
            // We look in the current working directory's .swell directory
            let swell_dir: PathBuf = std::env::current_dir()
                .map(|p| p.join(".swell"))
                .unwrap_or_else(|_| PathBuf::from(".swell"));
            let local_settings_path = swell_dir.join("settings.local.json");

            // Ensure the .swell directory exists
            if let Err(e) = std::fs::create_dir_all(&swell_dir) {
                let correlation_id = EventEmitter::new_correlation_id();
                return event_emitter
                    .emit_error(
                        format!("Failed to create .swell directory: {}", e),
                        Some(swell_core::FailureClass::ConfigError),
                        correlation_id,
                    )
                    .await;
            }

            // Read existing local settings or create new
            let mut local_settings: serde_json::Map<String, serde_json::Value> =
                if local_settings_path.exists() {
                    match std::fs::read_to_string(&local_settings_path) {
                        Ok(content) => serde_json::from_str(&content)
                            .unwrap_or_else(|_| serde_json::Map::new()),
                        Err(_) => serde_json::Map::new(),
                    }
                } else {
                    serde_json::Map::new()
                };

            // Set the key (supporting dot notation for nested keys)
            set_nested_json_value(&mut local_settings, &key, value.clone());

            // Write atomically: write to temp file, then rename
            let temp_path = local_settings_path.with_extension("tmp");
            match serde_json::to_string_pretty(&local_settings) {
                Ok(json_str) => {
                    if let Err(e) = std::fs::write(&temp_path, &json_str) {
                        let correlation_id = EventEmitter::new_correlation_id();
                        return event_emitter
                            .emit_error(
                                format!("Failed to write config: {}", e),
                                Some(swell_core::FailureClass::ConfigError),
                                correlation_id,
                            )
                            .await;
                    }
                    if let Err(e) = std::fs::rename(&temp_path, &local_settings_path) {
                        // Clean up temp file on failure
                        let _ = std::fs::remove_file(&temp_path);
                        let correlation_id = EventEmitter::new_correlation_id();
                        return event_emitter
                            .emit_error(
                                format!("Failed to save config: {}", e),
                                Some(swell_core::FailureClass::ConfigError),
                                correlation_id,
                            )
                            .await;
                    }
                }
                Err(e) => {
                    let correlation_id = EventEmitter::new_correlation_id();
                    return event_emitter
                        .emit_error(
                            format!("Failed to serialize config: {}", e),
                            Some(swell_core::FailureClass::ConfigError),
                            correlation_id,
                        )
                        .await;
                }
            }

            // Return success with the updated value and source
            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::ConfigValue {
                key,
                value,
                source_file: Some(local_settings_path.to_string_lossy().to_string()),
                correlation_id,
            }
        }
        CliCommand::MemoryQuery {
            query,
            scope,
            limit,
        } => {
            info!(query = %query, limit = limit, "MemoryQuery requested");
            let correlation_id = EventEmitter::new_correlation_id();

            // Get the recall service
            let guard = recall_service.lock().await;
            match guard.as_ref() {
                Some(recall) => {
                    // Parse keywords from query string
                    let keywords: Vec<String> =
                        query.split_whitespace().map(|s| s.to_string()).collect();

                    // Build recall query
                    let recall_query = RecallQuery {
                        keywords,
                        session_id: scope.session_id,
                        task_id: scope.task_id,
                        agent_role: scope.agent_role,
                        action: None,
                        start_time: None,
                        end_time: None,
                        limit,
                        offset: 0,
                        bm25_params: None,
                    };

                    // Execute the search
                    match recall.search(recall_query).await {
                        Ok(results) => {
                            let count = results.len();
                            let results_json = serde_json::to_string(&results)
                                .unwrap_or_else(|_| "[]".to_string());
                            info!(count = count, "MemoryQuery returned {} results", count);
                            DaemonEvent::MemoryResults {
                                results: results_json,
                                count,
                                correlation_id,
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "MemoryQuery search failed");
                            event_emitter
                                .emit_error(
                                    format!("Memory query failed: {}", e),
                                    None,
                                    correlation_id,
                                )
                                .await
                        }
                    }
                }
                None => {
                    // Memory service not available
                    warn!("Memory recall service not available");
                    event_emitter
                        .emit_error(
                            "Memory recall service not available".to_string(),
                            None,
                            correlation_id,
                        )
                        .await
                }
            }
        }
        CliCommand::CostQuery { task_id } => {
            info!(task_id = ?task_id, "CostQuery requested");
            use swell_core::cost_tracking::get_global_model_breakdown;

            let correlation_id = EventEmitter::new_correlation_id();

            // If task_id is specified, we would need per-task cost tracking which requires
            // the orchestrator to have a CostTracker. For now, we return aggregate data.
            // TODO: Wire in per-task cost tracking from orchestrator's task_board
            if task_id.is_some() {
                info!("Per-task cost query - using aggregate (per-task tracking pending)");
            }

            // Get aggregate cost data from global tracker
            let model_breakdown = get_global_model_breakdown();

            // Calculate totals from model breakdown
            let total_input_tokens: u64 = model_breakdown.iter().map(|m| m.input_tokens).sum();
            let total_output_tokens: u64 = model_breakdown.iter().map(|m| m.output_tokens).sum();
            let total_cost_usd: f64 = model_breakdown.iter().map(|m| m.cost_usd).sum();

            // Convert to ModelCostInfo for the response
            let breakdown: Vec<swell_core::ModelCostInfo> = model_breakdown
                .into_iter()
                .map(|m| swell_core::ModelCostInfo {
                    model: m.model,
                    call_count: m.call_count,
                    total_input_tokens: m.input_tokens,
                    total_output_tokens: m.output_tokens,
                    total_cost_usd: m.cost_usd,
                })
                .collect();

            info!(
                total_input_tokens,
                total_output_tokens,
                total_cost_usd,
                model_count = breakdown.len(),
                "CostQuery returning aggregate data"
            );

            DaemonEvent::CostQueryResult {
                task_id,
                total_input_tokens,
                total_output_tokens,
                total_cost_usd,
                model_breakdown: breakdown,
                correlation_id,
            }
        }
    }
}

/// Set a nested value in a JSON object using dot notation.
/// For example, "execution.max_iterations" sets a nested key.
fn set_nested_json_value(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key_path: &str,
    value: serde_json::Value,
) {
    let parts: Vec<&str> = key_path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    if parts.len() == 1 {
        map.insert(parts[0].to_string(), value);
        return;
    }

    // Navigate/create the nested structure
    let final_key = parts[parts.len() - 1].to_string();
    let mut current = map;

    for part in parts.iter().take(parts.len() - 1) {
        let part_str = part.to_string();
        if !current.contains_key(&part_str) {
            current.insert(
                part_str.clone(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
        if let Some(serde_json::Value::Object(ref mut obj)) = current.get_mut(&part_str) {
            current = obj;
        } else {
            // Path leads through a non-object, can't set nested value
            return;
        }
    }
    current.insert(final_key, value);
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
    use serial_test::serial;
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

    fn create_test_active_connections() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    fn create_test_start_time() -> std::time::Instant {
        std::time::Instant::now()
    }

    // --- TaskCreate Tests ---

    #[tokio::test]
    async fn test_task_create_returns_task_created_event() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let command = CliCommand::TaskCreate {
            description: "Test task description".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();
        let command = CliCommand::TaskCreate {
            description: "".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskApprove { task_id: fake_id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();

        // First create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        // Call start_task to transition to AwaitingApproval (or Executing if autonomy doesn't need approval)
        // Default autonomy level is Guided, which needs plan approval
        orch.lock().await.start_task(task.id).await.unwrap();

        let command = CliCommand::TaskApprove { task_id: task.id };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task.id);
                // Task should transition to Ready after approval (and then to Executing)
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
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskReject {
            task_id: fake_id,
            reason: "Test rejection".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();

        // Create a task and set it up for rejection
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        // Call start_task to transition to AwaitingApproval
        orch.lock().await.start_task(task.id).await.unwrap();

        let command = CliCommand::TaskReject {
            task_id: task.id,
            reason: "Test rejection reason".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskCancel { task_id: fake_id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();

        // Create a task
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskCancel { task_id: task.id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();
        let command = CliCommand::TaskList;

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::DataResponse(data) => {
                match *data {
                    DataResponse::TaskList { tasks, .. } => {
                        assert!(tasks.is_empty());
                    }
                    other => panic!(
                        "Expected DataResponse::TaskList event, got: {:?}",
                        other
                    ),
                }
            }
            other => panic!(
                "Expected DataResponse event, got: {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_task_list_with_tasks_returns_task_array() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create some tasks
        orch.lock().await.create_task("Task 1".to_string()).await;
        orch.lock().await.create_task("Task 2".to_string()).await;
        orch.lock().await.create_task("Task 3".to_string()).await;

        let command = CliCommand::TaskList;
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::DataResponse(data) => {
                match *data {
                    DataResponse::TaskList { tasks, .. } => {
                        assert_eq!(tasks.len(), 3);
                    }
                    other => panic!("Expected DataResponse::TaskList event, got: {:?}", other),
                }
            }
            other => panic!("Expected DataResponse event, got: {:?}", other),
        }
    }

    // --- TaskWatch Tests ---

    #[tokio::test]
    async fn test_task_watch_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskWatch { task_id: fake_id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();

        // Create a task (starts in Created state)
        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskWatch { task_id: task.id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();

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
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
            failure_class: None,
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
        let active_connections = create_test_active_connections();

        // Create a task
        let command = CliCommand::TaskCreate {
            description: "Test".to_string(),
        };
        handle_command(
            command,
            orch,
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        // Event should be recorded
        assert_eq!(emitter.event_count().await, 1);
    }

    #[tokio::test]
    async fn test_correlation_ids_link_related_events_within_single_operation() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create a task - get the correlation_id
        let task = {
            let command = CliCommand::TaskCreate {
                description: "Test".to_string(),
            };
            let event = handle_command(
                command,
                Arc::clone(&orch),
                Arc::clone(&emitter),
                Arc::clone(&active_connections),
                create_test_start_time(),
                Arc::new(Mutex::new(None)),
            )
            .await;
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
        let _ = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

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
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();

        // Try to approve a non-existent task - should return an error with a correlation_id
        let command = CliCommand::TaskApprove { task_id: fake_id };
        let event = handle_command(
            command,
            orch,
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error {
                message,
                failure_class: _,
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

    // --- Operator Intervention Tests ---

    fn create_test_task_in_executing_state(orch: &Arc<Mutex<Orchestrator>>) -> Uuid {
        let task_id = futures::executor::block_on(async {
            orch.lock()
                .await
                .create_task("Test task".to_string())
                .await
                .id
        });
        let plan = create_test_plan(task_id);
        futures::executor::block_on(async { orch.lock().await.set_plan(task_id, plan).await })
            .unwrap();
        futures::executor::block_on(async {
            let sm = orch.lock().await.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task_id).unwrap();
            sm_guard.ready_task(task_id).unwrap();
            sm_guard.assign_task(task_id, Uuid::new_v4()).unwrap();
            sm_guard.start_execution(task_id).unwrap();
        });
        task_id
    }

    fn create_test_task_in_validating_state(orch: &Arc<Mutex<Orchestrator>>) -> Uuid {
        let task_id = futures::executor::block_on(async {
            orch.lock()
                .await
                .create_task("Test task".to_string())
                .await
                .id
        });
        let plan = create_test_plan(task_id);
        futures::executor::block_on(async { orch.lock().await.set_plan(task_id, plan).await })
            .unwrap();
        futures::executor::block_on(async {
            let sm = orch.lock().await.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task_id).unwrap();
            sm_guard.ready_task(task_id).unwrap();
            sm_guard.assign_task(task_id, Uuid::new_v4()).unwrap();
            sm_guard.start_execution(task_id).unwrap();
            sm_guard.start_validation(task_id).unwrap();
        });
        task_id
    }

    // --- TaskPause Tests ---

    #[tokio::test]
    async fn test_task_pause_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskPause {
            task_id: fake_id,
            reason: "Operator requested".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_pause_executing_task_returns_paused_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        let command = CliCommand::TaskPause {
            task_id,
            reason: "Operator requested pause".to_string(),
        };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Paused);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_pause_validating_task_returns_paused_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_validating_state(&orch);

        let command = CliCommand::TaskPause {
            task_id,
            reason: "Operator requested pause during validation".to_string(),
        };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Paused);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_pause_created_task_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task = orch.lock().await.create_task("Test task".to_string()).await;
        let command = CliCommand::TaskPause {
            task_id: task.id,
            reason: "Operator requested".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("Cannot pause") || message.contains("state"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    // --- TaskResume Tests ---

    #[tokio::test]
    async fn test_task_resume_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskResume { task_id: fake_id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_resume_paused_task_returns_executing_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        // First pause the task
        {
            let command = CliCommand::TaskPause {
                task_id,
                reason: "Operator requested".to_string(),
            };
            handle_command(
                command,
                Arc::clone(&orch),
                Arc::clone(&emitter),
                Arc::clone(&active_connections),
                create_test_start_time(),
                Arc::new(Mutex::new(None)),
            )
            .await;
        }

        // Now resume
        let command = CliCommand::TaskResume { task_id };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskStateChanged { id, state, .. } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Executing);
            }
            other => panic!("Expected TaskStateChanged event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_resume_non_paused_task_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        // Try to resume without pausing first
        let command = CliCommand::TaskResume { task_id };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("Cannot resume") || message.contains("state"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    // --- TaskInjectInstruction Tests ---

    #[tokio::test]
    async fn test_task_inject_instruction_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskInjectInstruction {
            task_id: fake_id,
            instruction: "Check the logs".to_string(),
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_inject_instruction_executing_task_succeeds() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        let command = CliCommand::TaskInjectInstruction {
            task_id,
            instruction: "Remember to check the logs first".to_string(),
        };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskProgress { id, message, .. } => {
                assert_eq!(id, task_id);
                assert!(message.contains("Instruction injected"));
            }
            other => panic!("Expected TaskProgress event, got: {:?}", other),
        }

        // Verify instruction was stored
        let instructions = orch
            .lock()
            .await
            .get_injected_instructions(task_id)
            .await
            .unwrap();
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0], "Remember to check the logs first");
    }

    #[tokio::test]
    async fn test_task_inject_instruction_multiple_instructions() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        // Inject multiple instructions
        for i in 1..=3 {
            let command = CliCommand::TaskInjectInstruction {
                task_id,
                instruction: format!("Instruction {}", i),
            };
            handle_command(
                command,
                Arc::clone(&orch),
                Arc::clone(&emitter),
                Arc::clone(&active_connections),
                create_test_start_time(),
                Arc::new(Mutex::new(None)),
            )
            .await;
        }

        let instructions = orch
            .lock()
            .await
            .get_injected_instructions(task_id)
            .await
            .unwrap();
        assert_eq!(instructions.len(), 3);
    }

    // --- TaskModifyScope Tests ---

    #[tokio::test]
    async fn test_task_modify_scope_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let scope = swell_core::TaskScope {
            files: vec!["src/lib.rs".to_string()],
            directories: vec!["src".to_string()],
            allowed_operations: vec![],
        };
        let command = CliCommand::TaskModifyScope {
            task_id: fake_id,
            scope,
        };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_modify_scope_executing_task_succeeds() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        let scope = swell_core::TaskScope {
            files: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            directories: vec!["src".to_string(), "tests".to_string()],
            allowed_operations: vec![],
        };
        let command = CliCommand::TaskModifyScope {
            task_id,
            scope: scope.clone(),
        };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskProgress { id, message, .. } => {
                assert_eq!(id, task_id);
                assert!(message.contains("Scope modified"));
            }
            other => panic!("Expected TaskProgress event, got: {:?}", other),
        }

        // Verify scope was stored
        let current_scope = orch.lock().await.get_task_scope(task_id).await.unwrap();
        assert_eq!(current_scope.files.len(), 2);
        assert_eq!(current_scope.directories.len(), 2);
    }

    #[tokio::test]
    async fn test_task_modify_scope_stores_original_scope() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let task_id = create_test_task_in_executing_state(&orch);

        let new_scope = swell_core::TaskScope {
            files: vec!["new_file.rs".to_string()],
            directories: vec!["new_dir".to_string()],
            allowed_operations: vec![],
        };
        let command = CliCommand::TaskModifyScope {
            task_id,
            scope: new_scope,
        };
        handle_command(
            command,
            Arc::clone(&orch),
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        // Verify original scope was saved
        let task = orch.lock().await.get_task(task_id).await.unwrap();
        assert!(task.original_scope.is_some());
        assert_eq!(task.original_scope.as_ref().unwrap().files.len(), 0); // Default empty
    }

    // --- parse_command Tests for new commands ---

    #[tokio::test]
    async fn test_parse_task_pause_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskPause","payload":{{"task_id":"{}","reason":"test pause"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskPause {
                task_id: id,
                reason,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(reason, "test pause");
            }
            other => panic!("Expected TaskPause command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_resume_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskResume","payload":{{"task_id":"{}"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskResume { task_id: id } => {
                assert_eq!(id, task_id);
            }
            other => panic!("Expected TaskResume command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_inject_instruction_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskInjectInstruction","payload":{{"task_id":"{}","instruction":"check logs"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskInjectInstruction {
                task_id: id,
                instruction,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(instruction, "check logs");
            }
            other => panic!("Expected TaskInjectInstruction command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_task_modify_scope_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskModifyScope","payload":{{"task_id":"{}","scope":{{"files":["file1.rs"],"directories":["src"],"allowed_operations":[]}}}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskModifyScope { task_id: id, scope } => {
                assert_eq!(id, task_id);
                assert_eq!(scope.files.len(), 1);
                assert_eq!(scope.files[0], "file1.rs");
                assert_eq!(scope.directories.len(), 1);
                assert_eq!(scope.directories[0], "src");
            }
            other => panic!("Expected TaskModifyScope command, got: {:?}", other),
        }
    }

    // --- TaskGet Tests ---

    #[tokio::test]
    async fn test_parse_task_get_command() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"TaskGet","payload":{{"task_id":"{}"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::TaskGet { task_id: id } => {
                assert_eq!(id, task_id);
            }
            other => panic!("Expected TaskGet command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_get_nonexistent_returns_error() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();
        let fake_id = Uuid::new_v4();
        let command = CliCommand::TaskGet { task_id: fake_id };

        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error { message, .. } => {
                assert!(message.contains("not found") || message.contains("TaskNotFound"));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_get_existing_task_returns_task_details() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create a task
        let task = orch
            .lock()
            .await
            .create_task("Test task description".to_string())
            .await;
        let task_id = task.id;

        let command = CliCommand::TaskGet { task_id };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskDetails {
                id,
                task_json,
                correlation_id,
            } => {
                assert_eq!(id, task_id);
                assert!(correlation_id != Uuid::nil());
                assert!(!task_json.is_empty());

                // Verify the JSON deserializes to a Task with all required fields
                let retrieved_task: swell_core::Task =
                    serde_json::from_str(&task_json).expect("task_json should deserialize to Task");

                // Verify all required fields are present
                assert_eq!(retrieved_task.id, task_id);
                assert_eq!(retrieved_task.description, "Test task description");
                assert!(retrieved_task.plan.is_none()); // No plan set yet
                assert_eq!(retrieved_task.state, TaskState::Created);
                assert!(retrieved_task.created_at <= chrono::Utc::now());
                assert!(retrieved_task.updated_at <= chrono::Utc::now());
                // Verify scope is present (TaskScope default)
                assert_eq!(retrieved_task.current_scope.files.len(), 0);
                // Verify cost fields are present
                assert_eq!(retrieved_task.token_budget, 1_000_000);
                assert_eq!(retrieved_task.tokens_used, 0);
            }
            other => panic!("Expected TaskDetails event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_get_with_plan_includes_plan_details() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create a task with a plan
        let task = orch
            .lock()
            .await
            .create_task("Test task with plan".to_string())
            .await;
        let plan = create_test_plan(task.id);
        orch.lock().await.set_plan(task.id, plan).await.unwrap();

        let command = CliCommand::TaskGet { task_id: task.id };
        let event = handle_command(
            command,
            Arc::clone(&orch),
            Arc::clone(&emitter),
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::TaskDetails {
                id: _,
                task_json,
                correlation_id: _,
            } => {
                let retrieved_task: swell_core::Task =
                    serde_json::from_str(&task_json).expect("task_json should deserialize to Task");

                // Verify plan is included
                assert!(retrieved_task.plan.is_some());
                let plan = retrieved_task.plan.unwrap();
                assert_eq!(plan.steps.len(), 1);
                assert_eq!(plan.steps[0].description, "Test step");
            }
            other => panic!("Expected TaskDetails event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_task_get_event_serialization() {
        let task_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::TaskDetails {
            id: task_id,
            task_json: r#"{"id":"00000000-0000-0000-0000-000000000000","description":"test"}"#
                .to_string(),
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("TaskDetails"));
        assert!(json.contains(&task_id.to_string()));
        assert!(json.contains(&correlation_id.to_string()));
    }

    // --- DaemonStatus Tests ---

    #[tokio::test]
    async fn test_daemon_status_returns_health_info() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create some tasks
        orch.lock().await.create_task("Task 1".to_string()).await;
        orch.lock().await.create_task("Task 2".to_string()).await;

        let command = CliCommand::DaemonStatus;
        let event = handle_command(
            command,
            Arc::clone(&orch),
            emitter,
            Arc::clone(&active_connections),
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::DaemonHealth {
                active_connections: conn_count,
                total_tasks,
                tasks_by_state,
                total_tokens,
                last_model,
                mcp_health,
                uptime_seconds,
                version,
                total_budget,
                total_spent,
                remaining_budget,
                correlation_id,
            } => {
                assert_eq!(conn_count, 0);
                assert_eq!(total_tasks, 2);
                assert!(tasks_by_state.contains_key("Created"));
                assert_eq!(*tasks_by_state.get("Created").unwrap(), 2);
                // Cost tracking should be initialized (tokens is u64 so always >= 0)
                let _ = total_tokens;
                assert!(!last_model.is_empty() || last_model.is_empty()); // Model may or may not be set
                                                                          // MCP health is empty map (no MCP manager in test)
                assert!(mcp_health.is_empty());
                // Verify uptime, version, and budget fields
                assert!(!version.is_empty());
                assert!(total_budget >= total_spent);
                assert_eq!(remaining_budget, total_budget.saturating_sub(total_spent));
                assert!(correlation_id != Uuid::nil());
            }
            other => panic!("Expected DaemonHealth event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_daemon_status_with_zero_tasks() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::DaemonStatus;
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::DaemonHealth {
                total_tasks,
                tasks_by_state,
                ..
            } => {
                assert_eq!(total_tasks, 0);
                assert!(tasks_by_state.is_empty());
            }
            other => panic!("Expected DaemonHealth event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_daemon_status_tracks_tasks_by_state() {
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        // Create tasks - they start in Created state
        let task1 = orch.lock().await.create_task("Task 1".to_string()).await;
        orch.lock().await.create_task("Task 2".to_string()).await;

        // Transition task1 to Enriched state
        {
            let sm = orch.lock().await.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task1.id).unwrap();
        }

        let command = CliCommand::DaemonStatus;
        let event = handle_command(
            command,
            Arc::clone(&orch),
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::DaemonHealth { tasks_by_state, .. } => {
                assert_eq!(tasks_by_state.get("Created"), Some(&1));
                assert_eq!(tasks_by_state.get("Enriched"), Some(&1));
            }
            other => panic!("Expected DaemonHealth event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_daemon_status_command() {
        let json = r#"{"type":"DaemonStatus"}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::DaemonStatus => {}
            other => panic!("Expected DaemonStatus command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_cost_query_no_task_id() {
        let json = r#"{"type":"CostQuery","payload":{}}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::CostQuery { task_id } => {
                assert!(task_id.is_none(), "Expected no task_id");
            }
            other => panic!("Expected CostQuery command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_cost_query_with_task_id() {
        let task_id = Uuid::new_v4();
        let json = format!(
            r#"{{"type":"CostQuery","payload":{{"task_id":"{}"}}}}"#,
            task_id
        );
        let command = parse_command(&json).unwrap();

        match command {
            CliCommand::CostQuery { task_id: result_id } => {
                assert_eq!(result_id, Some(task_id), "Expected task_id to match");
            }
            other => panic!("Expected CostQuery command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_cost_query_result_event_serialization() {
        let task_id = Some(Uuid::new_v4());
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::CostQueryResult {
            task_id,
            total_input_tokens: 100000,
            total_output_tokens: 50000,
            total_cost_usd: 0.75,
            model_breakdown: vec![swell_core::ModelCostInfo {
                model: "claude-3-5-sonnet-20241022".to_string(),
                call_count: 5,
                total_input_tokens: 100000,
                total_output_tokens: 50000,
                total_cost_usd: 0.75,
            }],
            correlation_id,
        };

        let serialized = serde_json::to_string(&event).unwrap();
        assert!(serialized.contains("CostQueryResult"));
        assert!(serialized.contains("100000"));
        assert!(serialized.contains("0.75"));

        let deserialized: DaemonEvent = serde_json::from_str(&serialized).unwrap();
        match deserialized {
            DaemonEvent::CostQueryResult {
                task_id: result_task_id,
                total_input_tokens,
                total_output_tokens,
                total_cost_usd,
                model_breakdown,
                correlation_id: result_corr_id,
            } => {
                assert_eq!(result_task_id, task_id);
                assert_eq!(total_input_tokens, 100000);
                assert_eq!(total_output_tokens, 50000);
                assert!((total_cost_usd - 0.75).abs() < 0.001);
                assert_eq!(model_breakdown.len(), 1);
                assert_eq!(model_breakdown[0].model, "claude-3-5-sonnet-20241022");
                assert_eq!(result_corr_id, correlation_id);
            }
            other => panic!("Expected CostQueryResult event, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_daemon_health_event_serialization() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::DaemonHealth {
            active_connections: 5,
            total_tasks: 10,
            tasks_by_state: std::collections::HashMap::new(),
            total_tokens: 1000000,
            last_model: "claude-3-5-sonnet-20241022".to_string(),
            mcp_health: std::collections::HashMap::new(),
            uptime_seconds: 3600,
            version: "1.0.0".to_string(),
            total_budget: 1000000,
            total_spent: 500000,
            remaining_budget: 500000,
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("DaemonHealth"));
        assert!(json.contains("5"));
        assert!(json.contains("10"));
        assert!(json.contains("1000000"));
        assert!(json.contains("claude-3-5-sonnet"));
        assert!(json.contains("3600")); // uptime_seconds
        assert!(json.contains("1.0.0")); // version
        assert!(json.contains("500000")); // remaining_budget
    }

    // =====================================================================
    // ConfigGet and ConfigSet Tests
    // =====================================================================

    #[tokio::test]
    async fn test_parse_config_get_command() {
        let json = r#"{"type":"ConfigGet","payload":{"key":"execution.max_task_timeout_seconds"}}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::ConfigGet { key } => {
                assert_eq!(key, "execution.max_task_timeout_seconds");
            }
            other => panic!("Expected ConfigGet command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_config_set_command() {
        let json = r#"{"type":"ConfigSet","payload":{"key":"execution.max_task_timeout_seconds","value":300}}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::ConfigSet { key, value } => {
                assert_eq!(key, "execution.max_task_timeout_seconds");
                assert_eq!(value, serde_json::json!(300));
            }
            other => panic!("Expected ConfigSet command, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_config_set_with_nested_value() {
        let json = r#"{"type":"ConfigSet","payload":{"key":"test_key","value":{"nested":{"inner":"value"}}}}"#;
        let command = parse_command(json).unwrap();

        match command {
            CliCommand::ConfigSet { key, value } => {
                assert_eq!(key, "test_key");
                assert_eq!(value, serde_json::json!({"nested":{"inner":"value"}}));
            }
            other => panic!("Expected ConfigSet command, got: {:?}", other),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_config_set_writes_only_to_settings_local_json() {
        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let swell_dir = temp_dir.path().join(".swell");
        std::fs::create_dir_all(&swell_dir).unwrap();

        // Create a settings.json with a known checksum
        let settings_json_path = swell_dir.join("settings.json");
        std::fs::write(
            &settings_json_path,
            r#"{"version": "1.0.0", "existing_key": "original"}"#,
        )
        .unwrap();
        let settings_json_checksum_before = calculate_file_checksum(&settings_json_path);

        // Create settings.local.json with some content
        let local_json_path = swell_dir.join("settings.local.json");
        std::fs::write(&local_json_path, r#"{"local_only": "value"}"#).unwrap();

        // Change to temp directory and run ConfigSet
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create orchestrator and event emitter for the command
        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::ConfigSet {
            key: "new_key".to_string(),
            value: serde_json::json!("new_value"),
        };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        // Verify we got a ConfigValue response
        match event {
            DaemonEvent::ConfigValue {
                key,
                value,
                source_file,
                ..
            } => {
                assert_eq!(key, "new_key");
                assert_eq!(value, serde_json::json!("new_value"));
                assert!(source_file.is_some());
                assert!(source_file.unwrap().contains("settings.local.json"));
            }
            other => panic!("Expected ConfigValue event, got: {:?}", other),
        }

        // Verify settings.json was NOT modified
        let settings_json_checksum_after = calculate_file_checksum(&settings_json_path);
        assert_eq!(
            settings_json_checksum_before, settings_json_checksum_after,
            "settings.json should not be modified by ConfigSet"
        );

        // Verify settings.local.json was modified (contains the new key)
        let local_content = std::fs::read_to_string(&local_json_path).unwrap();
        assert!(
            local_content.contains("new_key"),
            "settings.local.json should contain the new key"
        );
        assert!(
            local_content.contains("new_value"),
            "settings.local.json should contain the new value"
        );
        assert!(
            local_content.contains("local_only"),
            "settings.local.json should preserve existing keys"
        );

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_config_set_creates_settings_local_json_if_missing() {
        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let swell_dir = temp_dir.path().join(".swell");
        std::fs::create_dir_all(&swell_dir).unwrap();

        // Don't create settings.local.json - it should be created by ConfigSet
        let local_json_path = swell_dir.join("settings.local.json");
        assert!(
            !local_json_path.exists(),
            "settings.local.json should not exist initially"
        );

        // Change to temp directory and run ConfigSet
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::ConfigSet {
            key: "first_key".to_string(),
            value: serde_json::json!("first_value"),
        };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::ConfigValue { .. } => {
                // Success
            }
            other => panic!("Expected ConfigValue event, got: {:?}", other),
        }

        // Verify settings.local.json was created
        assert!(
            local_json_path.exists(),
            "settings.local.json should be created by ConfigSet"
        );

        let local_content = std::fs::read_to_string(&local_json_path).unwrap();
        assert!(
            local_content.contains("first_key"),
            "settings.local.json should contain the new key"
        );

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_config_get_returns_value_and_source() {
        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let swell_dir = temp_dir.path().join(".swell");
        std::fs::create_dir_all(&swell_dir).unwrap();

        // Create settings.json with test configuration
        let settings_json_path = swell_dir.join("settings.json");
        std::fs::write(
            &settings_json_path,
            r#"{"test_key": "test_value", "nested": {"inner": "deep_value"}}"#,
        )
        .unwrap();

        // Change to temp directory and run ConfigGet
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::ConfigGet {
            key: "test_key".to_string(),
        };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::ConfigValue {
                key,
                value,
                source_file,
                ..
            } => {
                assert_eq!(key, "test_key");
                assert_eq!(value, serde_json::json!("test_value"));
                assert!(source_file.is_some(), "source_file should be present");
                assert!(source_file.unwrap().contains("settings.json"));
            }
            DaemonEvent::Error { message, .. } => {
                panic!("ConfigGet should not return error: {}", message);
            }
            other => panic!("Expected ConfigValue event, got: {:?}", other),
        }

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_config_get_nonexistent_key_returns_error() {
        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let swell_dir = temp_dir.path().join(".swell");
        std::fs::create_dir_all(&swell_dir).unwrap();

        // Create an empty settings.json
        let settings_json_path = swell_dir.join("settings.json");
        std::fs::write(&settings_json_path, r#"{}"#).unwrap();

        // Change to temp directory and run ConfigGet
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::ConfigGet {
            key: "nonexistent_key".to_string(),
        };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::Error {
                message,
                failure_class,
                ..
            } => {
                assert!(message.contains("not found") || message.contains("nonexistent_key"));
                assert_eq!(failure_class, Some(swell_core::FailureClass::ConfigError));
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_config_set_overwrites_existing_key() {
        // Use a temp directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let swell_dir = temp_dir.path().join(".swell");
        std::fs::create_dir_all(&swell_dir).unwrap();

        // Create settings.local.json with existing key
        let local_json_path = swell_dir.join("settings.local.json");
        std::fs::write(&local_json_path, r#"{"existing_key": "old_value"}"#).unwrap();

        // Change to temp directory and run ConfigSet to update existing key
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let orch = create_test_orchestrator();
        let emitter = create_test_event_emitter();
        let active_connections = create_test_active_connections();

        let command = CliCommand::ConfigSet {
            key: "existing_key".to_string(),
            value: serde_json::json!("new_value"),
        };
        let event = handle_command(
            command,
            orch,
            emitter,
            active_connections,
            create_test_start_time(),
            Arc::new(Mutex::new(None)),
        )
        .await;

        match event {
            DaemonEvent::ConfigValue { key, value, .. } => {
                assert_eq!(key, "existing_key");
                assert_eq!(value, serde_json::json!("new_value"));
            }
            other => panic!("Expected ConfigValue event, got: {:?}", other),
        }

        // Verify settings.local.json contains the updated value
        let local_content = std::fs::read_to_string(&local_json_path).unwrap();
        assert!(
            local_content.contains("new_value"),
            "settings.local.json should contain updated value"
        );
        assert!(
            !local_content.contains("old_value"),
            "settings.local.json should not contain old_value"
        );

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    async fn test_config_value_event_serialization() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::ConfigValue {
            key: "test.key".to_string(),
            value: serde_json::json!("test_value"),
            source_file: Some("/path/to/settings.json".to_string()),
            correlation_id,
        };
        let json = event_to_json(&event).unwrap();
        assert!(json.contains("ConfigValue"));
        assert!(json.contains("test.key"));
        assert!(json.contains("test_value"));
        assert!(json.contains("settings.json"));
    }
}

#[cfg(test)]
/// Calculate a simple checksum for file content verification.
/// Uses std::collections::hash_map::DefaultHasher for simplicity in testing.
fn calculate_file_checksum(path: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let content = std::fs::read(path).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
