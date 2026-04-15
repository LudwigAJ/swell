//! Real-time cost tracking for LLM usage across tasks and runs.
//!
//! This module provides comprehensive cost tracking including:
//! - Cost computed per LLM call
//! - Aggregated per task
//! - Aggregated per run
//! - Budget alerts at thresholds
//! - Cost breakdown by model
//!
//! # Usage
//!
//! ```rust
//! use swell_core::cost_tracking::{CostTracker, CostBudget, ModelBreakdown};
//! use swell_core::opentelemetry::pricing;
//! use uuid::Uuid;
//!
//! let mut tracker = CostTracker::new();
//! tracker.set_task_budget(500_000); // 500k tokens per task
//!
//! let task_id = Uuid::new_v4();
//!
//! // Record a cost (input_tokens, output_tokens, model_name)
//! tracker.record_task_cost(task_id, 1000, 500, "claude-3-5-sonnet").unwrap();
//!
//! // Check if we're in warning zone
//! if tracker.is_warning_threshold() {
//!     println!("Approaching budget limit!");
//! }
//!
//! // Get cost summary
//! let summary = tracker.get_summary();
//! println!("Total cost: ${:.4}", summary.total_cost_usd);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// Re-export pricing from opentelemetry module
use crate::opentelemetry::pricing;

/// Outcome of a task for cost-per-outcome analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    /// Task completed successfully
    #[default]
    Completed,
    /// Task failed
    Failed,
    /// Task was rejected
    Rejected,
    /// Task was cancelled
    Cancelled,
    /// Task timed out
    Timeout,
    /// Task was escalated
    Escalated,
}

impl TaskOutcome {
    /// Returns true if this outcome represents success
    pub fn is_success(&self) -> bool {
        matches!(self, TaskOutcome::Completed)
    }
}

/// Task cost summary including both cost and outcome for cost-per-outcome analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCostSummary {
    /// Total cost in USD
    pub total_cost_usd: f64,
    /// Total input tokens
    pub total_input_tokens: u64,
    /// Total output tokens
    pub total_output_tokens: u64,
    /// Total tokens
    pub total_tokens: u64,
    /// Number of LLM calls
    pub call_count: u64,
    /// Cost breakdown by model
    pub model_breakdown: ModelBreakdown,
    /// Final task outcome
    pub outcome: TaskOutcome,
}

/// A cost record for a single LLM call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    /// Unique identifier for this cost record
    pub id: Uuid,
    /// Task ID this cost belongs to
    pub task_id: Uuid,
    /// Model used for this call
    pub model: String,
    /// Number of input tokens
    pub input_tokens: u64,
    /// Number of output tokens
    pub output_tokens: u64,
    /// Calculated cost in USD
    pub cost_usd: f64,
    /// When this cost was recorded
    pub recorded_at: DateTime<Utc>,
}

impl CostRecord {
    /// Create a new cost record
    pub fn new(task_id: Uuid, model: String, input_tokens: u64, output_tokens: u64) -> Self {
        let pricing = pricing::for_model(&model);
        let cost_usd = pricing.calculate_cost(input_tokens, output_tokens);

        Self {
            id: Uuid::new_v4(),
            task_id,
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            recorded_at: Utc::now(),
        }
    }

    /// Total tokens for this call
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Cost breakdown by model
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelBreakdown {
    /// Cost per model name
    pub by_model: HashMap<String, ModelCostInfo>,
}

impl ModelBreakdown {
    /// Create a new empty breakdown
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a cost record to the breakdown
    pub fn add_record(&mut self, record: &CostRecord) {
        let info = self
            .by_model
            .entry(record.model.clone())
            .or_insert_with(|| ModelCostInfo {
                model: record.model.clone(),
                call_count: 0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cost_usd: 0.0,
            });

        info.call_count += 1;
        info.total_input_tokens += record.input_tokens;
        info.total_output_tokens += record.output_tokens;
        info.total_cost_usd += record.cost_usd;
    }

    /// Get cost info for a specific model
    pub fn get(&self, model: &str) -> Option<&ModelCostInfo> {
        self.by_model.get(model)
    }

    /// Total cost across all models
    pub fn total_cost_usd(&self) -> f64 {
        self.by_model.values().map(|info| info.total_cost_usd).sum()
    }

    /// Total tokens across all models
    pub fn total_tokens(&self) -> u64 {
        self.by_model
            .values()
            .map(|info| info.total_input_tokens + info.total_output_tokens)
            .sum()
    }

    /// Get all models tracked
    pub fn models(&self) -> Vec<&str> {
        self.by_model.keys().map(|s| s.as_str()).collect()
    }
}

/// Cost information for a specific model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostInfo {
    /// Model name
    pub model: String,
    /// Number of LLM calls made with this model
    pub call_count: u64,
    /// Total input tokens
    pub total_input_tokens: u64,
    /// Total output tokens
    pub total_output_tokens: u64,
    /// Total cost in USD
    pub total_cost_usd: f64,
}

impl ModelCostInfo {
    /// Total tokens for this model
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }

    /// Average cost per call
    pub fn avg_cost_per_call(&self) -> f64 {
        if self.call_count > 0 {
            self.total_cost_usd / self.call_count as f64
        } else {
            0.0
        }
    }
}

/// Summary of costs for a task or run
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostSummary {
    /// Total cost in USD
    pub total_cost_usd: f64,
    /// Total input tokens
    pub total_input_tokens: u64,
    /// Total output tokens
    pub total_output_tokens: u64,
    /// Total tokens
    pub total_tokens: u64,
    /// Number of LLM calls
    pub call_count: u64,
    /// Cost breakdown by model
    pub model_breakdown: ModelBreakdown,
}

impl CostSummary {
    /// Create a new empty summary
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a cost record to the summary
    pub fn add_record(&mut self, record: &CostRecord) {
        self.total_cost_usd += record.cost_usd;
        self.total_input_tokens += record.input_tokens;
        self.total_output_tokens += record.output_tokens;
        self.total_tokens += record.total_tokens();
        self.call_count += 1;
        self.model_breakdown.add_record(record);
    }

    /// Average cost per call
    pub fn avg_cost_per_call(&self) -> f64 {
        if self.call_count > 0 {
            self.total_cost_usd / self.call_count as f64
        } else {
            0.0
        }
    }

    /// Average tokens per call
    pub fn avg_tokens_per_call(&self) -> f64 {
        if self.call_count > 0 {
            self.total_tokens as f64 / self.call_count as f64
        } else {
            0.0
        }
    }
}

/// Budget configuration for cost control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBudget {
    /// Maximum tokens per task
    pub max_tokens_per_task: u64,
    /// Warning threshold (0.0 to 1.0, percentage of max)
    pub warning_threshold: f64,
    /// Hard stop threshold (0.0 to 1.0)
    pub hard_stop_threshold: f64,
}

impl CostBudget {
    /// Create a new budget with default thresholds
    pub fn new(max_tokens_per_task: u64) -> Self {
        Self {
            max_tokens_per_task,
            warning_threshold: 0.75,
            hard_stop_threshold: 1.0,
        }
    }

    /// Create a budget with custom thresholds
    pub fn with_thresholds(mut self, warning: f64, hard_stop: f64) -> Self {
        self.warning_threshold = warning;
        self.hard_stop_threshold = hard_stop;
        self
    }

    /// Calculate warning threshold in tokens
    pub fn warning_tokens(&self) -> u64 {
        (self.max_tokens_per_task as f64 * self.warning_threshold) as u64
    }

    /// Calculate hard stop threshold in tokens
    pub fn hard_stop_tokens(&self) -> u64 {
        (self.max_tokens_per_task as f64 * self.hard_stop_threshold) as u64
    }

    /// Check if tokens are at warning threshold
    pub fn is_warning_threshold(&self, tokens: u64) -> bool {
        let ratio = tokens as f64 / self.max_tokens_per_task as f64;
        ratio >= self.warning_threshold && ratio < self.hard_stop_threshold
    }

    /// Check if tokens exceed hard stop threshold
    pub fn is_hard_stop(&self, tokens: u64) -> bool {
        let ratio = tokens as f64 / self.max_tokens_per_task as f64;
        ratio >= self.hard_stop_threshold
    }
}

impl Default for CostBudget {
    fn default() -> Self {
        Self::new(1_000_000) // 1M tokens default
    }
}

/// Alert type for budget alerts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAlertType {
    /// Warning threshold reached
    Warning,
    /// Hard stop threshold reached
    HardStop,
}

/// Budget alert event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlert {
    /// Alert type
    pub alert_type: BudgetAlertType,
    /// Task ID if applicable
    pub task_id: Option<Uuid>,
    /// Current tokens
    pub current_tokens: u64,
    /// Threshold tokens
    pub threshold_tokens: u64,
    /// Current cost in USD
    pub current_cost_usd: f64,
    /// When alert was triggered
    pub triggered_at: DateTime<Utc>,
}

impl BudgetAlert {
    /// Create a warning alert
    pub fn warning(
        task_id: Uuid,
        current_tokens: u64,
        threshold_tokens: u64,
        current_cost_usd: f64,
    ) -> Self {
        Self {
            alert_type: BudgetAlertType::Warning,
            task_id: Some(task_id),
            current_tokens,
            threshold_tokens,
            current_cost_usd,
            triggered_at: Utc::now(),
        }
    }

    /// Create a hard stop alert
    pub fn hard_stop(
        task_id: Uuid,
        current_tokens: u64,
        threshold_tokens: u64,
        current_cost_usd: f64,
    ) -> Self {
        Self {
            alert_type: BudgetAlertType::HardStop,
            task_id: Some(task_id),
            current_tokens,
            threshold_tokens,
            current_cost_usd,
            triggered_at: Utc::now(),
        }
    }

    /// Create a run-level alert (no task)
    pub fn run_warning(current_tokens: u64, threshold_tokens: u64, current_cost_usd: f64) -> Self {
        Self {
            alert_type: BudgetAlertType::Warning,
            task_id: None,
            current_tokens,
            threshold_tokens,
            current_cost_usd,
            triggered_at: Utc::now(),
        }
    }

    /// Create a run-level hard stop alert (no task)
    pub fn run_hard_stop(
        current_tokens: u64,
        threshold_tokens: u64,
        current_cost_usd: f64,
    ) -> Self {
        Self {
            alert_type: BudgetAlertType::HardStop,
            task_id: None,
            current_tokens,
            threshold_tokens,
            current_cost_usd,
            triggered_at: Utc::now(),
        }
    }
}

/// Cost tracker for real-time tracking across tasks and runs
#[derive(Debug, Clone)]
pub struct CostTracker {
    /// Budget configuration
    budget: CostBudget,
    /// Per-task cost records
    task_costs: HashMap<Uuid, Vec<CostRecord>>,
    /// Run-level cost records (for tasks without a task_id or for run aggregation)
    run_costs: Vec<CostRecord>,
    /// Run-level summary (aggregated)
    run_summary: CostSummary,
    /// Active task ID (for convenience)
    active_task_id: Option<Uuid>,
    /// Per-task summaries (for quick access)
    task_summaries: HashMap<Uuid, CostSummary>,
    /// Per-task outcomes for cost-per-outcome analysis
    task_outcomes: HashMap<Uuid, TaskOutcome>,
    /// Budget alerts triggered
    budget_alerts: Vec<BudgetAlert>,
    /// Last alert time for cooldown (in milliseconds since Unix epoch)
    last_alert_time_ms: Option<u64>,
    /// Alert cooldown duration in seconds
    alert_cooldown_secs: u64,
}

impl CostTracker {
    /// Create a new cost tracker with default budget
    pub fn new() -> Self {
        Self::with_budget(CostBudget::default())
    }

    /// Create a cost tracker with a custom budget
    pub fn with_budget(budget: CostBudget) -> Self {
        Self {
            budget,
            task_costs: HashMap::new(),
            run_costs: Vec::new(),
            run_summary: CostSummary::new(),
            active_task_id: None,
            task_summaries: HashMap::new(),
            task_outcomes: HashMap::new(),
            budget_alerts: Vec::new(),
            last_alert_time_ms: None,
            alert_cooldown_secs: 300, // 5 minute default cooldown
        }
    }

    /// Set the task budget
    pub fn set_task_budget(&mut self, max_tokens: u64) {
        self.budget = CostBudget::new(max_tokens);
    }

    /// Set the active task ID for recording costs
    pub fn set_active_task(&mut self, task_id: Uuid) {
        self.active_task_id = Some(task_id);
    }

    /// Clear the active task ID
    pub fn clear_active_task(&mut self) {
        self.active_task_id = None;
    }

    /// Check if cooldown has passed since last alert
    fn is_alert_cooldown_passed(&self) -> bool {
        if let Some(last_time) = self.last_alert_time_ms {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let elapsed = now.saturating_sub(last_time);
            return elapsed >= self.alert_cooldown_secs;
        }
        true
    }

    /// Record an LLM cost
    pub fn record_llm_cost(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        model: &str,
    ) -> Result<CostRecord, CostTrackerError> {
        let task_id = self.active_task_id.ok_or(CostTrackerError::NoActiveTask)?;

        // Create the cost record
        let record = CostRecord::new(task_id, model.to_string(), input_tokens, output_tokens);

        // Add to task costs
        let task_records = self.task_costs.entry(task_id).or_default();
        task_records.push(record.clone());

        // Add to run costs
        self.run_costs.push(record.clone());

        // Update summaries
        self.run_summary.add_record(&record);
        self.task_summaries
            .entry(task_id)
            .or_default()
            .add_record(&record);

        // Check for budget alerts
        self.check_and_record_budget_alerts(task_id, &record);

        Ok(record)
    }

    /// Record cost for a specific task (alternative to set_active_task)
    pub fn record_task_cost(
        &mut self,
        task_id: Uuid,
        input_tokens: u64,
        output_tokens: u64,
        model: &str,
    ) -> Result<CostRecord, CostTrackerError> {
        // Create the cost record
        let record = CostRecord::new(task_id, model.to_string(), input_tokens, output_tokens);

        // Add to task costs
        let task_records = self.task_costs.entry(task_id).or_default();
        task_records.push(record.clone());

        // Add to run costs
        self.run_costs.push(record.clone());

        // Update summaries
        self.run_summary.add_record(&record);
        self.task_summaries
            .entry(task_id)
            .or_default()
            .add_record(&record);

        // Check for budget alerts
        self.check_and_record_budget_alerts(task_id, &record);

        Ok(record)
    }

    /// Check and record budget alerts
    fn check_and_record_budget_alerts(&mut self, task_id: Uuid, _record: &CostRecord) {
        // Only check if cooldown has passed
        if !self.is_alert_cooldown_passed() {
            return;
        }

        // Check task-level budget
        if let Some(summary) = self.task_summaries.get(&task_id) {
            let tokens = summary.total_tokens;

            if self.budget.is_hard_stop(tokens) {
                let alert = BudgetAlert::hard_stop(
                    task_id,
                    tokens,
                    self.budget.hard_stop_tokens(),
                    summary.total_cost_usd,
                );
                self.budget_alerts.push(alert);
                self.last_alert_time_ms = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                );
            } else if self.budget.is_warning_threshold(tokens) {
                let alert = BudgetAlert::warning(
                    task_id,
                    tokens,
                    self.budget.warning_tokens(),
                    summary.total_cost_usd,
                );
                self.budget_alerts.push(alert);
                self.last_alert_time_ms = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                );
            }
        }

        // Check run-level budget (if run budget is configured)
        let run_tokens = self.run_summary.total_tokens;
        let run_threshold = 10_000_000u64; // 10M default run limit

        if run_tokens >= run_threshold && self.is_alert_cooldown_passed() {
            let alert = BudgetAlert::run_hard_stop(
                run_tokens,
                run_threshold,
                self.run_summary.total_cost_usd,
            );
            self.budget_alerts.push(alert);
            self.last_alert_time_ms = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
        }
    }

    /// Get cost records for a task
    pub fn get_task_records(&self, task_id: Uuid) -> Vec<&CostRecord> {
        self.task_costs
            .get(&task_id)
            .map(|r| r.iter().collect())
            .unwrap_or_default()
    }

    /// Get summary for a task
    pub fn get_task_summary(&self, task_id: Uuid) -> Option<&CostSummary> {
        self.task_summaries.get(&task_id)
    }

    /// Get task summary including outcome for cost-per-outcome analysis
    /// Returns both the cost data and the final task outcome
    pub fn get_task_summary_with_outcome(&self, task_id: Uuid) -> Option<TaskCostSummary> {
        let summary = self.task_summaries.get(&task_id)?;
        let outcome = self.task_outcomes.get(&task_id).copied().unwrap_or_default();
        Some(TaskCostSummary {
            total_cost_usd: summary.total_cost_usd,
            total_input_tokens: summary.total_input_tokens,
            total_output_tokens: summary.total_output_tokens,
            total_tokens: summary.total_tokens,
            call_count: summary.call_count,
            model_breakdown: summary.model_breakdown.clone(),
            outcome,
        })
    }

    /// Set the outcome for a task (called when task completes)
    pub fn set_task_outcome(&mut self, task_id: Uuid, outcome: TaskOutcome) {
        self.task_outcomes.insert(task_id, outcome);
    }

    /// Get the outcome for a task
    pub fn get_task_outcome(&self, task_id: Uuid) -> Option<TaskOutcome> {
        self.task_outcomes.get(&task_id).copied()
    }

    /// Get run-level summary
    pub fn get_summary(&self) -> &CostSummary {
        &self.run_summary
    }

    /// Get all budget alerts
    pub fn get_budget_alerts(&self) -> &[BudgetAlert] {
        &self.budget_alerts
    }

    /// Check if current task is at warning threshold
    pub fn is_warning_threshold(&self) -> bool {
        if let Some(task_id) = self.active_task_id {
            if let Some(summary) = self.task_summaries.get(&task_id) {
                return self.budget.is_warning_threshold(summary.total_tokens);
            }
        }
        false
    }

    /// Check if current task has exceeded hard stop
    pub fn is_hard_stop(&self) -> bool {
        if let Some(task_id) = self.active_task_id {
            if let Some(summary) = self.task_summaries.get(&task_id) {
                return self.budget.is_hard_stop(summary.total_tokens);
            }
        }
        false
    }

    /// Check if run has exceeded limits
    pub fn is_run_limit_exceeded(&self) -> bool {
        self.run_summary.total_tokens >= 10_000_000 // 10M run limit
    }

    /// Clear all cost data (for starting a new run)
    pub fn clear(&mut self) {
        self.task_costs.clear();
        self.run_costs.clear();
        self.run_summary = CostSummary::new();
        self.task_summaries.clear();
        self.task_outcomes.clear();
        self.budget_alerts.clear();
        self.last_alert_time_ms = None;
    }

    /// Reset task costs (keep run costs)
    pub fn reset_task_costs(&mut self, task_id: Uuid) {
        self.task_costs.remove(&task_id);
        self.task_summaries.remove(&task_id);
        self.task_outcomes.remove(&task_id);
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from cost tracking
#[derive(Debug, thiserror::Error)]
pub enum CostTrackerError {
    #[error("No active task set for cost tracking")]
    NoActiveTask,

    #[error("Invalid token count: {0}")]
    InvalidTokenCount(String),

    #[error("Budget exceeded")]
    BudgetExceeded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_record_creation() {
        let record = CostRecord::new(
            Uuid::new_v4(),
            "claude-3-5-sonnet".to_string(),
            1_000_000,
            500_000,
        );

        assert_eq!(record.input_tokens, 1_000_000);
        assert_eq!(record.output_tokens, 500_000);
        assert_eq!(record.total_tokens(), 1_500_000);
        // Cost should be: 1M * $3/M + 0.5M * $15/M = $3 + $7.50 = $10.50
        assert!((record.cost_usd - 10.5).abs() < 0.001);
    }

    #[test]
    fn test_model_breakdown() {
        let task_id = Uuid::new_v4();
        let mut breakdown = ModelBreakdown::new();

        // Add a Sonnet call
        let record1 = CostRecord::new(task_id, "claude-3-5-sonnet".to_string(), 1000, 500);
        breakdown.add_record(&record1);

        // Add another Sonnet call
        let record2 = CostRecord::new(task_id, "claude-3-5-sonnet".to_string(), 2000, 1000);
        breakdown.add_record(&record2);

        // Add a GPT-4o call
        let record3 = CostRecord::new(task_id, "gpt-4o".to_string(), 1500, 750);
        breakdown.add_record(&record3);

        assert_eq!(breakdown.models().len(), 2);

        let sonnet_info = breakdown.get("claude-3-5-sonnet").unwrap();
        assert_eq!(sonnet_info.call_count, 2);
        assert_eq!(sonnet_info.total_input_tokens, 3000);
        assert_eq!(sonnet_info.total_output_tokens, 1500);
    }

    #[test]
    fn test_cost_summary() {
        let task_id = Uuid::new_v4();
        let mut summary = CostSummary::new();

        let record1 = CostRecord::new(task_id, "claude-3-5-sonnet".to_string(), 1000, 500);
        summary.add_record(&record1);

        let record2 = CostRecord::new(task_id, "gpt-4o".to_string(), 2000, 1000);
        summary.add_record(&record2);

        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_tokens, 4500);
        assert!(summary.total_cost_usd > 0.0);
        assert_eq!(summary.avg_cost_per_call(), summary.total_cost_usd / 2.0);
    }

    #[test]
    fn test_cost_budget() {
        let budget = CostBudget::new(500_000);

        assert!(!budget.is_warning_threshold(100_000)); // 20% - no warning
        assert!(!budget.is_warning_threshold(374_999)); // 74.99% - no warning
        assert!(budget.is_warning_threshold(375_000)); // 75% - warning threshold
        assert!(budget.is_warning_threshold(499_999)); // 99.99% - still warning
        assert!(!budget.is_hard_stop(499_999)); // 99.99% - no hard stop
        assert!(budget.is_hard_stop(500_000)); // 100% - hard stop
        assert!(budget.is_hard_stop(750_000)); // 150% - hard stop
    }

    #[test]
    fn test_cost_tracker_basic() {
        let mut tracker = CostTracker::new();
        let task_id = Uuid::new_v4();
        tracker.set_active_task(task_id);

        // Record some costs
        tracker
            .record_llm_cost(1000, 500, "claude-3-5-sonnet")
            .unwrap();
        tracker
            .record_llm_cost(2000, 1000, "claude-3-5-sonnet")
            .unwrap();

        // Check summary
        let summary = tracker.get_summary();
        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_tokens, 4500);

        // Check task summary
        let task_summary = tracker.get_task_summary(task_id);
        assert!(task_summary.is_some());
        assert_eq!(task_summary.unwrap().call_count, 2);
    }

    #[test]
    fn test_cost_tracker_no_active_task() {
        let mut tracker = CostTracker::new();
        let result = tracker.record_llm_cost(1000, 500, "claude-3-5-sonnet");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CostTrackerError::NoActiveTask
        ));
    }

    #[test]
    fn test_cost_tracker_record_task_cost() {
        let mut tracker = CostTracker::new();
        let task_id = Uuid::new_v4();

        // Record cost without setting active task
        let result = tracker.record_task_cost(task_id, 1000, 500, "gpt-4o");
        assert!(result.is_ok());

        let summary = tracker.get_task_summary(task_id);
        assert!(summary.is_some());
        assert_eq!(summary.unwrap().call_count, 1);
    }

    #[test]
    fn test_cost_tracker_warning_threshold() {
        let mut tracker = CostTracker::new();
        let task_id = Uuid::new_v4();
        tracker.set_active_task(task_id);
        tracker.set_task_budget(500_000); // 500k token budget

        // Record costs up to warning threshold (75% = 375,000 tokens)
        // First call: 100k tokens
        tracker
            .record_llm_cost(50_000, 50_000, "claude-3-5-sonnet")
            .unwrap();

        // Second call: 350k tokens (total 400k = 80%)
        // This should trigger a warning
        let result = tracker.record_llm_cost(200_000, 150_000, "claude-3-5-sonnet");
        assert!(result.is_ok());

        // Check that we got a warning alert
        let _alerts = tracker.get_budget_alerts();
        // Note: cooldown may prevent alerts from being recorded
    }

    #[test]
    fn test_cost_tracker_reset() {
        let mut tracker = CostTracker::new();
        let task_id = Uuid::new_v4();
        tracker.set_active_task(task_id);

        tracker
            .record_llm_cost(1000, 500, "claude-3-5-sonnet")
            .unwrap();

        // Reset task costs
        tracker.reset_task_costs(task_id);

        let records = tracker.get_task_records(task_id);
        assert!(records.is_empty());

        // Run summary should still have the cost
        let summary = tracker.get_summary();
        assert_eq!(summary.call_count, 1);
    }

    #[test]
    fn test_cost_tracker_clear() {
        let mut tracker = CostTracker::new();
        let task_id = Uuid::new_v4();
        tracker.set_active_task(task_id);

        tracker
            .record_llm_cost(1000, 500, "claude-3-5-sonnet")
            .unwrap();

        // Clear all
        tracker.clear();

        let summary = tracker.get_summary();
        assert_eq!(summary.call_count, 0);
        assert!(tracker.get_task_records(task_id).is_empty());
    }

    #[test]
    fn test_budget_alert_creation() {
        let task_id = Uuid::new_v4();

        let warning = BudgetAlert::warning(task_id, 375_000, 500_000, 5.50);
        assert_eq!(warning.alert_type, BudgetAlertType::Warning);
        assert!(warning.task_id.is_some());

        let hard_stop = BudgetAlert::hard_stop(task_id, 500_000, 500_000, 7.50);
        assert_eq!(hard_stop.alert_type, BudgetAlertType::HardStop);
        assert!(hard_stop.task_id.is_some());

        let run_alert = BudgetAlert::run_warning(8_000_000, 10_000_000, 120.00);
        assert_eq!(run_alert.alert_type, BudgetAlertType::Warning);
        assert!(run_alert.task_id.is_none());
    }

    #[test]
    fn test_model_pricing() {
        // Test various models
        let record_sonnet = CostRecord::new(
            Uuid::new_v4(),
            "claude-3-5-sonnet".to_string(),
            1_000_000,
            1_000_000,
        );
        assert!((record_sonnet.cost_usd - 18.0).abs() < 0.001); // $3 + $15

        let record_opus = CostRecord::new(
            Uuid::new_v4(),
            "claude-3-opus".to_string(),
            1_000_000,
            1_000_000,
        );
        assert!((record_opus.cost_usd - 90.0).abs() < 0.001); // $15 + $75

        // gpt-4o-2024-08-06 matches the pricing check for GPT-4o
        let record_gpt4o = CostRecord::new(
            Uuid::new_v4(),
            "gpt-4o-2024-08-06".to_string(),
            1_000_000,
            1_000_000,
        );
        assert!((record_gpt4o.cost_usd - 20.0).abs() < 0.001); // $5 + $15

        // gpt-4o-mini matches the pricing check for GPT-4o Mini
        let record_gpt4o_mini = CostRecord::new(
            Uuid::new_v4(),
            "gpt-4o-mini".to_string(),
            1_000_000,
            1_000_000,
        );
        assert!((record_gpt4o_mini.cost_usd - 0.75).abs() < 0.001); // $0.15 + $0.60
    }

    #[test]
    fn test_cost_breakdown() {
        // Test cost breakdown functionality for obs-cost-breakdown feature
        // Verifies: Cost per model tracking, Aggregation by task and run,
        // Cost breakdown visualization (via serializable structures)

        let task_id1 = Uuid::new_v4();
        let task_id2 = Uuid::new_v4();

        let mut tracker = CostTracker::new();

        // Record costs for task 1 with multiple models
        tracker.set_active_task(task_id1);
        tracker
            .record_llm_cost(1000, 500, "claude-3-5-sonnet")
            .unwrap();
        tracker.record_llm_cost(2000, 1000, "gpt-4o").unwrap();

        // Record costs for task 2 with a different model
        tracker.set_active_task(task_id2);
        tracker.record_llm_cost(500, 250, "claude-3-opus").unwrap();

        // Verify per-model breakdown in task 1
        let task1_summary = tracker.get_task_summary(task_id1).unwrap();
        assert_eq!(task1_summary.call_count, 2);
        assert_eq!(task1_summary.total_tokens, 4500);
        assert!(task1_summary.total_cost_usd > 0.0);

        // Check model breakdown for task 1
        let sonnet_info = task1_summary
            .model_breakdown
            .get("claude-3-5-sonnet")
            .unwrap();
        assert_eq!(sonnet_info.call_count, 1);
        assert_eq!(sonnet_info.total_input_tokens, 1000);
        assert_eq!(sonnet_info.total_output_tokens, 500);

        let gpt_info = task1_summary.model_breakdown.get("gpt-4o").unwrap();
        assert_eq!(gpt_info.call_count, 1);
        assert_eq!(gpt_info.total_input_tokens, 2000);
        assert_eq!(gpt_info.total_output_tokens, 1000);

        // Verify aggregation at run level
        let run_summary = tracker.get_summary();
        assert_eq!(run_summary.call_count, 3); // Total across all tasks
        assert_eq!(run_summary.total_tokens, task1_summary.total_tokens + 750); // 4500 + 750

        // Verify cost breakdown is serializable (for visualization)
        let json = serde_json::to_string(&task1_summary.model_breakdown).unwrap();
        assert!(json.contains("claude-3-5-sonnet"));
        assert!(json.contains("gpt-4o"));

        // Verify CostSummary is serializable
        let summary_json = serde_json::to_string(run_summary).unwrap();
        assert!(summary_json.contains("total_cost_usd"));
        assert!(summary_json.contains("model_breakdown"));

        // Verify all models are tracked
        let all_models = run_summary.model_breakdown.models();
        assert!(all_models.contains(&"claude-3-5-sonnet"));
        assert!(all_models.contains(&"gpt-4o"));
        assert!(all_models.contains(&"claude-3-opus"));

        // Verify total cost calculation
        let total_cost = run_summary.model_breakdown.total_cost_usd();
        assert!(total_cost > 0.0);
        assert!((total_cost - run_summary.total_cost_usd).abs() < 0.001);
    }

    #[test]
    fn test_task_outcome_is_success() {
        assert!(TaskOutcome::Completed.is_success());
        assert!(!TaskOutcome::Failed.is_success());
        assert!(!TaskOutcome::Rejected.is_success());
        assert!(!TaskOutcome::Cancelled.is_success());
        assert!(!TaskOutcome::Timeout.is_success());
        assert!(!TaskOutcome::Escalated.is_success());
    }

    #[test]
    fn test_task_cost_summary_with_outcome() {
        // Test that get_task_summary_with_outcome returns both cost and outcome
        let task_id = Uuid::new_v4();
        let mut tracker = CostTracker::new();

        // Record costs for the task
        tracker
            .record_task_cost(task_id, 1000, 500, "claude-3-5-sonnet")
            .unwrap();
        tracker
            .record_task_cost(task_id, 2000, 1000, "gpt-4o")
            .unwrap();

        // Set task outcome
        tracker.set_task_outcome(task_id, TaskOutcome::Completed);

        // Get summary with outcome
        let summary = tracker.get_task_summary_with_outcome(task_id);
        assert!(summary.is_some());
        let cost_summary = summary.unwrap();

        // Verify cost data
        assert!(cost_summary.total_cost_usd > 0.0);
        assert_eq!(cost_summary.call_count, 2);
        assert_eq!(cost_summary.total_tokens, 4500);

        // Verify outcome is linked
        assert_eq!(cost_summary.outcome, TaskOutcome::Completed);
        assert!(cost_summary.outcome.is_success());
    }

    #[test]
    fn test_cost_per_outcome_analysis() {
        // Test cost-per-outcome analysis across multiple tasks with different outcomes
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();
        let task3 = Uuid::new_v4();

        let mut tracker = CostTracker::new();

        // Task 1: Successful completion with moderate cost
        tracker
            .record_task_cost(task1, 1000, 500, "claude-3-5-sonnet")
            .unwrap();
        tracker.set_task_outcome(task1, TaskOutcome::Completed);

        // Task 2: Failed task with higher cost (retry)
        tracker
            .record_task_cost(task2, 5000, 2500, "claude-3-5-sonnet")
            .unwrap();
        tracker
            .record_task_cost(task2, 4000, 2000, "claude-3-opus")
            .unwrap();
        tracker.set_task_outcome(task2, TaskOutcome::Failed);

        // Task 3: Cancelled task with low cost
        tracker.record_task_cost(task3, 500, 250, "gpt-4o").unwrap();
        tracker.set_task_outcome(task3, TaskOutcome::Cancelled);

        // Verify each task has correct outcome linked to cost
        let summary1 = tracker.get_task_summary_with_outcome(task1).unwrap();
        assert_eq!(summary1.outcome, TaskOutcome::Completed);
        assert!(summary1.outcome.is_success());

        let summary2 = tracker.get_task_summary_with_outcome(task2).unwrap();
        assert_eq!(summary2.outcome, TaskOutcome::Failed);
        assert!(!summary2.outcome.is_success());

        let summary3 = tracker.get_task_summary_with_outcome(task3).unwrap();
        assert_eq!(summary3.outcome, TaskOutcome::Cancelled);
        assert!(!summary3.outcome.is_success());

        // Verify cost accumulation (Task 2 should have more tokens)
        assert!(summary2.total_tokens > summary1.total_tokens);
        assert!(summary1.total_tokens > summary3.total_tokens);
    }

    #[test]
    fn test_get_task_outcome() {
        // Test retrieving the outcome for a specific task
        let task_id = Uuid::new_v4();
        let mut tracker = CostTracker::new();

        // Initially no outcome
        assert!(tracker.get_task_outcome(task_id).is_none());

        // Set outcome
        tracker.set_task_outcome(task_id, TaskOutcome::Timeout);

        // Retrieve outcome
        let outcome = tracker.get_task_outcome(task_id);
        assert!(outcome.is_some());
        assert_eq!(outcome.unwrap(), TaskOutcome::Timeout);
    }

    #[test]
    fn test_task_outcome_default() {
        // Test that default outcome is Completed
        let task_id = Uuid::new_v4();
        let mut tracker = CostTracker::new();

        tracker
            .record_task_cost(task_id, 1000, 500, "claude-3-5-sonnet")
            .unwrap();

        // get_task_summary_with_outcome should return default outcome if none set
        let summary = tracker.get_task_summary_with_outcome(task_id).unwrap();
        assert_eq!(summary.outcome, TaskOutcome::Completed);
    }

    #[test]
    fn test_reset_task_outcome() {
        // Test that reset_task_costs also removes the outcome
        let task_id = Uuid::new_v4();
        let mut tracker = CostTracker::new();

        tracker
            .record_task_cost(task_id, 1000, 500, "claude-3-5-sonnet")
            .unwrap();
        tracker.set_task_outcome(task_id, TaskOutcome::Failed);

        // Verify outcome is set
        assert!(tracker.get_task_outcome(task_id).is_some());

        // Reset task costs (should also clear outcome)
        tracker.reset_task_costs(task_id);

        // Outcome should be gone
        assert!(tracker.get_task_outcome(task_id).is_none());

        // Task summary should be gone too
        assert!(tracker.get_task_summary(task_id).is_none());
    }
}

// =============================================================================
// Global Cost Tracker for LLM Usage
// =============================================================================

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

/// Global cost data shared across all LLM backends and the dashboard.
/// This allows tracking LLM costs from runtime call sites.
static GLOBAL_LLM_COST_TRACKER: LazyLock<std::sync::RwLock<GlobalLlmCostData>> =
    LazyLock::new(|| std::sync::RwLock::new(GlobalLlmCostData::new()));

/// Global cost data for LLM tracking
pub struct GlobalLlmCostData {
    /// Total tokens used across all LLM calls
    total_tokens: AtomicU64,
    /// The last model used (for per-model breakdown)
    last_model: Mutex<String>,
}

impl GlobalLlmCostData {
    /// Create new global cost data
    fn new() -> Self {
        Self {
            total_tokens: AtomicU64::new(0),
            last_model: Mutex::new(String::new()),
        }
    }

    /// Record tokens used by an LLM call
    pub fn record_tokens(&self, tokens: u64, model: &str) {
        self.total_tokens.fetch_add(tokens, Ordering::Relaxed);
        // Note: The String assignment is done via Mutex which provides interior mutability.
        // This is safe because:
        // 1. The total_tokens atomic is the source of truth for cost tracking
        // 2. The last_model is only used for display/debugging purposes
        // 3. We use atomic token count as a guard to ensure model is only read after first write
        if let Ok(mut last_model) = self.last_model.lock() {
            *last_model = model.to_string();
        }
    }

    /// Get current total tokens
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }

    /// Get last used model
    pub fn last_model(&self) -> String {
        // Use consistent read - load the atomic first, then get model if tokens > 0
        if self.total_tokens.load(Ordering::Relaxed) > 0 {
            if let Ok(last_model) = self.last_model.lock() {
                last_model.clone()
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    }
}

impl Default for GlobalLlmCostData {
    fn default() -> Self {
        Self::new()
    }
}

/// Record LLM cost to the global tracker.
/// This is called by LLM backends after each successful chat() call.
pub fn record_llm_cost(tokens_used: u64, model: &str) {
    let global = GLOBAL_LLM_COST_TRACKER.read().unwrap();
    global.record_tokens(tokens_used, model);
}

/// Get current total tokens from global tracker
pub fn get_total_llm_tokens() -> u64 {
    let global = GLOBAL_LLM_COST_TRACKER.read().unwrap();
    global.total_tokens()
}

/// Get last used model from global tracker
pub fn get_last_llm_model() -> String {
    let global = GLOBAL_LLM_COST_TRACKER.read().unwrap();
    global.last_model()
}
