//! Doom loop detection and context reset intervention tests.
//!
//! These tests verify the LoopBreaker context reset intervention:
//! - Doom loop detected when repetitive tool call patterns occur
//! - LoopBreaker intervention triggered on detection
//! - Context reset clears repetitive context and breaks the cycle
//!
//! This test module validates VAL-OBS-010: Doom-loop detection and context reset

use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Test that LoopBreaker detects doom loop and clears context.
#[tokio::test]
async fn test_doom_loop_reset_clears_context() {
    // Create a shared tool loop tracker
    let tracker = Arc::new(RwLock::new(swell_tools::ToolLoopTracker::new()));
    let task_id = Uuid::new_v4();

    // Record 3 consecutive read_file failures (triggers same_tool_retry_threshold of 3)
    {
        let mut tracker = tracker.write().await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
    }

    // Analyze and check loop is detected
    let result = {
        let mut tracker = tracker.write().await;
        tracker.analyze(task_id).await
    };

    assert!(result.loop_detected, "Doom loop should be detected");
    assert!(
        result.should_intervene,
        "Intervention should be triggered on doom loop"
    );

    // Apply LoopBreaker context reset
    {
        let mut tracker = tracker.write().await;
        tracker.clear_loop_context(task_id).await;
    }

    // Verify context was cleared
    let result_after = {
        let mut tracker = tracker.write().await;
        tracker.analyze(task_id).await
    };

    // After context reset, loop should no longer be detected
    assert!(
        !result_after.loop_detected,
        "Loop should not be detected after context reset"
    );
    assert_eq!(
        result_after.total_executions, 0,
        "Execution history should be cleared"
    );
}

/// Test that LoopBreaker clears repetitive context specifically.
#[tokio::test]
async fn test_loop_breaker_clears_repetitive_context() {
    let tracker = Arc::new(RwLock::new(swell_tools::ToolLoopTracker::new()));
    let task_id = Uuid::new_v4();

    // Record 5 consecutive shell failures (repetitive pattern)
    {
        let mut tracker = tracker.write().await;
        for _ in 0..5 {
            tracker
                .record_execution(
                    task_id,
                    "shell",
                    false,
                    serde_json::json!({"command": "ls -la"}),
                )
                .await;
        }
    }

    // Verify loop is detected before reset
    {
        let mut tracker = tracker.write().await;
        let result = tracker.analyze(task_id).await;
        assert!(result.loop_detected);
        assert_eq!(result.same_tool_streak, 5);
    }

    // Apply LoopBreaker context reset
    {
        let mut tracker = tracker.write().await;
        tracker.clear_loop_context(task_id).await;
    }

    // Verify the streak is cleared
    {
        let mut tracker = tracker.write().await;
        let result = tracker.analyze(task_id).await;
        assert!(!result.loop_detected);
        assert_eq!(result.same_tool_streak, 0);
    }
}

/// Test that LoopBreaker intervention fires via callback.
#[tokio::test]
async fn test_loop_breaker_callback_fires() {
    let intervention_triggered = Arc::new(std::sync::Mutex::new(false));
    let intervention_triggered_clone = intervention_triggered.clone();

    let tracker = Arc::new(RwLock::new(
        swell_tools::ToolLoopTracker::new().with_intervention_callback(move |pattern| {
            assert_eq!(
                pattern.pattern_type,
                swell_tools::LoopPatternType::SameToolRetry
            );
            assert_eq!(pattern.iteration_count, 3);
            *intervention_triggered_clone.lock().unwrap() = true;
        }),
    ));

    let task_id = Uuid::new_v4();

    // Record 3 consecutive failures
    {
        let mut tracker = tracker.write().await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
    }

    // Analyze triggers the callback
    {
        let mut tracker = tracker.write().await;
        let result = tracker.analyze(task_id).await;
        assert!(result.should_intervene);
    }

    assert!(
        *intervention_triggered.lock().unwrap(),
        "Intervention callback should have fired"
    );
}

/// Test oscillation pattern triggers LoopBreaker.
#[tokio::test]
async fn test_oscillation_triggers_loop_breaker() {
    let tracker = Arc::new(RwLock::new(swell_tools::ToolLoopTracker::new()));
    let task_id = Uuid::new_v4();

    // Record oscillation: read_file → edit_file → read_file → edit_file → read_file
    {
        let mut tracker = tracker.write().await;
        tracker
            .record_execution(task_id, "read_file", true, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "edit_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "edit_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
    }

    let result = {
        let mut tracker = tracker.write().await;
        tracker.analyze(task_id).await
    };

    assert!(result.loop_detected);
    assert_eq!(result.oscillation_count, 3);

    // Apply LoopBreaker
    {
        let mut tracker = tracker.write().await;
        tracker.clear_loop_context(task_id).await;
    }

    // Verify context was cleared
    let result_after = {
        let mut tracker = tracker.write().await;
        tracker.analyze(task_id).await
    };

    assert!(!result_after.loop_detected);
}

/// Test that different tasks have independent loop tracking.
#[tokio::test]
async fn test_doom_loop_independent_tasks() {
    let tracker = Arc::new(RwLock::new(swell_tools::ToolLoopTracker::new()));
    let task1 = Uuid::new_v4();
    let task2 = Uuid::new_v4();

    // Task 1: Has a doom loop (3 consecutive shell failures)
    {
        let mut tracker = tracker.write().await;
        for _ in 0..3 {
            tracker
                .record_execution(task1, "shell", false, serde_json::json!({}))
                .await;
        }
    }

    // Task 2: Normal execution (1 success)
    {
        let mut tracker = tracker.write().await;
        tracker
            .record_execution(task2, "read_file", true, serde_json::json!({}))
            .await;
    }

    // Task 1 should detect loop
    {
        let mut tracker = tracker.write().await;
        let result1 = tracker.analyze(task1).await;
        assert!(result1.loop_detected);
    }

    // Task 2 should not detect loop
    {
        let mut tracker = tracker.write().await;
        let result2 = tracker.analyze(task2).await;
        assert!(!result2.loop_detected);
    }

    // Reset Task 1's context
    {
        let mut tracker = tracker.write().await;
        tracker.clear_loop_context(task1).await;
    }

    // Task 1 should no longer detect loop
    {
        let mut tracker = tracker.write().await;
        let result1 = tracker.analyze(task1).await;
        assert!(!result1.loop_detected);
    }

    // Task 2 should still not detect loop
    {
        let mut tracker = tracker.write().await;
        let result2 = tracker.analyze(task2).await;
        assert!(!result2.loop_detected);
    }
}
