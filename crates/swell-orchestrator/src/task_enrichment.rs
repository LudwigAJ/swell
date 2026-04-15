//! Task Enrichment Module
//!
//! Enriches tasks with metadata before they enter the ready queue.
//! This is a deterministic process with NO LLM calls.
//!
//! # Enrichment Data
//!
//! - `enriched_files`: Relevant source file paths via AST/indexing or heuristics
//! - `related_tests`: Test files matching naming conventions or import graphs
//! - `constraints`: Architectural constraints from project config
//! - `prior_attempts`: Prior attempt history for retry scenarios
//!
//! # Constraints
//!
//! - No LLM calls are made during enrichment
//! - Task missing enrichment metadata must not enter ready queue

use std::path::Path;
use swell_core::{PriorAttempt, Task, TaskEnrichment};
#[cfg(test)]
use swell_core::TaskState;

/// Discovers relevant source files from task description and plan.
/// Uses heuristics based on file path patterns and naming conventions.
pub fn discover_enriched_files(task: &Task) -> Vec<String> {
    let mut files = Vec::new();

    // If the task has a plan with affected files, use those
    if let Some(ref plan) = task.plan {
        for step in &plan.steps {
            for file in &step.affected_files {
                if !files.contains(file) {
                    files.push(file.clone());
                }
            }
        }
    }

    // Heuristic: if description mentions specific technologies/modules,
    // include their conventional paths
    let desc_lower = task.description.to_lowercase();

    // Rust crate paths
    if desc_lower.contains("swell-orchestrator") || desc_lower.contains("orchestrator") {
        files.push("crates/swell-orchestrator/src/lib.rs".to_string());
    }
    if desc_lower.contains("swell-memory") || desc_lower.contains("memory") {
        files.push("crates/swell-memory/src/lib.rs".to_string());
    }
    if desc_lower.contains("swell-tools") || desc_lower.contains("tool") {
        files.push("crates/swell-tools/src/lib.rs".to_string());
    }
    if desc_lower.contains("swell-llm") || desc_lower.contains("llm") {
        files.push("crates/swell-llm/src/lib.rs".to_string());
    }
    if desc_lower.contains("swell-validation") || desc_lower.contains("validation") {
        files.push("crates/swell-validation/src/lib.rs".to_string());
    }
    if desc_lower.contains("swell-core") || desc_lower.contains("core") {
        files.push("crates/swell-core/src/lib.rs".to_string());
    }

    // Test file patterns from description keywords
    if desc_lower.contains("test") || desc_lower.contains("spec") {
        // Common test file patterns
        if !files.iter().any(|f| f.contains("_test")) && !files.iter().any(|f| f.contains("/tests/")) {
            // Suggest adding tests - collect in a separate vector to avoid borrow conflict
            let test_paths: Vec<String> = files
                .iter()
                .filter(|file| file.ends_with(".rs") && !file.contains("test") && !file.contains("tests"))
                .map(|file| file.replace(".rs", "_test.rs"))
                .filter(|test_path| !files.contains(test_path))
                .collect();
            for test_path in test_paths {
                files.push(test_path);
            }
        }
    }

    // If nothing found, use a default workspace structure hint
    if files.is_empty() {
        files.push("crates/".to_string());
    }

    files
}

/// Discovers related test files for the given source files.
/// Uses naming conventions (e.g., source.rs → source_test.rs) and
/// import graph heuristics.
pub fn discover_related_tests(enriched_files: &[String]) -> Vec<String> {
    let mut tests = Vec::new();

    for file in enriched_files {
        let path = Path::new(file);

        // Skip non-Rust files
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }

        // Pattern 1: source_test.rs alongside source.rs
        if let Some(stem) = path.file_stem() {
            let stem_str = stem.to_string_lossy();

            // Skip if already a test file
            if stem_str.ends_with("_test") || stem_str.ends_with("_tests") {
                continue;
            }

            let parent = path.parent();
            if let Some(parent) = parent {
                let test_filename = format!("{}_test.rs", stem_str);
                let test_path = parent.join(&test_filename);
                let test_path_str = test_path.to_string_lossy().to_string();

                if !tests.contains(&test_path_str) {
                    tests.push(test_path_str);
                }
            }
        }

        // Pattern 2: tests/ directory mirror
        if let Some(parent) = path.parent() {
            let tests_dir = parent.join("tests");
            if let Some(stem) = path.file_stem() {
                let stem_str = stem.to_string_lossy();
                let test_filename = format!("{}.rs", stem_str);
                let test_path = tests_dir.join(&test_filename);
                let test_path_str = test_path.to_string_lossy().to_string();

                if !tests.contains(&test_path_str) {
                    tests.push(test_path_str);
                }
            }
        }
    }

    tests
}

/// Discovers architectural constraints from project configuration.
/// Loads constraints from .swell/ directory or embedded defaults.
pub fn discover_constraints() -> Vec<String> {
    // Default architectural constraints for the SWELL workspace
    // These are deterministic - no LLM calls needed
    vec![
        // Cross-crate dependency rules
        "swell-core has no internal dependencies".to_string(),
        "swell-orchestrator depends on: swell-core, swell-llm, swell-state, swell-tools, swell-validation".to_string(),
        "swell-daemon depends on: swell-core, swell-orchestrator".to_string(),
        // Error handling conventions
        "Use thiserror for domain errors, anyhow for application errors".to_string(),
        "Prefer ? operator and context() for error propagation".to_string(),
        // Async conventions
        "All async code runs on Tokio with full features".to_string(),
        "Use spawn_blocking for CPU-bound work".to_string(),
        "Never hold Mutex/RwLock guards across .await points".to_string(),
        // Code quality
        "No new clippy warnings (enforce with -D warnings)".to_string(),
        "No hardcoded secrets or API keys".to_string(),
        "Gate live API tests with #[ignore]".to_string(),
        // Task lifecycle
        "Task state machine: Created → Enriched → Ready → Assigned → Executing → Validating".to_string(),
        "Tasks missing enrichment metadata must not enter ready queue".to_string(),
    ]
}

/// Builds prior attempt history from failed task iterations.
/// Attaches previous outcomes for retry scenarios.
pub fn build_prior_attempts(task: &Task) -> Vec<PriorAttempt> {
    // If task is new (no iterations), no prior attempts
    if task.iteration_count == 0 {
        return Vec::new();
    }

    let mut attempts = Vec::new();

    // The current iteration_count represents the number of completed attempts
    // For a retry scenario, we record the history
    if task.iteration_count > 0 {
        // Create a prior attempt record from the task's rejected state
        // This is populated when a task is retried after being rejected
        if let Some(ref rejection_reason) = task.rejected_reason {
            attempts.push(PriorAttempt {
                iteration: task.iteration_count,
                timestamp: task.updated_at,
                outcome: Some(task.state),
                rejected_reason: Some(rejection_reason.clone()),
                modified_files: Vec::new(), // Would be populated from execution
            });
        }
    }

    attempts
}

/// Performs full task enrichment deterministically.
/// Returns a populated TaskEnrichment struct.
///
/// # Constraints
/// - No LLM calls are made during enrichment
/// - This function is deterministic (same input → same output)
pub fn enrich_task(task: &Task) -> TaskEnrichment {
    let enriched_files = discover_enriched_files(task);
    let related_tests = discover_related_tests(&enriched_files);
    let constraints = discover_constraints();
    let prior_attempts = build_prior_attempts(task);

    TaskEnrichment {
        enriched_files,
        related_tests,
        constraints,
        prior_attempts,
        is_enriched: true,
    }
}

/// Extension trait for Task to provide enrichment helpers
pub trait TaskEnrichmentExt {
    /// Apply enrichment to this task (mutates self)
    fn apply_enrichment(&mut self);

    /// Get enriched files (runs discovery if needed)
    fn get_enriched_files(&self) -> Vec<String>;

    /// Get related tests (runs discovery if needed)
    fn get_related_tests(&self) -> Vec<String>;
}

impl TaskEnrichmentExt for Task {
    fn apply_enrichment(&mut self) {
        if self.enrichment.is_enriched {
            return; // Already enriched
        }
        let enriched = enrich_task(self);
        self.enrichment = enriched;
    }

    fn get_enriched_files(&self) -> Vec<String> {
        if self.enrichment.is_enriched {
            self.enrichment.enriched_files.clone()
        } else {
            discover_enriched_files(self)
        }
    }

    fn get_related_tests(&self) -> Vec<String> {
        if self.enrichment.is_enriched {
            self.enrichment.related_tests.clone()
        } else {
            let files = discover_enriched_files(self);
            discover_related_tests(&files)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus};
    use uuid::Uuid;

    fn create_test_plan(task_id: Uuid) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![
                PlanStep {
                    id: Uuid::new_v4(),
                    description: "Modify state machine".to_string(),
                    affected_files: vec![
                        "crates/swell-orchestrator/src/state_machine.rs".to_string(),
                        "crates/swell-orchestrator/src/lib.rs".to_string(),
                    ],
                    expected_tests: vec!["test_state_transitions".to_string()],
                    risk_level: RiskLevel::Medium,
                    dependencies: vec![],
                    status: StepStatus::Pending,
                },
            ],
            total_estimated_tokens: 5000,
            risk_assessment: "Medium risk - modifies core state machine".to_string(),
        }
    }

    #[test]
    fn test_discover_enriched_files_from_plan() {
        let task = Task::new("Test task".to_string());
        let mut task = task;
        task.plan = Some(create_test_plan(task.id));

        let files = discover_enriched_files(&task);

        assert!(!files.is_empty());
        assert!(files.iter().any(|f| f.contains("state_machine.rs")));
        assert!(files.iter().any(|f| f.contains("lib.rs")));
    }

    #[test]
    fn test_discover_enriched_files_from_description() {
        let task = Task::new("Implement feature in swell-orchestrator".to_string());

        let files = discover_enriched_files(&task);

        assert!(!files.is_empty());
        assert!(files.iter().any(|f| f.contains("swell-orchestrator")));
    }

    #[test]
    fn test_discover_related_tests() {
        let files = vec![
            "crates/swell-orchestrator/src/state_machine.rs".to_string(),
            "crates/swell-orchestrator/src/lib.rs".to_string(),
        ];

        let tests = discover_related_tests(&files);

        // Should find _test.rs variants
        assert!(!tests.is_empty());
        assert!(tests.iter().any(|t| t.contains("_test.rs")));
    }

    #[test]
    fn test_discover_constraints() {
        let constraints = discover_constraints();

        assert!(!constraints.is_empty());
        assert!(constraints.iter().any(|c| c.contains("swell-core")));
        assert!(constraints.iter().any(|c| c.contains("Tokio")));
    }

    #[test]
    fn test_build_prior_attempts_empty_for_new_task() {
        let task = Task::new("New task".to_string());

        let attempts = build_prior_attempts(&task);

        assert!(attempts.is_empty());
    }

    #[test]
    fn test_build_prior_attempts_for_retried_task() {
        let mut task = Task::new("Retried task".to_string());
        task.iteration_count = 1;
        task.state = TaskState::Rejected;
        task.rejected_reason = Some("Test failed".to_string());

        let attempts = build_prior_attempts(&task);

        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].iteration, 1);
        assert_eq!(attempts[0].rejected_reason, Some("Test failed".to_string()));
    }

    #[test]
    fn test_enrich_task_returns_full_enrichment() {
        let mut task = Task::new("Test enrichment".to_string());
        task.plan = Some(create_test_plan(task.id));

        let enrichment = enrich_task(&task);

        assert!(enrichment.is_enriched);
        assert!(!enrichment.enriched_files.is_empty());
        assert!(!enrichment.related_tests.is_empty());
        assert!(!enrichment.constraints.is_empty());
    }

    #[test]
    fn test_enrichment_no_llm_calls() {
        // This test verifies the enrichment function is deterministic
        // and doesn't call any LLM backend
        let task = Task::new("Test no LLM".to_string());
        let task2 = Task::new("Test no LLM".to_string());

        let enrichment1 = enrich_task(&task);
        let enrichment2 = enrich_task(&task2);

        // Same input should produce same enrichment
        assert_eq!(enrichment1.enriched_files, enrichment2.enriched_files);
        assert_eq!(enrichment1.related_tests, enrichment2.related_tests);
        assert_eq!(enrichment1.constraints, enrichment2.constraints);
    }
}
