//! Benchmark Suite for SWELL Autonomous Coding Engine
//!
//! This crate provides a curated set of 50 benchmark tasks spanning:
//! - Bug fixes
//! - Features
//! - Refactoring
//! - Tests
//!
//! The benchmark suite is designed to be:
//! - **Diverse**: Tasks cover a wide range of coding patterns and challenges
//! - **Measurable**: Each task has clear success criteria
//! - **Repeatable**: Tasks can be run multiple times with consistent results

mod metrics;
mod runner;
mod task;

pub use metrics::{BenchmarkMetrics, TaskOutcome};
pub use runner::BenchmarkRunner;
pub use task::{BenchmarkTask, TaskCategory, TaskDifficulty, TaskId, TaskResult};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Benchmark identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkId(pub Uuid);

impl BenchmarkId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BenchmarkId {
    fn default() -> Self {
        Self::new()
    }
}

/// A collection of benchmark tasks forming a benchmark suite
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub id: BenchmarkId,
    pub name: String,
    pub description: String,
    pub tasks: Vec<BenchmarkTask>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl BenchmarkSuite {
    /// Create a new benchmark suite with the standard 50-task set
    pub fn standard() -> Self {
        let tasks = Self::generate_standard_tasks();
        Self {
            id: BenchmarkId::new(),
            name: "SWELL Standard Benchmark Suite".to_string(),
            description: "50 curated tasks spanning bug fixes, features, refactoring, and tests"
                .to_string(),
            tasks,
            created_at: chrono::Utc::now(),
        }
    }

    /// Generate the standard set of 50 benchmark tasks
    #[allow(clippy::vec_init_then_push)]
    fn generate_standard_tasks() -> Vec<BenchmarkTask> {
        let mut tasks = Vec::with_capacity(50);

        // =====================================================================
        // BUG FIXES (12 tasks) - Issues that need to be resolved
        // =====================================================================

        // Bug Fix 1-3: State Machine Issues
        tasks.push(BenchmarkTask::new(
            "state_double_transition".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix state machine to prevent double transitions".to_string(),
            vec![
                "Task should only transition once even with concurrent requests".to_string(),
                "State should remain consistent after multiple transition attempts".to_string(),
            ],
            vec!["swell-orchestrator/src/state_machine.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "state_race_condition".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            "Fix race condition in task state transitions".to_string(),
            vec![
                "Concurrent state updates should be serialized".to_string(),
                "No inconsistent state should be observable".to_string(),
            ],
            vec!["swell-orchestrator/src/state_machine.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "checkpoint_restore_corruption".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            "Fix checkpoint restoration that can cause data corruption".to_string(),
            vec![
                "Checkpoint restore should be atomic".to_string(),
                "Partial restores should rollback cleanly".to_string(),
            ],
            vec!["swell-state/src/checkpoint_manager.rs".to_string()],
        ));

        // Bug Fix 4-6: LLM Integration Issues
        tasks.push(BenchmarkTask::new(
            "llm_timeout_handling".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix LLM timeout not propagating correctly".to_string(),
            vec![
                "Timeouts should trigger appropriate error types".to_string(),
                "Resources should be cleaned up on timeout".to_string(),
            ],
            vec![
                "swell-llm/src/anthropic.rs".to_string(),
                "swell-llm/src/openai.rs".to_string(),
            ],
        ));

        tasks.push(BenchmarkTask::new(
            "llm_token_overflow".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix token counting overflow for large contexts".to_string(),
            vec![
                "Token counts should use 64-bit integers".to_string(),
                "Large prompts should not cause integer overflow".to_string(),
            ],
            vec!["swell-llm/src/router.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "llm_model_fallback_stuck".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            "Fix model fallback getting stuck on permanent failures".to_string(),
            vec![
                "Should eventually exhaust fallback options".to_string(),
                "Should return meaningful error after all fallbacks fail".to_string(),
            ],
            vec!["swell-llm/src/router.rs".to_string()],
        ));

        // Bug Fix 7-9: Tool Execution Issues
        tasks.push(BenchmarkTask::new(
            "file_tool_path_traversal".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            "Fix path traversal vulnerability in file tool".to_string(),
            vec![
                "Should prevent access outside workspace".to_string(),
                "Should normalize paths before access checks".to_string(),
            ],
            vec!["swell-tools/src/tools.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "shell_injection".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            "Fix shell command injection in shell tool".to_string(),
            vec![
                "Shell commands should be properly escaped".to_string(),
                "Special characters should not bypass command boundaries".to_string(),
            ],
            vec!["swell-tools/src/executor.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "git_tool_branch_leak".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix git operations affecting wrong branch".to_string(),
            vec![
                "Each worktree should have isolated branch state".to_string(),
                "Branch operations should be scoped to worktree".to_string(),
            ],
            vec!["swell-tools/src/branch_strategy.rs".to_string()],
        ));

        // Bug Fix 10-12: Validation Issues
        tasks.push(BenchmarkTask::new(
            "validation_flaky_detection".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix flaky test detection producing false positives".to_string(),
            vec![
                "Detection algorithm should have low false positive rate".to_string(),
                "History-based detection should be statistically sound".to_string(),
            ],
            vec!["swell-validation/src/flakiness.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "confidence_score_nan".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Low,
            "Fix confidence score returning NaN for edge cases".to_string(),
            vec![
                "Confidence should always return valid float".to_string(),
                "Edge cases should have defined fallback values".to_string(),
            ],
            vec!["swell-validation/src/confidence.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "evidence_pack_incomplete".to_string(),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Fix evidence pack missing required artifacts".to_string(),
            vec![
                "All required evidence types should be captured".to_string(),
                "Missing evidence should not cause pack failure".to_string(),
            ],
            vec!["swell-validation/src/evidence.rs".to_string()],
        ));

        // =====================================================================
        // FEATURES (14 tasks) - New functionality to implement
        // =====================================================================

        // Feature 1-4: Orchestrator Features
        tasks.push(BenchmarkTask::new(
            "autonomy_level_l1".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement L1 Supervised autonomy level".to_string(),
            vec![
                "Every action requires approval before execution".to_string(),
                "User can approve or reject each step".to_string(),
            ],
            vec!["swell-orchestrator/src/autonomy.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "autonomy_level_l3".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement L3 Autonomous autonomy level".to_string(),
            vec![
                "Minimal guidance needed from user".to_string(),
                "Only high-risk actions require approval".to_string(),
            ],
            vec!["swell-orchestrator/src/autonomy.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "work_backlog_aggregation".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement 4-source backlog aggregation".to_string(),
            vec![
                "Aggregate from plan tasks, failure-derived, spec-gap, improvements".to_string(),
                "Deduplicate and prioritize backlog items".to_string(),
            ],
            vec!["swell-orchestrator/src/backlog.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "retry_policy_smart".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement smart retry policy".to_string(),
            vec![
                "First 2 retries with same agent".to_string(),
                "3rd retry switches model".to_string(),
                "4+ triggers human escalation".to_string(),
            ],
            vec!["swell-orchestrator/src/execution.rs".to_string()],
        ));

        // Feature 5-7: Memory Features
        tasks.push(BenchmarkTask::new(
            "memory_confidence_bayesian".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement Bayesian confidence tracking".to_string(),
            vec![
                "Alpha/Beta posterior tracking".to_string(),
                "Thresholds: <0.3 deprecated, 0.3-0.6 uncertain, 0.6-0.8 probable, >0.8 confident"
                    .to_string(),
            ],
            vec!["swell-memory/src/pattern_learning.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "memory_decay_function".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement time-based memory decay".to_string(),
            vec![
                "Procedural decay: 0.99^(days)".to_string(),
                "Environmental decay: 0.95^(days)".to_string(),
                "Buffer decay: 0.90^(days)".to_string(),
            ],
            vec!["swell-memory/src/recall.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "memory_similarity_check".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement cosine similarity deduplication".to_string(),
            vec![
                "Reject memories within cosine distance 0.15".to_string(),
                "Use embeddings for similarity computation".to_string(),
            ],
            vec!["swell-memory/src/recall.rs".to_string()],
        ));

        // Feature 8-10: Tool Features
        tasks.push(BenchmarkTask::new(
            "tool_annotations".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement tool behavioral annotations".to_string(),
            vec![
                "readOnlyHint for non-modifying tools".to_string(),
                "destructiveHint for permanently changing tools".to_string(),
                "idempotentHint for safe-to-retry tools".to_string(),
            ],
            vec!["swell-tools/src/tools.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "resource_limits".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement resource limit enforcement".to_string(),
            vec![
                "Max turns per session".to_string(),
                "Wall-clock timeout enforcement".to_string(),
                "Token and cost caps".to_string(),
            ],
            vec!["swell-tools/src/executor.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "loop_detection".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement loop detection".to_string(),
            vec![
                "Detect same-tool repeated retries".to_string(),
                "Identify oscillation patterns".to_string(),
                "Detect re-planning loops".to_string(),
            ],
            vec!["swell-tools/src/executor.rs".to_string()],
        ));

        // Feature 11-14: Validation Features
        tasks.push(BenchmarkTask::new(
            "staged_test_execution".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement 4-stage test execution".to_string(),
            vec![
                "Stage 0: Instant checks (lint, type)".to_string(),
                "Stage 1: Unit tests".to_string(),
                "Stage 2: Integration tests".to_string(),
                "Stage 3: Comprehensive validation".to_string(),
            ],
            vec!["swell-validation/src/lib.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "result_interpreter".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::High,
            "Implement 5-category failure taxonomy".to_string(),
            vec![
                "Categories: implementation bug, test bug, environment, flaky, unclear".to_string(),
                "Confidence score per classification".to_string(),
                "Action recommendations per category".to_string(),
            ],
            vec!["swell-validation/src/lib.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_generator".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::VeryHigh,
            "Generate tests from acceptance criteria".to_string(),
            vec![
                "Parse acceptance criteria from specs".to_string(),
                "Generate unit, integration, and property-based tests".to_string(),
            ],
            vec!["swell-validation/src/lib.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "traceability_store".to_string(),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            "Implement bidirectional traceability links".to_string(),
            vec![
                "Goal → Criteria → Tests → Results → Evidence".to_string(),
                "Query traceability in both directions".to_string(),
            ],
            vec!["swell-validation/src/evidence.rs".to_string()],
        ));

        // =====================================================================
        // REFACTORING (12 tasks) - Code improvements without behavior change
        // =====================================================================

        // Refactoring 1-4: Orchestrator Refactoring
        tasks.push(BenchmarkTask::new(
            "refactor_state_machine".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Extract state machine into reusable trait".to_string(),
            vec![
                "State transitions should use trait object".to_string(),
                "Testability should improve".to_string(),
            ],
            vec!["swell-orchestrator/src/state_machine.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_agent_pool".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Split large agent pool into focused modules".to_string(),
            vec![
                "No behavior change".to_string(),
                "Better separation of concerns".to_string(),
            ],
            vec!["swell-orchestrator/src/agents.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_policy_engine".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Extract policy evaluation into pure function".to_string(),
            vec![
                "Policy evaluation should be deterministic".to_string(),
                "Easier to test in isolation".to_string(),
            ],
            vec!["swell-orchestrator/src/policy.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_scheduler".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Simplify scheduler priority calculation".to_string(),
            vec![
                "Extract priority scoring into separate function".to_string(),
                "Make scoring algorithm more transparent".to_string(),
            ],
            vec!["swell-orchestrator/src/scheduler.rs".to_string()],
        ));

        // Refactoring 5-8: LLM Refactoring
        tasks.push(BenchmarkTask::new(
            "refactor_llm_traits".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Unify LLM backend traits".to_string(),
            vec![
                "Common interface for all backends".to_string(),
                "Reduce code duplication".to_string(),
            ],
            vec!["swell-llm/src/traits.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_message_builder".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Low,
            "Extract message building into reusable builder".to_string(),
            vec![
                "Fluent API for message construction".to_string(),
                "Reduce boilerplate in message creation".to_string(),
            ],
            vec![
                "swell-llm/src/anthropic.rs".to_string(),
                "swell-llm/src/openai.rs".to_string(),
            ],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_router".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Simplify model router with strategy pattern".to_string(),
            vec![
                "Routing logic should be pluggable".to_string(),
                "Easier to add new routing strategies".to_string(),
            ],
            vec!["swell-llm/src/router.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_error_handling".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Unify error handling across LLM backends".to_string(),
            vec![
                "Consistent error type across backends".to_string(),
                "Better error context preservation".to_string(),
            ],
            vec![
                "swell-llm/src/anthropic.rs".to_string(),
                "swell-llm/src/openai.rs".to_string(),
            ],
        ));

        // Refactoring 9-12: Tool Refactoring
        tasks.push(BenchmarkTask::new(
            "refactor_tool_registry".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Low,
            "Extract tool registration into separate module".to_string(),
            vec![
                "Registry should be self-contained".to_string(),
                "Tools can register themselves".to_string(),
            ],
            vec!["swell-tools/src/registry.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_executor".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Separate command execution from result parsing".to_string(),
            vec![
                "Executor handles execution".to_string(),
                "Result parser handles output interpretation".to_string(),
            ],
            vec!["swell-tools/src/executor.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_worktree_pool".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::Medium,
            "Apply object pool pattern to worktree management".to_string(),
            vec![
                "Worktrees should be reusable".to_string(),
                "Reduce creation overhead".to_string(),
            ],
            vec!["swell-tools/src/worktree_pool.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "refactor_mcp_client".to_string(),
            TaskCategory::Refactoring,
            TaskDifficulty::High,
            "Extract MCP protocol handling into dedicated module".to_string(),
            vec![
                "Protocol logic separated from tool logic".to_string(),
                "Easier to test protocol handling".to_string(),
            ],
            vec!["swell-tools/src/mcp.rs".to_string()],
        ));

        // =====================================================================
        // TESTS (12 tasks) - Test writing and improvement
        // =====================================================================

        // Test 1-4: Orchestrator Tests
        tasks.push(BenchmarkTask::new(
            "test_state_machine_coverage".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add missing state transition tests".to_string(),
            vec![
                "All valid transitions should have test coverage".to_string(),
                "All invalid transitions should return errors".to_string(),
            ],
            vec!["swell-orchestrator/src/state_machine.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_agent_pool_concurrency".to_string(),
            TaskCategory::Test,
            TaskDifficulty::High,
            "Add concurrent agent pool tests".to_string(),
            vec![
                "Test simultaneous reserve/release".to_string(),
                "Test pool exhaustion handling".to_string(),
            ],
            vec!["swell-orchestrator/src/agents.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_policy_evaluation".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add property-based tests for policy evaluation".to_string(),
            vec![
                "Test edge cases in policy rules".to_string(),
                "Use proptest or similar for property testing".to_string(),
            ],
            vec!["swell-orchestrator/src/policy.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_scheduler_priority".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add comprehensive scheduler priority tests".to_string(),
            vec![
                "Test all priority factors".to_string(),
                "Test priority calculation edge cases".to_string(),
            ],
            vec!["swell-orchestrator/src/scheduler.rs".to_string()],
        ));

        // Test 5-8: LLM Tests
        tasks.push(BenchmarkTask::new(
            "test_llm_backoff".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add retry backoff tests for LLM backends".to_string(),
            vec![
                "Test exponential backoff calculation".to_string(),
                "Test max retry limit".to_string(),
            ],
            vec![
                "swell-llm/src/anthropic.rs".to_string(),
                "swell-llm/src/openai.rs".to_string(),
            ],
        ));

        tasks.push(BenchmarkTask::new(
            "test_llm_token_counting".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Low,
            "Add token counting accuracy tests".to_string(),
            vec![
                "Test various input sizes".to_string(),
                "Test special character handling".to_string(),
            ],
            vec!["swell-llm/src/router.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_llm_health_check".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Low,
            "Add health check timeout tests".to_string(),
            vec![
                "Test timeout behavior".to_string(),
                "Test unhealthy backend detection".to_string(),
            ],
            vec!["swell-llm/src/traits.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_model_router".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add model routing decision tests".to_string(),
            vec![
                "Test fallback chain".to_string(),
                "Test routing based on task type".to_string(),
            ],
            vec!["swell-llm/src/router.rs".to_string()],
        ));

        // Test 9-12: Tool Tests
        tasks.push(BenchmarkTask::new(
            "test_file_tool_security".to_string(),
            TaskCategory::Test,
            TaskDifficulty::High,
            "Add security tests for file tool".to_string(),
            vec![
                "Test path traversal prevention".to_string(),
                "Test symlink handling".to_string(),
                "Test permission boundaries".to_string(),
            ],
            vec!["swell-tools/src/tools.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_shell_sandbox".to_string(),
            TaskCategory::Test,
            TaskDifficulty::High,
            "Add shell sandbox isolation tests".to_string(),
            vec![
                "Test command isolation".to_string(),
                "Test environment variable scoping".to_string(),
                "Test resource limit enforcement".to_string(),
            ],
            vec!["swell-tools/src/executor.rs".to_string()],
        ));

        tasks.push(BenchmarkTask::new(
            "test_git_operations".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add git operation atomicity tests".to_string(),
            vec![
                "Test commit atomicity".to_string(),
                "Test branch operation isolation".to_string(),
                "Test worktree cleanup on failure".to_string(),
            ],
            vec![
                "swell-tools/src/commit_strategy.rs".to_string(),
                "swell-tools/src/branch_strategy.rs".to_string(),
            ],
        ));

        tasks.push(BenchmarkTask::new(
            "test_worktree_pool".to_string(),
            TaskCategory::Test,
            TaskDifficulty::Medium,
            "Add worktree pool lifecycle tests".to_string(),
            vec![
                "Test pool initialization".to_string(),
                "Test worktree checkout".to_string(),
                "Test pool cleanup".to_string(),
            ],
            vec!["swell-tools/src/worktree_pool.rs".to_string()],
        ));

        // Ensure we have exactly 50 tasks
        assert_eq!(
            tasks.len(),
            50,
            "Benchmark suite must have exactly 50 tasks"
        );

        tasks
    }

    /// Get tasks by category
    pub fn tasks_by_category(&self, category: TaskCategory) -> Vec<&BenchmarkTask> {
        self.tasks
            .iter()
            .filter(|t| t.category == category)
            .collect()
    }

    /// Get tasks by difficulty
    pub fn tasks_by_difficulty(&self, difficulty: TaskDifficulty) -> Vec<&BenchmarkTask> {
        self.tasks
            .iter()
            .filter(|t| t.difficulty == difficulty)
            .collect()
    }

    /// Get task statistics
    pub fn statistics(&self) -> BenchmarkStatistics {
        let by_category = TaskCategory::iter()
            .map(|cat| {
                let count = self.tasks.iter().filter(|t| t.category == cat).count();
                (cat, count)
            })
            .collect();

        let by_difficulty = TaskDifficulty::iter()
            .map(|diff| {
                let count = self.tasks.iter().filter(|t| t.difficulty == diff).count();
                (diff, count)
            })
            .collect();

        BenchmarkStatistics {
            total_tasks: self.tasks.len(),
            by_category,
            by_difficulty,
        }
    }
}

/// Benchmark statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkStatistics {
    pub total_tasks: usize,
    pub by_category: Vec<(TaskCategory, usize)>,
    pub by_difficulty: Vec<(TaskDifficulty, usize)>,
}

// Implement iteration for enums
impl TaskCategory {
    pub fn iter() -> impl Iterator<Item = TaskCategory> {
        [
            TaskCategory::BugFix,
            TaskCategory::Feature,
            TaskCategory::Refactoring,
            TaskCategory::Test,
        ]
        .iter()
        .copied()
    }
}

impl TaskDifficulty {
    pub fn iter() -> impl Iterator<Item = TaskDifficulty> {
        [
            TaskDifficulty::Low,
            TaskDifficulty::Medium,
            TaskDifficulty::High,
            TaskDifficulty::VeryHigh,
        ]
        .iter()
        .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_suite_has_50_tasks() {
        let suite = BenchmarkSuite::standard();
        assert_eq!(suite.tasks.len(), 50);
    }

    #[test]
    fn test_suite_diversity() {
        let suite = BenchmarkSuite::standard();

        // Check category distribution
        let bug_fixes = suite.tasks_by_category(TaskCategory::BugFix);
        let features = suite.tasks_by_category(TaskCategory::Feature);
        let refactoring = suite.tasks_by_category(TaskCategory::Refactoring);
        let tests = suite.tasks_by_category(TaskCategory::Test);

        assert_eq!(bug_fixes.len(), 12, "Should have 12 bug fixes");
        assert_eq!(features.len(), 14, "Should have 14 features");
        assert_eq!(refactoring.len(), 12, "Should have 12 refactoring tasks");
        assert_eq!(tests.len(), 12, "Should have 12 test tasks");
    }

    #[test]
    fn test_task_ids_are_unique() {
        let suite = BenchmarkSuite::standard();
        let mut ids: Vec<_> = suite.tasks.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            suite.tasks.len(),
            "All task IDs should be unique"
        );
    }

    #[test]
    fn test_all_tasks_have_success_criteria() {
        let suite = BenchmarkSuite::standard();
        for task in &suite.tasks {
            assert!(
                !task.success_criteria.is_empty(),
                "Task {} should have success criteria",
                task.id
            );
        }
    }

    #[test]
    fn test_all_tasks_have_affected_files() {
        let suite = BenchmarkSuite::standard();
        for task in &suite.tasks {
            assert!(
                !task.affected_files.is_empty(),
                "Task {} should have affected files",
                task.id
            );
        }
    }

    #[test]
    fn test_statistics() {
        let suite = BenchmarkSuite::standard();
        let stats = suite.statistics();

        assert_eq!(stats.total_tasks, 50);
        assert_eq!(stats.by_category.len(), 4);
        assert_eq!(stats.by_difficulty.len(), 4);

        // Sum of categories should equal total
        let category_sum: usize = stats.by_category.iter().map(|(_, c)| *c).sum();
        assert_eq!(category_sum, 50);
    }
}
