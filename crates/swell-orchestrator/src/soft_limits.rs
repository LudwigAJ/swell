//! Soft limits module for warning alerts and no-progress detection.
//!
//! This module provides soft limits that trigger warnings at configurable
//! thresholds before hard limits are reached. It complements `hard_limits`
//! by providing early warning capabilities.
//!
//! # Features
//!
//! - **Warning alerts at thresholds**: Generate warnings before hard limits are exceeded
//! - **No-progress detection**: Detect when a task is stuck without making progress
//! - **Configurable timeouts**: All thresholds are configurable via SoftLimitsConfig
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_orchestrator::soft_limits::{SoftLimits, SoftLimitsConfig};
//!
//! let config = SoftLimitsConfig::default();
//! let soft_limits = SoftLimits::new(config);
//!
//! // Check for warnings before hard limit is reached
//! if let Some(warning) = soft_limits.check_task_warning(current_count, max_tasks) {
//!     // Emit warning alert
//! }
//!
//! // Track progress and detect no-progress conditions
//! soft_limits.record_iteration(task_id);
//! if soft_limits.is_no_progress_detected(task_id) {
//!     // Handle no-progress condition
//! }
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

// Re-export types for convenience
use crate::alerts::{Alert, AlertCategory, AlertSeverity, AlertType};

/// Configuration for soft limits
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SoftLimitsConfig {
    /// Warning threshold for task count (percentage of hard limit, 0.0 to 1.0)
    pub task_count_warning_pct: f64,
    /// Warning threshold for time per task (percentage of hard limit, 0.0 to 1.0)
    pub time_warning_pct: f64,
    /// Warning threshold for cost (percentage of hard limit, 0.0 to 1.0)
    pub cost_warning_pct: f64,
    /// Warning threshold for failure count (percentage of hard limit, 0.0 to 1.0)
    pub failure_warning_pct: f64,
    /// Maximum iterations without progress before warning
    pub no_progress_iterations_warning: u32,
    /// Maximum iterations without progress before critical alert
    pub no_progress_iterations_critical: u32,
    /// Maximum time without progress in seconds
    pub no_progress_timeout_secs: u64,
    /// Cooldown between warnings of the same type (seconds)
    pub warning_cooldown_secs: u64,
    /// Enable no-progress detection
    pub no_progress_detection_enabled: bool,
    /// Enable task count warnings
    pub task_count_warnings_enabled: bool,
    /// Enable time warnings
    pub time_warnings_enabled: bool,
    /// Enable cost warnings
    pub cost_warnings_enabled: bool,
    /// Enable failure count warnings
    pub failure_warnings_enabled: bool,
}

impl Default for SoftLimitsConfig {
    fn default() -> Self {
        Self {
            task_count_warning_pct: 0.8, // 80% of hard limit
            time_warning_pct: 0.8,       // 80% of hard limit
            cost_warning_pct: 0.8,       // 80% of hard limit
            failure_warning_pct: 0.7,    // 70% of hard limit
            no_progress_iterations_warning: 3,
            no_progress_iterations_critical: 5,
            no_progress_timeout_secs: 300, // 5 minutes
            warning_cooldown_secs: 60,     // 1 minute
            no_progress_detection_enabled: true,
            task_count_warnings_enabled: true,
            time_warnings_enabled: true,
            cost_warnings_enabled: true,
            failure_warnings_enabled: true,
        }
    }
}

impl SoftLimitsConfig {
    /// Create a strict config for testing (lower thresholds)
    pub fn strict() -> Self {
        Self {
            task_count_warning_pct: 0.5,
            time_warning_pct: 0.5,
            cost_warning_pct: 0.5,
            failure_warning_pct: 0.5,
            no_progress_iterations_warning: 2,
            no_progress_iterations_critical: 3,
            no_progress_timeout_secs: 60,
            warning_cooldown_secs: 30,
            no_progress_detection_enabled: true,
            task_count_warnings_enabled: true,
            time_warnings_enabled: true,
            cost_warnings_enabled: true,
            failure_warnings_enabled: true,
        }
    }
}

/// Represents the type of soft limit warning
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SoftLimitType {
    /// Task count approaching hard limit
    TaskCount,
    /// Time limit approaching hard limit
    TimeLimit,
    /// Cost limit approaching hard limit
    CostLimit,
    /// Failure count approaching hard limit
    FailureCount,
    /// No progress detected
    NoProgress,
}

/// A soft limit warning with context
#[derive(Debug, Clone)]
pub struct SoftLimitWarning {
    /// Type of warning
    pub limit_type: SoftLimitType,
    /// Severity level
    pub severity: AlertSeverity,
    /// Current value
    pub current_value: f64,
    /// Threshold that triggered the warning
    pub threshold_value: f64,
    /// Human-readable message
    pub message: String,
    /// Actionable guidance
    pub action: String,
    /// Associated task ID if applicable
    pub task_id: Option<Uuid>,
    /// When the warning was generated
    pub generated_at: DateTime<Utc>,
}

impl SoftLimitWarning {
    /// Create a new warning
    pub fn new(
        limit_type: SoftLimitType,
        severity: AlertSeverity,
        current_value: f64,
        threshold_value: f64,
        message: String,
        action: String,
    ) -> Self {
        Self {
            limit_type,
            severity,
            current_value,
            threshold_value,
            message,
            action,
            task_id: None,
            generated_at: Utc::now(),
        }
    }

    /// Set the associated task ID
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Convert to an Alert
    pub fn to_alert(&self) -> Alert {
        let alert_type = match self.limit_type {
            SoftLimitType::TaskCount => AlertType::CompletionRateLow,
            SoftLimitType::TimeLimit => AlertType::CompletionRateLow,
            SoftLimitType::CostLimit => AlertType::CostPerTaskHigh,
            SoftLimitType::FailureCount => AlertType::ValidationFailureRateHigh,
            SoftLimitType::NoProgress => AlertType::CompletionRateLow,
        };

        Alert::new(
            AlertCategory::Metrics,
            alert_type,
            self.severity,
            self.message.clone(),
            self.action.clone(),
            self.current_value,
            self.threshold_value,
        )
        .with_task_id(self.task_id.unwrap_or(Uuid::nil()))
    }
}

/// Progress tracking for no-progress detection
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    /// Current iteration count
    pub iteration: u32,
    /// Last iteration where progress was made
    pub last_progress_iteration: u32,
    /// Last time progress was made
    pub last_progress_time: DateTime<Utc>,
    /// Whether no-progress has been detected
    pub no_progress_detected: bool,
    /// Severity of the no-progress condition
    pub no_progress_severity: Option<AlertSeverity>,
}

impl ProgressTracker {
    /// Create a new progress tracker
    pub fn new() -> Self {
        Self {
            iteration: 0,
            last_progress_iteration: 0,
            last_progress_time: Utc::now(),
            no_progress_detected: false,
            no_progress_severity: None,
        }
    }

    /// Record an iteration
    pub fn record_iteration(&mut self) {
        self.iteration += 1;
    }

    /// Record progress (file change, successful step, etc.)
    pub fn record_progress(&mut self) {
        self.last_progress_iteration = self.iteration;
        self.last_progress_time = Utc::now();
        self.no_progress_detected = false;
        self.no_progress_severity = None;
    }

    /// Check if no-progress condition exists and update status
    pub fn check_no_progress(&mut self, config: &SoftLimitsConfig) -> bool {
        let iterations_without_progress = self.iteration - self.last_progress_iteration;
        let time_without_progress = Utc::now() - self.last_progress_time;

        // Check iteration-based no-progress
        if iterations_without_progress >= config.no_progress_iterations_critical {
            self.no_progress_detected = true;
            self.no_progress_severity = Some(AlertSeverity::Critical);
            return true;
        } else if iterations_without_progress >= config.no_progress_iterations_warning {
            self.no_progress_detected = true;
            self.no_progress_severity = Some(AlertSeverity::Warning);
            return true;
        }

        // Check time-based no-progress
        if time_without_progress.num_seconds() as u64 >= config.no_progress_timeout_secs {
            self.no_progress_detected = true;
            self.no_progress_severity = if time_without_progress.num_seconds() as u64
                >= config.no_progress_timeout_secs * 2
            {
                Some(AlertSeverity::Critical)
            } else {
                Some(AlertSeverity::Warning)
            };
            return true;
        }

        false
    }

    /// Get iterations without progress
    pub fn iterations_without_progress(&self) -> u32 {
        self.iteration - self.last_progress_iteration
    }

    /// Get time without progress
    pub fn time_without_progress(&self) -> Duration {
        Utc::now() - self.last_progress_time
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Soft limits manager for tracking and warning on thresholds
#[derive(Debug, Clone)]
pub struct SoftLimits {
    config: SoftLimitsConfig,
    /// Progress trackers per task
    progress_trackers: HashMap<Uuid, ProgressTracker>,
    /// Last warning time per limit type (for cooldown)
    last_warning_time: HashMap<SoftLimitType, DateTime<Utc>>,
    /// Global iteration count
    global_iteration: u32,
}

impl SoftLimits {
    /// Create new soft limits with default config
    pub fn new() -> Self {
        Self::with_config(SoftLimitsConfig::default())
    }

    /// Create new soft limits with custom config
    pub fn with_config(config: SoftLimitsConfig) -> Self {
        Self {
            config,
            progress_trackers: HashMap::new(),
            last_warning_time: HashMap::new(),
            global_iteration: 0,
        }
    }

    /// Get current configuration
    pub fn config(&self) -> SoftLimitsConfig {
        self.config
    }

    /// Update configuration at runtime
    pub fn set_config(&mut self, config: SoftLimitsConfig) {
        self.config = config;
    }

    // ========================================================================
    // Progress Tracking
    // ========================================================================

    /// Initialize progress tracking for a task
    pub fn init_task_tracking(&mut self, task_id: Uuid) {
        self.progress_trackers
            .insert(task_id, ProgressTracker::new());
    }

    /// Record an iteration for a task
    pub fn record_iteration(&mut self, task_id: Uuid) {
        self.global_iteration += 1;
        if let Some(tracker) = self.progress_trackers.get_mut(&task_id) {
            tracker.record_iteration();
        }
    }

    /// Record progress for a task (successful step, file change, etc.)
    pub fn record_progress(&mut self, task_id: Uuid) {
        if let Some(tracker) = self.progress_trackers.get_mut(&task_id) {
            tracker.record_progress();
            info!(
                task_id = %task_id,
                iteration = tracker.iteration,
                "Progress recorded for task"
            );
        }
    }

    /// Check if no-progress is detected for a task
    pub fn is_no_progress_detected(&mut self, task_id: Uuid) -> bool {
        if let Some(tracker) = self.progress_trackers.get_mut(&task_id) {
            tracker.check_no_progress(&self.config)
        } else {
            false
        }
    }

    /// Get no-progress severity for a task
    pub fn get_no_progress_severity(&self, task_id: Uuid) -> Option<AlertSeverity> {
        self.progress_trackers
            .get(&task_id)
            .and_then(|t| t.no_progress_severity)
    }

    /// Get iterations without progress for a task
    pub fn get_iterations_without_progress(&self, task_id: Uuid) -> u32 {
        self.progress_trackers
            .get(&task_id)
            .map(|t| t.iterations_without_progress())
            .unwrap_or(0)
    }

    /// Remove task tracking
    pub fn remove_task_tracking(&mut self, task_id: Uuid) {
        self.progress_trackers.remove(&task_id);
    }

    // ========================================================================
    // Warning Checks
    // ========================================================================

    /// Check if cooldown has passed for a warning type
    fn is_warning_cooldown_passed(&self, limit_type: &SoftLimitType) -> bool {
        if let Some(last_time) = self.last_warning_time.get(limit_type) {
            let elapsed = Utc::now().signed_duration_since(*last_time).num_seconds() as u64;
            return elapsed >= self.config.warning_cooldown_secs;
        }
        true
    }

    /// Record a warning time
    fn record_warning_time(&mut self, limit_type: SoftLimitType) {
        self.last_warning_time.insert(limit_type, Utc::now());
    }

    /// Check task count warning
    pub fn check_task_count_warning(
        &mut self,
        current_count: usize,
        hard_limit: usize,
    ) -> Option<SoftLimitWarning> {
        if !self.config.task_count_warnings_enabled {
            return None;
        }

        if !self.is_warning_cooldown_passed(&SoftLimitType::TaskCount) {
            return None;
        }

        let threshold = (hard_limit as f64 * self.config.task_count_warning_pct) as usize;
        let percentage = if hard_limit > 0 {
            current_count as f64 / hard_limit as f64
        } else {
            0.0
        };

        if current_count >= threshold {
            let severity = if percentage >= 0.9 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };

            let warning = SoftLimitWarning::new(
                SoftLimitType::TaskCount,
                severity,
                current_count as f64,
                threshold as f64,
                format!(
                    "Task count {} approaching hard limit {} ({:.0}%)",
                    current_count,
                    hard_limit,
                    percentage * 100.0
                ),
                "Consider completing or canceling some tasks before creating new ones.".to_string(),
            );

            self.record_warning_time(SoftLimitType::TaskCount);
            return Some(warning);
        }

        None
    }

    /// Check time warning
    pub fn check_time_warning(
        &mut self,
        task_id: Uuid,
        elapsed_secs: u64,
        hard_limit_secs: u64,
    ) -> Option<SoftLimitWarning> {
        if !self.config.time_warnings_enabled {
            return None;
        }

        if !self.is_warning_cooldown_passed(&SoftLimitType::TimeLimit) {
            return None;
        }

        let threshold = (hard_limit_secs as f64 * self.config.time_warning_pct) as u64;

        if elapsed_secs >= threshold {
            let severity = if elapsed_secs as f64 / hard_limit_secs as f64 >= 0.9 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };

            let warning = SoftLimitWarning::new(
                SoftLimitType::TimeLimit,
                severity,
                elapsed_secs as f64,
                threshold as f64,
                format!(
                    "Task {} time {}s approaching limit {}s ({:.0}%)",
                    task_id,
                    elapsed_secs,
                    hard_limit_secs,
                    (elapsed_secs as f64 / hard_limit_secs as f64) * 100.0
                ),
                "Monitor task progress closely. Consider intervening if no progress is made."
                    .to_string(),
            )
            .with_task_id(task_id);

            self.record_warning_time(SoftLimitType::TimeLimit);
            return Some(warning);
        }

        None
    }

    /// Check cost warning
    pub fn check_cost_warning(
        &mut self,
        current_cost: f64,
        hard_limit: f64,
    ) -> Option<SoftLimitWarning> {
        if !self.config.cost_warnings_enabled {
            return None;
        }

        if !self.is_warning_cooldown_passed(&SoftLimitType::CostLimit) {
            return None;
        }

        let threshold = hard_limit * self.config.cost_warning_pct;

        if current_cost >= threshold {
            let severity = if current_cost / hard_limit >= 0.9 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };

            let warning = SoftLimitWarning::new(
                SoftLimitType::CostLimit,
                severity,
                current_cost,
                threshold,
                format!(
                    "Cost ${:.2} approaching limit ${:.2} ({:.0}%)",
                    current_cost,
                    hard_limit,
                    (current_cost / hard_limit) * 100.0
                ),
                "Review token usage and optimize prompts if possible.".to_string(),
            );

            self.record_warning_time(SoftLimitType::CostLimit);
            return Some(warning);
        }

        None
    }

    /// Check failure count warning
    pub fn check_failure_warning(
        &mut self,
        current_failures: u32,
        hard_limit: u32,
    ) -> Option<SoftLimitWarning> {
        if !self.config.failure_warnings_enabled {
            return None;
        }

        if !self.is_warning_cooldown_passed(&SoftLimitType::FailureCount) {
            return None;
        }

        let threshold = (hard_limit as f64 * self.config.failure_warning_pct) as u32;

        if current_failures >= threshold {
            let severity = if current_failures as f64 / hard_limit as f64 >= 0.9 {
                AlertSeverity::Critical
            } else {
                AlertSeverity::Warning
            };

            let warning = SoftLimitWarning::new(
                SoftLimitType::FailureCount,
                severity,
                current_failures as f64,
                threshold as f64,
                format!(
                    "Failure count {} approaching limit {} ({:.0}%)",
                    current_failures,
                    hard_limit,
                    (current_failures as f64 / hard_limit as f64) * 100.0
                ),
                "Investigate failure causes. Task may need adjustment or human review.".to_string(),
            );

            self.record_warning_time(SoftLimitType::FailureCount);
            return Some(warning);
        }

        None
    }

    /// Check no-progress warning for a task
    pub fn check_no_progress_warning(&mut self, task_id: Uuid) -> Option<SoftLimitWarning> {
        if !self.config.no_progress_detection_enabled {
            return None;
        }

        if !self.is_warning_cooldown_passed(&SoftLimitType::NoProgress) {
            return None;
        }

        let tracker = self.progress_trackers.get_mut(&task_id)?;

        // Update no-progress status
        let is_no_progress = tracker.check_no_progress(&self.config);

        if is_no_progress {
            let severity = tracker
                .no_progress_severity
                .unwrap_or(AlertSeverity::Warning);
            let iterations = tracker.iterations_without_progress();
            let time_secs = tracker.time_without_progress().num_seconds() as u64;

            let warning = SoftLimitWarning::new(
                SoftLimitType::NoProgress,
                severity,
                iterations as f64,
                self.config.no_progress_iterations_warning as f64,
                format!(
                    "No progress detected for task {}: {} iterations, {}s without changes",
                    task_id, iterations, time_secs
                ),
                "Task may be stuck in a loop. Consider reviewing logs and intervening.".to_string(),
            )
            .with_task_id(task_id);

            self.record_warning_time(SoftLimitType::NoProgress);
            return Some(warning);
        }

        None
    }

    /// Get all active warnings for a task
    pub fn get_task_warnings(&mut self, task_id: Uuid) -> Vec<SoftLimitWarning> {
        let mut warnings = Vec::new();

        if let Some(warning) = self.check_no_progress_warning(task_id) {
            warnings.push(warning);
        }

        warnings
    }

    // ========================================================================
    // Utility Methods
    // ========================================================================

    /// Get global iteration count
    pub fn global_iteration(&self) -> u32 {
        self.global_iteration
    }

    /// Get number of tracked tasks
    pub fn tracked_task_count(&self) -> usize {
        self.progress_trackers.len()
    }

    /// Clear all progress trackers
    pub fn clear_trackers(&mut self) {
        self.progress_trackers.clear();
        info!("All soft limit progress trackers cleared");
    }

    /// Reset cooldown for a specific limit type
    pub fn reset_cooldown(&mut self, limit_type: SoftLimitType) {
        self.last_warning_time.remove(&limit_type);
    }

    /// Reset all cooldowns
    pub fn reset_all_cooldowns(&mut self) {
        self.last_warning_time.clear();
    }
}

impl Default for SoftLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper for SoftLimits
pub type SharedSoftLimits = Arc<RwLock<SoftLimits>>;

/// Create a new shared soft limits instance
pub fn create_soft_limits() -> SharedSoftLimits {
    Arc::new(RwLock::new(SoftLimits::new()))
}

/// Create with custom config
pub fn create_soft_limits_with_config(config: SoftLimitsConfig) -> SharedSoftLimits {
    Arc::new(RwLock::new(SoftLimits::with_config(config)))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- SoftLimitsConfig Tests ---

    #[test]
    fn test_config_default() {
        let config = SoftLimitsConfig::default();
        assert_eq!(config.task_count_warning_pct, 0.8);
        assert_eq!(config.time_warning_pct, 0.8);
        assert_eq!(config.cost_warning_pct, 0.8);
        assert_eq!(config.failure_warning_pct, 0.7);
        assert_eq!(config.no_progress_iterations_warning, 3);
        assert_eq!(config.no_progress_iterations_critical, 5);
        assert_eq!(config.no_progress_timeout_secs, 300);
    }

    #[test]
    fn test_config_strict() {
        let config = SoftLimitsConfig::strict();
        assert_eq!(config.task_count_warning_pct, 0.5);
        assert_eq!(config.no_progress_iterations_warning, 2);
        assert_eq!(config.no_progress_iterations_critical, 3);
    }

    // --- ProgressTracker Tests ---

    #[test]
    fn test_progress_tracker_initialization() {
        let tracker = ProgressTracker::new();
        assert_eq!(tracker.iteration, 0);
        assert_eq!(tracker.last_progress_iteration, 0);
        assert!(!tracker.no_progress_detected);
        assert!(tracker.no_progress_severity.is_none());
    }

    #[test]
    fn test_progress_tracker_record_iteration() {
        let mut tracker = ProgressTracker::new();
        tracker.record_iteration();
        assert_eq!(tracker.iteration, 1);
        tracker.record_iteration();
        assert_eq!(tracker.iteration, 2);
    }

    #[test]
    fn test_progress_tracker_record_progress() {
        let mut tracker = ProgressTracker::new();
        tracker.record_iteration();
        tracker.record_iteration();
        tracker.record_progress();
        assert_eq!(tracker.last_progress_iteration, 2);
        assert!(!tracker.no_progress_detected);
    }

    #[test]
    fn test_progress_tracker_no_progress_warning() {
        let mut tracker = ProgressTracker::new();
        let config = SoftLimitsConfig::default();

        // Record iterations without progress
        for _ in 0..3 {
            tracker.record_iteration();
        }

        let is_no_progress = tracker.check_no_progress(&config);
        assert!(is_no_progress);
        assert_eq!(tracker.no_progress_severity, Some(AlertSeverity::Warning));
    }

    #[test]
    fn test_progress_tracker_no_progress_critical() {
        let mut tracker = ProgressTracker::new();
        let config = SoftLimitsConfig::default();

        // Record iterations without progress
        for _ in 0..6 {
            tracker.record_iteration();
        }

        let is_no_progress = tracker.check_no_progress(&config);
        assert!(is_no_progress);
        assert_eq!(tracker.no_progress_severity, Some(AlertSeverity::Critical));
    }

    #[test]
    fn test_progress_tracker_with_progress_resets() {
        let mut tracker = ProgressTracker::new();
        let config = SoftLimitsConfig::default();

        // Build up no-progress
        for _ in 0..4 {
            tracker.record_iteration();
        }
        tracker.check_no_progress(&config);
        assert!(tracker.no_progress_detected);

        // Record progress
        tracker.record_progress();

        // Should reset
        assert!(!tracker.no_progress_detected);
        assert!(tracker.no_progress_severity.is_none());
    }

    // --- SoftLimits Tests ---

    #[test]
    fn test_soft_limits_initialization() {
        let soft_limits = SoftLimits::new();
        assert_eq!(soft_limits.tracked_task_count(), 0);
        assert_eq!(soft_limits.global_iteration(), 0);
    }

    #[test]
    fn test_init_task_tracking() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);
        assert_eq!(soft_limits.tracked_task_count(), 1);
    }

    #[test]
    fn test_record_iteration() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);
        soft_limits.record_iteration(task_id);

        assert_eq!(soft_limits.global_iteration(), 1);
    }

    #[test]
    fn test_record_progress() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);
        soft_limits.record_iteration(task_id);
        soft_limits.record_iteration(task_id);
        soft_limits.record_progress(task_id);

        assert_eq!(soft_limits.get_iterations_without_progress(task_id), 0);
    }

    #[test]
    fn test_remove_task_tracking() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);
        assert_eq!(soft_limits.tracked_task_count(), 1);

        soft_limits.remove_task_tracking(task_id);
        assert_eq!(soft_limits.tracked_task_count(), 0);
    }

    // --- Task Count Warning Tests ---

    #[test]
    fn test_task_count_warning_below_threshold() {
        let mut soft_limits = SoftLimits::new();
        let warning = soft_limits.check_task_count_warning(50, 100);
        assert!(warning.is_none());
    }

    #[test]
    fn test_task_count_warning_at_threshold() {
        let mut soft_limits = SoftLimits::new();
        // At 80% of 100 = 80
        let warning = soft_limits.check_task_count_warning(80, 100);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.limit_type, SoftLimitType::TaskCount);
        assert_eq!(warning.severity, AlertSeverity::Warning);
        assert_eq!(warning.current_value, 80.0);
    }

    #[test]
    fn test_task_count_warning_critical() {
        let mut soft_limits = SoftLimits::new();
        // At 95% of 100 = 95
        let warning = soft_limits.check_task_count_warning(95, 100);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.severity, AlertSeverity::Critical);
    }

    #[test]
    fn test_task_count_warning_cooldown() {
        let mut soft_limits = SoftLimits::new();
        // First warning
        let warning1 = soft_limits.check_task_count_warning(80, 100);
        assert!(warning1.is_some());

        // Second warning immediately - should be blocked by cooldown
        let warning2 = soft_limits.check_task_count_warning(85, 100);
        assert!(warning2.is_none());
    }

    // --- Time Warning Tests ---

    #[test]
    fn test_time_warning() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        // 250 seconds with 300 second hard limit = 83% (above 80% threshold)
        let warning = soft_limits.check_time_warning(task_id, 250, 300);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.limit_type, SoftLimitType::TimeLimit);
        assert!(warning.task_id.is_some());
    }

    #[test]
    fn test_time_warning_below_threshold() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        // 200 seconds with 300 second hard limit = 67% (below 80% threshold)
        let warning = soft_limits.check_time_warning(task_id, 200, 300);
        assert!(warning.is_none());
    }

    // --- Cost Warning Tests ---

    #[test]
    fn test_cost_warning() {
        let mut soft_limits = SoftLimits::new();

        // $85 with $100 hard limit = 85% (above 80% threshold)
        let warning = soft_limits.check_cost_warning(85.0, 100.0);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.limit_type, SoftLimitType::CostLimit);
        assert_eq!(warning.current_value, 85.0);
    }

    #[test]
    fn test_cost_warning_critical() {
        let mut soft_limits = SoftLimits::new();

        // $95 with $100 hard limit = 95% (above 90% critical threshold)
        let warning = soft_limits.check_cost_warning(95.0, 100.0);
        assert!(warning.is_some());

        assert_eq!(warning.unwrap().severity, AlertSeverity::Critical);
    }

    // --- Failure Count Warning Tests ---

    #[test]
    fn test_failure_warning() {
        let mut soft_limits = SoftLimits::new();

        // 7 failures with 10 hard limit = 70% (at warning threshold)
        let warning = soft_limits.check_failure_warning(7, 10);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.limit_type, SoftLimitType::FailureCount);
    }

    // --- No Progress Warning Tests ---

    #[test]
    fn test_no_progress_warning() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);

        // Record iterations without progress
        for _ in 0..4 {
            soft_limits.record_iteration(task_id);
        }

        let warning = soft_limits.check_no_progress_warning(task_id);
        assert!(warning.is_some());

        let warning = warning.unwrap();
        assert_eq!(warning.limit_type, SoftLimitType::NoProgress);
        assert!(warning.task_id.is_some());
    }

    #[test]
    fn test_no_progress_warning_with_progress_resets() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);

        // Build up iterations
        for _ in 0..4 {
            soft_limits.record_iteration(task_id);
        }

        // Verify no-progress detected
        assert!(soft_limits.is_no_progress_detected(task_id));

        // Record progress - should reset
        soft_limits.record_progress(task_id);

        // Check again
        assert!(!soft_limits.is_no_progress_detected(task_id));
    }

    // --- SoftLimitWarning Tests ---

    #[test]
    fn test_soft_limit_warning_to_alert() {
        let warning = SoftLimitWarning::new(
            SoftLimitType::CostLimit,
            AlertSeverity::Warning,
            85.0,
            80.0,
            "Cost approaching limit".to_string(),
            "Review usage".to_string(),
        );

        let alert = warning.to_alert();
        assert_eq!(alert.category, AlertCategory::Metrics);
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.value, 85.0);
        assert_eq!(alert.threshold, 80.0);
    }

    #[test]
    fn test_soft_limit_warning_with_task_id() {
        let task_id = Uuid::new_v4();
        let warning = SoftLimitWarning::new(
            SoftLimitType::NoProgress,
            AlertSeverity::Critical,
            5.0,
            3.0,
            "No progress".to_string(),
            "Intervene".to_string(),
        )
        .with_task_id(task_id);

        assert_eq!(warning.task_id, Some(task_id));
    }

    // --- Shared SoftLimits Tests ---

    #[tokio::test]
    async fn test_shared_soft_limits() {
        let soft_limits = create_soft_limits();
        let task_id = Uuid::new_v4();

        // Initialize
        {
            let mut guard = soft_limits.write().await;
            guard.init_task_tracking(task_id);
            guard.record_iteration(task_id);
        }

        // Read state
        {
            let guard = soft_limits.read().await;
            assert_eq!(guard.global_iteration(), 1);
            assert_eq!(guard.tracked_task_count(), 1);
        }

        // Record progress
        {
            let mut guard = soft_limits.write().await;
            guard.record_progress(task_id);
        }

        // Verify progress reset
        {
            let guard = soft_limits.read().await;
            assert_eq!(guard.get_iterations_without_progress(task_id), 0);
        }
    }

    // --- Cooldown Tests ---

    #[test]
    fn test_reset_cooldown() {
        let mut soft_limits = SoftLimits::new();

        // Trigger warning to set cooldown
        let _warning = soft_limits.check_task_count_warning(80, 100);
        assert!(!soft_limits.is_warning_cooldown_passed(&SoftLimitType::TaskCount));

        // Reset cooldown
        soft_limits.reset_cooldown(SoftLimitType::TaskCount);
        assert!(soft_limits.is_warning_cooldown_passed(&SoftLimitType::TaskCount));
    }

    #[test]
    fn test_reset_all_cooldowns() {
        let mut soft_limits = SoftLimits::new();

        // Trigger multiple warnings
        let _ = soft_limits.check_task_count_warning(80, 100);
        let _ = soft_limits.check_cost_warning(85.0, 100.0);

        // Reset all
        soft_limits.reset_all_cooldowns();

        assert!(soft_limits.is_warning_cooldown_passed(&SoftLimitType::TaskCount));
        assert!(soft_limits.is_warning_cooldown_passed(&SoftLimitType::CostLimit));
    }

    // --- Disabled Warnings Tests ---

    #[test]
    fn test_task_count_warnings_disabled() {
        let mut config = SoftLimitsConfig::default();
        config.task_count_warnings_enabled = false;

        let mut soft_limits = SoftLimits::with_config(config);
        let warning = soft_limits.check_task_count_warning(100, 100);
        assert!(warning.is_none());
    }

    #[test]
    fn test_no_progress_detection_disabled() {
        let mut config = SoftLimitsConfig::default();
        config.no_progress_detection_enabled = false;

        let mut soft_limits = SoftLimits::with_config(config);
        let task_id = Uuid::new_v4();

        soft_limits.init_task_tracking(task_id);
        for _ in 0..10 {
            soft_limits.record_iteration(task_id);
        }

        let warning = soft_limits.check_no_progress_warning(task_id);
        assert!(warning.is_none());
    }

    // --- Integration Tests ---

    #[test]
    fn test_full_warning_lifecycle() {
        let mut soft_limits = SoftLimits::new();
        let task_id = Uuid::new_v4();

        // Initialize
        soft_limits.init_task_tracking(task_id);

        // Record some iterations with progress
        for _ in 0..3 {
            soft_limits.record_iteration(task_id);
            soft_limits.record_progress(task_id);
            // Should not trigger warning
            assert!(!soft_limits.is_no_progress_detected(task_id));
        }

        // Now stop recording progress
        for _ in 0..4 {
            soft_limits.record_iteration(task_id);
        }

        // Should now detect no-progress
        assert!(soft_limits.is_no_progress_detected(task_id));

        let warnings = soft_limits.get_task_warnings(task_id);
        assert!(!warnings.is_empty());

        // Clean up
        soft_limits.remove_task_tracking(task_id);
        assert_eq!(soft_limits.tracked_task_count(), 0);
    }
}
