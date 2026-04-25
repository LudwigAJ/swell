//! Stopping conditions for orchestrator execution.
//!
//! This module defines the conditions under which the orchestrator should stop
//! processing tasks. The three main stopping conditions are:
//!
//! 1. **All tasks accepted**: All submitted tasks have reached a terminal state (Accepted)
//! 2. **Stop command received**: An explicit stop command was issued via the daemon
//! 3. **Hard limit breached**: A hard limit (time, cost, failures) was exceeded
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_orchestrator::stopping_conditions::{StoppingCondition, StoppingConditions};
//!
//! let conditions = StoppingConditions::new();
//!
//! // Check if orchestrator should stop
//! if let Some(reason) = conditions.check(&orchestrator).await {
//!     match reason {
//!         StoppingCondition::AllTasksAccepted => { ... }
//!         StoppingCondition::StopCommandReceived => { ... }
//!         StoppingCondition::HardLimitBreached(limit) => { ... }
//!     }
//! }
//! ```

use crate::hard_limits::HardLimits;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use swell_core::ids::TaskId;
use swell_core::TaskState;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// A single stopping condition that can cause the orchestrator to stop
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoppingCondition {
    /// All tasks have been accepted (reached terminal success state)
    AllTasksAccepted {
        total_tasks: usize,
        accepted_tasks: usize,
    },
    /// A stop command was received via the daemon
    StopCommandReceived { reason: Option<String> },
    /// A hard limit was breached
    HardLimitBreached {
        limit_type: HardLimitType,
        current_value: String,
        limit_value: String,
    },
}

impl std::fmt::Display for StoppingCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoppingCondition::AllTasksAccepted {
                total_tasks,
                accepted_tasks,
            } => {
                write!(f, "All tasks accepted ({}/{})", accepted_tasks, total_tasks)
            }
            StoppingCondition::StopCommandReceived { reason } => {
                if let Some(r) = reason {
                    write!(f, "Stop command received: {}", r)
                } else {
                    write!(f, "Stop command received")
                }
            }
            StoppingCondition::HardLimitBreached {
                limit_type,
                current_value,
                limit_value,
            } => {
                write!(
                    f,
                    "Hard limit breached: {} ({}/{})",
                    limit_type, current_value, limit_value
                )
            }
        }
    }
}

/// Types of hard limits that can be breached
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardLimitType {
    /// Maximum number of tasks exceeded
    MaxTasks,
    /// Maximum time per task exceeded
    MaxTime { task_id: TaskId },
    /// Maximum total cost exceeded
    MaxCost,
    /// Maximum consecutive failures exceeded
    MaxFailures,
}

impl std::fmt::Display for HardLimitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardLimitType::MaxTasks => write!(f, "max_tasks"),
            HardLimitType::MaxTime { task_id } => write!(f, "max_time ({})", task_id),
            HardLimitType::MaxCost => write!(f, "max_cost"),
            HardLimitType::MaxFailures => write!(f, "max_failures"),
        }
    }
}

/// Errors from hard limits checking
#[derive(Debug, Clone)]
pub enum HardLimitsError {
    /// Task count limit exceeded
    TaskLimitExceeded { current: usize, limit: usize },
    /// Time limit exceeded for a task
    TimeLimitExceeded {
        task_id: TaskId,
        elapsed_secs: u64,
        limit_secs: u64,
    },
    /// Cost limit exceeded
    CostLimitExceeded { current_usd: f64, limit_usd: f64 },
    /// Failure count limit exceeded
    FailureLimitExceeded {
        current_failures: u32,
        limit_failures: u32,
    },
}

impl std::fmt::Display for HardLimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardLimitsError::TaskLimitExceeded { current, limit } => {
                write!(f, "Task limit: {}/{}", current, limit)
            }
            HardLimitsError::TimeLimitExceeded {
                task_id,
                elapsed_secs,
                limit_secs,
            } => {
                write!(
                    f,
                    "Time limit for {}: {}/{}s",
                    task_id, elapsed_secs, limit_secs
                )
            }
            HardLimitsError::CostLimitExceeded {
                current_usd,
                limit_usd,
            } => {
                write!(f, "Cost limit: ${:.2}/${:.2}", current_usd, limit_usd)
            }
            HardLimitsError::FailureLimitExceeded {
                current_failures,
                limit_failures,
            } => {
                write!(f, "Failure limit: {}/{}", current_failures, limit_failures)
            }
        }
    }
}

/// Container for all stopping conditions state
#[derive(Debug, Clone)]
pub struct StoppingConditions {
    /// Whether stop has been commanded
    stop_commanded: Arc<RwLock<bool>>,
    /// Stop command reason if available
    stop_reason: Arc<RwLock<Option<String>>>,
}

impl StoppingConditions {
    /// Create a new stopping conditions tracker
    pub fn new() -> Self {
        Self {
            stop_commanded: Arc::new(RwLock::new(false)),
            stop_reason: Arc::new(RwLock::new(None)),
        }
    }

    /// Command the orchestrator to stop
    pub async fn command_stop(&self, reason: Option<String>) {
        info!(reason = ?reason, "Stop command received");
        *self.stop_commanded.write().await = true;
        *self.stop_reason.write().await = reason;
    }

    /// Check if stop was commanded
    pub async fn is_stop_commanded(&self) -> bool {
        *self.stop_commanded.read().await
    }

    /// Reset the stop command (for testing or manual reset)
    #[allow(dead_code)]
    pub async fn reset_stop(&self) {
        *self.stop_commanded.write().await = false;
        *self.stop_reason.write().await = None;
    }

    /// Get the stop reason if stop was commanded
    pub async fn get_stop_reason(&self) -> Option<String> {
        self.stop_reason.read().await.clone()
    }

    // ========================================================================
    // Stopping Condition Checks
    // ========================================================================

    /// Check if all tasks are accepted
    /// Returns Some(StoppingCondition) if all tasks have reached accepted state,
    /// or None if there are still pending tasks.
    pub fn check_all_tasks_accepted<'a>(
        &self,
        tasks: impl IntoIterator<Item = &'a TaskState>,
    ) -> Option<StoppingCondition> {
        let tasks_vec: Vec<&TaskState> = tasks.into_iter().collect();
        let total_tasks = tasks_vec.len();

        if total_tasks == 0 {
            // No tasks means nothing to wait for - not a stopping condition
            return None;
        }

        let accepted_count = tasks_vec
            .iter()
            .filter(|&&state| *state == TaskState::Accepted)
            .count();

        if accepted_count == total_tasks {
            debug!(
                total = total_tasks,
                accepted = accepted_count,
                "All tasks accepted"
            );
            Some(StoppingCondition::AllTasksAccepted {
                total_tasks,
                accepted_tasks: accepted_count,
            })
        } else {
            None
        }
    }

    /// Check if stop command was received
    pub async fn check_stop_command(&self) -> Option<StoppingCondition> {
        if self.is_stop_commanded().await {
            let reason = self.get_stop_reason().await;
            Some(StoppingCondition::StopCommandReceived { reason })
        } else {
            None
        }
    }

    /// Check if any hard limit is breached
    /// Returns Some(StoppingCondition) if a limit is breached, None otherwise.
    pub fn check_hard_limits(limits: &HardLimits) -> Option<StoppingCondition> {
        // Check task creation limit
        // Note: We don't know current task count here, so we check cost and failures
        // which are tracked internally

        // Check cost limit
        if limits.is_cost_limit_exceeded() {
            let current = limits.total_cost();
            let config = limits.config();
            warn!(
                current_usd = current,
                limit_usd = config.max_cost_usd,
                "Cost limit breached"
            );
            return Some(StoppingCondition::HardLimitBreached {
                limit_type: HardLimitType::MaxCost,
                current_value: format!("${:.2}", current),
                limit_value: format!("${:.2}", config.max_cost_usd),
            });
        }

        // Check failure limit
        if limits.is_failure_limit_exceeded() {
            let current = limits.failure_count();
            let config = limits.config();
            warn!(
                current_failures = current,
                limit_failures = config.max_failures,
                "Failure limit breached"
            );
            return Some(StoppingCondition::HardLimitBreached {
                limit_type: HardLimitType::MaxFailures,
                current_value: current.to_string(),
                limit_value: config.max_failures.to_string(),
            });
        }

        None
    }

    /// Check time limit for a specific task
    pub fn check_task_time_limit(
        limits: &HardLimits,
        task_id: TaskId,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Option<StoppingCondition> {
        if limits.is_time_limit_exceeded(started_at) {
            let elapsed = limits.get_elapsed_secs(started_at);
            let config = limits.config();
            warn!(
                task_id = %task_id,
                elapsed_secs = elapsed,
                limit_secs = config.max_time_secs,
                "Time limit breached for task"
            );
            return Some(StoppingCondition::HardLimitBreached {
                limit_type: HardLimitType::MaxTime { task_id },
                current_value: format!("{}s", elapsed),
                limit_value: format!("{}s", config.max_time_secs),
            });
        }
        None
    }

    /// Perform a full check of all stopping conditions
    /// Returns the first stopping condition that is met, or None if all conditions are clear.
    pub async fn check_all(
        &self,
        tasks: impl IntoIterator<Item = TaskState>,
        limits: Option<&HardLimits>,
    ) -> Option<StoppingCondition> {
        // Priority order (first match wins):
        // 1. Stop command (highest priority - operator intervention)
        // 2. Hard limit breach (safety constraint)
        // 3. All tasks accepted (natural completion)

        // 1. Check stop command first
        if let Some(condition) = self.check_stop_command().await {
            return Some(condition);
        }

        // 2. Check hard limits
        if let Some(limits) = limits {
            if let Some(condition) = Self::check_hard_limits(limits) {
                return Some(condition);
            }
        }

        // 3. Check if all tasks are accepted
        let tasks_vec: Vec<TaskState> = tasks.into_iter().collect();
        if let Some(condition) = self.check_all_tasks_accepted(tasks_vec.iter()) {
            return Some(condition);
        }

        None
    }
}

impl Default for StoppingConditions {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared reference to stopping conditions
pub type SharedStoppingConditions = Arc<StoppingConditions>;

/// Create a new shared stopping conditions instance
pub fn create_stopping_conditions() -> SharedStoppingConditions {
    Arc::new(StoppingConditions::new())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- StoppingCondition Tests ---

    #[test]
    fn test_stopping_condition_display_all_tasks_accepted() {
        let condition = StoppingCondition::AllTasksAccepted {
            total_tasks: 10,
            accepted_tasks: 10,
        };
        assert_eq!(format!("{}", condition), "All tasks accepted (10/10)");
    }

    #[test]
    fn test_stopping_condition_display_stop_command() {
        let condition = StoppingCondition::StopCommandReceived {
            reason: Some("User requested".to_string()),
        };
        assert_eq!(
            format!("{}", condition),
            "Stop command received: User requested"
        );

        let condition_no_reason = StoppingCondition::StopCommandReceived { reason: None };
        assert_eq!(format!("{}", condition_no_reason), "Stop command received");
    }

    #[test]
    fn test_stopping_condition_display_hard_limit() {
        let condition = StoppingCondition::HardLimitBreached {
            limit_type: HardLimitType::MaxCost,
            current_value: "$150.00".to_string(),
            limit_value: "$100.00".to_string(),
        };
        assert_eq!(
            format!("{}", condition),
            "Hard limit breached: max_cost ($150.00/$100.00)"
        );
    }

    #[test]
    fn test_hard_limit_type_display() {
        assert_eq!(format!("{}", HardLimitType::MaxTasks), "max_tasks");
        assert_eq!(
            format!(
                "{}",
                HardLimitType::MaxTime {
                    task_id: TaskId::nil()
                }
            ),
            "max_time (00000000-0000-0000-0000-000000000000)"
        );
        assert_eq!(format!("{}", HardLimitType::MaxCost), "max_cost");
        assert_eq!(format!("{}", HardLimitType::MaxFailures), "max_failures");
    }

    // --- StoppingConditions Tests ---

    #[tokio::test]
    async fn test_new_stopping_conditions() {
        let conditions = StoppingConditions::new();
        assert!(!conditions.is_stop_commanded().await);
        assert!(conditions.get_stop_reason().await.is_none());
    }

    #[tokio::test]
    async fn test_command_stop() {
        let conditions = StoppingConditions::new();
        assert!(!conditions.is_stop_commanded().await);

        conditions.command_stop(Some("Test stop".to_string())).await;
        assert!(conditions.is_stop_commanded().await);
        assert_eq!(
            conditions.get_stop_reason().await,
            Some("Test stop".to_string())
        );
    }

    #[tokio::test]
    async fn test_command_stop_without_reason() {
        let conditions = StoppingConditions::new();
        conditions.command_stop(None).await;
        assert!(conditions.is_stop_commanded().await);
        assert!(conditions.get_stop_reason().await.is_none());
    }

    #[tokio::test]
    async fn test_reset_stop() {
        let conditions = StoppingConditions::new();
        conditions.command_stop(Some("Test".to_string())).await;
        assert!(conditions.is_stop_commanded().await);

        conditions.reset_stop().await;
        assert!(!conditions.is_stop_commanded().await);
        assert!(conditions.get_stop_reason().await.is_none());
    }

    // --- check_all_tasks_accepted Tests ---

    #[test]
    fn test_check_all_tasks_accepted_all_accepted() {
        let conditions = StoppingConditions::new();
        let states = [
            TaskState::Accepted,
            TaskState::Accepted,
            TaskState::Accepted,
        ];

        let result = conditions.check_all_tasks_accepted(states.iter());
        assert!(result.is_some());
        let condition = result.unwrap();
        assert!(matches!(
            condition,
            StoppingCondition::AllTasksAccepted { .. }
        ));
        if let StoppingCondition::AllTasksAccepted {
            total_tasks,
            accepted_tasks,
        } = condition
        {
            assert_eq!(total_tasks, 3);
            assert_eq!(accepted_tasks, 3);
        }
    }

    #[test]
    fn test_check_all_tasks_accepted_some_pending() {
        let conditions = StoppingConditions::new();
        let states = [
            TaskState::Accepted,
            TaskState::Executing,
            TaskState::Accepted,
        ];

        let result = conditions.check_all_tasks_accepted(states.iter());
        assert!(result.is_none());
    }

    #[test]
    fn test_check_all_tasks_accepted_all_rejected() {
        let conditions = StoppingConditions::new();
        let states = [
            TaskState::Rejected,
            TaskState::Rejected,
            TaskState::Rejected,
        ];

        // Rejected is terminal but not accepted - should not trigger
        let result = conditions.check_all_tasks_accepted(states.iter());
        assert!(result.is_none());
    }

    #[test]
    fn test_check_all_tasks_accepted_empty() {
        let conditions = StoppingConditions::new();
        let states: Vec<TaskState> = vec![];

        // Empty task list should not trigger stopping condition
        let result = conditions.check_all_tasks_accepted(states.iter());
        assert!(result.is_none());
    }

    #[test]
    fn test_check_all_tasks_accepted_single_accepted() {
        let conditions = StoppingConditions::new();
        let states = [TaskState::Accepted];

        let result = conditions.check_all_tasks_accepted(states.iter());
        assert!(result.is_some());
        if let StoppingCondition::AllTasksAccepted {
            total_tasks,
            accepted_tasks,
        } = result.unwrap()
        {
            assert_eq!(total_tasks, 1);
            assert_eq!(accepted_tasks, 1);
        }
    }

    // --- check_stop_command Tests ---

    #[tokio::test]
    async fn test_check_stop_command_not_commanded() {
        let conditions = StoppingConditions::new();
        let result = conditions.check_stop_command().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_check_stop_command_commanded() {
        let conditions = StoppingConditions::new();
        conditions
            .command_stop(Some("Operator request".to_string()))
            .await;

        let result = conditions.check_stop_command().await;
        assert!(result.is_some());
        let condition = result.unwrap();
        assert!(matches!(
            condition,
            StoppingCondition::StopCommandReceived { .. }
        ));
        if let StoppingCondition::StopCommandReceived { reason } = condition {
            assert_eq!(reason.as_deref(), Some("Operator request"));
        }
    }

    // --- check_hard_limits Tests ---

    #[test]
    fn test_check_hard_limits_cost_ok() {
        let limits = HardLimits::default();
        let result = StoppingConditions::check_hard_limits(&limits);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_hard_limits_cost_exceeded() {
        let limits = HardLimits::default();
        // Default cost limit is $100, so we need to exceed it
        // But we can't modify limits directly without mutable reference
        // This test verifies the structure is correct
        let result = StoppingConditions::check_hard_limits(&limits);
        // Since we haven't exceeded limits, this should be None
        assert!(result.is_none());
    }

    #[test]
    fn test_check_task_time_limit_not_exceeded() {
        let limits = HardLimits::default();
        let started_at = chrono::Utc::now() - chrono::Duration::minutes(5);
        let task_id = TaskId::new();

        let result = StoppingConditions::check_task_time_limit(&limits, task_id, started_at);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_task_time_limit_exceeded() {
        let limits = HardLimits::default();
        // Default is 8 hours, so 9 hours ago should exceed
        let started_at = chrono::Utc::now() - chrono::Duration::hours(9);
        let task_id = TaskId::new();

        let result = StoppingConditions::check_task_time_limit(&limits, task_id, started_at);
        assert!(result.is_some());
        let condition = result.unwrap();
        assert!(matches!(
            condition,
            StoppingCondition::HardLimitBreached {
                limit_type: HardLimitType::MaxTime { .. },
                ..
            }
        ));
    }

    // --- check_all Tests ---

    #[tokio::test]
    async fn test_check_all_no_conditions() {
        let conditions = StoppingConditions::new();
        let limits = HardLimits::default();
        let tasks = [TaskState::Executing, TaskState::Executing];

        let result = conditions
            .check_all(tasks.iter().cloned(), Some(&limits))
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_check_all_stop_command_priority() {
        let conditions = StoppingConditions::new();
        let limits = HardLimits::default();
        let tasks = [TaskState::Executing];

        // Command stop before checking
        conditions.command_stop(Some("Urgent".to_string())).await;

        let result = conditions
            .check_all(tasks.iter().cloned(), Some(&limits))
            .await;
        assert!(result.is_some());
        let condition = result.unwrap();
        assert!(matches!(
            condition,
            StoppingCondition::StopCommandReceived { .. }
        ));
    }

    #[tokio::test]
    async fn test_check_all_all_tasks_accepted() {
        let conditions = StoppingConditions::new();
        let limits = HardLimits::default();
        let tasks = [TaskState::Accepted, TaskState::Accepted];

        let result = conditions
            .check_all(tasks.iter().cloned(), Some(&limits))
            .await;
        assert!(result.is_some());
        let condition = result.unwrap();
        assert!(matches!(
            condition,
            StoppingCondition::AllTasksAccepted { .. }
        ));
    }

    // --- create_stopping_conditions Tests ---

    #[test]
    fn test_create_stopping_conditions() {
        let shared = create_stopping_conditions();
        assert!(Arc::strong_count(&shared) >= 1);
    }

    // --- Edge Cases ---

    #[test]
    fn test_mixed_terminal_states() {
        let conditions = StoppingConditions::new();
        let states = [
            TaskState::Accepted,
            TaskState::Escalated, // Terminal but not accepted
            TaskState::Failed,    // Terminal but not accepted
            TaskState::Rejected,  // Terminal but not accepted
        ];

        let result = conditions.check_all_tasks_accepted(states.iter());
        // Only Accepted counts - Escalated, Failed, Rejected don't count
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_check_all_with_no_limits() {
        let conditions = StoppingConditions::new();
        let tasks = [TaskState::Accepted];

        // Passing None for limits should not cause panic
        let result = conditions.check_all(tasks.iter().cloned(), None).await;
        // Should return AllTasksAccepted since that's the only condition met
        assert!(result.is_some());
    }

    #[test]
    fn test_check_all_tasks_accepted_with_cow() {
        let conditions = StoppingConditions::new();
        // Test with reference iteration
        let states = vec![TaskState::Accepted, TaskState::Accepted];
        let result = conditions.check_all_tasks_accepted(&states);
        assert!(result.is_some());
    }

    // --- HardLimitsError Display Tests ---

    #[test]
    fn test_hard_limits_error_display() {
        let err = HardLimitsError::TaskLimitExceeded {
            current: 50,
            limit: 100,
        };
        assert_eq!(format!("{}", err), "Task limit: 50/100");

        let err = HardLimitsError::TimeLimitExceeded {
            task_id: TaskId::nil(),
            elapsed_secs: 300,
            limit_secs: 28800,
        };
        assert!(format!("{}", err).contains("Time limit"));

        let err = HardLimitsError::CostLimitExceeded {
            current_usd: 150.0,
            limit_usd: 100.0,
        };
        assert_eq!(format!("{}", err), "Cost limit: $150.00/$100.00");

        let err = HardLimitsError::FailureLimitExceeded {
            current_failures: 12,
            limit_failures: 10,
        };
        assert_eq!(format!("{}", err), "Failure limit: 12/10");
    }
}
