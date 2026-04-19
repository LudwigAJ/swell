//! Loop detection wiring for agent execution patterns in the orchestrator.
//!
//! This module wires the low-level [`ToolLoopTracker`] from `swell-tools` into
//! the orchestrator's agent execution layer and maps each detected loop pattern
//! to a concrete intervention that is taken against the running task:
//!
//! | Pattern                | Intervention      | Description                                      |
//! |------------------------|-------------------|--------------------------------------------------|
//! | [`SameToolRetry`]      | [`Escalation`]    | Same tool fails N consecutive times → escalate   |
//! | [`Oscillation`]        | [`StrategyChange`]| Tool sequence A→B→A→B for N cycles → new strategy|
//! | [`ReplanningLoop`]     | [`Halt`]          | Plan discarded/regenerated N times → hard stop   |
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_orchestrator::loop_detection::{OrchestratorLoopDetector, LoopIntervention};
//!
//! let mut detector = OrchestratorLoopDetector::new();
//! let task_id = TaskId::new();
//!
//! // Feed tool call results into the detector
//! detector.record_tool_call(task_id, "read_file", false, serde_json::json!({})).await;
//! detector.record_tool_call(task_id, "read_file", false, serde_json::json!({})).await;
//! detector.record_tool_call(task_id, "read_file", false, serde_json::json!({})).await;
//!
//! // Check whether an intervention is required
//! if let Some(intervention) = detector.check(task_id).await {
//!     match intervention {
//!         LoopIntervention::Escalation { reason } => { /* escalate the task */ },
//!         LoopIntervention::StrategyChange { reason } => { /* switch strategy */ },
//!         LoopIntervention::Halt { reason } => { /* hard stop */ },
//!     }
//! }
//! ```

use serde::{Deserialize, Serialize};
use swell_core::ids::TaskId;
use swell_tools::loop_detection::{
    LoopDetectionConfig, LoopPatternType, SharedToolLoopTracker, ToolLoopTracker,
};

/// The action the orchestrator should take when a loop is detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoopIntervention {
    /// Task should be escalated to a human or a senior coordinator.
    ///
    /// Triggered when the same tool is retried consecutively N times with
    /// similar arguments and keeps failing — the agent is stuck.
    Escalation {
        /// Human-readable reason describing the detected pattern.
        reason: String,
    },

    /// The execution strategy should change (e.g. pick a different tool,
    /// re-prompt the model with a hint, or switch to a fallback approach).
    ///
    /// Triggered when two tools oscillate in an A→B→A→B pattern, indicating
    /// the agent is cycling without making forward progress.
    StrategyChange {
        /// Human-readable reason describing the detected pattern.
        reason: String,
    },

    /// Execution must be halted immediately.
    ///
    /// Triggered when the planner has been called N times, discarding previous
    /// plans each time — a strong signal that the task is unsolvable in its
    /// current form or that something is fundamentally wrong.
    Halt {
        /// Human-readable reason describing the detected pattern.
        reason: String,
    },
}

impl LoopIntervention {
    /// Return the reason string regardless of variant.
    pub fn reason(&self) -> &str {
        match self {
            LoopIntervention::Escalation { reason } => reason,
            LoopIntervention::StrategyChange { reason } => reason,
            LoopIntervention::Halt { reason } => reason,
        }
    }
}

/// Maps a [`LoopPatternType`] to the corresponding [`LoopIntervention`].
///
/// This is the canonical intervention mapping used throughout the orchestrator.
pub fn intervention_for_pattern(
    pattern_type: LoopPatternType,
    description: &str,
) -> LoopIntervention {
    match pattern_type {
        LoopPatternType::SameToolRetry => LoopIntervention::Escalation {
            reason: format!("Same-tool retry loop detected: {}", description),
        },
        LoopPatternType::Oscillation => LoopIntervention::StrategyChange {
            reason: format!("Oscillation loop detected: {}", description),
        },
        LoopPatternType::ReplanningLoop => LoopIntervention::Halt {
            reason: format!("Re-planning loop detected: {}", description),
        },
        LoopPatternType::NoProgressDoom => LoopIntervention::Halt {
            reason: format!("No-progress doom loop detected: {}", description),
        },
    }
}

/// Orchestrator-level loop detector that wraps [`ToolLoopTracker`] and
/// translates loop patterns into concrete [`LoopIntervention`] values.
///
/// This is the primary integration point between the `swell-tools` loop
/// detection primitives and the orchestrator's task execution machinery.
///
/// One `OrchestratorLoopDetector` instance should be held per running
/// orchestrator and shared (via `Arc<RwLock<_>>`) across agent execution
/// threads.
pub struct OrchestratorLoopDetector {
    tracker: ToolLoopTracker,
}

impl OrchestratorLoopDetector {
    /// Create a detector with default thresholds.
    pub fn new() -> Self {
        Self {
            tracker: ToolLoopTracker::new(),
        }
    }

    /// Create a detector with custom detection thresholds.
    pub fn with_config(config: LoopDetectionConfig) -> Self {
        Self {
            tracker: ToolLoopTracker::with_config(config),
        }
    }

    /// Record that a tool was called for `task_id`, capturing whether it
    /// succeeded and the arguments supplied.
    ///
    /// This should be called after every tool invocation in the agent
    /// execution loop.
    pub async fn record_tool_call(
        &mut self,
        task_id: TaskId,
        tool_name: impl Into<String>,
        success: bool,
        arguments: serde_json::Value,
    ) {
        self.tracker
            .record_execution(task_id, tool_name, success, arguments)
            .await;
    }

    /// Record that the planner was invoked for `task_id` — i.e. a plan was
    /// discarded and regeneration was requested.
    ///
    /// This should be called whenever the orchestrator discards an existing
    /// plan and re-runs the planning agent.
    pub async fn record_replan(&mut self, task_id: TaskId) {
        self.tracker.record_replan(task_id).await;
    }

    /// Analyse the execution history for `task_id` and return the
    /// intervention that should be taken, or `None` if no loop is detected.
    ///
    /// The returned intervention type is determined by
    /// [`intervention_for_pattern`].
    pub async fn check(&mut self, task_id: TaskId) -> Option<LoopIntervention> {
        let result = self.tracker.analyze(task_id).await;
        if !result.loop_detected {
            return None;
        }
        let pattern = result.loop_pattern?;
        Some(intervention_for_pattern(
            pattern.pattern_type,
            &pattern.description,
        ))
    }

    /// Reset all state for a task (e.g. after successful completion).
    pub fn clear(&mut self, task_id: TaskId) {
        self.tracker.clear(task_id);
    }
}

impl Default for OrchestratorLoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct an [`OrchestratorLoopDetector`] behind a shared, async-safe
/// reference counter.
///
/// This is the recommended way to share a detector across multiple agent
/// threads within the same orchestrator instance.
pub fn create_shared_detector() -> SharedToolLoopTracker {
    swell_tools::loop_detection::create_tool_loop_tracker()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Same-tool retry pattern
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_same_tool_retry_triggers_escalation() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        // Default threshold is 3 consecutive same-tool failures.
        for _ in 0..3 {
            detector
                .record_tool_call(task_id, "read_file", false, serde_json::json!({}))
                .await;
        }

        let intervention = detector.check(task_id).await;
        assert!(intervention.is_some(), "expected an intervention");

        match intervention.unwrap() {
            LoopIntervention::Escalation { reason } => {
                assert!(
                    reason.to_lowercase().contains("same-tool"),
                    "reason should mention same-tool retry, got: {reason}"
                );
            }
            other => panic!("expected Escalation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_same_tool_retry_below_threshold_no_intervention() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        // Only 2 calls — below the default threshold of 3.
        for _ in 0..2 {
            detector
                .record_tool_call(task_id, "read_file", false, serde_json::json!({}))
                .await;
        }

        assert!(
            detector.check(task_id).await.is_none(),
            "should not trigger below threshold"
        );
    }

    // ------------------------------------------------------------------
    // Oscillation pattern
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_oscillation_triggers_strategy_change() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        // Pattern: read_file → edit_file → read_file → edit_file → read_file
        // produces 3 oscillation counts which equals the default threshold (3).
        let tools = [
            "read_file",
            "edit_file",
            "read_file",
            "edit_file",
            "read_file",
        ];
        for tool in &tools {
            detector
                .record_tool_call(task_id, *tool, false, serde_json::json!({}))
                .await;
        }

        let intervention = detector.check(task_id).await;
        assert!(intervention.is_some(), "expected an intervention");

        match intervention.unwrap() {
            LoopIntervention::StrategyChange { reason } => {
                assert!(
                    reason.to_lowercase().contains("oscillation"),
                    "reason should mention oscillation, got: {reason}"
                );
            }
            other => panic!("expected StrategyChange, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_oscillation_same_tool_repeated() {
        // Using the same tool repeatedly is NOT an oscillation.
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        for _ in 0..6 {
            detector
                .record_tool_call(task_id, "shell", true, serde_json::json!({}))
                .await;
        }

        let result = detector.tracker.analyze(task_id).await;
        assert_eq!(
            result.oscillation_count, 0,
            "repeated same tool is not oscillation"
        );
    }

    // ------------------------------------------------------------------
    // Re-planning pattern
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_replanning_loop_triggers_halt() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        // Default replan threshold is 3.
        for _ in 0..3 {
            detector.record_replan(task_id).await;
        }

        let intervention = detector.check(task_id).await;
        assert!(intervention.is_some(), "expected an intervention");

        match intervention.unwrap() {
            LoopIntervention::Halt { reason } => {
                assert!(
                    reason.to_lowercase().contains("re-planning"),
                    "reason should mention re-planning, got: {reason}"
                );
            }
            other => panic!("expected Halt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_replanning_below_threshold_no_intervention() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        // Only 2 replans — below the default threshold of 3.
        for _ in 0..2 {
            detector.record_replan(task_id).await;
        }

        assert!(
            detector.check(task_id).await.is_none(),
            "should not trigger below threshold"
        );
    }

    // ------------------------------------------------------------------
    // Intervention mapping
    // ------------------------------------------------------------------

    #[test]
    fn test_intervention_for_same_tool_retry_is_escalation() {
        let intervention =
            intervention_for_pattern(LoopPatternType::SameToolRetry, "read_file failed 3 times");
        assert!(
            matches!(intervention, LoopIntervention::Escalation { .. }),
            "SameToolRetry should map to Escalation"
        );
    }

    #[test]
    fn test_intervention_for_oscillation_is_strategy_change() {
        let intervention =
            intervention_for_pattern(LoopPatternType::Oscillation, "A→B→A→B for 3 cycles");
        assert!(
            matches!(intervention, LoopIntervention::StrategyChange { .. }),
            "Oscillation should map to StrategyChange"
        );
    }

    #[test]
    fn test_intervention_for_replanning_is_halt() {
        let intervention =
            intervention_for_pattern(LoopPatternType::ReplanningLoop, "plan discarded 3 times");
        assert!(
            matches!(intervention, LoopIntervention::Halt { .. }),
            "ReplanningLoop should map to Halt"
        );
    }

    #[test]
    fn test_intervention_for_no_progress_is_halt() {
        let intervention =
            intervention_for_pattern(LoopPatternType::NoProgressDoom, "all executions failed");
        assert!(
            matches!(intervention, LoopIntervention::Halt { .. }),
            "NoProgressDoom should map to Halt"
        );
    }

    // ------------------------------------------------------------------
    // Clear / isolation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_clear_resets_state() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        for _ in 0..3 {
            detector
                .record_tool_call(task_id, "shell", false, serde_json::json!({}))
                .await;
        }

        // Loop should be detected before clearing.
        assert!(detector.check(task_id).await.is_some());

        // After clearing, the history is gone.
        detector.clear(task_id);

        // Analyse again — should be clean slate (no history = no detection).
        let result = detector.tracker.analyze(task_id).await;
        assert!(!result.loop_detected, "should be no loop after clear");
    }

    #[tokio::test]
    async fn test_different_tasks_are_independent() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_a = TaskId::new();
        let task_b = TaskId::new();

        // Task A gets a loop.
        for _ in 0..3 {
            detector
                .record_tool_call(task_a, "shell", false, serde_json::json!({}))
                .await;
        }

        // Task B is healthy.
        detector
            .record_tool_call(task_b, "read_file", true, serde_json::json!({}))
            .await;

        assert!(
            detector.check(task_a).await.is_some(),
            "task A should trigger intervention"
        );
        assert!(
            detector.check(task_b).await.is_none(),
            "task B should be unaffected"
        );
    }

    // ------------------------------------------------------------------
    // Custom thresholds
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_custom_thresholds_respected() {
        let config = LoopDetectionConfig {
            same_tool_retry_threshold: 5,
            oscillation_threshold: 6,
            oscillation_window_size: 12,
            replan_threshold: 5,
            min_executions_for_detection: 5,
            trigger_intervention: true,
        };

        let mut detector = OrchestratorLoopDetector::with_config(config);
        let task_id = TaskId::new();

        // 3 consecutive failures: below the custom threshold of 5.
        for _ in 0..3 {
            detector
                .record_tool_call(task_id, "shell", false, serde_json::json!({}))
                .await;
        }

        assert!(
            detector.check(task_id).await.is_none(),
            "should not trigger with custom higher threshold"
        );

        // Now add 2 more to reach the threshold (total 5, but min_executions is also 5).
        for _ in 0..2 {
            detector
                .record_tool_call(task_id, "shell", false, serde_json::json!({}))
                .await;
        }

        assert!(
            detector.check(task_id).await.is_some(),
            "should trigger at custom threshold"
        );
    }

    // ------------------------------------------------------------------
    // Reason content
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_intervention_reason_is_non_empty() {
        let mut detector = OrchestratorLoopDetector::new();
        let task_id = TaskId::new();

        for _ in 0..3 {
            detector.record_replan(task_id).await;
        }

        let intervention = detector.check(task_id).await.unwrap();
        assert!(
            !intervention.reason().is_empty(),
            "intervention reason must not be empty"
        );
    }
}
