//! Drift detector for comparing actual file changes against planned estimates.
//!
//! This module provides functionality to detect when actual file modifications
//! diverge significantly from the planner's estimates, which can indicate:
//! - Scope creep
//! - Incomplete planning
//! - Unexpected complexity in implementation

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use swell_core::TaskId;
use uuid::Uuid;

/// Drift detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    /// Task ID this report is for
    pub task_id: TaskId,
    /// Estimated file count from plan
    pub estimated_files: usize,
    /// Actual file count that were modified
    pub actual_files: usize,
    /// Drift percentage: (actual - estimated) / estimated * 100
    pub drift_percentage: f64,
    /// Whether drift exceeds the threshold
    pub exceeds_threshold: bool,
    /// Threshold that was used (default 30%)
    pub threshold_percentage: f64,
    /// Files that were added beyond the plan
    pub extra_files: Vec<String>,
    /// Files that were planned but not modified
    pub missing_files: Vec<String>,
    /// Detailed breakdown by step
    pub step_drift: Vec<StepDrift>,
}

/// Drift for an individual step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDrift {
    /// Step ID
    pub step_id: Uuid,
    /// Estimated files for this step
    pub estimated: usize,
    /// Actual files touched in this step
    pub actual: usize,
    /// Drift percentage for this step
    pub drift_percentage: f64,
}

/// Drift detector configuration
#[derive(Debug, Clone)]
pub struct DriftDetectorConfig {
    /// Threshold percentage above which drift is flagged (default 30.0)
    pub threshold_percentage: f64,
}

impl Default for DriftDetectorConfig {
    fn default() -> Self {
        Self {
            threshold_percentage: 30.0,
        }
    }
}

impl DriftDetectorConfig {
    pub fn new(threshold_percentage: f64) -> Self {
        Self {
            threshold_percentage: threshold_percentage.max(0.0),
        }
    }
}

/// Drift detector for comparing actual vs planned file modifications
#[derive(Debug, Clone)]
pub struct DriftDetector {
    config: DriftDetectorConfig,
}

impl DriftDetector {
    /// Create a new drift detector with default config
    pub fn new() -> Self {
        Self {
            config: DriftDetectorConfig::default(),
        }
    }

    /// Create a new drift detector with custom config
    pub fn with_config(config: DriftDetectorConfig) -> Self {
        Self { config }
    }

    /// Detect drift between planned files and actual modifications
    ///
    /// # Arguments
    /// * `estimated_files` - Files from the plan (e.g., PlanStep.affected_files aggregated)
    /// * `actual_files` - Files that were actually modified
    ///
    /// # Returns
    /// A DriftReport with drift analysis
    pub fn detect_drift(
        &self,
        task_id: TaskId,
        estimated_files: &[String],
        actual_files: &[String],
    ) -> DriftReport {
        let estimated_set: HashSet<&str> = estimated_files.iter().map(|s| s.as_str()).collect();
        let actual_set: HashSet<&str> = actual_files.iter().map(|s| s.as_str()).collect();

        let estimated_count = estimated_set.len();
        let actual_count = actual_set.len();

        // Calculate drift percentage
        // Positive drift = more files than expected (scope creep)
        // Negative drift = fewer files than expected (under-implementation)
        let drift_percentage = if estimated_count == 0 {
            if actual_count == 0 {
                0.0
            } else {
                100.0 // Completely new files
            }
        } else {
            ((actual_count as f64 - estimated_count as f64) / estimated_count as f64) * 100.0
        };

        // Files that were modified but not in the plan
        let extra_files: Vec<String> = actual_files
            .iter()
            .filter(|f| !estimated_set.contains(f.as_str()))
            .cloned()
            .collect();

        // Files that were planned but not modified
        let missing_files: Vec<String> = estimated_files
            .iter()
            .filter(|f| !actual_set.contains(f.as_str()))
            .cloned()
            .collect();

        // Only flag positive drift (actual > estimated = scope creep)
        // Negative drift (under-implementation) is not considered problematic
        let exceeds_threshold = drift_percentage > self.config.threshold_percentage;

        DriftReport {
            task_id,
            estimated_files: estimated_count,
            actual_files: actual_count,
            drift_percentage,
            exceeds_threshold,
            threshold_percentage: self.config.threshold_percentage,
            extra_files,
            missing_files,
            step_drift: Vec::new(), // Populated by detect_drift_with_steps
        }
    }

    /// Detect drift with per-step breakdown
    ///
    /// # Arguments
    /// * `task_id` - Task identifier
    /// * `plan_steps` - Plan steps with their affected_files
    /// * `actual_files` - Files that were actually modified
    ///
    /// # Returns
    /// A DriftReport with step-level analysis
    pub fn detect_drift_with_steps(
        &self,
        task_id: TaskId,
        plan_steps: &[(Uuid, Vec<String>)], // (step_id, affected_files)
        actual_files: &[String],
    ) -> DriftReport {
        let all_estimated: Vec<String> = plan_steps
            .iter()
            .flat_map(|(_, files)| files.clone())
            .collect();
        let mut base_report = self.detect_drift(task_id, &all_estimated, actual_files);

        // Calculate per-step drift
        let step_drift: Vec<StepDrift> = plan_steps
            .iter()
            .map(|(step_id, estimated)| {
                let actual_for_step: Vec<&String> = actual_files
                    .iter()
                    .filter(|f| estimated.iter().any(|e| e == *f))
                    .collect();

                let est_count = estimated.len();
                let act_count = actual_for_step.len();

                let drift = if est_count == 0 {
                    if act_count == 0 {
                        0.0
                    } else {
                        100.0
                    }
                } else {
                    ((act_count as f64 - est_count as f64) / est_count as f64) * 100.0
                };

                StepDrift {
                    step_id: *step_id,
                    estimated: est_count,
                    actual: act_count,
                    drift_percentage: drift,
                }
            })
            .collect();

        base_report.step_drift = step_drift;
        base_report
    }

    /// Get the configured threshold
    pub fn threshold(&self) -> f64 {
        self.config.threshold_percentage
    }
}

impl Default for DriftDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test task ID
    fn test_task_id() -> TaskId {
        TaskId::new()
    }

    // --- DriftDetector Creation Tests ---

    #[test]
    fn test_new_detector_has_default_threshold() {
        let detector = DriftDetector::new();
        assert_eq!(detector.threshold(), 30.0);
    }

    #[test]
    fn test_with_config_custom_threshold() {
        let config = DriftDetectorConfig::new(50.0);
        let detector = DriftDetector::with_config(config);
        assert_eq!(detector.threshold(), 50.0);
    }

    #[test]
    fn test_config_enforces_non_negative_threshold() {
        let config = DriftDetectorConfig::new(-10.0);
        let detector = DriftDetector::with_config(config);
        assert_eq!(detector.threshold(), 0.0);
    }

    // --- Basic Drift Detection Tests ---

    #[test]
    fn test_no_drift_when_files_match() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let estimated = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        let actual = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.estimated_files, 2);
        assert_eq!(report.actual_files, 2);
        assert_eq!(report.drift_percentage, 0.0);
        assert!(!report.exceeds_threshold);
        assert!(report.extra_files.is_empty());
        assert!(report.missing_files.is_empty());
    }

    #[test]
    fn test_positive_drift_scope_creep() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // Plan had 2 files, but 4 were actually modified
        let estimated = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        let actual = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
            "src/d.rs".to_string(),
        ];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.estimated_files, 2);
        assert_eq!(report.actual_files, 4);
        assert_eq!(report.drift_percentage, 100.0); // (4-2)/2 * 100 = 100%
        assert!(report.exceeds_threshold);
        assert_eq!(
            report.extra_files,
            vec!["src/c.rs".to_string(), "src/d.rs".to_string()]
        );
    }

    #[test]
    fn test_negative_drift_under_implementation() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // Plan had 4 files, but only 2 were actually modified
        let estimated = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
            "src/d.rs".to_string(),
        ];
        let actual = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.estimated_files, 4);
        assert_eq!(report.actual_files, 2);
        assert_eq!(report.drift_percentage, -50.0); // (2-4)/4 * 100 = -50%
                                                    // Negative drift (under-implementation) is not flagged as exceeds_threshold
                                                    // Only positive drift (scope creep) triggers the warning
        assert!(!report.exceeds_threshold);
        assert!(report.extra_files.is_empty());
        assert_eq!(
            report.missing_files,
            vec!["src/c.rs".to_string(), "src/d.rs".to_string()]
        );
    }

    #[test]
    fn test_drift_within_threshold() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // 10 files planned, 12 actual = 20% drift (within 30% threshold)
        let estimated: Vec<String> = (0..10).map(|i| format!("src/file{}.rs", i)).collect();
        let actual: Vec<String> = (0..12).map(|i| format!("src/file{}.rs", i)).collect();

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.drift_percentage, 20.0);
        assert!(!report.exceeds_threshold);
    }

    #[test]
    fn test_drift_exceeds_threshold() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // 10 files planned, 15 actual = 50% drift (exceeds 30% threshold)
        let estimated: Vec<String> = (0..10).map(|i| format!("src/file{}.rs", i)).collect();
        let actual: Vec<String> = (0..15).map(|i| format!("src/file{}.rs", i)).collect();

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.drift_percentage, 50.0);
        assert!(report.exceeds_threshold);
    }

    // --- Edge Cases ---

    #[test]
    fn test_empty_estimated_and_actual() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let report = detector.detect_drift(task_id, &[], &[]);

        assert_eq!(report.estimated_files, 0);
        assert_eq!(report.actual_files, 0);
        assert_eq!(report.drift_percentage, 0.0);
        assert!(!report.exceeds_threshold);
    }

    #[test]
    fn test_empty_estimated_with_actual() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let report = detector.detect_drift(task_id, &[], &["src/a.rs".to_string()]);

        assert_eq!(report.estimated_files, 0);
        assert_eq!(report.actual_files, 1);
        assert_eq!(report.drift_percentage, 100.0);
        assert!(report.exceeds_threshold);
        assert_eq!(report.extra_files, vec!["src/a.rs".to_string()]);
    }

    #[test]
    fn test_estimated_with_empty_actual() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let report = detector.detect_drift(
            task_id,
            &["src/a.rs".to_string(), "src/b.rs".to_string()],
            &[],
        );

        assert_eq!(report.estimated_files, 2);
        assert_eq!(report.actual_files, 0);
        assert_eq!(report.drift_percentage, -100.0);
        // Negative drift (under-implementation) is not flagged as exceeds_threshold
        // Only positive drift (scope creep) triggers the warning
        assert!(!report.exceeds_threshold);
        assert!(report.extra_files.is_empty());
        assert_eq!(
            report.missing_files,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }

    #[test]
    fn test_duplicate_files_deduplicated() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // Duplicates in actual should be counted once
        let estimated = vec!["src/a.rs".to_string()];
        let actual = vec![
            "src/a.rs".to_string(),
            "src/a.rs".to_string(),
            "src/a.rs".to_string(),
        ];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.estimated_files, 1);
        assert_eq!(report.actual_files, 1); // Deduplicated to 1
        assert_eq!(report.drift_percentage, 0.0);
        assert!(!report.exceeds_threshold);
    }

    // --- Step Drift Tests ---

    #[test]
    fn test_detect_drift_with_steps() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let step1_id = Uuid::new_v4();
        let step2_id = Uuid::new_v4();

        let plan_steps = vec![
            (
                step1_id,
                vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            ),
            (step2_id, vec!["src/c.rs".to_string()]),
        ];

        // Only 2 of 3 planned files were modified
        let actual = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let report = detector.detect_drift_with_steps(task_id, &plan_steps, &actual);

        assert_eq!(report.estimated_files, 3);
        assert_eq!(report.actual_files, 2);
        // (2-3)/3 * 100 ≈ -33.33%
        assert!((report.drift_percentage - (-33.33)).abs() < 0.01);
        // Negative drift (under-implementation) is not flagged as exceeds_threshold
        // Only positive drift (scope creep) triggers the warning
        assert!(!report.exceeds_threshold);
        assert_eq!(report.step_drift.len(), 2);

        // Step 1 drift
        assert_eq!(report.step_drift[0].step_id, step1_id);
        assert_eq!(report.step_drift[0].estimated, 2);
        assert_eq!(report.step_drift[0].actual, 2);
        assert!((report.step_drift[0].drift_percentage - 0.0).abs() < 0.01);

        // Step 2 drift
        assert_eq!(report.step_drift[1].step_id, step2_id);
        assert_eq!(report.step_drift[1].estimated, 1);
        assert_eq!(report.step_drift[1].actual, 0);
        // -100% drift
        assert!((report.step_drift[1].drift_percentage - (-100.0)).abs() < 0.01);
    }

    #[test]
    fn test_step_drift_with_extra_files() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let step1_id = Uuid::new_v4();

        let plan_steps = vec![(step1_id, vec!["src/a.rs".to_string()])];

        // 1 planned file, but 3 were actually modified
        let actual = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
        ];

        let report = detector.detect_drift_with_steps(task_id, &plan_steps, &actual);

        assert_eq!(report.drift_percentage, 200.0); // (3-1)/1 * 100 = 200%
        assert!(report.exceeds_threshold);
        assert_eq!(report.step_drift[0].estimated, 1);
        assert_eq!(report.step_drift[0].actual, 1); // Only 1 was from the plan
        assert_eq!(report.step_drift[0].drift_percentage, 0.0); // Step-level sees only the overlap
    }

    // --- Threshold Boundary Tests ---

    #[test]
    fn test_at_threshold_boundary() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // 100 files planned, 130 actual = 30% drift (exactly at threshold)
        let estimated: Vec<String> = (0..100).map(|i| format!("src/file{}.rs", i)).collect();
        let actual: Vec<String> = (0..130).map(|i| format!("src/file{}.rs", i)).collect();

        let report = detector.detect_drift(task_id, &estimated, &actual);

        // Default threshold is 30%, so 30% exactly should NOT exceed
        // because the check is `drift_percentage.abs() > threshold`
        assert_eq!(report.drift_percentage, 30.0);
        assert!(!report.exceeds_threshold);
    }

    #[test]
    fn test_just_over_threshold() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        // 100 files planned, 131 actual ≈ 31% drift
        let estimated: Vec<String> = (0..100).map(|i| format!("src/file{}.rs", i)).collect();
        let actual: Vec<String> = (0..131).map(|i| format!("src/file{}.rs", i)).collect();

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.drift_percentage, 31.0);
        assert!(report.exceeds_threshold);
    }

    // --- DriftReport Fields Tests ---

    #[test]
    fn test_report_task_id_populated() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let report = detector.detect_drift(task_id, &[], &[]);

        assert_eq!(report.task_id, task_id);
    }

    #[test]
    fn test_report_threshold_stored() {
        let config = DriftDetectorConfig::new(50.0);
        let detector = DriftDetector::with_config(config);

        let report = detector.detect_drift(test_task_id(), &[], &[]);

        assert_eq!(report.threshold_percentage, 50.0);
    }

    #[test]
    fn test_report_extra_files_listed() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let estimated = vec!["src/a.rs".to_string()];
        let actual = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
        ];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.extra_files.len(), 2);
        assert!(report.extra_files.contains(&"src/b.rs".to_string()));
        assert!(report.extra_files.contains(&"src/c.rs".to_string()));
    }

    #[test]
    fn test_report_missing_files_listed() {
        let detector = DriftDetector::new();
        let task_id = test_task_id();

        let estimated = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
        ];
        let actual = vec!["src/a.rs".to_string()];

        let report = detector.detect_drift(task_id, &estimated, &actual);

        assert_eq!(report.missing_files.len(), 2);
        assert!(report.missing_files.contains(&"src/b.rs".to_string()));
        assert!(report.missing_files.contains(&"src/c.rs".to_string()));
    }
}
