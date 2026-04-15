//! Interactive approval workflow tests.
//!
//! These tests verify the approval gate feature:
//! - After planning phase completes, execution blocks until `swell approve <id>` is received
//! - `swell reject <id>` moves the task to REJECTED state
//!
//! This test module validates VAL-OBS-005: Interactive approval workflow

use swell_core::{AutonomyLevel, Plan, PlanStep, RiskLevel, StepStatus, TaskState};
use swell_orchestrator::Orchestrator;
use uuid::Uuid;

/// Helper to create a test plan for a task.
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

/// Test that a task with L1 (Supervised) autonomy level stays in AwaitingApproval
/// after planning and requires explicit approval to proceed.
#[tokio::test]
async fn test_task_awaits_approval_with_supervised_autonomy() {
    let orchestrator = Orchestrator::new();

    // Create a task with L1 Supervised autonomy (requires plan approval)
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;

    // Set a plan
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();

    // Start the task - this should transition to AwaitingApproval for L1
    orchestrator.start_task(task_id).await.unwrap();

    // Verify task is now in AwaitingApproval state (blocked at approval gate)
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::AwaitingApproval,
        "Task with L1 Supervised autonomy should be in AwaitingApproval state"
    );

    // Verify execution is blocked - task should NOT be in Executing state
    assert_ne!(
        task.state,
        TaskState::Executing,
        "Task should be blocked at approval gate, not executing"
    );
}

/// Test that a task with L2 (Guided) autonomy level also requires approval.
#[tokio::test]
async fn test_task_awaits_approval_with_guided_autonomy() {
    let orchestrator = Orchestrator::new();

    // Create a task with L2 Guided autonomy (also requires plan approval)
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Guided)
        .await;
    let task_id = task.id;

    // Set a plan
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();

    // Start the task - this should transition to AwaitingApproval for L2
    orchestrator.start_task(task_id).await.unwrap();

    // Verify task is in AwaitingApproval state
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::AwaitingApproval,
        "Task with L2 Guided autonomy should be in AwaitingApproval state"
    );
}

/// Test that `swell approve <id>` unblocks execution and proceeds to Executing.
#[tokio::test]
async fn test_approval_unblocks_execution() {
    let orchestrator = Orchestrator::new();

    // Create a task with L1 Supervised autonomy
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;

    // Set a plan
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();

    // Start task - should transition to AwaitingApproval
    orchestrator.start_task(task_id).await.unwrap();

    // Verify we're at the approval gate
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::AwaitingApproval);

    // Approve the task via the orchestrator's approve_task method
    // This simulates what `swell approve <id>` does
    orchestrator.approve_task(task_id).await.unwrap();

    // Verify task has proceeded past the approval gate to Executing
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::Executing,
        "After approval, task should transition to Executing state"
    );
}

/// Test that `swell reject <id>` moves task to Rejected state.
#[tokio::test]
async fn test_rejection_moves_to_rejected_state() {
    let orchestrator = Orchestrator::new();

    // Create a task with L1 Supervised autonomy
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;

    // Set a plan
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();

    // Start task - should transition to AwaitingApproval
    orchestrator.start_task(task_id).await.unwrap();

    // Verify we're at the approval gate
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::AwaitingApproval);

    // Reject the task via the orchestrator's reject_task method
    // This simulates what `swell reject <id>` does
    orchestrator
        .reject_task(task_id, "Test rejection".to_string())
        .await
        .unwrap();

    // Verify task has moved to Rejected state
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::Rejected,
        "After rejection, task should transition to Rejected state"
    );
}

/// Test that L3 (Autonomous) and L4 (FullAuto) autonomy levels
/// do NOT require plan approval and proceed directly to execution.
#[tokio::test]
async fn test_autonomous_levels_skip_approval_gate() {
    let orchestrator = Orchestrator::new();

    // Test L3 Autonomous - should skip approval gate
    let task_l3 = orchestrator
        .create_task_with_autonomy("L3 task".to_string(), AutonomyLevel::Autonomous)
        .await;
    let plan_l3 = create_test_plan(task_l3.id);
    orchestrator.set_plan(task_l3.id, plan_l3).await.unwrap();
    orchestrator.start_task(task_l3.id).await.unwrap();

    let task_l3_state = orchestrator.get_task(task_l3.id).await.unwrap().state;
    assert_eq!(
        task_l3_state,
        TaskState::Executing,
        "L3 Autonomous should skip approval gate and execute directly"
    );

    // Test L4 FullAuto - should also skip approval gate
    let task_l4 = orchestrator
        .create_task_with_autonomy("L4 task".to_string(), AutonomyLevel::FullAuto)
        .await;
    let plan_l4 = create_test_plan(task_l4.id);
    orchestrator.set_plan(task_l4.id, plan_l4).await.unwrap();
    orchestrator.start_task(task_l4.id).await.unwrap();

    let task_l4_state = orchestrator.get_task(task_l4.id).await.unwrap().state;
    assert_eq!(
        task_l4_state,
        TaskState::Executing,
        "L4 FullAuto should skip approval gate and execute directly"
    );
}

/// Test that a rejected task cannot be approved directly - it must be retried.
#[tokio::test]
async fn test_cannot_approve_rejected_task() {
    let orchestrator = Orchestrator::new();

    // Create a task with L1 Supervised autonomy
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;

    // Set a plan and go through to Rejected
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();
    orchestrator.start_task(task_id).await.unwrap();
    orchestrator
        .reject_task(task_id, "Test rejection".to_string())
        .await
        .unwrap();

    // Verify task is Rejected
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Rejected);

    // Attempting to approve a rejected task should fail
    let result = orchestrator.approve_task(task_id).await;
    assert!(
        result.is_err(),
        "Should not be able to approve a rejected task directly"
    );
}

/// Test the full workflow: create -> plan -> await approval -> approve -> execute -> validate -> accept
#[tokio::test]
async fn test_full_approval_workflow() {
    let orchestrator = Orchestrator::new();

    // 1. Create task with L1 Supervised (requires approval)
    let task = orchestrator
        .create_task_with_autonomy("Implement feature".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;
    assert_eq!(task.state, TaskState::Created);

    // 2. Set plan
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();

    // 3. Start task - should block at approval gate
    orchestrator.start_task(task_id).await.unwrap();
    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::AwaitingApproval
    );

    // 4. Approve the task
    orchestrator.approve_task(task_id).await.unwrap();
    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Executing
    );

    // 5. Start validation
    orchestrator.start_validation(task_id).await.unwrap();
    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Validating
    );

    // 6. Complete validation with success
    orchestrator
        .complete_task(
            task_id,
            swell_core::ValidationResult {
                passed: true,
                lint_passed: true,
                tests_passed: true,
                security_passed: true,
                ai_review_passed: true,
                errors: vec![],
                warnings: vec![],
            },
        )
        .await
        .unwrap();

    // 7. Task should be Accepted
    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Accepted
    );
}

/// Test rejection during validation (not at approval gate).
/// Rejection should work from Validating state as well.
#[tokio::test]
async fn test_rejection_during_validation() {
    let orchestrator = Orchestrator::new();

    // Create a task with L1 Supervised
    let task = orchestrator
        .create_task_with_autonomy("Test task".to_string(), AutonomyLevel::Supervised)
        .await;
    let task_id = task.id;

    // Set plan and proceed to validation
    let plan = create_test_plan(task_id);
    orchestrator.set_plan(task_id, plan).await.unwrap();
    orchestrator.start_task(task_id).await.unwrap();
    orchestrator.approve_task(task_id).await.unwrap();
    orchestrator.start_validation(task_id).await.unwrap();

    // Verify we're in Validating state
    assert_eq!(
        orchestrator.get_task(task_id).await.unwrap().state,
        TaskState::Validating
    );

    // Reject the task during validation
    orchestrator
        .reject_task(task_id, "Validation failed".to_string())
        .await
        .unwrap();

    // Task should be Rejected
    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(task.state, TaskState::Rejected);
}
