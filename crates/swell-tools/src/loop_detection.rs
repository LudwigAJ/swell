//! Loop detection for tool execution patterns.
//!
//! This module provides detection and intervention for problematic execution patterns:
//! - **Same-tool repeated retries**: Same tool executed multiple times in a row (likely failing)
//! - **Oscillation patterns**: Alternating between tools without making progress (A→B→A→B)
//! - **Re-planning loops**: Planner called repeatedly without implementation progress
//!
//! # Architecture
//!
//! The loop detection system consists of:
//! - [`ToolLoopTracker`] - tracks tool execution history per session/task
//! - [`LoopPattern`] - classifies detected loop types
//! - [`LoopDetectionResult`] - result of loop analysis with intervention recommendation
//! - [`LoopDetectionConfig`] - configurable thresholds for loop detection
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_tools::loop_detection::{ToolLoopTracker, LoopDetectionConfig};
//!
//! let tracker = ToolLoopTracker::new(LoopDetectionConfig::default());
//! let task_id = TaskId::new();
//!
//! // Record tool executions
//! tracker.record_execution(task_id, "read_file", true).await;
//! tracker.record_execution(task_id, "read_file", false).await;
//! tracker.record_execution(task_id, "read_file", false).await;
//!
//! // Check for loops
//! let result = tracker.analyze(task_id).await;
//! if result.loop_detected {
//!     // Handle loop - intervene or retry with different approach
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use swell_core::ids::TaskId;
use tokio::sync::RwLock;

/// Configuration for loop detection thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectionConfig {
    /// Maximum consecutive same-tool executions before warning
    pub same_tool_retry_threshold: u32,
    /// Maximum oscillation count (A→B→A→B counts as 2 oscillations)
    pub oscillation_threshold: u32,
    /// Window size for oscillation detection (number of recent executions to consider)
    pub oscillation_window_size: usize,
    /// Maximum re-planning calls per task before warning
    pub replan_threshold: u32,
    /// Minimum executions needed before loop detection is active
    pub min_executions_for_detection: u32,
    /// Whether to trigger intervention on loop detection
    pub trigger_intervention: bool,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            same_tool_retry_threshold: 3,
            oscillation_threshold: 3,
            oscillation_window_size: 10,
            replan_threshold: 3,
            min_executions_for_detection: 3,
            trigger_intervention: true,
        }
    }
}

/// A single tool execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    /// Tool name
    pub tool_name: String,
    /// Whether execution succeeded
    pub success: bool,
    /// Arguments (for uniqueness tracking)
    pub arguments: serde_json::Value,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Type of loop pattern detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopPatternType {
    /// Same tool executed repeatedly (likely stuck in retry loop)
    SameToolRetry,
    /// Tools alternating back and forth (A→B→A→B)
    Oscillation,
    /// Planner called repeatedly (stuck in planning phase)
    ReplanningLoop,
    /// Mix of failures and retries with no progress
    NoProgressDoom,
}

/// A detected loop pattern with details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopPattern {
    /// Type of loop detected
    pub pattern_type: LoopPatternType,
    /// Tools involved in the loop
    pub tools_involved: Vec<String>,
    /// How many iterations of this pattern were detected
    pub iteration_count: u32,
    /// Severity of the loop (1-5, 5 being most severe)
    pub severity: u8,
    /// Human-readable description
    pub description: String,
    /// Recommended intervention
    pub recommended_action: String,
}

impl LoopPattern {
    /// Create a new loop pattern
    pub fn new(
        pattern_type: LoopPatternType,
        tools_involved: Vec<String>,
        iteration_count: u32,
        severity: u8,
    ) -> Self {
        let (description, recommended_action) = match pattern_type {
            LoopPatternType::SameToolRetry => (
                format!(
                    "Same tool '{}' executed {} times consecutively without success",
                    tools_involved.first().unwrap_or(&"unknown".to_string()),
                    iteration_count
                ),
                "Consider trying a different approach or checking tool arguments".to_string(),
            ),
            LoopPatternType::Oscillation => (
                format!(
                    "Oscillation detected: tools {} alternating {} times",
                    tools_involved.join(" → "),
                    iteration_count
                ),
                "Identify the root cause and fix the underlying issue".to_string(),
            ),
            LoopPatternType::ReplanningLoop => (
                format!(
                    "Planner called {} times without implementation progress",
                    iteration_count
                ),
                "Review the plan or simplify the task scope".to_string(),
            ),
            LoopPatternType::NoProgressDoom => (
                format!(
                    "No progress detected after {} tool executions",
                    iteration_count
                ),
                "Abort current approach and try a different strategy".to_string(),
            ),
        };

        Self {
            pattern_type,
            tools_involved,
            iteration_count,
            severity,
            description,
            recommended_action,
        }
    }
}

/// Result of loop detection analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopDetectionResult {
    /// Whether a loop was detected
    pub loop_detected: bool,
    /// The type of loop detected (if any)
    pub loop_pattern: Option<LoopPattern>,
    /// Whether intervention should be triggered
    pub should_intervene: bool,
    /// Current streak of consecutive same-tool executions
    pub same_tool_streak: u32,
    /// Current oscillation count in recent history
    pub oscillation_count: u32,
    /// Number of re-plan calls detected
    pub replan_count: u32,
    /// Total executions analyzed
    pub total_executions: u32,
}

/// Tracker for tool execution history per task/session
pub struct ToolLoopTracker {
    /// Configuration
    config: LoopDetectionConfig,
    /// Execution history per task: task_id -> execution history
    history: HashMap<TaskId, VecDeque<ToolExecution>>,
    /// Re-plan count per task
    replan_counts: HashMap<TaskId, u32>,
    /// Pending intervention callbacks
    intervention_callback: Option<Box<dyn FnMut(LoopPattern) + Send + Sync>>,
}

impl ToolLoopTracker {
    /// Create a new tool loop tracker with default configuration
    pub fn new() -> Self {
        Self::with_config(LoopDetectionConfig::default())
    }

    /// Create a new tool loop tracker with custom configuration
    pub fn with_config(config: LoopDetectionConfig) -> Self {
        Self {
            config,
            history: HashMap::new(),
            replan_counts: HashMap::new(),
            intervention_callback: None,
        }
    }

    /// Set a callback to be called when intervention is recommended
    pub fn with_intervention_callback<F>(mut self, callback: F) -> Self
    where
        F: FnMut(LoopPattern) + Send + Sync + 'static,
    {
        self.intervention_callback = Some(Box::new(callback));
        self
    }

    /// Record a tool execution
    pub async fn record_execution(
        &mut self,
        task_id: TaskId,
        tool_name: impl Into<String>,
        success: bool,
        arguments: serde_json::Value,
    ) {
        let tool_name = tool_name.into();
        let execution = ToolExecution {
            tool_name,
            success,
            arguments,
            timestamp: chrono::Utc::now(),
        };

        let history = self.history.entry(task_id).or_default();

        // Keep history bounded by window size * 2 (to allow oscillation detection)
        let max_history = self.config.oscillation_window_size * 2;
        while history.len() >= max_history {
            history.pop_front();
        }

        history.push_back(execution);
    }

    /// Record a planner call (increments re-plan counter)
    pub async fn record_replan(&mut self, task_id: TaskId) {
        let count = self.replan_counts.entry(task_id).or_insert(0);
        *count += 1;
    }

    /// Get re-plan count for a task
    pub fn get_replan_count(&self, task_id: TaskId) -> u32 {
        self.replan_counts.get(&task_id).copied().unwrap_or(0)
    }

    /// Analyze tool execution history for loop patterns
    pub async fn analyze(&mut self, task_id: TaskId) -> LoopDetectionResult {
        // Get history (may be empty if only replans were recorded)
        let history = match self.history.get(&task_id) {
            Some(h) => h.clone(),
            None => VecDeque::new(),
        };

        let total_executions = history.len() as u32;

        // Get re-plan count (tracked separately from executions)
        let replan_count = self.get_replan_count(task_id);

        // If no executions AND no replans, nothing to analyze
        if total_executions == 0 && replan_count == 0 {
            return LoopDetectionResult::default();
        }

        let config = &self.config;
        let executions: Vec<_> = history
            .iter()
            .rev()
            .take(config.oscillation_window_size)
            .collect();

        // Determine if we have enough executions for pattern detection
        let has_enough_executions = total_executions >= config.min_executions_for_detection;

        // Check for same-tool retry pattern (requires executions)
        let (same_tool_streak, last_tool) = if has_enough_executions {
            self.detect_same_tool_retry(&executions)
        } else {
            (0, None)
        };

        // Check for oscillation pattern (requires executions)
        let oscillation_count = if has_enough_executions {
            self.detect_oscillation(&executions)
        } else {
            0
        };

        // Determine if loop is detected
        let mut loop_detected = false;
        let mut loop_pattern = None;
        let mut should_intervene = false;
        let mut severity = 0u8;

        // Same-tool retry detection
        if has_enough_executions && same_tool_streak >= config.same_tool_retry_threshold {
            loop_detected = true;
            severity = self.calculate_severity(same_tool_streak, config.same_tool_retry_threshold);
            loop_pattern = Some(LoopPattern::new(
                LoopPatternType::SameToolRetry,
                vec![last_tool.unwrap_or_else(|| "unknown".to_string())],
                same_tool_streak,
                severity,
            ));
            should_intervene = config.trigger_intervention;
        }

        // Oscillation detection (higher priority if more severe)
        if oscillation_count >= config.oscillation_threshold {
            let oscillation_severity =
                self.calculate_severity(oscillation_count, config.oscillation_threshold);
            if oscillation_severity > severity {
                loop_detected = true;
                severity = oscillation_severity;
                let tools = self.get_oscillating_tools(&executions);
                loop_pattern = Some(LoopPattern::new(
                    LoopPatternType::Oscillation,
                    tools,
                    oscillation_count,
                    severity,
                ));
                should_intervene = config.trigger_intervention;
            }
        }

        // Replanning loop detection (only requires replan_count, not executions)
        if replan_count >= config.replan_threshold {
            loop_detected = true;
            severity = self.calculate_severity(replan_count, config.replan_threshold);
            loop_pattern = Some(LoopPattern::new(
                LoopPatternType::ReplanningLoop,
                vec!["planner".to_string()],
                replan_count,
                severity,
            ));
            should_intervene = config.trigger_intervention;
        }

        // No-progress doom detection: if lots of failures with no progress
        // Only trigger if no other pattern has been detected (SameToolRetry, Oscillation, Replan)
        if has_enough_executions && loop_pattern.is_none() {
            let failure_count = executions.iter().filter(|e| !e.success).count();
            if failure_count >= config.min_executions_for_detection as usize
                && executions.iter().all(|e| !e.success)
            {
                loop_detected = true;
                severity = 5; // Max severity
                loop_pattern = Some(LoopPattern::new(
                    LoopPatternType::NoProgressDoom,
                    vec!["multiple_tools".to_string()],
                    total_executions,
                    severity,
                ));
                should_intervene = config.trigger_intervention;
            }
        }

        let result = LoopDetectionResult {
            loop_detected,
            loop_pattern,
            should_intervene,
            same_tool_streak,
            oscillation_count,
            replan_count,
            total_executions,
        };

        // Trigger intervention callback if needed
        if should_intervene {
            if let Some(ref pattern) = result.loop_pattern {
                if let Some(ref mut callback) = self.intervention_callback {
                    callback(pattern.clone());
                }
            }
        }

        result
    }

    /// Detect consecutive same-tool executions
    fn detect_same_tool_retry(&self, executions: &[&ToolExecution]) -> (u32, Option<String>) {
        if executions.is_empty() {
            return (0, None);
        }

        let mut streak = 1u32;
        let first_tool = &executions[0].tool_name;

        for window in executions.iter().skip(1) {
            if &window.tool_name == first_tool {
                streak += 1;
            } else {
                break;
            }
        }

        (streak, Some(first_tool.clone()))
    }

    /// Detect oscillation pattern (A→B→A→B)
    fn detect_oscillation(&self, executions: &[&ToolExecution]) -> u32 {
        if executions.len() < 4 {
            return 0;
        }

        let mut oscillations = 0u32;
        let mut prev_prev_tool = &executions[0].tool_name;
        let mut prev_tool = &executions[1].tool_name;

        for window in executions.iter().skip(2) {
            let current_tool = &window.tool_name;

            // Detect A→B→A pattern (oscillation)
            if prev_tool != current_tool && prev_prev_tool == current_tool {
                // Don't count if the tools are the same (not a true oscillation)
                if prev_tool != prev_prev_tool {
                    oscillations += 1;
                }
            }

            prev_prev_tool = prev_tool;
            prev_tool = current_tool;
        }

        oscillations
    }

    /// Get the tools involved in oscillation
    fn get_oscillating_tools(&self, executions: &[&ToolExecution]) -> Vec<String> {
        let mut tools: Vec<String> = executions.iter().map(|e| e.tool_name.clone()).collect();
        tools.dedup();
        tools.truncate(4); // Limit to first 4 unique tools
        tools
    }

    /// Calculate severity based on threshold and actual value
    fn calculate_severity(&self, actual: u32, threshold: u32) -> u8 {
        if actual >= threshold * 2 {
            5 // Critical: double the threshold
        } else if actual >= threshold * 3 / 2 {
            4 // High: 1.5x threshold
        } else if actual >= threshold {
            3 // Medium: at threshold
        } else if actual >= threshold * 2 / 3 {
            2 // Low: 2/3 of threshold
        } else {
            1 // Minimal: below threshold
        }
    }

    /// Clear history for a task (e.g., after successful completion)
    pub fn clear(&mut self, task_id: TaskId) {
        self.history.remove(&task_id);
        self.replan_counts.remove(&task_id);
    }

    /// Clear loop context for a task - used by LoopBreaker to reset repetitive context.
    /// Delegates to the existing clear() method, which handles the same cleanup.
    /// The async signature allows this to be called from async intervention contexts.
    pub async fn clear_loop_context(&mut self, task_id: TaskId) {
        self.clear(task_id);
    }

    /// Clear all history
    pub fn clear_all(&mut self) {
        self.history.clear();
        self.replan_counts.clear();
    }

    /// Get current history size for a task
    pub fn history_size(&self, task_id: TaskId) -> usize {
        self.history.get(&task_id).map(|h| h.len()).unwrap_or(0)
    }
}

impl Default for ToolLoopTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper for ToolLoopTracker
pub type SharedToolLoopTracker = Arc<RwLock<ToolLoopTracker>>;

/// Create a new shared tool loop tracker
pub fn create_tool_loop_tracker() -> SharedToolLoopTracker {
    Arc::new(RwLock::new(ToolLoopTracker::new()))
}

/// Create a shared tool loop tracker with custom configuration
pub fn create_tool_loop_tracker_with_config(config: LoopDetectionConfig) -> SharedToolLoopTracker {
    Arc::new(RwLock::new(ToolLoopTracker::with_config(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_no_loop_when_history_empty() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        let result = tracker.analyze(task_id).await;

        assert!(!result.loop_detected);
        assert_eq!(result.total_executions, 0);
    }

    #[tokio::test]
    async fn test_same_tool_retry_detection() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Record 3 consecutive read_file failures
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;

        let result = tracker.analyze(task_id).await;

        assert!(result.loop_detected);
        assert!(result.should_intervene);
        assert_eq!(result.same_tool_streak, 3);
        assert!(matches!(
            result.loop_pattern,
            Some(LoopPattern {
                pattern_type: LoopPatternType::SameToolRetry,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn test_oscillation_detection() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Record oscillation: read_file → edit_file → read_file → edit_file → read_file
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

        let result = tracker.analyze(task_id).await;

        assert!(result.loop_detected);
        // 5 executions in A→B→A→B→A pattern produces 3 oscillation detections
        // Each A→B→A transition is counted when we see the final A
        assert_eq!(result.oscillation_count, 3);
    }

    #[tokio::test]
    async fn test_replan_loop_detection() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Record 3 re-plan calls
        tracker.record_replan(task_id).await;
        tracker.record_replan(task_id).await;
        tracker.record_replan(task_id).await;

        let result = tracker.analyze(task_id).await;

        assert!(result.loop_detected);
        assert_eq!(result.replan_count, 3);
        assert!(matches!(
            result.loop_pattern,
            Some(LoopPattern {
                pattern_type: LoopPatternType::ReplanningLoop,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn test_no_progress_doom_detection() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // All failures
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "edit_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "shell", false, serde_json::json!({}))
            .await;

        let result = tracker.analyze(task_id).await;

        assert!(result.loop_detected);
        assert!(matches!(
            result.loop_pattern,
            Some(LoopPattern {
                pattern_type: LoopPatternType::NoProgressDoom,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn test_clear_clears_history() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker.record_replan(task_id).await;

        assert_eq!(tracker.history_size(task_id), 1);
        assert_eq!(tracker.get_replan_count(task_id), 1);

        tracker.clear(task_id);

        assert_eq!(tracker.history_size(task_id), 0);
        assert_eq!(tracker.get_replan_count(task_id), 0);
    }

    #[tokio::test]
    async fn test_intervention_callback() {
        let pattern_clone: std::sync::Arc<std::sync::Mutex<Option<LoopPattern>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));

        let pattern_clone_clone = pattern_clone.clone();
        let mut tracker = ToolLoopTracker::new().with_intervention_callback(move |pattern| {
            *pattern_clone_clone.lock().unwrap() = Some(pattern);
        });

        let task_id = TaskId::new();

        // Record 3 consecutive failures to trigger intervention
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;

        let result = tracker.analyze(task_id).await;

        assert!(result.should_intervene);
        // Verify callback was triggered by checking the Arc
        let captured = pattern_clone.lock().unwrap();
        assert!(captured.is_some());
    }

    #[tokio::test]
    async fn test_severity_calculation() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Test with 6 consecutive same-tool failures (threshold is 3)
        for _ in 0..6 {
            tracker
                .record_execution(task_id, "shell", false, serde_json::json!({}))
                .await;
        }

        let result = tracker.analyze(task_id).await;

        assert!(result.loop_detected);
        // 6 >= 3*2 = 6, so severity should be 5 (critical)
        if let Some(pattern) = result.loop_pattern {
            assert_eq!(pattern.severity, 5);
        }
    }

    #[tokio::test]
    async fn test_min_executions_threshold() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Only 2 executions (min is 3)
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;

        let result = tracker.analyze(task_id).await;

        // Should not detect loop due to min_executions_for_detection
        assert!(!result.loop_detected);
    }

    #[tokio::test]
    async fn test_config_customization() {
        let config = LoopDetectionConfig {
            same_tool_retry_threshold: 5, // Higher threshold
            oscillation_threshold: 6,
            oscillation_window_size: 12,
            replan_threshold: 5,
            min_executions_for_detection: 5,
            trigger_intervention: false, // Don't trigger by default
        };

        let mut tracker = ToolLoopTracker::with_config(config.clone());
        let task_id = TaskId::new();

        // 4 same-tool executions (threshold is now 5)
        for _ in 0..4 {
            tracker
                .record_execution(task_id, "read_file", false, serde_json::json!({}))
                .await;
        }

        let result = tracker.analyze(task_id).await;

        // Should not detect loop due to higher threshold
        assert!(!result.loop_detected);
        assert!(!result.should_intervene); // Even if detected, intervention is disabled
    }

    #[tokio::test]
    async fn test_oscillation_with_identical_tools() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        // Same tool repeatedly should NOT count as oscillation
        tracker
            .record_execution(task_id, "read_file", true, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", true, serde_json::json!({}))
            .await;
        tracker
            .record_execution(task_id, "read_file", false, serde_json::json!({}))
            .await;

        let result = tracker.analyze(task_id).await;

        // Same tool isn't oscillation
        assert_eq!(result.oscillation_count, 0);
    }

    #[tokio::test]
    async fn test_replan_count_tracking() {
        let mut tracker = ToolLoopTracker::new();
        let task_id = TaskId::new();

        assert_eq!(tracker.get_replan_count(task_id), 0);

        tracker.record_replan(task_id).await;
        assert_eq!(tracker.get_replan_count(task_id), 1);

        tracker.record_replan(task_id).await;
        tracker.record_replan(task_id).await;
        assert_eq!(tracker.get_replan_count(task_id), 3);
    }

    #[tokio::test]
    async fn test_different_tasks_independent() {
        let mut tracker = ToolLoopTracker::new();

        let task1 = Uuid::new_v4();
        let task2 = TaskId::new();

        // Task 1: Has a loop
        for _ in 0..3 {
            tracker
                .record_execution(task1, "shell", false, serde_json::json!({}))
                .await;
        }

        // Task 2: No loop
        tracker
            .record_execution(task2, "read_file", true, serde_json::json!({}))
            .await;

        let result1 = tracker.analyze(task1).await;
        let result2 = tracker.analyze(task2).await;

        assert!(result1.loop_detected);
        assert!(!result2.loop_detected);
    }

    #[tokio::test]
    async fn test_shared_tracker() {
        let shared = create_tool_loop_tracker();
        let task_id = TaskId::new();

        // Write to shared tracker
        {
            let mut tracker = shared.write().await;
            tracker
                .record_execution(task_id, "read_file", false, serde_json::json!({}))
                .await;
        }

        // Read from shared tracker
        {
            let mut tracker = shared.write().await;
            let result = tracker.analyze(task_id).await;
            assert_eq!(result.total_executions, 1);
        }
    }
}
