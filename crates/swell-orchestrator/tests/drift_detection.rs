//! Drift Detection Tests
//!
//! Tests for the drift detector wiring in ExecutionController.
//!
//! These tests verify that:
//! - Drift detection correctly identifies when actual modifications exceed planned scope
//! - Drift warnings are emitted when >30% of modifications target unexpected files
//! - No warnings are emitted when modifications stay within planned files

use std::sync::Arc;
use swell_orchestrator::builder::OrchestratorBuilder;
use uuid::Uuid;

/// Helper to create an ExecutionController for testing.
async fn create_test_controller() -> swell_orchestrator::ExecutionController {
    let orchestrator = OrchestratorBuilder::new().build();
    let mock_llm = Arc::new(swell_llm::MockLlm::new("claude-sonnet"));
    let tool_registry = Arc::new(swell_tools::ToolRegistry::new());

    swell_orchestrator::ExecutionController::new(Arc::downgrade(&orchestrator), mock_llm, tool_registry)
}

// ========================================================================
// DriftDetector Basic Tests (via ExecutionController)
// ========================================================================

/// Test that drift detection flags when >30% of modifications target unexpected files.
///
/// Plan: files A, B, C
/// Modifications: A, B, D, E, F (3 out of 5 are unexpected = 60% drift)
/// Expected: Drift warning emitted.
#[tokio::test]
async fn test_drift_flagged_when_modifications_exceed_threshold() {
    let controller = create_test_controller().await;

    // Track file modifications
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/d.rs"); // unexpected
    controller.track_file_modification("src/e.rs"); // unexpected
    controller.track_file_modification("src/f.rs"); // unexpected
                                                    // Plan had only A, B, C
    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
    ];
    let task_id = Uuid::new_v4();

    // Check drift - should detect >30% drift
    let report = controller.check_drift(task_id, &estimated);

    assert!(
        report.is_some(),
        "Drift should be flagged when >30% of modifications target unexpected files"
    );

    let report = report.unwrap();
    assert!(report.exceeds_threshold);
    assert_eq!(report.estimated_files, 3);
    assert_eq!(report.actual_files, 5);
    // (5-3)/3 * 100 ≈ 66.67%
    assert!((report.drift_percentage - 66.67).abs() < 0.1);
    assert!(report.extra_files.contains(&"src/d.rs".to_string()));
    assert!(report.extra_files.contains(&"src/e.rs".to_string()));
    assert!(report.extra_files.contains(&"src/f.rs".to_string()));
}

/// Test that no drift warning is emitted when modifications stay within planned files.
///
/// Plan: files A, B, C
/// Modifications: A, B (both in plan, no drift)
/// Expected: No drift warning.
#[tokio::test]
async fn test_no_drift_flag_when_modifications_within_plan() {
    let controller = create_test_controller().await;

    // Track file modifications - all within plan
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");

    // Plan had A, B, C
    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
    ];
    let task_id = Uuid::new_v4();

    // Check drift - returns None when drift is within limits
    let report = controller.check_drift(task_id, &estimated);

    // No drift warning should be emitted
    assert!(
        report.is_none(),
        "Drift should NOT be flagged when all modifications are within planned files"
    );
}

/// Test that drift is NOT flagged when drift is exactly at 30% threshold.
///
/// Plan: 10 files
/// Modifications: 13 files (3 extra = 30% drift exactly)
/// Expected: No flag (30% is at threshold, not exceeding)
#[tokio::test]
async fn test_no_drift_flag_at_exactly_threshold() {
    let controller = create_test_controller().await;

    // Plan: 10 files (0-9)
    let estimated: Vec<String> = (0..10).map(|i| format!("src/file{}.rs", i)).collect();
    // Actual: 13 files (0-12) - 3 extra = 30% drift
    controller.track_file_modification("src/file0.rs");
    controller.track_file_modification("src/file1.rs");
    controller.track_file_modification("src/file2.rs");
    controller.track_file_modification("src/file3.rs");
    controller.track_file_modification("src/file4.rs");
    controller.track_file_modification("src/file5.rs");
    controller.track_file_modification("src/file6.rs");
    controller.track_file_modification("src/file7.rs");
    controller.track_file_modification("src/file8.rs");
    controller.track_file_modification("src/file9.rs");
    controller.track_file_modification("src/file10.rs"); // extra
    controller.track_file_modification("src/file11.rs"); // extra
    controller.track_file_modification("src/file12.rs"); // extra

    let task_id = Uuid::new_v4();
    let report = controller.check_drift(task_id, &estimated);

    // 30% is at threshold, should NOT be flagged (exceeds requires > threshold)
    assert!(
        report.is_none(),
        "Drift at exactly 30% threshold should NOT be flagged"
    );
}

/// Test that drift IS flagged when drift exceeds 30% threshold.
///
/// Plan: 10 files
/// Modifications: 14 files (4 extra = 40% drift)
/// Expected: Flagged
#[tokio::test]
async fn test_drift_flagged_just_over_threshold() {
    let controller = create_test_controller().await;

    // Plan: 10 files (0-9)
    let estimated: Vec<String> = (0..10).map(|i| format!("src/file{}.rs", i)).collect();

    // Actual: 14 files (0-13) - 4 extra = 40% drift
    for i in 0..14 {
        controller.track_file_modification(&format!("src/file{}.rs", i));
    }

    let task_id = Uuid::new_v4();
    let report = controller.check_drift(task_id, &estimated);

    assert!(report.is_some(), "Drift at 40% should be flagged");

    let report = report.unwrap();
    assert!((report.drift_percentage - 40.0).abs() < 0.1);
}

// ========================================================================
// Modified Files Tracking Tests
// ========================================================================

/// Test that track_file_modification correctly records files.
#[tokio::test]
async fn test_track_file_modification_records_files() {
    let controller = create_test_controller().await;

    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/c.rs");

    let modified = controller.get_modified_files();

    assert_eq!(modified.len(), 3);
    assert!(modified.contains(&"src/a.rs".to_string()));
    assert!(modified.contains(&"src/b.rs".to_string()));
    assert!(modified.contains(&"src/c.rs".to_string()));
}

/// Test that reset_modified_files clears the tracked files.
#[tokio::test]
async fn test_reset_modified_files_clears_tracking() {
    let controller = create_test_controller().await;

    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");

    controller.reset_modified_files();

    let modified = controller.get_modified_files();
    assert!(modified.is_empty());
}

/// Test that duplicate file modifications are deduplicated.
#[tokio::test]
async fn test_track_file_modification_deduplicates() {
    let controller = create_test_controller().await;
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/a.rs"); // duplicate
    controller.track_file_modification("src/a.rs"); // duplicate

    let modified = controller.get_modified_files();

    assert_eq!(modified.len(), 1);
    assert!(modified.contains(&"src/a.rs".to_string()));
}

// ========================================================================
// Drift Report Content Tests
// ========================================================================

/// Test that DriftReport contains correct unexpected file list.
#[tokio::test]
async fn test_drift_report_lists_unexpected_files() {
    let controller = create_test_controller().await;

    // Plan: A, B
    let estimated = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

    // Modified: A, B, C, D, E (3 unexpected)
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/c.rs");
    controller.track_file_modification("src/d.rs");
    controller.track_file_modification("src/e.rs");

    let task_id = Uuid::new_v4();
    let report = controller.check_drift(task_id, &estimated).unwrap();

    assert_eq!(report.extra_files.len(), 3);
    assert!(report.extra_files.contains(&"src/c.rs".to_string()));
    assert!(report.extra_files.contains(&"src/d.rs".to_string()));
    assert!(report.extra_files.contains(&"src/e.rs".to_string()));
}

/// Test that DriftReport lists missing files (planned but not modified).
#[tokio::test]
async fn test_drift_report_lists_missing_files() {
    let controller = create_test_controller().await;

    // Plan: A, B, C, D, E
    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
        "src/d.rs".to_string(),
        "src/e.rs".to_string(),
    ];

    // Modified: A, B, C only (no unexpected, just D, E not touched)
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/c.rs");

    let task_id = Uuid::new_v4();

    // Get drift report directly from drift detector (negative drift = under-implementation)
    let actual_files = controller.get_modified_files();
    let report = controller
        .drift_detector()
        .detect_drift(task_id, &estimated, &actual_files);

    // Negative drift case - report exists but doesn't exceed threshold
    assert!(
        !report.exceeds_threshold,
        "Negative drift should not trigger warning"
    );
    assert_eq!(report.missing_files.len(), 2);
    assert!(report.missing_files.contains(&"src/d.rs".to_string()));
    assert!(report.missing_files.contains(&"src/e.rs".to_string()));
}

// ========================================================================
// Drift Event Emission Tests
// ========================================================================

/// Test that Orchestrator emits DriftWarning event when drift exceeds threshold.
#[tokio::test]
async fn test_orchestrator_emits_drift_warning_event() {
    use swell_orchestrator::OrchestratorEvent;

    let orchestrator = OrchestratorBuilder::new().build();
    let mut receiver = orchestrator.subscribe();

    // Setup controller with the same orchestrator
    let mock_llm = Arc::new(swell_llm::MockLlm::new("claude-sonnet"));
    let tool_registry = Arc::new(swell_tools::ToolRegistry::new());
    let controller =
        swell_orchestrator::ExecutionController::new(Arc::downgrade(&orchestrator), mock_llm, tool_registry);

    // Track files with 60% drift (5 actual vs 3 planned)
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/c.rs");
    controller.track_file_modification("src/d.rs"); // unexpected
    controller.track_file_modification("src/e.rs"); // unexpected

    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
    ];
    let task_id = Uuid::new_v4();

    // Check drift - this should trigger a warning
    let report = controller.check_drift(task_id, &estimated);
    assert!(report.is_some());

    // Emit the drift warning to orchestrator
    let report = report.unwrap();
    orchestrator.emit_drift_warning(
        task_id,
        report.drift_percentage,
        report.extra_files.clone(),
        report.estimated_files,
        report.actual_files,
    );

    // Receive and verify the event
    let event = receiver.try_recv();
    assert!(event.is_ok(), "Should receive a drift warning event");

    let event = event.unwrap();
    match event {
        OrchestratorEvent::DriftWarning {
            task_id: received_task_id,
            drift_percentage,
            unexpected_files,
            planned_file_count,
            actual_file_count,
        } => {
            assert_eq!(received_task_id, task_id);
            assert!((drift_percentage - 66.67).abs() < 0.1);
            assert_eq!(unexpected_files.len(), 2);
            assert_eq!(planned_file_count, 3);
            assert_eq!(actual_file_count, 5);
        }
        _ => panic!("Expected DriftWarning event, got {:?}", event),
    }
}

// ========================================================================
// Edge Cases
// ========================================================================

/// Test with empty plan and actual files.
#[tokio::test]
async fn test_drift_with_empty_plan() {
    let controller = create_test_controller().await;

    let estimated: Vec<String> = vec![];
    let task_id = Uuid::new_v4();

    // Track some files
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");

    // Empty plan with actual modifications - drift report should exist
    let report = controller.check_drift(task_id, &estimated);
    // With empty plan, drift percentage is infinite, so exceeds_threshold is true
    // We check that report exists (drift was calculated)
    assert!(
        report.is_some(),
        "Empty plan should still generate drift report"
    );
    let report = report.unwrap();
    assert!(
        report.exceeds_threshold,
        "Infinite drift should trigger flag"
    );
}

/// Test with no modifications (planned files only).
#[tokio::test]
async fn test_no_drift_when_no_files_modified() {
    let controller = create_test_controller().await;

    // Plan: A, B, C
    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
    ];
    let task_id = Uuid::new_v4();

    // No modifications - should not trigger drift warning
    let report = controller.check_drift(task_id, &estimated);
    assert!(
        report.is_none(),
        "No modifications should not trigger drift warning"
    );
}

/// Test with exact same files as plan.
#[tokio::test]
async fn test_no_drift_when_exact_match() {
    let controller = create_test_controller().await;

    // Plan: A, B, C
    let estimated = vec![
        "src/a.rs".to_string(),
        "src/b.rs".to_string(),
        "src/c.rs".to_string(),
    ];
    let task_id = Uuid::new_v4();

    // Exact same files as plan
    controller.track_file_modification("src/a.rs");
    controller.track_file_modification("src/b.rs");
    controller.track_file_modification("src/c.rs");

    let report = controller.check_drift(task_id, &estimated);
    assert!(report.is_none(), "Exact match should not trigger drift");
}

/// Test drift check interval getter/setter.
#[tokio::test]
async fn test_drift_check_interval_accessors() {
    let mut controller = create_test_controller().await;

    // Default should be 0 (only check at end of execution)
    assert_eq!(controller.drift_check_interval_seconds(), 0);

    // Setter should work
    controller.set_drift_check_interval(30);
    assert_eq!(controller.drift_check_interval_seconds(), 30);
}
