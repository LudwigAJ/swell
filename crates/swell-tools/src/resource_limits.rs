//! Resource limits for tool execution sessions.
//!
//! This module provides comprehensive resource limit tracking for sessions,
//! including max turns, wall-clock timeout, token/cost caps, and failure tracking.
//!
//! # Session Limits
//!
//! Sessions track resource usage across multiple tool executions. Limits can be
//! configured to prevent runaway operations or excessive resource consumption.
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_tools::resource_limits::{SessionLimits, SessionResourceTracker};
//! use std::time::Duration;
//!
//! let limits = SessionLimits::default()
//!     .with_max_turns(100)
//!     .with_wall_clock_timeout(Duration::from_secs(3600))
//!     .with_token_cap(1_000_000)
//!     .with_cost_cap(10.0)
//!     .with_failure_threshold(5);
//!
//! let tracker = SessionResourceTracker::new(limits);
//!
//! // Check before each operation
//! if let Err(e) = tracker.check_limits() {
//!     // Handle limit exceeded
//! }
//!
//! // Record turn after tool execution
//! tracker.record_turn();
//! ```

use std::time::{Duration, Instant};
use swell_core::cost_tracking::{CostRecord, CostSummary};

/// Configuration for session resource limits
#[derive(Debug, Clone)]
pub struct SessionLimits {
    /// Maximum number of tool execution turns per session
    pub max_turns: u32,
    /// Wall-clock timeout for the entire session
    pub wall_clock_timeout: Duration,
    /// Maximum tokens allowed per session
    pub token_cap: u64,
    /// Maximum cost in USD allowed per session
    pub cost_cap: f64,
    /// Maximum consecutive failures before session termination
    pub failure_threshold: u32,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_turns: 100,
            wall_clock_timeout: Duration::from_secs(3600), // 1 hour default
            token_cap: 1_000_000,                          // 1M tokens
            cost_cap: 50.0,                                // $50 default
            failure_threshold: 5,
        }
    }
}

impl SessionLimits {
    /// Create new session limits with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum number of turns
    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Set wall-clock timeout
    pub fn with_wall_clock_timeout(mut self, timeout: Duration) -> Self {
        self.wall_clock_timeout = timeout;
        self
    }

    /// Set token cap
    pub fn with_token_cap(mut self, cap: u64) -> Self {
        self.token_cap = cap;
        self
    }

    /// Set cost cap
    pub fn with_cost_cap(mut self, cap: f64) -> Self {
        self.cost_cap = cap;
        self
    }

    /// Set failure threshold
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }
}

/// State of a session resource limit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitState {
    /// Within limits
    Ok,
    /// Warning threshold reached (soft limit)
    Warning,
    /// Hard limit exceeded
    Exceeded,
}

impl LimitState {
    pub fn is_ok(&self) -> bool {
        matches!(self, LimitState::Ok | LimitState::Warning)
    }

    pub fn is_exceeded(&self) -> bool {
        matches!(self, LimitState::Exceeded)
    }
}

/// Result of checking a single resource limit
#[derive(Debug, Clone)]
pub struct LimitCheckResult {
    /// State of the limit
    pub state: LimitState,
    /// Current usage value
    pub current: u64,
    /// Maximum allowed value
    pub limit: u64,
    /// Name of the limit
    pub limit_name: &'static str,
}

impl LimitCheckResult {
    fn new(limit_name: &'static str, current: u64, limit: u64, warning_ratio: f64) -> Self {
        let ratio = if limit > 0 {
            current as f64 / limit as f64
        } else {
            f64::INFINITY
        };

        let state = if ratio >= 1.0 {
            LimitState::Exceeded
        } else if ratio >= warning_ratio {
            LimitState::Warning
        } else {
            LimitState::Ok
        };

        Self {
            state,
            current,
            limit,
            limit_name,
        }
    }

    /// Create a result in the Exceeded state
    pub fn exceeded(limit_name: &'static str, current: u64, limit: u64) -> Self {
        Self {
            state: LimitState::Exceeded,
            current,
            limit,
            limit_name,
        }
    }
}

/// Overall resource limit check result
#[derive(Debug, Clone)]
pub struct ResourceLimitResult {
    /// Turns limit state
    pub turns: LimitCheckResult,
    /// Wall-clock timeout state
    pub wall_clock: LimitCheckResult,
    /// Token cap state
    pub tokens: LimitCheckResult,
    /// Cost cap state
    pub cost: LimitCheckResult,
    /// Failure threshold state
    pub failures: LimitCheckResult,
}

impl ResourceLimitResult {
    /// Check if any limit is exceeded
    pub fn is_any_exceeded(&self) -> bool {
        self.turns.state.is_exceeded()
            || self.wall_clock.state.is_exceeded()
            || self.tokens.state.is_exceeded()
            || self.cost.state.is_exceeded()
            || self.failures.state.is_exceeded()
    }

    /// Check if any limit is in warning state
    pub fn has_warning(&self) -> bool {
        self.turns.state == LimitState::Warning
            || self.wall_clock.state == LimitState::Warning
            || self.tokens.state == LimitState::Warning
            || self.cost.state == LimitState::Warning
            || self.failures.state == LimitState::Warning
    }

    /// Get all exceeded limits
    pub fn exceeded_limits(&self) -> Vec<&'static str> {
        let mut exceeded = Vec::new();
        if self.turns.state.is_exceeded() {
            exceeded.push("turns");
        }
        if self.wall_clock.state.is_exceeded() {
            exceeded.push("wall_clock");
        }
        if self.tokens.state.is_exceeded() {
            exceeded.push("tokens");
        }
        if self.cost.state.is_exceeded() {
            exceeded.push("cost");
        }
        if self.failures.state.is_exceeded() {
            exceeded.push("failures");
        }
        exceeded
    }
}

/// Error returned when a resource limit is exceeded
#[derive(Debug, thiserror::Error)]
pub enum ResourceLimitError {
    #[error("Max turns exceeded: {current} / {limit}")]
    MaxTurnsExceeded { current: u32, limit: u32 },

    #[error("Wall-clock timeout exceeded: {elapsed:?} / {limit:?}")]
    WallClockTimeoutExceeded { elapsed: Duration, limit: Duration },

    #[error("Token cap exceeded: {current} / {limit}")]
    TokenCapExceeded { current: u64, limit: u64 },

    #[error("Cost cap exceeded: ${current:.2} / ${limit:.2}")]
    CostCapExceeded { current: f64, limit: f64 },

    #[error("Failure threshold exceeded: {current} / {limit}")]
    FailureThresholdExceeded { current: u32, limit: u32 },

    #[error("Multiple limits exceeded: {limits:?}")]
    MultipleLimitsExceeded { limits: Vec<&'static str> },
}

impl ResourceLimitError {
    /// Get the limit name that caused this error
    pub fn limit_name(&self) -> &'static str {
        match self {
            ResourceLimitError::MaxTurnsExceeded { .. } => "turns",
            ResourceLimitError::WallClockTimeoutExceeded { .. } => "wall_clock",
            ResourceLimitError::TokenCapExceeded { .. } => "tokens",
            ResourceLimitError::CostCapExceeded { .. } => "cost",
            ResourceLimitError::FailureThresholdExceeded { .. } => "failures",
            ResourceLimitError::MultipleLimitsExceeded { .. } => "multiple",
        }
    }
}

/// Tracks resource usage for a session
#[derive(Debug, Clone)]
pub struct SessionResourceTracker {
    /// Session limits configuration
    limits: SessionLimits,
    /// Current turn count
    turns: u32,
    /// Session start time
    start_time: Instant,
    /// Total tokens used
    tokens_used: u64,
    /// Total cost in USD
    cost_used: f64,
    /// Consecutive failure count
    consecutive_failures: u32,
    /// Total failure count
    total_failures: u32,
    /// Warning thresholds ratio (0.0 to 1.0)
    warning_ratio: f64,
}

impl SessionResourceTracker {
    /// Create a new tracker with the given limits
    pub fn new(limits: SessionLimits) -> Self {
        Self {
            limits,
            turns: 0,
            start_time: Instant::now(),
            tokens_used: 0,
            cost_used: 0.0,
            consecutive_failures: 0,
            total_failures: 0,
            warning_ratio: 0.75, // 75% threshold for warnings
        }
    }

    /// Create a tracker with default limits
    pub fn with_default_limits() -> Self {
        Self::new(SessionLimits::default())
    }

    /// Set warning ratio (default 0.75 = 75%)
    pub fn with_warning_ratio(mut self, ratio: f64) -> Self {
        self.warning_ratio = ratio.clamp(0.0, 1.0);
        self
    }

    /// Get current limits
    pub fn limits(&self) -> &SessionLimits {
        &self.limits
    }

    /// Get current turn count
    pub fn turns(&self) -> u32 {
        self.turns
    }

    /// Get elapsed time since session start
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get tokens used
    pub fn tokens_used(&self) -> u64 {
        self.tokens_used
    }

    /// Get cost used
    pub fn cost_used(&self) -> f64 {
        self.cost_used
    }

    /// Get consecutive failure count
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Get total failure count
    pub fn total_failures(&self) -> u32 {
        self.total_failures
    }

    /// Record a new turn (after tool execution)
    pub fn record_turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
    }

    /// Record tokens used
    pub fn record_tokens(&mut self, tokens: u64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
    }

    /// Record cost
    pub fn record_cost(&mut self, cost: f64) {
        self.cost_used += cost;
    }

    /// Record from a cost record
    pub fn record_cost_record(&mut self, record: &CostRecord) {
        self.record_tokens(record.total_tokens());
        self.record_cost(record.cost_usd);
    }

    /// Record from a cost summary
    pub fn record_cost_summary(&mut self, summary: &CostSummary) {
        self.record_tokens(summary.total_tokens);
        self.record_cost(summary.total_cost_usd);
    }

    /// Record a successful operation (resets consecutive failures)
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Record a failure
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.total_failures = self.total_failures.saturating_add(1);
    }

    /// Check if within limits and return detailed results
    pub fn check_limits(&self) -> ResourceLimitResult {
        let turns_current = self.turns as u64;
        let wall_clock_current = self.start_time.elapsed().as_secs();
        let tokens_current = self.tokens_used;
        let cost_current = (self.cost_used * 100.0) as u64; // Convert to cents for integer comparison
        let failures_current = self.consecutive_failures as u64;

        let cost_limit = (self.limits.cost_cap * 100.0) as u64;

        ResourceLimitResult {
            turns: LimitCheckResult::new(
                "turns",
                turns_current,
                self.limits.max_turns as u64,
                self.warning_ratio,
            ),
            wall_clock: LimitCheckResult::new(
                "wall_clock",
                wall_clock_current,
                self.limits.wall_clock_timeout.as_secs(),
                self.warning_ratio,
            ),
            tokens: LimitCheckResult::new(
                "tokens",
                tokens_current,
                self.limits.token_cap,
                self.warning_ratio,
            ),
            cost: LimitCheckResult::new("cost", cost_current, cost_limit, self.warning_ratio),
            failures: LimitCheckResult::new(
                "failures",
                failures_current,
                self.limits.failure_threshold as u64,
                self.warning_ratio,
            ),
        }
    }

    /// Check if any limit is exceeded (fast check without details)
    pub fn is_any_limit_exceeded(&self) -> bool {
        self.turns >= self.limits.max_turns
            || self.start_time.elapsed() > self.limits.wall_clock_timeout
            || self.tokens_used >= self.limits.token_cap
            || self.cost_used >= self.limits.cost_cap
            || self.consecutive_failures >= self.limits.failure_threshold
    }

    /// Get the first error encountered (for error reporting)
    pub fn get_first_error(&self) -> Option<ResourceLimitError> {
        if self.turns >= self.limits.max_turns {
            return Some(ResourceLimitError::MaxTurnsExceeded {
                current: self.turns,
                limit: self.limits.max_turns,
            });
        }

        let elapsed = self.start_time.elapsed();
        if elapsed > self.limits.wall_clock_timeout {
            return Some(ResourceLimitError::WallClockTimeoutExceeded {
                elapsed,
                limit: self.limits.wall_clock_timeout,
            });
        }

        if self.tokens_used >= self.limits.token_cap {
            return Some(ResourceLimitError::TokenCapExceeded {
                current: self.tokens_used,
                limit: self.limits.token_cap,
            });
        }

        if self.cost_used >= self.limits.cost_cap {
            return Some(ResourceLimitError::CostCapExceeded {
                current: self.cost_used,
                limit: self.limits.cost_cap,
            });
        }

        if self.consecutive_failures >= self.limits.failure_threshold {
            return Some(ResourceLimitError::FailureThresholdExceeded {
                current: self.consecutive_failures,
                limit: self.limits.failure_threshold,
            });
        }

        None
    }

    /// Get all exceeded errors (for comprehensive error reporting)
    pub fn get_all_errors(&self) -> Vec<ResourceLimitError> {
        let mut errors = Vec::new();

        if self.turns >= self.limits.max_turns {
            errors.push(ResourceLimitError::MaxTurnsExceeded {
                current: self.turns,
                limit: self.limits.max_turns,
            });
        }

        let elapsed = self.start_time.elapsed();
        if elapsed > self.limits.wall_clock_timeout {
            errors.push(ResourceLimitError::WallClockTimeoutExceeded {
                elapsed,
                limit: self.limits.wall_clock_timeout,
            });
        }

        if self.tokens_used >= self.limits.token_cap {
            errors.push(ResourceLimitError::TokenCapExceeded {
                current: self.tokens_used,
                limit: self.limits.token_cap,
            });
        }

        if self.cost_used >= self.limits.cost_cap {
            errors.push(ResourceLimitError::CostCapExceeded {
                current: self.cost_used,
                limit: self.limits.cost_cap,
            });
        }

        if self.consecutive_failures >= self.limits.failure_threshold {
            errors.push(ResourceLimitError::FailureThresholdExceeded {
                current: self.consecutive_failures,
                limit: self.limits.failure_threshold,
            });
        }

        errors
    }

    /// Reset the tracker for a new session
    /// Note: total_failures is preserved across resets as it's a running total
    pub fn reset(&mut self) {
        self.turns = 0;
        self.start_time = Instant::now();
        self.tokens_used = 0;
        self.cost_used = 0.0;
        self.consecutive_failures = 0;
        // total_failures is intentionally NOT reset - it's a cumulative counter
    }

    /// Reset only failures (keep turn/time tracking)
    pub fn reset_failures(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Get session duration
    pub fn session_duration(&self) -> Duration {
        self.start_time.elapsed()
    }
}

impl Default for SessionResourceTracker {
    fn default() -> Self {
        Self::with_default_limits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_limits_default() {
        let limits = SessionLimits::default();
        assert_eq!(limits.max_turns, 100);
        assert_eq!(limits.wall_clock_timeout, Duration::from_secs(3600));
        assert_eq!(limits.token_cap, 1_000_000);
        assert_eq!(limits.cost_cap, 50.0);
        assert_eq!(limits.failure_threshold, 5);
    }

    #[test]
    fn test_session_limits_builder() {
        let limits = SessionLimits::new()
            .with_max_turns(50)
            .with_wall_clock_timeout(Duration::from_secs(1800))
            .with_token_cap(500_000)
            .with_cost_cap(25.0)
            .with_failure_threshold(3);

        assert_eq!(limits.max_turns, 50);
        assert_eq!(limits.wall_clock_timeout, Duration::from_secs(1800));
        assert_eq!(limits.token_cap, 500_000);
        assert_eq!(limits.cost_cap, 25.0);
        assert_eq!(limits.failure_threshold, 3);
    }

    #[test]
    fn test_limit_state() {
        assert!(LimitState::Ok.is_ok());
        assert!(!LimitState::Ok.is_exceeded());

        assert!(LimitState::Warning.is_ok());
        assert!(!LimitState::Warning.is_exceeded());

        assert!(!LimitState::Exceeded.is_ok());
        assert!(LimitState::Exceeded.is_exceeded());
    }

    #[test]
    fn test_limit_check_result() {
        let result = LimitCheckResult::new("test", 50, 100, 0.75);
        assert_eq!(result.state, LimitState::Ok);
        assert_eq!(result.current, 50);
        assert_eq!(result.limit, 100);

        // At warning threshold (75%)
        let result = LimitCheckResult::new("test", 75, 100, 0.75);
        assert_eq!(result.state, LimitState::Warning);

        // At exceeded (100%)
        let result = LimitCheckResult::new("test", 100, 100, 0.75);
        assert_eq!(result.state, LimitState::Exceeded);

        // Over exceeded
        let result = LimitCheckResult::new("test", 150, 100, 0.75);
        assert_eq!(result.state, LimitState::Exceeded);
    }

    #[test]
    fn test_resource_limit_result_exceeded() {
        let result = ResourceLimitResult {
            turns: LimitCheckResult::exceeded("turns", 100, 100),
            wall_clock: LimitCheckResult::new("wall_clock", 100, 3600, 0.75),
            tokens: LimitCheckResult::new("tokens", 100, 1_000_000, 0.75),
            cost: LimitCheckResult::new("cost", 100, 5000, 0.75),
            failures: LimitCheckResult::new("failures", 0, 5, 0.75),
        };

        assert!(result.is_any_exceeded());
        assert!(!result.has_warning());
        assert_eq!(result.exceeded_limits(), vec!["turns"]);
    }

    #[test]
    fn test_resource_limit_result_warning() {
        let result = ResourceLimitResult {
            turns: LimitCheckResult::new("turns", 80, 100, 0.75),
            wall_clock: LimitCheckResult::new("wall_clock", 3500, 3600, 0.75), // 97% = Warning (not Exceeded)
            tokens: LimitCheckResult::new("tokens", 750_000, 1_000_000, 0.75),
            cost: LimitCheckResult::new("cost", 4000, 5000, 0.75),
            failures: LimitCheckResult::new("failures", 3, 5, 0.75),
        };

        assert!(!result.is_any_exceeded());
        assert!(result.has_warning());
    }

    #[test]
    fn test_session_resource_tracker_record_turn() {
        let mut tracker = SessionResourceTracker::with_default_limits();
        assert_eq!(tracker.turns(), 0);

        tracker.record_turn();
        assert_eq!(tracker.turns(), 1);

        for _ in 0..9 {
            tracker.record_turn();
        }
        assert_eq!(tracker.turns(), 10);
    }

    #[test]
    fn test_session_resource_tracker_record_tokens() {
        let mut tracker = SessionResourceTracker::with_default_limits();
        assert_eq!(tracker.tokens_used(), 0);

        tracker.record_tokens(1000);
        assert_eq!(tracker.tokens_used(), 1000);

        tracker.record_tokens(500);
        assert_eq!(tracker.tokens_used(), 1500);
    }

    #[test]
    fn test_session_resource_tracker_record_cost() {
        let mut tracker = SessionResourceTracker::with_default_limits();
        assert_eq!(tracker.cost_used(), 0.0);

        tracker.record_cost(5.50);
        assert!((tracker.cost_used() - 5.50).abs() < 0.001);

        tracker.record_cost(2.25);
        assert!((tracker.cost_used() - 7.75).abs() < 0.001);
    }

    #[test]
    fn test_session_resource_tracker_failures() {
        let mut tracker = SessionResourceTracker::with_default_limits();
        assert_eq!(tracker.consecutive_failures(), 0);
        assert_eq!(tracker.total_failures(), 0);

        tracker.record_failure();
        assert_eq!(tracker.consecutive_failures(), 1);
        assert_eq!(tracker.total_failures(), 1);

        tracker.record_failure();
        tracker.record_failure();
        assert_eq!(tracker.consecutive_failures(), 3);
        assert_eq!(tracker.total_failures(), 3);

        // Success resets consecutive but not total
        tracker.record_success();
        assert_eq!(tracker.consecutive_failures(), 0);
        assert_eq!(tracker.total_failures(), 3);

        tracker.record_failure();
        assert_eq!(tracker.consecutive_failures(), 1);
        assert_eq!(tracker.total_failures(), 4);
    }

    #[test]
    fn test_session_resource_tracker_is_any_limit_exceeded() {
        let limits = SessionLimits::new()
            .with_max_turns(10)
            .with_wall_clock_timeout(Duration::from_secs(3600))
            .with_token_cap(1000)
            .with_cost_cap(10.0)
            .with_failure_threshold(3);

        let mut tracker = SessionResourceTracker::new(limits);

        // Initially no limits exceeded
        assert!(!tracker.is_any_limit_exceeded());

        // Turns exceeded
        for _ in 0..10 {
            tracker.record_turn();
        }
        assert!(tracker.is_any_limit_exceeded());

        // Reset and test failures
        tracker.reset();
        assert!(!tracker.is_any_limit_exceeded());

        for _ in 0..3 {
            tracker.record_failure();
        }
        assert!(tracker.is_any_limit_exceeded());
    }

    #[test]
    fn test_session_resource_tracker_get_first_error() {
        let limits = SessionLimits::new()
            .with_max_turns(5)
            .with_failure_threshold(2);

        let mut tracker = SessionResourceTracker::new(limits);

        // No errors initially
        assert!(tracker.get_first_error().is_none());

        // Record failures - should get failure threshold error first (checked before turns)
        tracker.record_failure();
        tracker.record_failure();
        let error = tracker.get_first_error().unwrap();
        assert!(matches!(
            error,
            ResourceLimitError::FailureThresholdExceeded { .. }
        ));

        // Reset and test turns
        tracker.reset();
        for _ in 0..5 {
            tracker.record_turn();
        }
        let error = tracker.get_first_error().unwrap();
        assert!(matches!(error, ResourceLimitError::MaxTurnsExceeded { .. }));
    }

    #[test]
    fn test_session_resource_tracker_get_all_errors() {
        let limits = SessionLimits::new()
            .with_max_turns(5)
            .with_failure_threshold(2);

        let mut tracker = SessionResourceTracker::new(limits);

        // Exceed both limits
        for _ in 0..5 {
            tracker.record_turn();
        }
        for _ in 0..2 {
            tracker.record_failure();
        }

        let errors = tracker.get_all_errors();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_session_resource_tracker_reset() {
        let mut tracker = SessionResourceTracker::with_default_limits();

        tracker.record_turn();
        tracker.record_tokens(1000);
        tracker.record_cost(5.0);
        tracker.record_failure();

        assert_eq!(tracker.turns(), 1);
        assert_eq!(tracker.tokens_used(), 1000);
        assert!(tracker.cost_used() > 0.0);
        assert_eq!(tracker.consecutive_failures(), 1);

        tracker.reset();

        assert_eq!(tracker.turns(), 0);
        assert_eq!(tracker.tokens_used(), 0);
        assert_eq!(tracker.cost_used(), 0.0);
        assert_eq!(tracker.consecutive_failures(), 0);
        assert_eq!(tracker.total_failures(), 1); // Total failures preserved
    }

    #[test]
    fn test_resource_limit_error_display() {
        let error = ResourceLimitError::MaxTurnsExceeded {
            current: 100,
            limit: 50,
        };
        assert!(error.to_string().contains("Max turns exceeded"));

        let error = ResourceLimitError::WallClockTimeoutExceeded {
            elapsed: Duration::from_secs(3700),
            limit: Duration::from_secs(3600),
        };
        assert!(error.to_string().contains("Wall-clock timeout exceeded"));

        let error = ResourceLimitError::TokenCapExceeded {
            current: 1_000_000,
            limit: 500_000,
        };
        assert!(error.to_string().contains("Token cap exceeded"));

        let error = ResourceLimitError::CostCapExceeded {
            current: 10.0,
            limit: 5.0,
        };
        assert!(error.to_string().contains("Cost cap exceeded"));

        let error = ResourceLimitError::FailureThresholdExceeded {
            current: 5,
            limit: 3,
        };
        assert!(error.to_string().contains("Failure threshold exceeded"));
    }

    #[test]
    fn test_resource_limit_error_limit_name() {
        assert_eq!(
            ResourceLimitError::MaxTurnsExceeded {
                current: 100,
                limit: 50
            }
            .limit_name(),
            "turns"
        );
        assert_eq!(
            ResourceLimitError::WallClockTimeoutExceeded {
                elapsed: Duration::from_secs(3700),
                limit: Duration::from_secs(3600)
            }
            .limit_name(),
            "wall_clock"
        );
        assert_eq!(
            ResourceLimitError::TokenCapExceeded {
                current: 1_000_000,
                limit: 500_000
            }
            .limit_name(),
            "tokens"
        );
        assert_eq!(
            ResourceLimitError::CostCapExceeded {
                current: 10.0,
                limit: 5.0
            }
            .limit_name(),
            "cost"
        );
        assert_eq!(
            ResourceLimitError::FailureThresholdExceeded {
                current: 5,
                limit: 3
            }
            .limit_name(),
            "failures"
        );
    }

    #[tokio::test]
    async fn test_session_resource_tracker_with_cost_record() {
        use swell_core::ids::TaskId;

        let mut tracker = SessionResourceTracker::with_default_limits();

        let record = CostRecord::new(TaskId::new(), "claude-3-5-sonnet".to_string(), 1000, 500);

        tracker.record_cost_record(&record);

        assert_eq!(tracker.tokens_used(), 1500);
        assert!(tracker.cost_used() > 0.0);
    }

    #[tokio::test]
    async fn test_session_resource_tracker_with_cost_summary() {
        use swell_core::cost_tracking::CostSummary;

        let mut tracker = SessionResourceTracker::with_default_limits();

        let mut summary = CostSummary::new();
        summary.total_tokens = 5000;
        summary.total_cost_usd = 0.50;

        tracker.record_cost_summary(&summary);

        assert_eq!(tracker.tokens_used(), 5000);
        assert!((tracker.cost_used() - 0.50).abs() < 0.001);
    }
}
