//! Task interruption tests (pause/resume/redirect).
//!
//! These tests verify the task interruption feature:
//! - Pause moves task to PAUSED state
//! - Resume restores task to EXECUTING from latest checkpoint
//! - Redirect injects new instructions without stopping
//!
//! This test module validates VAL-OBS-008: Task interruption (pause/resume/redirect)

use std::sync::Arc;
use swell_core::{AgentId, AutonomyLevel, Plan, PlanStep, RiskLevel, StepStatus, TaskId, TaskState};
use swell_orchestrator::{
    builder::OrchestratorBuilder, checkpoint_wiring::CheckpointingTaskStateMachine, Orchestrator,
};
use swell_state::traits::in_memory::InMemoryCheckpointStore;
use uuid::Uuid;

/// Helper to create a test plan for a task.
fn create_test_plan(task_id: TaskId) -> Plan {
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

/// Setup a task through to Executing state.
async fn setup_executing_task(orchestrator: &Orchestrator) -> (uuid::Uuid, swell_core::Task) {
    // Use FullAuto to bypass approval gate
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::FullAuto, vec![])
        .await
        .unwrap();
    let plan = create_test_plan(task.id);
    orchestrator.set_plan(task.id, plan).await.unwrap();
    orchestrator.start_task(task.id).await.unwrap();

    let task = orchestrator.get_task(task.id).await.unwrap();
    (task.id, task)
}

/// Test that pause_task transitions a running task to PAUSED state.
#[tokio::test]
async fn test_pause_transitions_to_paused() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Verify task is in Executing state before pause
    let task_before = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task_before.state, TaskState::Executing);

    // Pause the task
    orchestrator
        .pause_task(task_id, "Operator requested pause".to_string())
        .await
        .unwrap();

    // Verify task is now in Paused state
    let task_after = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task_after.state, TaskState::Paused);
    assert_eq!(
        task_after.paused_reason,
        Some("Operator requested pause".to_string())
    );
}

/// Test that pause_task can be called during Validating state.
#[tokio::test]
async fn test_pause_during_validating() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Transition to Validating
    orchestrator.start_validation(task_id).await.unwrap();
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Validating);

    // Pause during validation
    orchestrator
        .pause_task(task_id, "Pause during validation".to_string())
        .await
        .unwrap();

    // Verify task is Paused
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Paused);
}

/// Test that resume_task restores task to EXECUTING state.
#[tokio::test]
async fn test_resume_restores_to_executing() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Pause the task
    orchestrator
        .pause_task(task_id, "Test pause".to_string())
        .await
        .unwrap();

    let task_paused = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task_paused.state, TaskState::Paused);

    // Resume the task
    orchestrator.resume_task(task_id).await.unwrap();

    // Verify task is back to Executing
    let task_resumed = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task_resumed.state, TaskState::Executing);
    assert!(task_resumed.paused_reason.is_none());
}

/// Test that resume restores from Validating state back to Validating.
#[tokio::test]
async fn test_resume_from_validating_paused() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Transition to Validating then pause
    orchestrator.start_validation(task_id).await.unwrap();
    orchestrator
        .pause_task(task_id, "Pause during validation".to_string())
        .await
        .unwrap();

    // Resume
    orchestrator.resume_task(task_id).await.unwrap();

    // Verify task is back to Validating (not Executing)
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Validating);
}

/// Test that inject_instruction adds instructions without stopping the task.
#[tokio::test]
async fn test_redirect_injects_instructions() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Verify no injected instructions initially
    let task_before = orchestrator.get_task(task_id).await.unwrap();
    assert!(task_before.injected_instructions.is_empty());

    // Inject instructions
    orchestrator
        .inject_instruction(task_id, "Focus on error handling".to_string())
        .await
        .unwrap();
    orchestrator
        .inject_instruction(task_id, "Add unit tests for the new feature".to_string())
        .await
        .unwrap();

    // Verify task is still Executing (not stopped)
    let task_after = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task_after.state, TaskState::Executing);

    // Verify instructions were injected
    assert_eq!(task_after.injected_instructions.len(), 2);
    assert_eq!(
        task_after.injected_instructions[0],
        "Focus on error handling"
    );
    assert_eq!(
        task_after.injected_instructions[1],
        "Add unit tests for the new feature"
    );
}

/// Test that injected instructions are preserved across pause/resume.
#[tokio::test]
async fn test_injected_instructions_preserved_across_pause_resume() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Inject instructions while executing
    orchestrator
        .inject_instruction(task_id, "Priority: Fix security bug".to_string())
        .await
        .unwrap();

    // Pause
    orchestrator
        .pause_task(task_id, "Pausing for review".to_string())
        .await
        .unwrap();

    // Resume
    orchestrator.resume_task(task_id).await.unwrap();

    // Verify instructions are still present after resume
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Executing);
    assert_eq!(task.injected_instructions.len(), 1);
    assert_eq!(task.injected_instructions[0], "Priority: Fix security bug");
}

/// Test that inject_instruction works during Paused state.
#[tokio::test]
async fn test_inject_instruction_during_paused() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Pause the task
    orchestrator
        .pause_task(task_id, "Paused".to_string())
        .await
        .unwrap();

    // Inject instruction while paused
    orchestrator
        .inject_instruction(task_id, "New priority instruction".to_string())
        .await
        .unwrap();

    // Verify instruction was injected
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Paused); // Still paused
    assert_eq!(task.injected_instructions.len(), 1);
}

/// Test that injected instructions can be retrieved via get_injected_instructions.
#[tokio::test]
async fn test_get_injected_instructions() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Inject instructions
    orchestrator
        .inject_instruction(task_id, "First instruction".to_string())
        .await
        .unwrap();
    orchestrator
        .inject_instruction(task_id, "Second instruction".to_string())
        .await
        .unwrap();

    // Retrieve via get_injected_instructions
    let instructions = orchestrator
        .get_injected_instructions(task_id)
        .await
        .unwrap();

    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0], "First instruction");
    assert_eq!(instructions[1], "Second instruction");
}

/// Test that CheckpointingTaskStateMachine checkpoints pause/resume.
#[tokio::test]
async fn test_checkpointing_task_state_machine_pause_resume() {
    let store = Arc::new(InMemoryCheckpointStore::new());
    let sm: CheckpointingTaskStateMachine<InMemoryCheckpointStore> =
        CheckpointingTaskStateMachine::new(store.clone());

    // Create task and set plan
    let task = sm.create_task("Test task".to_string()).await;
    let task_id = task.id;
    sm.set_plan(
        task_id,
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![],
            total_estimated_tokens: 1000,
            risk_assessment: "Low".to_string(),
        },
    )
    .unwrap();

    // Transition through enrich, ready, assign, execute
    sm.enrich_task(task_id).await.unwrap();
    sm.ready_task(task_id).await.unwrap();
    sm.assign_task(task_id, AgentId::new()).await.unwrap();
    sm.start_execution(task_id).await.unwrap();

    // Verify state is Executing
    assert_eq!(sm.get_task(task_id).unwrap().state, TaskState::Executing);

    // Pause and checkpoint
    sm.pause_task(task_id, "Operator pause".to_string())
        .await
        .unwrap();

    // Verify checkpoints increased
    let checkpoint_count = sm.checkpoint_count(task_id).await;
    assert!(checkpoint_count >= 5); // create + enrich + ready + assign + start_execution + pause

    // Resume
    sm.resume_task(task_id).await.unwrap();

    // Verify state restored
    assert_eq!(sm.get_task(task_id).unwrap().state, TaskState::Executing);
}

/// Test that CheckpointingTaskStateMachine checkpoints inject_instruction.
/// Note: inject_instruction doesn't change state so it doesn't create a checkpoint,
/// but the state is preserved.
#[tokio::test]
async fn test_checkpointing_inject_instruction_preserves_state() {
    let store = Arc::new(InMemoryCheckpointStore::new());
    let sm: CheckpointingTaskStateMachine<InMemoryCheckpointStore> =
        CheckpointingTaskStateMachine::new(store.clone());

    // Create task and set plan
    let task = sm.create_task("Test task".to_string()).await;
    let task_id = task.id;
    sm.set_plan(
        task_id,
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![],
            total_estimated_tokens: 1000,
            risk_assessment: "Low".to_string(),
        },
    )
    .unwrap();

    // Transition through enrich, ready, assign, execute
    sm.enrich_task(task_id).await.unwrap();
    sm.ready_task(task_id).await.unwrap();
    sm.assign_task(task_id, AgentId::new()).await.unwrap();
    sm.start_execution(task_id).await.unwrap();

    // Inject instruction
    sm.inject_instruction(task_id, "Add logging".to_string())
        .unwrap();

    // Verify instruction was injected
    let task = sm.get_task(task_id).unwrap();
    assert_eq!(task.injected_instructions.len(), 1);
    assert_eq!(task.injected_instructions[0], "Add logging");
    assert_eq!(task.state, TaskState::Executing); // State unchanged
}

/// Test that injecting instructions during pause and then resuming works correctly.
#[tokio::test]
async fn test_inject_during_pause_then_resume() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Pause
    orchestrator
        .pause_task(task_id, "Paused for review".to_string())
        .await
        .unwrap();

    // Inject new instructions while paused
    orchestrator
        .inject_instruction(task_id, "Change approach: use async/await".to_string())
        .await
        .unwrap();

    // Resume
    orchestrator.resume_task(task_id).await.unwrap();

    // Verify task is executing and has instruction
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Executing);
    assert_eq!(
        task.injected_instructions[0],
        "Change approach: use async/await"
    );
}

/// Test that modify_scope and restore_original_scope work correctly.
#[tokio::test]
async fn test_modify_and_restore_scope() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Get original scope
    let original_scope = orchestrator.get_task_scope(task_id).await.unwrap();
    assert!(original_scope.files.is_empty()); // Default is empty

    // Modify scope
    let new_scope = swell_core::types::TaskScope {
        files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        directories: vec!["src".to_string()],
        allowed_operations: vec!["read".to_string(), "write".to_string()],
    };
    orchestrator
        .modify_scope(task_id, new_scope.clone())
        .await
        .unwrap();

    // Verify scope was modified
    let modified_scope = orchestrator.get_task_scope(task_id).await.unwrap();
    assert_eq!(modified_scope.files.len(), 2);
    assert_eq!(modified_scope.files[0], "src/main.rs");

    // Restore original scope
    orchestrator.restore_original_scope(task_id).await.unwrap();

    // Verify original scope restored
    let restored_scope = orchestrator.get_task_scope(task_id).await.unwrap();
    assert!(restored_scope.files.is_empty());
}

/// Test that cannot pause a task in non-active states.
#[tokio::test]
async fn test_cannot_pause_created_task() {
    let orchestrator = OrchestratorBuilder::new().build();
    let task = orchestrator
        .create_task("Test".to_string(), vec![])
        .await
        .unwrap();

    let result = orchestrator
        .pause_task(task.id, "Attempted pause".to_string())
        .await;

    assert!(result.is_err());
}

/// Test that cannot resume a task that is not paused.
#[tokio::test]
async fn test_cannot_resume_executing_task() {
    let orchestrator = OrchestratorBuilder::new().build();
    let (task_id, _) = setup_executing_task(&orchestrator).await;

    // Try to resume an executing task (not paused)
    let result = orchestrator.resume_task(task_id).await;

    assert!(result.is_err());
}

/// Test full pause/resume/redirect workflow.
#[tokio::test]
async fn test_full_interruption_workflow() {
    let orchestrator = OrchestratorBuilder::new().build();

    // 1. Create and start task
    let task = orchestrator
        .create_task_with_autonomy(
            "Implement feature".to_string(),
            AutonomyLevel::FullAuto,
            vec![],
        )
        .await
        .unwrap();
    let task_id = task.id;
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();
    orchestrator.start_task(task_id).await.unwrap();

    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Executing
    );

    // 2. Inject instruction
    orchestrator
        .inject_instruction(task_id, "Priority: security hardening".to_string())
        .await
        .unwrap();

    // 3. Pause for operator review
    orchestrator
        .pause_task(task_id, "Reviewing security aspects".to_string())
        .await
        .unwrap();

    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Paused
    );

    // 4. Inject more instructions while paused
    orchestrator
        .inject_instruction(task_id, "Also add performance tests".to_string())
        .await
        .unwrap();

    // 5. Resume after review
    orchestrator.resume_task(task_id).await.unwrap();

    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Executing
    );

    // 6. Verify all instructions present
    let instructions = orchestrator
        .get_injected_instructions(task_id)
        .await
        .unwrap();
    assert_eq!(instructions.len(), 2);
    assert_eq!(instructions[0], "Priority: security hardening");
    assert_eq!(instructions[1], "Also add performance tests");
}
