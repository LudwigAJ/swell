//! Circuit breaker for failure handling.
//!
//! This module implements the circuit breaker pattern with three states:
//! - **Closed**: Normal operation, requests pass through. Failures are tracked.
//!   When failures exceed the threshold, the circuit transitions to Open.
//! - **Open**: The circuit is tripped, requests fail immediately without attempting.
//!   After the recovery timeout, the circuit transitions to Half-Open.
//! - **Half-Open**: Limited requests are allowed to test if the service has recovered.
//!   If enough consecutive successes occur, transition to Closed.
//!   If any failure occurs, transition back to Open.
//!
//! # Configuration
//!
//! - `failure_threshold`: Number of failures before opening the circuit (default: 5)
//! - `recovery_timeout`: Time to wait before attempting recovery (default: 60 seconds)
//! - `half_open_max_requests`: Max requests allowed in half-open state (default: 3)
//! - `half_open_success_threshold`: Consecutive successes needed to close circuit (default: 2)

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation, requests pass through
    Closed,
    /// Circuit is tripped, requests fail immediately
    Open,
    /// Testing recovery, limited requests allowed
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: u32,
    /// Time to wait before attempting recovery (in seconds)
    pub recovery_timeout_secs: u64,
    /// Maximum requests allowed in half-open state
    pub half_open_max_requests: u32,
    /// Consecutive successes needed to close the circuit
    pub half_open_success_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout_secs: 60,
            half_open_max_requests: 3,
            half_open_success_threshold: 2,
        }
    }
}

impl CircuitBreakerConfig {
    pub fn new(failure_threshold: u32, recovery_timeout_secs: u64) -> Self {
        Self {
            failure_threshold,
            recovery_timeout_secs,
            ..Default::default()
        }
    }

    pub fn with_half_open_config(
        mut self,
        max_requests: u32,
        success_threshold: u32,
    ) -> Self {
        self.half_open_max_requests = max_requests;
        self.half_open_success_threshold = success_threshold;
        self
    }
}

/// Circuit breaker state snapshot for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerSnapshot {
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_state_change_at: DateTime<Utc>,
    pub total_requests: u64,
    pub total_failures: u64,
    pub total_successes: u64,
}

impl CircuitBreakerSnapshot {
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_at: None,
            last_state_change_at: now,
            total_requests: 0,
            total_failures: 0,
            total_successes: 0,
        }
    }

    pub fn from_breaker(breaker: &CircuitBreaker) -> Self {
        Self {
            state: breaker.state,
            failure_count: breaker.failure_count,
            success_count: breaker.success_count,
            last_failure_at: breaker.last_failure_at,
            last_state_change_at: breaker.last_state_change_at,
            total_requests: breaker.total_requests,
            total_failures: breaker.total_failures,
            total_successes: breaker.total_successes,
        }
    }
}

impl Default for CircuitBreakerSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// Circuit breaker errors
#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError {
    #[error("Circuit is open: service unavailable")]
    CircuitOpen,

    #[error("Circuit is half-open: only {0} requests allowed")]
    HalfOpenCapacityLimited(u32),

    #[error("Circuit breaker not available")]
    NotAvailable,
}

impl CircuitBreakerError {
    pub fn is_open(&self) -> bool {
        matches!(self, CircuitBreakerError::CircuitOpen)
    }
}

/// The main circuit breaker guard
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    half_open_requests: u32,
    last_failure_at: Option<DateTime<Utc>>,
    last_state_change_at: DateTime<Utc>,
    total_requests: u64,
    total_failures: u64,
    total_successes: u64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default configuration
    pub fn new() -> Self {
        Self::with_config(CircuitBreakerConfig::default())
    }

    /// Create a new circuit breaker with custom configuration
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        let now = Utc::now();
        Self {
            config,
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            half_open_requests: 0,
            last_failure_at: None,
            last_state_change_at: now,
            total_requests: 0,
            total_failures: 0,
            total_successes: 0,
        }
    }

    /// Create a new circuit breaker with Arc<RwLock> wrapper for shared access
    pub fn new_arc() -> Arc<RwLock<CircuitBreaker>> {
        Arc::new(RwLock::new(Self::new()))
    }

    /// Create a new circuit breaker with config, wrapped in Arc<RwLock>
    pub fn with_config_arc(config: CircuitBreakerConfig) -> Arc<RwLock<CircuitBreaker>> {
        Arc::new(RwLock::new(Self::with_config(config)))
    }

    /// Get current state
    pub fn state(&self) -> CircuitState {
        self.state
    }

    /// Get current configuration
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Get a snapshot of the current state
    pub fn snapshot(&self) -> CircuitBreakerSnapshot {
        CircuitBreakerSnapshot::from_breaker(self)
    }

    /// Check if the circuit allows requests
    /// Returns Ok(()) if request can proceed, Err if blocked
    pub fn check(&self) -> Result<(), CircuitBreakerError> {
        match self.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                // Check if recovery timeout has elapsed
                let elapsed = Utc::now() - self.last_state_change_at;
                if elapsed >= Duration::seconds(self.config.recovery_timeout_secs as i64) {
                    // Transition to half-open
                    return Ok(());
                }
                Err(CircuitBreakerError::CircuitOpen)
            }
            CircuitState::HalfOpen => {
                if self.half_open_requests < self.config.half_open_max_requests {
                    Ok(())
                } else {
                    Err(CircuitBreakerError::HalfOpenCapacityLimited(
                        self.config.half_open_max_requests - self.half_open_requests,
                    ))
                }
            }
        }
    }

    /// Record a successful operation
    pub fn record_success(&mut self) {
        self.total_requests += 1;
        self.total_successes += 1;

        match self.state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.half_open_requests += 1;
                self.success_count += 1;

                // Check if we've reached success threshold
                if self.success_count >= self.config.half_open_success_threshold {
                    self.transition_to(CircuitState::Closed);
                }
            }
            CircuitState::Open => {
                // Should not happen - check() should prevent this
                // But handle gracefully
                self.total_requests -= 1;
                self.total_successes -= 1;
                self.total_failures -= 1;
            }
        }

        tracing::debug!(
            state = %self.state,
            failure_count = self.failure_count,
            success_count = self.success_count,
            "Circuit breaker success recorded"
        );
    }

    /// Record a failed operation
    pub fn record_failure(&mut self) {
        self.total_requests += 1;
        self.total_failures += 1;
        self.last_failure_at = Some(Utc::now());

        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.config.failure_threshold {
                    self.transition_to(CircuitState::Open);
                }
            }
            CircuitState::HalfOpen => {
                self.half_open_requests += 1;
                // Any failure in half-open state opens the circuit again
                self.transition_to(CircuitState::Open);
            }
            CircuitState::Open => {
                // Already open, just reset the timeout
                // (in case someone is repeatedly failing)
            }
        }

        tracing::debug!(
            state = %self.state,
            failure_count = self.failure_count,
            success_count = self.success_count,
            "Circuit breaker failure recorded"
        );
    }

    /// Force the circuit to a specific state (for testing or manual intervention)
    pub fn force_state(&mut self, new_state: CircuitState) {
        if new_state != self.state {
            tracing::warn!(
                from = %self.state,
                to = %new_state,
                "Circuit breaker manually forced"
            );
            self.transition_to(new_state);
        }
    }

    /// Reset the circuit breaker to initial closed state
    pub fn reset(&mut self) {
        self.transition_to(CircuitState::Closed);
        self.failure_count = 0;
        self.success_count = 0;
        self.half_open_requests = 0;
        self.last_failure_at = None;
        self.total_requests = 0;
        self.total_failures = 0;
        self.total_successes = 0;
    }

    /// Transition to a new state
    fn transition_to(&mut self, new_state: CircuitState) {
        if new_state != self.state {
            tracing::info!(
                from = %self.state,
                to = %new_state,
                "Circuit breaker state transition"
            );
            self.state = new_state;
            self.last_state_change_at = Utc::now();

            // Reset counters appropriate for the new state
            match new_state {
                CircuitState::Closed => {
                    self.failure_count = 0;
                    self.success_count = 0;
                    self.half_open_requests = 0;
                }
                CircuitState::HalfOpen => {
                    self.success_count = 0;
                    self.half_open_requests = 0;
                }
                CircuitState::Open => {
                    self.half_open_requests = 0;
                }
            }
        }
    }

    /// Check if the circuit is in a healthy (closed) state
    pub fn is_healthy(&self) -> bool {
        self.state == CircuitState::Closed
    }

    /// Check if the circuit is open (blocking requests)
    pub fn is_open(&self) -> bool {
        self.state == CircuitState::Open
    }

    /// Get time until next retry (for open circuits)
    pub fn time_until_retry(&self) -> Option<Duration> {
        if self.state != CircuitState::Open {
            return None;
        }
        let elapsed = Utc::now() - self.last_state_change_at;
        let timeout = Duration::seconds(self.config.recovery_timeout_secs as i64);
        if elapsed >= timeout {
            None
        } else {
            Some(timeout - elapsed)
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Async wrapper for CircuitBreaker with Arc<RwLock>
impl CircuitBreaker {
    /// Async check if request is allowed
    pub async fn check_async(breaker: &Arc<RwLock<CircuitBreaker>>) -> Result<(), CircuitBreakerError> {
        let b = breaker.read().await;
        b.check()
    }

    /// Async record success
    pub async fn record_success_async(breaker: &Arc<RwLock<CircuitBreaker>>) {
        let mut b = breaker.write().await;
        b.record_success();
    }

    /// Async record failure
    pub async fn record_failure_async(breaker: &Arc<RwLock<CircuitBreaker>>) {
        let mut b = breaker.write().await;
        b.record_failure();
    }

    /// Async get state
    pub async fn state_async(breaker: &Arc<RwLock<CircuitBreaker>>) -> CircuitState {
        let b = breaker.read().await;
        b.state()
    }

    /// Async get snapshot
    pub async fn snapshot_async(breaker: &Arc<RwLock<CircuitBreaker>>) -> CircuitBreakerSnapshot {
        let b = breaker.read().await;
        b.snapshot()
    }

    /// Async reset
    pub async fn reset_async(breaker: &Arc<RwLock<CircuitBreaker>>) {
        let mut b = breaker.write().await;
        b.reset();
    }

    /// Execute an async operation with circuit breaker protection
    pub async fn execute<F, Fut, T, E>(
        breaker: &Arc<RwLock<CircuitBreaker>>,
        operation: F,
    ) -> Result<T, CircuitBreakerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::error::Error,
    {
        // Check if circuit allows request
        Self::check_async(breaker).await?;

        // Execute the operation
        match operation().await {
            Ok(result) => {
                Self::record_success_async(breaker).await;
                Ok(result)
            }
            Err(_) => {
                Self::record_failure_async(breaker).await;
                // Determine the appropriate error based on circuit state
                let b = breaker.read().await;
                match b.state() {
                    CircuitState::Open => Err(CircuitBreakerError::CircuitOpen),
                    CircuitState::HalfOpen => Err(CircuitBreakerError::HalfOpenCapacityLimited(
                        b.config.half_open_max_requests - b.half_open_requests,
                    )),
                    CircuitState::Closed => {
                        // This shouldn't happen after record_failure, but for safety
                        Err(CircuitBreakerError::CircuitOpen)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_initial_state() {
        let breaker = CircuitBreaker::new();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.check().is_ok());
        assert!(breaker.is_healthy());
    }

    #[test]
    fn test_circuit_breaker_config_default() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert_eq!(config.recovery_timeout_secs, 60);
        assert_eq!(config.half_open_max_requests, 3);
        assert_eq!(config.half_open_success_threshold, 2);
    }

    #[test]
    fn test_circuit_breaker_config_custom() {
        let config = CircuitBreakerConfig::new(3, 30).with_half_open_config(5, 3);
        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.recovery_timeout_secs, 30);
        assert_eq!(config.half_open_max_requests, 5);
        assert_eq!(config.half_open_success_threshold, 3);
    }

    #[test]
    fn test_circuit_breaker_failure_tracking() {
        let config = CircuitBreakerConfig::new(3, 60); // threshold of 3
        let mut breaker = CircuitBreaker::with_config(config);

        // Record failures up to threshold
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.failure_count, 1);

        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.failure_count, 2);

        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        assert_eq!(breaker.failure_count, 3);
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let config = CircuitBreakerConfig::new(3, 60);
        let mut breaker = CircuitBreaker::with_config(config);

        // Fail until threshold
        for _ in 0..3 {
            breaker.record_failure();
        }

        assert!(breaker.is_open());
        assert!(breaker.check().is_err());
    }

    #[test]
    fn test_circuit_breaker_success_resets() {
        let config = CircuitBreakerConfig::new(3, 60);
        let mut breaker = CircuitBreaker::with_config(config);

        // Record some failures
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.failure_count, 2);

        // Success should reset
        breaker.record_success();
        assert_eq!(breaker.failure_count, 0);
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_snapshot() {
        let mut breaker = CircuitBreaker::new();
        breaker.record_failure();
        breaker.record_success();

        let snapshot = breaker.snapshot();
        assert_eq!(snapshot.state, CircuitState::Closed);
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.total_failures, 1);
        assert_eq!(snapshot.total_successes, 1);
    }

    #[test]
    fn test_circuit_breaker_half_open_transition() {
        let config = CircuitBreakerConfig::new(1, 1); // threshold of 1, 1 second timeout
        let mut breaker = CircuitBreaker::with_config(config);

        // Open the circuit
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Simulate time passing by forcing state
        breaker.force_state(CircuitState::HalfOpen);
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        assert_eq!(breaker.half_open_requests, 0);
    }

    #[test]
    fn test_circuit_breaker_half_open_success_closes() {
        let config = CircuitBreakerConfig::new(1, 60).with_half_open_config(3, 2);
        let mut breaker = CircuitBreaker::with_config(config);

        // Open and transition to half-open
        breaker.record_failure();
        breaker.force_state(CircuitState::HalfOpen);

        // Record successes until threshold
        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        assert_eq!(breaker.success_count, 1);

        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.success_count, 0); // Reset on transition
    }

    #[test]
    fn test_circuit_breaker_half_open_failure_reopens() {
        let config = CircuitBreakerConfig::new(1, 60);
        let mut breaker = CircuitBreaker::with_config(config);

        // Open and transition to half-open
        breaker.record_failure();
        breaker.force_state(CircuitState::HalfOpen);

        // Failure in half-open should reopen
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let config = CircuitBreakerConfig::new(3, 60);
        let mut breaker = CircuitBreaker::with_config(config);

        // Open the circuit
        for _ in 0..3 {
            breaker.record_failure();
        }
        assert!(breaker.is_open());

        // Reset should close it
        breaker.reset();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.check().is_ok());
        assert_eq!(breaker.total_requests, 0);
    }

    #[test]
    fn test_circuit_breaker_force_state() {
        let mut breaker = CircuitBreaker::new();

        breaker.force_state(CircuitState::Open);
        assert_eq!(breaker.state(), CircuitState::Open);

        breaker.force_state(CircuitState::HalfOpen);
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        breaker.force_state(CircuitState::Closed);
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_time_until_retry() {
        let config = CircuitBreakerConfig::new(1, 5); // 5 second timeout
        let mut breaker = CircuitBreaker::with_config(config);

        // Open the circuit
        breaker.record_failure();
        assert!(breaker.time_until_retry().is_some());

        // Force old timestamp to simulate time passing
        breaker.last_state_change_at = Utc::now() - chrono::Duration::seconds(10);
        assert!(breaker.time_until_retry().is_none());
    }

    #[test]
    fn test_circuit_breaker_error_display() {
        let err = CircuitBreakerError::CircuitOpen;
        assert_eq!(err.to_string(), "Circuit is open: service unavailable");
        assert!(err.is_open());

        let err = CircuitBreakerError::HalfOpenCapacityLimited(2);
        assert_eq!(err.to_string(), "Circuit is half-open: only 2 requests allowed");

        let err = CircuitBreakerError::NotAvailable;
        assert_eq!(err.to_string(), "Circuit breaker not available");
    }

    #[test]
    fn test_circuit_state_display() {
        assert_eq!(CircuitState::Closed.to_string(), "closed");
        assert_eq!(CircuitState::Open.to_string(), "open");
        assert_eq!(CircuitState::HalfOpen.to_string(), "half_open");
    }

    #[tokio::test]
    async fn test_circuit_breaker_async_operations() {
        let breaker = CircuitBreaker::new_arc();

        // Test check_async
        assert!(CircuitBreaker::check_async(&breaker).await.is_ok());

        // Test state_async
        assert_eq!(CircuitBreaker::state_async(&breaker).await, CircuitState::Closed);

        // Test record_success_async
        CircuitBreaker::record_success_async(&breaker).await;
        let snapshot = CircuitBreaker::snapshot_async(&breaker).await;
        assert_eq!(snapshot.total_successes, 1);

        // Test record_failure_async
        CircuitBreaker::record_failure_async(&breaker).await;
        let snapshot = CircuitBreaker::snapshot_async(&breaker).await;
        assert_eq!(snapshot.total_failures, 1);

        // Test reset_async
        CircuitBreaker::reset_async(&breaker).await;
        let snapshot = CircuitBreaker::snapshot_async(&breaker).await;
        assert_eq!(snapshot.total_requests, 0);
    }

    #[test]
    fn test_circuit_breaker_half_open_request_limit() {
        let config = CircuitBreakerConfig::new(1, 60).with_half_open_config(2, 2);
        let mut breaker = CircuitBreaker::with_config(config);

        // Open and transition to half-open
        breaker.record_failure();
        breaker.force_state(CircuitState::HalfOpen);

        // First request should be allowed
        assert!(breaker.check().is_ok());

        // Second request should be allowed
        assert!(breaker.check().is_ok());

        // Third request should be blocked
        breaker.half_open_requests = 2; // Simulate two requests already made
        let result = breaker.check();
        assert!(result.is_err());
        assert!(matches!(result, Err(CircuitBreakerError::HalfOpenCapacityLimited(_))));
    }

    #[test]
    fn test_circuit_breaker_serialization() {
        let snapshot = CircuitBreakerSnapshot::new();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("closed"));

        let deserialized: CircuitBreakerSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.state, CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_config_serialization() {
        let config = CircuitBreakerConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("failure_threshold"));

        let deserialized: CircuitBreakerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.failure_threshold, 5);
    }
}
