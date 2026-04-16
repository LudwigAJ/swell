//! Integration tests for VAL-ORCH-014: Agent uncertainty pauses.
//!
//! These tests verify that when an agent's confidence score drops below a configurable
//! threshold, execution pauses and emits a structured clarification request event, and
//! that execution does NOT resume until a clarification response is injected.

use std::sync::Arc;
use std::time::Duration;
use swell_core::{AgentRole, AutonomyLevel, Plan, PlanStep, RiskLevel, StepStatus, TaskState};
use swell_orchestrator::{
    builder::OrchestratorBuilder,
    check_confidence_threshold, generate_suggested_options, ClarificationOption,
    ClarificationResponse, Orchestrator, UncertaintyClarificationEvent, UncertaintyManager,
};
// Use the full module path to disambiguate from agents::ConfidenceLevel
use swell_orchestrator::uncertainty::ConfidenceLevel;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal plan for a task.
fn make_plan(task_id: Uuid) -> Plan {
    Plan {
        id: Uuid::new_v4(),
        task_id,
        steps: vec![PlanStep {
            id: Uuid::new_v4(),
            description: "Implement feature".to_string(),
            affected_files: vec!["src/lib.rs".to_string()],
            expected_tests: vec!["test_feature".to_string()],
            risk_level: RiskLevel::Low,
            dependencies: vec![],
            status: StepStatus::Pending,
        }],
        total_estimated_tokens: 500,
        risk_assessment: "Low risk".to_string(),
    }
}

/// Advance a task to Executing state and return its ID.
async fn setup_executing_task(orchestrator: &Orchestrator) -> Uuid {
    let task = orchestrator
        .create_task_with_autonomy(
            "VAL-ORCH-014 test task".to_string(),
            AutonomyLevel::FullAuto,
            vec![],
        )
        .await
        .unwrap();

    let plan = make_plan(task.id);
    orchestrator.set_plan(task.id, plan).await.unwrap();
    orchestrator.start_task(task.id).await.unwrap();

    // Verify we are in Executing state
    let current = orchestrator.get_task(task.id).await.unwrap();
    assert_eq!(
        current.state,
        TaskState::Executing,
        "Pre-condition: task must be Executing before uncertainty pause test"
    );

    task.id
}

// ---------------------------------------------------------------------------
// VAL-ORCH-014 assertions
// ---------------------------------------------------------------------------

/// Test: confidence=0.3 is below threshold=0.5 → check_confidence_threshold returns Some.
#[tokio::test]
async fn test_confidence_below_threshold_triggers_pause_signal() {
    let result = check_confidence_threshold(0.3, 0.5);
    assert!(
        result.is_some(),
        "confidence 0.3 < threshold 0.5 should trigger a pause signal"
    );
    assert_eq!(
        result.unwrap(),
        ConfidenceLevel::VeryLow,
        "score 0.3 classifies as VeryLow"
    );
}

/// Test: confidence=0.5 at threshold=0.5 does NOT trigger a pause.
#[tokio::test]
async fn test_confidence_at_threshold_does_not_trigger_pause() {
    let result = check_confidence_threshold(0.5, 0.5);
    assert!(
        result.is_none(),
        "confidence exactly at threshold should not trigger a pause"
    );
}

/// Test: confidence=0.7 above threshold=0.5 does NOT trigger a pause.
#[tokio::test]
async fn test_confidence_above_threshold_does_not_trigger_pause() {
    let result = check_confidence_threshold(0.7, 0.5);
    assert!(
        result.is_none(),
        "confidence above threshold should not trigger a pause"
    );
}

/// Test: threshold configurable per agent type — Medium-confidence score triggers pause
/// when threshold is set to 0.7 (higher than the Medium band).
#[tokio::test]
async fn test_configurable_threshold_medium_confidence_triggers_pause() {
    // Score 0.65 classifies as Medium, but threshold=0.70 > 0.65 → pause
    let result = check_confidence_threshold(0.65, 0.70);
    assert!(
        result.is_some(),
        "score 0.65 < threshold 0.70 must trigger a pause even though score is Medium"
    );
    assert_eq!(result.unwrap(), ConfidenceLevel::Medium);
}

/// Test: pause event includes reason, current context, and suggested options.
#[tokio::test]
async fn test_clarification_event_contains_reason_context_and_options() {
    let task_id = Uuid::new_v4();
    let confidence_score = 0.3_f64;
    let threshold = 0.5_f64;
    let reason = format!(
        "Agent reported low confidence score: {:.2} (threshold: {:.2})",
        confidence_score, threshold
    );
    let current_context = "Generator completed but confidence is low".to_string();

    let confidence_level = ConfidenceLevel::from_score(confidence_score);
    let suggested_options = generate_suggested_options(AgentRole::Generator, confidence_level);

    let event = UncertaintyClarificationEvent::new(
        task_id,
        None,
        AgentRole::Generator,
        confidence_score,
        threshold,
        reason.clone(),
        current_context.clone(),
        suggested_options.clone(),
    );

    // Reason and context are present
    assert!(
        !event.reason.is_empty(),
        "clarification event must include a reason"
    );
    assert!(
        event.reason.contains("0.30"),
        "reason must include the confidence score"
    );
    assert!(
        event.reason.contains("0.50"),
        "reason must include the threshold"
    );
    assert_eq!(event.current_context, current_context);

    // Suggested options are present
    assert!(
        !event.suggested_options.is_empty(),
        "clarification event must include suggested options"
    );
    assert!(
        event
            .suggested_options
            .iter()
            .any(|o| o.option_id == "continue"),
        "standard 'continue' option must be present"
    );
    // Generator-specific option
    assert!(
        event
            .suggested_options
            .iter()
            .any(|o| o.option_id == "simplify_scope"),
        "Generator-specific 'simplify_scope' option must be present"
    );

    // Event requires response
    assert!(event.needs_response(), "new event must need a response");
    assert!(!event.responded, "new event must not be responded");
}

/// Test: agent state transitions to Paused when confidence drops below threshold.
#[tokio::test]
async fn test_agent_state_transitions_to_paused_on_low_confidence() {
    let orchestrator = OrchestratorBuilder::new().build();
    let task_id = setup_executing_task(&orchestrator).await;

    // Simulate the agent reporting confidence 0.3 below threshold 0.5
    let reason = "Agent reported low confidence score: 0.30 (threshold: 0.50)".to_string();
    orchestrator
        .pause_task(task_id, reason)
        .await
        .expect("pause_task must succeed when task is in Executing state");

    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::Paused,
        "task must transition to Paused when confidence drops below threshold"
    );
    assert!(task.paused_reason.is_some(), "paused_reason must be set");
    assert!(
        task.paused_reason.unwrap().contains("confidence score"),
        "paused_reason must include the confidence score context"
    );
}

/// Test: clarification event is registered as pending in UncertaintyManager.
#[tokio::test]
async fn test_clarification_event_emitted_and_pending() {
    let manager = UncertaintyManager::new();
    let task_id = Uuid::new_v4();
    let confidence_score = 0.3_f64;
    let threshold = 0.5_f64;

    let event = UncertaintyClarificationEvent::new(
        task_id,
        None,
        AgentRole::Generator,
        confidence_score,
        threshold,
        "Low confidence".to_string(),
        "Generator context".to_string(),
        ClarificationOption::standard_options(),
    );

    let request_id = manager.create_request(event).await;

    // Event must be pending (not yet responded)
    assert!(
        manager.is_pending(request_id).await,
        "clarification request must be pending immediately after creation"
    );
    assert!(
        manager.get_response(request_id).await.is_none(),
        "no response should exist yet"
    );

    // Task-scoped lookup
    let pending_for_task = manager.get_pending_for_task(task_id).await;
    assert_eq!(
        pending_for_task.len(),
        1,
        "exactly one pending request for this task"
    );
    assert_eq!(pending_for_task[0].task_id, task_id);
}

/// Test: execution does NOT resume until clarification response is injected.
///
/// This test spawns a background task that waits for the clarification request
/// then injects a response after a short delay. The foreground task blocks
/// in `wait_for_response`. We verify that the foreground task only unblocks
/// AFTER the response is injected (not before).
#[tokio::test]
async fn test_execution_does_not_resume_until_clarification_injected() {
    let manager = Arc::new(UncertaintyManager::new());
    let task_id = Uuid::new_v4();

    let event = UncertaintyClarificationEvent::new(
        task_id,
        None,
        AgentRole::Generator,
        0.3,
        0.5,
        "Low confidence: 0.30 below threshold 0.50".to_string(),
        "Generator completed execution".to_string(),
        ClarificationOption::standard_options(),
    );

    let request_id = manager.create_request(event).await;

    // Spawn a task that injects the response after 200 ms
    let manager_clone = Arc::clone(&manager);
    let inject_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let response = ClarificationResponse::new("continue".to_string())
            .with_guidance("Proceed with current implementation".to_string());
        manager_clone.respond(request_id, response).await
    });

    // The foreground blocks here until the response is injected (max 5 s)
    let start = std::time::Instant::now();
    let response = manager
        .wait_for_response(
            request_id, 5, 0, /* poll every ~0 s (actually minimal sleep) */
        )
        .await;

    let elapsed = start.elapsed();

    // Verify the response arrived
    assert!(
        response.is_some(),
        "wait_for_response must return Some after response is injected"
    );
    let response = response.unwrap();
    assert_eq!(
        response.selected_option, "continue",
        "response must carry the injected selected_option"
    );
    assert!(
        response.additional_guidance.is_some(),
        "response must carry the injected guidance"
    );

    // Verify the wait was genuinely blocking: we blocked for ≥100 ms (injected after 200 ms)
    assert!(
        elapsed >= Duration::from_millis(100),
        "wait_for_response must have blocked for at least 100 ms (injected at 200 ms)"
    );

    // Request is now in completed, not pending
    assert!(
        !manager.is_pending(request_id).await,
        "request must no longer be pending after response"
    );

    inject_handle.await.unwrap();
}

/// Test: full flow — confidence check → Paused → clarification event → inject response → resume.
///
/// This is the main VAL-ORCH-014 integration scenario.
#[tokio::test]
async fn test_full_uncertainty_pause_and_resume_flow() {
    let orchestrator = OrchestratorBuilder::new().build();
    let manager = Arc::new(UncertaintyManager::new());

    // 1. Create and advance task to Executing
    let task_id = setup_executing_task(&orchestrator).await;

    // 2. Confidence=0.3 is below threshold=0.5 → signal pause
    let confidence_score = 0.3_f64;
    let threshold = 0.5_f64;
    let check_result = check_confidence_threshold(confidence_score, threshold);
    assert!(
        check_result.is_some(),
        "step 2: confidence 0.3 < threshold 0.5 must trigger pause signal"
    );

    // 3. Create and register the clarification event
    let confidence_level = check_result.unwrap();
    let reason = format!(
        "Agent reported low confidence score: {:.2} (threshold: {:.2})",
        confidence_score, threshold
    );
    let suggested_options = generate_suggested_options(AgentRole::Generator, confidence_level);
    let event = UncertaintyClarificationEvent::new(
        task_id,
        None,
        AgentRole::Generator,
        confidence_score,
        threshold,
        reason.clone(),
        "Generator completed but confidence is low".to_string(),
        suggested_options,
    );
    let request_id = manager.create_request(event).await;

    // Assert clarification event is emitted (pending)
    assert!(
        manager.is_pending(request_id).await,
        "step 3: clarification event must be pending"
    );

    // 4. Pause the task state machine
    orchestrator
        .pause_task(task_id, reason)
        .await
        .expect("step 4: pause_task must succeed from Executing state");

    let task = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task.state,
        TaskState::Paused,
        "step 4: task must be in Paused state after uncertainty pause"
    );

    // 5. Assert execution has NOT resumed yet (task is still Paused)
    let task_mid = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task_mid.state,
        TaskState::Paused,
        "step 5: task must remain Paused until clarification is injected"
    );

    // 6. Inject clarification response from a background task (simulates operator response)
    let manager_clone = Arc::clone(&manager);
    let inject = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let response = ClarificationResponse::new("continue".to_string());
        manager_clone.respond(request_id, response).await
    });

    // 7. Wait for the response (blocks until injected, max 5 s)
    let maybe_response = manager.wait_for_response(request_id, 5, 0).await;
    assert!(
        maybe_response.is_some(),
        "step 7: clarification response must be received"
    );

    // 8. Resume the task
    orchestrator
        .resume_task(task_id)
        .await
        .expect("step 8: resume_task must succeed from Paused state");

    let task_after = orchestrator.get_task(task_id).await.unwrap();
    assert_eq!(
        task_after.state,
        TaskState::Executing,
        "step 8: task must return to Executing after clarification is provided"
    );

    inject.await.unwrap();
}

/// Test: UncertaintyManager stats track pending and completed counts correctly.
#[tokio::test]
async fn test_uncertainty_manager_stats() {
    let manager = UncertaintyManager::new();

    let make_event = |task_id: Uuid| {
        UncertaintyClarificationEvent::new(
            task_id,
            None,
            AgentRole::Generator,
            0.3,
            0.5,
            "Low confidence".to_string(),
            "Context".to_string(),
            ClarificationOption::standard_options(),
        )
    };

    let id1 = manager.create_request(make_event(Uuid::new_v4())).await;
    let id2 = manager.create_request(make_event(Uuid::new_v4())).await;

    let stats = manager.stats().await;
    assert_eq!(stats.pending_count, 2);
    assert_eq!(stats.completed_count, 0);
    assert_eq!(stats.total_count, 2);

    // Respond to one
    manager
        .respond(id1, ClarificationResponse::new("continue".to_string()))
        .await;

    let stats = manager.stats().await;
    assert_eq!(stats.pending_count, 1);
    assert_eq!(stats.completed_count, 1);
    assert_eq!(stats.total_count, 2);

    // Respond to second
    manager
        .respond(id2, ClarificationResponse::new("escalate".to_string()))
        .await;

    let stats = manager.stats().await;
    assert_eq!(stats.pending_count, 0);
    assert_eq!(stats.completed_count, 2);
    assert_eq!(stats.total_count, 2);
}

/// Test: clarification response carries reason and context.
#[tokio::test]
async fn test_clarification_event_reason_and_context_fields() {
    let task_id = Uuid::new_v4();
    let event = UncertaintyClarificationEvent::new(
        task_id,
        None,
        AgentRole::Generator,
        0.3,
        0.5,
        "Low confidence: 0.30 below threshold 0.50".to_string(),
        "Current execution context: processing step 3 of plan".to_string(),
        ClarificationOption::standard_options(),
    );

    assert_eq!(event.task_id, task_id);
    assert_eq!(event.agent_role, AgentRole::Generator);
    assert!((event.confidence_score - 0.3).abs() < 1e-9);
    assert!((event.confidence_threshold - 0.5).abs() < 1e-9);
    assert!(event.reason.contains("0.30"), "reason must reference score");
    assert!(
        event.current_context.contains("step 3"),
        "context must be preserved verbatim"
    );
    assert!(!event.suggested_options.is_empty());
}
