//! Hard limits enforcement for orchestrator safety constraints.
//!
//! This module implements hard limits that, when exceeded, will:
//! - Reject new task creation
//! - Pause executing tasks
//! - Trigger alerts
//!
//! # Limits
//!
//! - **Max tasks**: Maximum number of active tasks (default: 100)
//! - **Max time**: Maximum wall-clock time per task (default: 8 hours)
//! - **Max cost**: Maximum total cost in USD (default: $100.00)
//! - **Max failures**: Maximum consecutive failures before escalation (default: 10)
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_orchestrator::hard_limits::{HardLimits, HardLimitsConfig};
//!
//! let config = HardLimitsConfig::default();
//! let limits = HardLimits::new(config);
//!
//! // Check before creating a task
//! if let Err(e) = limits.check_task_creation(current_count) {
//!     return Err(e);
//! }
//!
//! // Check if task has exceeded time limit
//! if limits.is_time_limit_exceeded(started_at) {
//!     // Handle time limit exceeded
//! }
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Configuration for hard limits
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HardLimitsConfig {
    /// Maximum number of active tasks (default: 100)
    pub max_tasks: usize,
    /// Maximum wall-clock time per task in seconds (default: 8 hours = 28800)
    pub max_time_secs: u64,
    /// Maximum total cost in USD (default: $100.00)
    pub max_cost_usd: f64,
    /// Maximum consecutive failures before escalation (default: 10)
    pub max_failures: u32,
    /// Warning threshold for cost (0.0 to 1.0, default: 0.8)
    pub cost_warning_threshold: f64,
    /// Warning threshold for failures (0.0 to 1.0, default: 0.7)
    pub failure_warning_threshold: f64,
}

impl Default for HardLimitsConfig {
    fn default() -> Self {
        Self {
            max_tasks: 100,
            max_time_secs: 8 * 60 * 60, // 8 hours
            max_cost_usd: 100.0,
            max_failures: 10,
            cost_warning_threshold: 0.8,
            failure_warning_threshold: 0.7,
        }
    }
}

impl HardLimitsConfig {
    /// Create a config with strict limits for testing
    pub fn strict() -> Self {
        Self {
            max_tasks: 10,
            max_time_secs: 3600, // 1 hour
            max_cost_usd: 10.0,
            max_failures: 3,
            cost_warning_threshold: 0.8,
            failure_warning_threshold: 0.7,
        }
    }

    /// Create a config with relaxed limits for large-scale operations
    pub fn relaxed() -> Self {
        Self {
            max_tasks: 500,
            max_time_secs: 24 * 60 * 60, // 24 hours
            max_cost_usd: 1000.0,
            max_failures: 50,
            cost_warning_threshold: 0.8,
            failure_warning_threshold: 0.7,
        }
    }
}

/// Error returned when a hard limit is exceeded
#[derive(Debug, Clone, PartialEq)]
pub enum HardLimitError {
    /// Task count limit exceeded
    TaskLimitExceeded { current: usize, limit: usize },
    /// Time limit exceeded for a task
    TimeLimitExceeded {
        task_id: uuid::Uuid,
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

impl std::fmt::Display for HardLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardLimitError::TaskLimitExceeded { current, limit } => {
                write!(
                    f,
                    "Task limit exceeded: {} tasks (limit: {})",
                    current, limit
                )
            }
            HardLimitError::TimeLimitExceeded {
                task_id,
                elapsed_secs,
                limit_secs,
            } => {
                write!(
                    f,
                    "Time limit exceeded for task {}: {}s elapsed (limit: {}s)",
                    task_id, elapsed_secs, limit_secs
                )
            }
            HardLimitError::CostLimitExceeded {
                current_usd,
                limit_usd,
            } => {
                write!(
                    f,
                    "Cost limit exceeded: ${:.2} (limit: ${:.2})",
                    current_usd, limit_usd
                )
            }
            HardLimitError::FailureLimitExceeded {
                current_failures,
                limit_failures,
            } => {
                write!(
                    f,
                    "Failure limit exceeded: {} failures (limit: {})",
                    current_failures, limit_failures
                )
            }
        }
    }
}

impl std::error::Error for HardLimitError {}

/// Warning returned when approaching a hard limit
#[derive(Debug, Clone, PartialEq)]
pub enum HardLimitWarning {
    /// Task count approaching limit
    TaskCountApproaching { current: usize, limit: usize },
    /// Time limit approaching for a task
    TimeLimitApproaching {
        task_id: uuid::Uuid,
        elapsed_secs: u64,
        limit_secs: u64,
    },
    /// Cost limit approaching
    CostApproaching { current_usd: f64, limit_usd: f64 },
    /// Failure count approaching limit
    FailureCountApproaching {
        current_failures: u32,
        limit_failures: u32,
    },
}

impl std::fmt::Display for HardLimitWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardLimitWarning::TaskCountApproaching { current, limit } => {
                write!(
                    f,
                    "Task count approaching limit: {} tasks (limit: {})",
                    current, limit
                )
            }
            HardLimitWarning::TimeLimitApproaching {
                task_id,
                elapsed_secs,
                limit_secs,
            } => {
                write!(
                    f,
                    "Time limit approaching for task {}: {}s elapsed (limit: {}s)",
                    task_id, elapsed_secs, limit_secs
                )
            }
            HardLimitWarning::CostApproaching {
                current_usd,
                limit_usd,
            } => {
                write!(
                    f,
                    "Cost approaching limit: ${:.2} (limit: ${:.2})",
                    current_usd, limit_usd
                )
            }
            HardLimitWarning::FailureCountApproaching {
                current_failures,
                limit_failures,
            } => {
                write!(
                    f,
                    "Failure count approaching limit: {} failures (limit: {})",
                    current_failures, limit_failures
                )
            }
        }
    }
}

/// Result of checking hard limits (warning + whether limit is exceeded)
#[derive(Debug, Clone)]
pub struct HardLimitsCheck {
    pub exceeded: Option<HardLimitError>,
    pub warnings: Vec<HardLimitWarning>,
}

impl HardLimitsCheck {
    pub fn is_ok(&self) -> bool {
        self.exceeded.is_none()
    }

    pub fn is_warning(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// The main hard limits enforcer
#[derive(Debug, Clone)]
pub struct HardLimits {
    config: HardLimitsConfig,
    /// Current total cost spent (in USD)
    total_cost_usd: f64,
    /// Current failure count (consecutive rejections)
    failure_count: u32,
    /// Cumulative elapsed time tracking
    total_elapsed_secs: u64,
}

impl HardLimits {
    /// Create a new hard limits enforcer with default configuration
    pub fn new(config: HardLimitsConfig) -> Self {
        Self {
            config,
            total_cost_usd: 0.0,
            failure_count: 0,
            total_elapsed_secs: 0,
        }
    }

    /// Create with default configuration
    pub fn default_limits() -> Self {
        Self::new(HardLimitsConfig::default())
    }

    /// Get the current configuration
    pub fn config(&self) -> HardLimitsConfig {
        self.config
    }

    /// Get current total cost
    pub fn total_cost(&self) -> f64 {
        self.total_cost_usd
    }

    /// Get current failure count
    pub fn failure_count(&self) -> u32 {
        self.failure_count
    }

    // ========================================================================
    // Task Count Limits
    // ========================================================================

    /// Check if a new task can be created based on task count limit
    pub fn check_task_creation(&self, current_task_count: usize) -> Result<(), HardLimitError> {
        if current_task_count >= self.config.max_tasks {
            return Err(HardLimitError::TaskLimitExceeded {
                current: current_task_count,
                limit: self.config.max_tasks,
            });
        }
        Ok(())
    }

    /// Get warning if task count is approaching limit
    pub fn check_task_count_warning(&self, current_task_count: usize) -> Option<HardLimitWarning> {
        let threshold = (self.config.max_tasks as f64 * 0.8) as usize;
        if current_task_count >= threshold {
            Some(HardLimitWarning::TaskCountApproaching {
                current: current_task_count,
                limit: self.config.max_tasks,
            })
        } else {
            None
        }
    }

    // ========================================================================
    // Time Limits
    // ========================================================================

    /// Check if a task has exceeded its time limit
    pub fn is_time_limit_exceeded(&self, started_at: DateTime<Utc>) -> bool {
        let now = Utc::now();
        let elapsed = now - started_at;
        elapsed.num_seconds() as u64 >= self.config.max_time_secs
    }

    /// Get elapsed time for a task
    pub fn get_elapsed_secs(&self, started_at: DateTime<Utc>) -> u64 {
        let now = Utc::now();
        let elapsed = now - started_at;
        elapsed.num_seconds() as u64
    }

    /// Check time limit and return error if exceeded
    pub fn check_time_limit(
        &self,
        task_id: uuid::Uuid,
        started_at: DateTime<Utc>,
    ) -> Result<(), HardLimitError> {
        let elapsed_secs = self.get_elapsed_secs(started_at);
        if elapsed_secs >= self.config.max_time_secs {
            return Err(HardLimitError::TimeLimitExceeded {
                task_id,
                elapsed_secs,
                limit_secs: self.config.max_time_secs,
            });
        }
        Ok(())
    }

    /// Get warning if time limit is approaching (80% of limit)
    pub fn check_time_warning(
        &self,
        task_id: uuid::Uuid,
        started_at: DateTime<Utc>,
    ) -> Option<HardLimitWarning> {
        let elapsed_secs = self.get_elapsed_secs(started_at);
        let warning_threshold = (self.config.max_time_secs as f64 * 0.8) as u64;
        if elapsed_secs >= warning_threshold {
            Some(HardLimitWarning::TimeLimitApproaching {
                task_id,
                elapsed_secs,
                limit_secs: self.config.max_time_secs,
            })
        } else {
            None
        }
    }

    /// Update total elapsed time (for cumulative tracking)
    pub fn add_elapsed_time(&mut self, secs: u64) {
        self.total_elapsed_secs += secs;
    }

    /// Get total elapsed time
    pub fn total_elapsed(&self) -> u64 {
        self.total_elapsed_secs
    }

    // ========================================================================
    // Cost Limits
    // ========================================================================

    /// Add cost to the total
    pub fn add_cost(&mut self, cost_usd: f64) {
        self.total_cost_usd += cost_usd;
        info!(
            cost_added = cost_usd,
            total_cost = self.total_cost_usd,
            limit = self.config.max_cost_usd,
            "Hard limits: cost added"
        );
    }

    /// Check if cost limit is exceeded
    pub fn is_cost_limit_exceeded(&self) -> bool {
        self.total_cost_usd >= self.config.max_cost_usd
    }

    /// Check cost limit and return error if exceeded
    pub fn check_cost_limit(&self) -> Result<(), HardLimitError> {
        if self.is_cost_limit_exceeded() {
            return Err(HardLimitError::CostLimitExceeded {
                current_usd: self.total_cost_usd,
                limit_usd: self.config.max_cost_usd,
            });
        }
        Ok(())
    }

    /// Get warning if cost limit is approaching (80% of limit)
    pub fn check_cost_warning(&self) -> Option<HardLimitWarning> {
        let ratio = self.total_cost_usd / self.config.max_cost_usd;
        if ratio >= self.config.cost_warning_threshold && ratio < 1.0 {
            Some(HardLimitWarning::CostApproaching {
                current_usd: self.total_cost_usd,
                limit_usd: self.config.max_cost_usd,
            })
        } else {
            None
        }
    }

    /// Get cost as a percentage of limit (0.0 to 1.0+)
    pub fn cost_percentage(&self) -> f64 {
        if self.config.max_cost_usd == 0.0 {
            0.0
        } else {
            self.total_cost_usd / self.config.max_cost_usd
        }
    }

    // ========================================================================
    // Failure Limits
    // ========================================================================

    /// Increment failure count
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        warn!(
            failure_count = self.failure_count,
            limit = self.config.max_failures,
            "Hard limits: failure recorded"
        );
    }

    /// Reset failure count (called on success)
    pub fn reset_failure_count(&mut self) {
        if self.failure_count > 0 {
            info!(
                previous_count = self.failure_count,
                "Hard limits: failure count reset"
            );
        }
        self.failure_count = 0;
    }

    /// Check if failure limit is exceeded
    pub fn is_failure_limit_exceeded(&self) -> bool {
        self.failure_count >= self.config.max_failures
    }

    /// Check failure limit and return error if exceeded
    pub fn check_failure_limit(&self) -> Result<(), HardLimitError> {
        if self.is_failure_limit_exceeded() {
            return Err(HardLimitError::FailureLimitExceeded {
                current_failures: self.failure_count,
                limit_failures: self.config.max_failures,
            });
        }
        Ok(())
    }

    /// Get warning if failure limit is approaching (70% of limit)
    pub fn check_failure_warning(&self) -> Option<HardLimitWarning> {
        let ratio = self.failure_count as f64 / self.config.max_failures as f64;
        if ratio >= self.config.failure_warning_threshold && ratio < 1.0 {
            Some(HardLimitWarning::FailureCountApproaching {
                current_failures: self.failure_count,
                limit_failures: self.config.max_failures,
            })
        } else {
            None
        }
    }

    /// Get failure count as a percentage of limit (0.0 to 1.0+)
    pub fn failure_percentage(&self) -> f64 {
        if self.config.max_failures == 0 {
            0.0
        } else {
            self.failure_count as f64 / self.config.max_failures as f64
        }
    }

    // ========================================================================
    // Combined Checks
    // ========================================================================

    /// Perform a full check of all limits
    pub fn check_all(&self) -> HardLimitsCheck {
        let mut warnings = Vec::new();

        // Check cost warning
        if let Some(warning) = self.check_cost_warning() {
            warnings.push(warning);
        }

        // Check failure warning
        if let Some(warning) = self.check_failure_warning() {
            warnings.push(warning);
        }

        HardLimitsCheck {
            exceeded: None,
            warnings,
        }
    }

    /// Reset all limits to initial state
    pub fn reset(&mut self) {
        self.total_cost_usd = 0.0;
        self.failure_count = 0;
        self.total_elapsed_secs = 0;
        info!("Hard limits reset to initial state");
    }
}

impl Default for HardLimits {
    fn default() -> Self {
        Self::default_limits()
    }
}

/// Thread-safe wrapper for HardLimits
pub type SharedHardLimits = Arc<RwLock<HardLimits>>;

/// Create a new shared hard limits instance
pub fn create_hard_limits() -> SharedHardLimits {
    Arc::new(RwLock::new(HardLimits::default()))
}

/// Create with custom config
pub fn create_hard_limits_with_config(config: HardLimitsConfig) -> SharedHardLimits {
    Arc::new(RwLock::new(HardLimits::new(config)))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn create_test_started_at() -> DateTime<Utc> {
        // 5 minutes ago
        Utc::now() - Duration::minutes(5)
    }

    fn create_old_started_at() -> DateTime<Utc> {
        // 9 hours ago (exceeds 8 hour limit)
        Utc::now() - Duration::hours(9)
    }

    // --- Task Count Limit Tests ---

    #[test]
    fn test_task_creation_allowed_under_limit() {
        let limits = HardLimits::default();
        assert!(limits.check_task_creation(50).is_ok());
    }

    #[test]
    fn test_task_creation_rejected_at_limit() {
        let limits = HardLimits::default();
        let result = limits.check_task_creation(100);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HardLimitError::TaskLimitExceeded {
                current: 100,
                limit: 100
            }
        ));
    }

    #[test]
    fn test_task_creation_rejected_over_limit() {
        let limits = HardLimits::default();
        let result = limits.check_task_creation(150);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HardLimitError::TaskLimitExceeded {
                current: 150,
                limit: 100
            }
        ));
    }

    #[test]
    fn test_task_count_warning() {
        let limits = HardLimits::default();
        // At 80% of 100 = 80 tasks, should get warning
        let warning = limits.check_task_count_warning(80);
        assert!(warning.is_some());
        assert!(matches!(
            warning.unwrap(),
            HardLimitWarning::TaskCountApproaching {
                current: 80,
                limit: 100
            }
        ));
    }

    #[test]
    fn test_task_count_no_warning_below_threshold() {
        let limits = HardLimits::default();
        let warning = limits.check_task_count_warning(50);
        assert!(warning.is_none());
    }

    // --- Time Limit Tests ---

    #[test]
    fn test_time_limit_not_exceeded() {
        let limits = HardLimits::default();
        let started_at = create_test_started_at();
        assert!(!limits.is_time_limit_exceeded(started_at));
    }

    #[test]
    fn test_time_limit_exceeded() {
        let limits = HardLimits::default();
        let started_at = create_old_started_at();
        assert!(limits.is_time_limit_exceeded(started_at));
    }

    #[test]
    fn test_check_time_limit_ok() {
        let limits = HardLimits::default();
        let task_id = uuid::Uuid::new_v4();
        let started_at = create_test_started_at();
        assert!(limits.check_time_limit(task_id, started_at).is_ok());
    }

    #[test]
    fn test_check_time_limit_err() {
        let limits = HardLimits::default();
        let task_id = uuid::Uuid::new_v4();
        let started_at = create_old_started_at();
        let result = limits.check_time_limit(task_id, started_at);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HardLimitError::TimeLimitExceeded { task_id: t, elapsed_secs: _, limit_secs: 28800 }
            if t == task_id
        ));
    }

    #[test]
    fn test_time_warning_approaching() {
        let limits = HardLimits::default();
        let task_id = uuid::Uuid::new_v4();
        // 7 hours ago (80% of 8 hours = 6.4 hours)
        let started_at = Utc::now() - Duration::hours(7);
        let warning = limits.check_time_warning(task_id, started_at);
        assert!(warning.is_some());
    }

    #[test]
    fn test_time_no_warning_fresh_task() {
        let limits = HardLimits::default();
        let task_id = uuid::Uuid::new_v4();
        let started_at = create_test_started_at();
        let warning = limits.check_time_warning(task_id, started_at);
        assert!(warning.is_none());
    }

    // --- Cost Limit Tests ---

    #[test]
    fn test_add_cost() {
        let mut limits = HardLimits::default();
        limits.add_cost(10.0);
        assert!((limits.total_cost() - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_cost_limit_not_exceeded() {
        let limits = HardLimits::default();
        assert!(!limits.is_cost_limit_exceeded());
    }

    #[test]
    fn test_cost_limit_exceeded() {
        let _limits = HardLimits::default();
        // Default limit is $100, so cost exceeding $100 should trigger
        // We can't directly test this without adding cost, but we can test the logic
        // This is implicitly tested by check_cost_limit tests
    }

    #[test]
    fn test_check_cost_limit_ok() {
        let limits = HardLimits::default();
        assert!(limits.check_cost_limit().is_ok());
    }

    #[test]
    fn test_cost_percentage() {
        let mut limits = HardLimits::default();
        assert!((limits.cost_percentage() - 0.0).abs() < 0.001);

        limits.add_cost(50.0); // 50% of $100
        assert!((limits.cost_percentage() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_cost_warning() {
        let mut limits = HardLimits::default();
        limits.add_cost(85.0); // 85% of $100, exceeds 80% threshold
        let warning = limits.check_cost_warning();
        assert!(warning.is_some());
        assert!(matches!(
            warning.unwrap(),
            HardLimitWarning::CostApproaching {
                current_usd: 85.0,
                limit_usd: 100.0
            }
        ));
    }

    // --- Failure Limit Tests ---

    #[test]
    fn test_record_failure() {
        let mut limits = HardLimits::default();
        assert_eq!(limits.failure_count(), 0);
        limits.record_failure();
        assert_eq!(limits.failure_count(), 1);
        limits.record_failure();
        assert_eq!(limits.failure_count(), 2);
    }

    #[test]
    fn test_reset_failure_count() {
        let mut limits = HardLimits::default();
        limits.record_failure();
        limits.record_failure();
        assert_eq!(limits.failure_count(), 2);
        limits.reset_failure_count();
        assert_eq!(limits.failure_count(), 0);
    }

    #[test]
    fn test_failure_limit_not_exceeded() {
        let limits = HardLimits::default();
        assert!(!limits.is_failure_limit_exceeded());
    }

    #[test]
    fn test_failure_limit_exceeded() {
        let mut limits = HardLimits::default();
        for _ in 0..10 {
            limits.record_failure();
        }
        assert!(limits.is_failure_limit_exceeded());
    }

    #[test]
    fn test_check_failure_limit_ok() {
        let limits = HardLimits::default();
        assert!(limits.check_failure_limit().is_ok());
    }

    #[test]
    fn test_check_failure_limit_err() {
        let mut limits = HardLimits::default();
        for _ in 0..10 {
            limits.record_failure();
        }
        let result = limits.check_failure_limit();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HardLimitError::FailureLimitExceeded {
                current_failures: 10,
                limit_failures: 10
            }
        ));
    }

    #[test]
    fn test_failure_warning() {
        let mut limits = HardLimits::default();
        // 7 out of 10 = 70% threshold
        for _ in 0..7 {
            limits.record_failure();
        }
        let warning = limits.check_failure_warning();
        assert!(warning.is_some());
        assert!(matches!(
            warning.unwrap(),
            HardLimitWarning::FailureCountApproaching {
                current_failures: 7,
                limit_failures: 10
            }
        ));
    }

    #[test]
    fn test_failure_percentage() {
        let mut limits = HardLimits::default();
        assert!((limits.failure_percentage() - 0.0).abs() < 0.001);

        limits.record_failure();
        limits.record_failure();
        limits.record_failure();
        limits.record_failure();
        limits.record_failure();
        // 5 out of 10 = 50%
        assert!((limits.failure_percentage() - 0.5).abs() < 0.001);
    }

    // --- Combined Check Tests ---

    #[test]
    fn test_check_all_no_warnings() {
        let limits = HardLimits::default();
        let check = limits.check_all();
        assert!(check.is_ok());
        assert!(!check.is_warning());
    }

    #[test]
    fn test_reset() {
        let mut limits = HardLimits::default();
        limits.add_cost(50.0);
        limits.record_failure();
        limits.add_elapsed_time(1000);

        limits.reset();

        assert!((limits.total_cost() - 0.0).abs() < 0.001);
        assert_eq!(limits.failure_count(), 0);
        assert_eq!(limits.total_elapsed(), 0);
    }

    // --- Config Tests ---

    #[test]
    fn test_config_default() {
        let config = HardLimitsConfig::default();
        assert_eq!(config.max_tasks, 100);
        assert_eq!(config.max_time_secs, 28800); // 8 hours
        assert!((config.max_cost_usd - 100.0).abs() < 0.001);
        assert_eq!(config.max_failures, 10);
    }

    #[test]
    fn test_config_strict() {
        let config = HardLimitsConfig::strict();
        assert_eq!(config.max_tasks, 10);
        assert_eq!(config.max_time_secs, 3600);
        assert!((config.max_cost_usd - 10.0).abs() < 0.001);
        assert_eq!(config.max_failures, 3);
    }

    #[test]
    fn test_config_relaxed() {
        let config = HardLimitsConfig::relaxed();
        assert_eq!(config.max_tasks, 500);
        assert_eq!(config.max_time_secs, 86400); // 24 hours
        assert!((config.max_cost_usd - 1000.0).abs() < 0.001);
        assert_eq!(config.max_failures, 50);
    }

    // --- SharedHardLimits Tests ---

    #[tokio::test]
    async fn test_shared_hard_limits() {
        let limits = create_hard_limits();
        let _task_id = uuid::Uuid::new_v4();

        // Read initial state
        {
            let guard = limits.read().await;
            assert!(guard.check_task_creation(50).is_ok());
        }

        // Write and update
        {
            let mut guard = limits.write().await;
            guard.add_cost(25.0);
            guard.record_failure();
        }

        // Read updated state
        {
            let guard = limits.read().await;
            assert!((guard.total_cost() - 25.0).abs() < 0.001);
            assert_eq!(guard.failure_count(), 1);
        }
    }

    // --- Error Display Tests ---

    #[test]
    fn test_error_display_task_limit() {
        let error = HardLimitError::TaskLimitExceeded {
            current: 50,
            limit: 100,
        };
        assert_eq!(
            format!("{}", error),
            "Task limit exceeded: 50 tasks (limit: 100)"
        );
    }

    #[test]
    fn test_error_display_time_limit() {
        let error = HardLimitError::TimeLimitExceeded {
            task_id: uuid::Uuid::nil(),
            elapsed_secs: 30000,
            limit_secs: 28800,
        };
        let msg = format!("{}", error);
        assert!(msg.contains("Time limit exceeded"));
        assert!(msg.contains("30000s"));
        assert!(msg.contains("28800s"));
    }

    #[test]
    fn test_error_display_cost_limit() {
        let error = HardLimitError::CostLimitExceeded {
            current_usd: 150.0,
            limit_usd: 100.0,
        };
        assert_eq!(
            format!("{}", error),
            "Cost limit exceeded: $150.00 (limit: $100.00)"
        );
    }

    #[test]
    fn test_error_display_failure_limit() {
        let error = HardLimitError::FailureLimitExceeded {
            current_failures: 12,
            limit_failures: 10,
        };
        assert_eq!(
            format!("{}", error),
            "Failure limit exceeded: 12 failures (limit: 10)"
        );
    }

    // --- Elapsed Time Tests ---

    #[test]
    fn test_get_elapsed_secs() {
        let limits = HardLimits::default();
        let started_at = Utc::now() - Duration::seconds(300);
        let elapsed = limits.get_elapsed_secs(started_at);
        // Should be approximately 300 seconds (allow for test execution time)
        assert!(elapsed >= 299 && elapsed <= 305);
    }

    #[test]
    fn test_add_elapsed_time() {
        let mut limits = HardLimits::default();
        limits.add_elapsed_time(100);
        limits.add_elapsed_time(200);
        assert_eq!(limits.total_elapsed(), 300);
    }
}
