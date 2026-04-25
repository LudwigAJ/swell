//! Alert system for monitoring threshold breaches, loop detection, consecutive failures,
//! cost thresholds, and policy violations.
//!
//! This module provides an AlertManager that tracks various alert conditions and generates
//! actionable alerts when thresholds are breached.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// Re-export alert types from metrics module
pub use super::metrics::{AlertSeverity, AlertThresholds, AlertType, MetricsAlert};

use swell_core::ids::{AgentId, TaskId};

/// Alert categories for the enhanced alert system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertCategory {
    /// Metrics-based alerts (from metrics module)
    Metrics,
    /// Loop detection alerts (ReAct loop issues)
    LoopDetection,
    /// Consecutive failure tracking
    ConsecutiveFailures,
    /// Cost threshold breaches
    CostThreshold,
    /// Policy violation alerts
    PolicyViolation,
}

/// Extended alert that includes category and actionable guidance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Unique alert ID
    pub id: Uuid,
    /// Alert category
    pub category: AlertCategory,
    /// Alert type within the category
    pub alert_type: AlertType,
    /// Severity level
    pub severity: AlertSeverity,
    /// Human-readable message
    pub message: String,
    /// Actionable guidance for resolving the alert
    pub action: String,
    /// The value that triggered the alert
    pub value: f64,
    /// The threshold that was breached
    pub threshold: f64,
    /// When the alert was triggered
    pub triggered_at: DateTime<Utc>,
    /// Associated task ID if applicable
    pub task_id: Option<TaskId>,
    /// Associated agent ID if applicable
    pub agent_id: Option<AgentId>,
}

impl Alert {
    /// Create a new alert
    pub fn new(
        category: AlertCategory,
        alert_type: AlertType,
        severity: AlertSeverity,
        message: String,
        action: String,
        value: f64,
        threshold: f64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            category,
            alert_type,
            severity,
            message,
            action,
            value,
            threshold,
            triggered_at: Utc::now(),
            task_id: None,
            agent_id: None,
        }
    }

    /// Set the task ID
    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }
}

/// Loop detection state for tracking potential infinite loops
#[derive(Debug, Clone)]
pub struct LoopDetectionState {
    /// Current iteration count for the loop
    pub iterations: u32,
    /// Last iteration where files were modified
    pub last_file_change_iteration: u32,
    /// Number of consecutive failures in the loop
    pub consecutive_failures: u32,
    /// Whether loop has converged
    pub converged: bool,
    /// Whether max iterations were reached
    pub max_iterations_reached: bool,
    /// Whether no-progress doom loop was detected
    pub no_progress_detected: bool,
}

/// Configuration for loop detection
#[derive(Debug, Clone)]
pub struct LoopDetectionConfig {
    /// Maximum iterations before warning
    pub max_iterations_warning: u32,
    /// Maximum iterations before critical alert
    pub max_iterations_critical: u32,
    /// No-progress threshold (iterations without file changes)
    pub no_progress_threshold: u32,
    /// Consecutive failure threshold
    pub consecutive_failure_threshold: u32,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            max_iterations_warning: 10,
            max_iterations_critical: 20,
            no_progress_threshold: 5,
            consecutive_failure_threshold: 3,
        }
    }
}

/// Configuration for cost threshold monitoring
#[derive(Debug, Clone)]
pub struct CostThresholdConfig {
    /// Maximum cost per task in tokens
    pub max_cost_per_task: u64,
    /// Maximum cost per task warning threshold (percentage of max)
    pub warning_threshold_pct: f64,
    /// Maximum total cost per run
    pub max_total_cost: u64,
    /// Maximum cost per agent per hour
    pub max_cost_per_agent_hour: u64,
}

impl Default for CostThresholdConfig {
    fn default() -> Self {
        Self {
            max_cost_per_task: 500_000,
            warning_threshold_pct: 0.75,
            max_total_cost: 10_000_000,
            max_cost_per_agent_hour: 1_000_000,
        }
    }
}

/// Configuration for consecutive failure tracking
#[derive(Debug, Clone)]
pub struct ConsecutiveFailureConfig {
    /// Number of consecutive failures before warning
    pub warning_threshold: u32,
    /// Number of consecutive failures before critical alert
    pub critical_threshold: u32,
    /// Number of consecutive failures before task escalation
    pub escalation_threshold: u32,
}

impl Default for ConsecutiveFailureConfig {
    fn default() -> Self {
        Self {
            warning_threshold: 2,
            critical_threshold: 3,
            escalation_threshold: 3,
        }
    }
}

/// Configuration for policy violation alerts
#[derive(Debug, Clone)]
pub struct PolicyViolationConfig {
    /// Whether to generate alerts on policy violations
    pub enabled: bool,
    /// Whether to alert on deny actions
    pub alert_on_deny: bool,
    /// Whether to alert on no-match (default deny)
    pub alert_on_no_match: bool,
}

impl Default for PolicyViolationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            alert_on_deny: true,
            alert_on_no_match: false,
        }
    }
}

/// Alert manager configuration
#[derive(Debug, Clone)]
pub struct AlertManagerConfig {
    /// Maximum alerts to keep in history
    pub max_alert_history: usize,
    /// Cooldown period between alerts of the same type (seconds)
    pub alert_cooldown_secs: u64,
    /// Loop detection configuration
    pub loop_detection: LoopDetectionConfig,
    /// Cost threshold configuration
    pub cost_threshold: CostThresholdConfig,
    /// Consecutive failure configuration
    pub consecutive_failure: ConsecutiveFailureConfig,
    /// Policy violation configuration
    pub policy_violation: PolicyViolationConfig,
}

impl Default for AlertManagerConfig {
    fn default() -> Self {
        Self {
            max_alert_history: 100,
            alert_cooldown_secs: 300, // 5 minutes
            loop_detection: LoopDetectionConfig::default(),
            cost_threshold: CostThresholdConfig::default(),
            consecutive_failure: ConsecutiveFailureConfig::default(),
            policy_violation: PolicyViolationConfig::default(),
        }
    }
}

/// Alert manager that tracks and generates alerts
pub struct AlertManager {
    /// Configuration
    config: AlertManagerConfig,
    /// Alert history
    alerts: VecDeque<Alert>,
    /// Consecutive failures per task
    task_failures: HashMap<TaskId, u32>,
    /// Consecutive failures globally
    global_failures: u32,
    /// Total cost accumulated
    total_cost: u64,
    /// Cost per task
    task_costs: HashMap<TaskId, u64>,
    /// Last alert time per alert type (for cooldown)
    last_alert_time: HashMap<AlertType, DateTime<Utc>>,
    /// Loop detection states per task
    loop_states: HashMap<TaskId, LoopDetectionState>,
}

impl AlertManager {
    /// Create a new alert manager with default configuration
    pub fn new() -> Self {
        Self::with_config(AlertManagerConfig::default())
    }

    /// Create a new alert manager with custom configuration
    pub fn with_config(config: AlertManagerConfig) -> Self {
        Self {
            config,
            alerts: VecDeque::with_capacity(100),
            task_failures: HashMap::new(),
            global_failures: 0,
            total_cost: 0,
            task_costs: HashMap::new(),
            last_alert_time: HashMap::new(),
            loop_states: HashMap::new(),
        }
    }

    /// Update configuration
    pub fn set_config(&mut self, config: AlertManagerConfig) {
        self.config = config;
    }

    /// Check if cooldown has passed for an alert type
    fn is_cooldown_passed(&self, alert_type: &AlertType) -> bool {
        if let Some(last_time) = self.last_alert_time.get(alert_type) {
            let elapsed = Utc::now().signed_duration_since(*last_time).num_seconds() as u64;
            return elapsed >= self.config.alert_cooldown_secs;
        }
        true
    }

    /// Record the time of an alert
    fn record_alert_time(&mut self, alert_type: &AlertType) {
        self.last_alert_time.insert(*alert_type, Utc::now());
    }

    /// Add an alert to the history
    fn add_alert(&mut self, alert: Alert) {
        // Enforce max history size
        while self.alerts.len() >= self.config.max_alert_history {
            self.alerts.pop_front();
        }
        self.alerts.push_back(alert);
    }

    // ========================================================================
    // Loop Detection
    // ========================================================================

    /// Initialize loop detection for a task
    pub fn init_loop_detection(&mut self, task_id: TaskId) {
        self.loop_states.insert(
            task_id,
            LoopDetectionState {
                iterations: 0,
                last_file_change_iteration: 0,
                consecutive_failures: 0,
                converged: false,
                max_iterations_reached: false,
                no_progress_detected: false,
            },
        );
    }

    /// Record a loop iteration
    pub fn record_loop_iteration(&mut self, task_id: TaskId) {
        if let Some(state) = self.loop_states.get_mut(&task_id) {
            state.iterations += 1;
        }
    }

    /// Record a file change in the loop
    pub fn record_file_change(&mut self, task_id: TaskId) {
        if let Some(state) = self.loop_states.get_mut(&task_id) {
            state.last_file_change_iteration = state.iterations;
            state.consecutive_failures = 0;
        }
    }

    /// Record a loop failure
    pub fn record_loop_failure(&mut self, task_id: TaskId) {
        if let Some(state) = self.loop_states.get_mut(&task_id) {
            state.consecutive_failures += 1;
        }
    }

    /// Record loop convergence
    pub fn record_loop_converged(&mut self, task_id: TaskId) {
        if let Some(state) = self.loop_states.get_mut(&task_id) {
            state.converged = true;
        }
    }

    /// Record max iterations reached
    pub fn record_max_iterations_reached(&mut self, task_id: TaskId) {
        if let Some(state) = self.loop_states.get_mut(&task_id) {
            state.max_iterations_reached = true;
        }
    }

    /// Check for loop detection alerts and return any triggered
    pub fn check_loop_alerts(&mut self, task_id: TaskId) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let config = &self.config.loop_detection;

        let state = match self.loop_states.get(&task_id) {
            Some(s) => s,
            None => return alerts,
        };

        // Check max iterations warning
        if state.iterations >= config.max_iterations_warning
            && state.iterations < config.max_iterations_critical
            && !state.converged
            && !state.max_iterations_reached
            && self.is_cooldown_passed(&AlertType::RetryRateHigh)
        {
            let alert = Alert::new(
                AlertCategory::LoopDetection,
                AlertType::RetryRateHigh,
                AlertSeverity::Warning,
                format!(
                    "Loop iterations ({}) approaching maximum ({})",
                    state.iterations, config.max_iterations_critical
                ),
                "Consider reviewing the loop logic. Task may be taking too long to converge."
                    .to_string(),
                state.iterations as f64,
                config.max_iterations_critical as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        }

        // Check max iterations critical
        if state.max_iterations_reached && self.is_cooldown_passed(&AlertType::RetryRateHigh) {
            let alert = Alert::new(
                AlertCategory::LoopDetection,
                AlertType::RetryRateHigh,
                AlertSeverity::Critical,
                format!(
                    "Loop reached maximum iterations ({}) without converging",
                    state.iterations
                ),
                "Task should be transitioned to failed state and potential retry initiated."
                    .to_string(),
                state.iterations as f64,
                config.max_iterations_critical as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        }

        // Check no-progress doom loop
        let iterations_without_progress = state.iterations - state.last_file_change_iteration;
        if iterations_without_progress >= config.no_progress_threshold
            && !state.converged
            && self.is_cooldown_passed(&AlertType::CompletionRateLow)
        {
            let alert = Alert::new(
                AlertCategory::LoopDetection,
                AlertType::CompletionRateLow,
                AlertSeverity::Critical,
                format!(
                    "No-progress doom loop detected: {} iterations without file changes",
                    iterations_without_progress
                ),
                "Task is stuck in a loop without making progress. Consider interrupting and reviewing approach."
                    .to_string(),
                iterations_without_progress as f64,
                config.no_progress_threshold as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        }

        // Check consecutive failures in loop
        if state.consecutive_failures >= config.consecutive_failure_threshold
            && self.is_cooldown_passed(&AlertType::ValidationFailureRateHigh)
        {
            let alert = Alert::new(
                AlertCategory::LoopDetection,
                AlertType::ValidationFailureRateHigh,
                AlertSeverity::Critical,
                format!(
                    "{} consecutive failures in ReAct loop",
                    state.consecutive_failures
                ),
                "Loop is failing repeatedly. Consider reviewing error handling and loop termination conditions."
                    .to_string(),
                state.consecutive_failures as f64,
                config.consecutive_failure_threshold as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        }

        // Record alert times and add to history
        for alert in &alerts {
            self.record_alert_time(&alert.alert_type);
            self.add_alert(alert.clone());
        }

        alerts
    }

    /// Remove loop state for a task
    pub fn remove_loop_state(&mut self, task_id: TaskId) {
        self.loop_states.remove(&task_id);
    }

    // ========================================================================
    // Consecutive Failures
    // ========================================================================

    /// Record a task failure
    pub fn record_task_failure(&mut self, task_id: TaskId) -> u32 {
        let count = self.task_failures.entry(task_id).or_insert(0);
        *count += 1;
        self.global_failures += 1;
        *count
    }

    /// Record a task success (resets failure count)
    pub fn record_task_success(&mut self, task_id: TaskId) {
        self.task_failures.remove(&task_id);
        self.global_failures = self.global_failures.saturating_sub(1);
    }

    /// Get consecutive failure count for a task
    pub fn get_task_failure_count(&self, task_id: TaskId) -> u32 {
        self.task_failures.get(&task_id).copied().unwrap_or(0)
    }

    /// Get global consecutive failure count
    pub fn get_global_failure_count(&self) -> u32 {
        self.global_failures
    }

    /// Check for consecutive failure alerts
    pub fn check_failure_alerts(&mut self, task_id: TaskId) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let count = self.get_task_failure_count(task_id);
        let config = &self.config.consecutive_failure;

        if count >= config.critical_threshold
            && self.is_cooldown_passed(&AlertType::ValidationFailureRateHigh)
        {
            let alert = Alert::new(
                AlertCategory::ConsecutiveFailures,
                AlertType::ValidationFailureRateHigh,
                AlertSeverity::Critical,
                format!(
                    "{} consecutive task failures (critical threshold: {})",
                    count, config.critical_threshold
                ),
                format!(
                    "Task has failed {} times consecutively. Manual review recommended or escalate to human.",
                    count
                ),
                count as f64,
                config.critical_threshold as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        } else if count >= config.warning_threshold
            && self.is_cooldown_passed(&AlertType::ValidationFailureRateHigh)
        {
            let alert = Alert::new(
                AlertCategory::ConsecutiveFailures,
                AlertType::ValidationFailureRateHigh,
                AlertSeverity::Warning,
                format!(
                    "{} consecutive task failures (warning threshold: {})",
                    count, config.warning_threshold
                ),
                "Task is experiencing repeated failures. Monitor closely.".to_string(),
                count as f64,
                config.warning_threshold as f64,
            )
            .with_task_id(task_id);
            alerts.push(alert);
        }

        for alert in &alerts {
            self.record_alert_time(&alert.alert_type);
            self.add_alert(alert.clone());
        }

        alerts
    }

    // ========================================================================
    // Cost Threshold
    // ========================================================================

    /// Record cost for a task
    pub fn record_task_cost(&mut self, task_id: TaskId, cost: u64) {
        let current = self.task_costs.entry(task_id).or_insert(0);
        *current += cost;
        self.total_cost += cost;
    }

    /// Get total cost for a task
    pub fn get_task_cost(&self, task_id: TaskId) -> u64 {
        self.task_costs.get(&task_id).copied().unwrap_or(0)
    }

    /// Get total accumulated cost
    pub fn get_total_cost(&self) -> u64 {
        self.total_cost
    }

    /// Check cost threshold alerts for a task
    pub fn check_cost_alerts(&mut self, task_id: TaskId) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let task_cost = self.get_task_cost(task_id);
        let config = &self.config.cost_threshold;

        // Check per-task cost
        if task_cost as f64 >= config.max_cost_per_task as f64 * config.warning_threshold_pct {
            if task_cost >= config.max_cost_per_task
                && self.is_cooldown_passed(&AlertType::CostPerTaskHigh)
            {
                let alert = Alert::new(
                    AlertCategory::CostThreshold,
                    AlertType::CostPerTaskHigh,
                    AlertSeverity::Critical,
                    format!(
                        "Task cost {} tokens exceeds maximum {} tokens",
                        task_cost, config.max_cost_per_task
                    ),
                    "Task has exceeded cost threshold. Consider terminating or reviewing token usage.".to_string(),
                    task_cost as f64,
                    config.max_cost_per_task as f64,
                )
                .with_task_id(task_id);
                alerts.push(alert);
            } else if self.is_cooldown_passed(&AlertType::CostPerTaskHigh) {
                let alert = Alert::new(
                    AlertCategory::CostThreshold,
                    AlertType::CostPerTaskHigh,
                    AlertSeverity::Warning,
                    format!(
                        "Task cost {} tokens approaching maximum {} tokens ({:.0}%)",
                        task_cost,
                        config.max_cost_per_task,
                        (task_cost as f64 / config.max_cost_per_task as f64) * 100.0
                    ),
                    "Task is approaching cost threshold. Monitor token usage closely.".to_string(),
                    task_cost as f64,
                    config.max_cost_per_task as f64,
                )
                .with_task_id(task_id);
                alerts.push(alert);
            }
        }

        // Check total cost
        if self.total_cost >= config.max_total_cost
            && self.is_cooldown_passed(&AlertType::CostPerTaskHigh)
        {
            let alert = Alert::new(
                AlertCategory::CostThreshold,
                AlertType::CostPerTaskHigh,
                AlertSeverity::Critical,
                format!(
                    "Total run cost {} tokens exceeds maximum {} tokens",
                    self.total_cost, config.max_total_cost
                ),
                "Run has exceeded total cost threshold. Consider stopping the run.".to_string(),
                self.total_cost as f64,
                config.max_total_cost as f64,
            );
            alerts.push(alert);
        }

        for alert in &alerts {
            self.record_alert_time(&alert.alert_type);
            self.add_alert(alert.clone());
        }

        alerts
    }

    /// Remove task cost tracking
    pub fn remove_task_cost(&mut self, task_id: TaskId) {
        self.task_costs.remove(&task_id);
    }

    // ========================================================================
    // Policy Violations
    // ========================================================================

    /// Record a policy violation
    pub fn record_policy_violation(
        &mut self,
        action_type: &str,
        reason: &str,
        task_id: Option<TaskId>,
        agent_id: Option<AgentId>,
    ) -> Option<Alert> {
        if !self.config.policy_violation.enabled {
            return None;
        }

        if !self.config.policy_violation.alert_on_deny {
            return None;
        }

        let alert = Alert::new(
            AlertCategory::PolicyViolation,
            AlertType::ValidationFailureRateHigh, // Reusing this alert type
            AlertSeverity::Warning,
            format!("Policy violation: {} - {}", action_type, reason),
            "Review the policy rules and ensure the action complies with configured policies."
                .to_string(),
            1.0,
            0.0,
        )
        .with_task_id(task_id.unwrap_or_else(TaskId::nil))
        .with_agent_id(agent_id.unwrap_or_default());

        self.record_alert_time(&alert.alert_type);
        self.add_alert(alert.clone());

        Some(alert)
    }

    // ========================================================================
    // Alert History
    // ========================================================================

    /// Get all alerts
    pub fn get_alerts(&self) -> Vec<Alert> {
        self.alerts.iter().cloned().collect()
    }

    /// Get recent alerts (last N)
    pub fn get_recent_alerts(&self, count: usize) -> Vec<Alert> {
        self.alerts.iter().rev().take(count).cloned().collect()
    }

    /// Get alerts by category
    pub fn get_alerts_by_category(&self, category: AlertCategory) -> Vec<Alert> {
        self.alerts
            .iter()
            .filter(|a| a.category == category)
            .cloned()
            .collect()
    }

    /// Get alerts by severity
    pub fn get_alerts_by_severity(&self, severity: AlertSeverity) -> Vec<Alert> {
        self.alerts
            .iter()
            .filter(|a| a.severity == severity)
            .cloned()
            .collect()
    }

    /// Clear all alerts
    pub fn clear_alerts(&mut self) {
        self.alerts.clear();
    }

    /// Clear alerts older than a timestamp
    pub fn clear_alerts_before(&mut self, before: DateTime<Utc>) {
        self.alerts.retain(|a| a.triggered_at > before);
    }
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper for alert manager
pub type SharedAlertManager = Arc<RwLock<AlertManager>>;

/// Create a new shared alert manager
pub fn create_alert_manager() -> SharedAlertManager {
    Arc::new(RwLock::new(AlertManager::new()))
}

/// Create with custom configuration
pub fn create_alert_manager_with_config(config: AlertManagerConfig) -> SharedAlertManager {
    Arc::new(RwLock::new(AlertManager::with_config(config)))
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new(
            AlertCategory::LoopDetection,
            AlertType::CompletionRateLow,
            AlertSeverity::Warning,
            "Test message".to_string(),
            "Take action".to_string(),
            10.0,
            5.0,
        );

        assert_eq!(alert.category, AlertCategory::LoopDetection);
        assert_eq!(alert.alert_type, AlertType::CompletionRateLow);
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.value, 10.0);
        assert_eq!(alert.threshold, 5.0);
        assert!(alert.task_id.is_none());

        let alert_with_task = alert.with_task_id(TaskId::new());
        assert!(alert_with_task.task_id.is_some());
    }

    #[test]
    fn test_loop_detection_state() {
        let state = LoopDetectionState {
            iterations: 10,
            last_file_change_iteration: 5,
            consecutive_failures: 2,
            converged: false,
            max_iterations_reached: false,
            no_progress_detected: false,
        };

        assert_eq!(state.iterations - state.last_file_change_iteration, 5);
    }

    #[test]
    fn test_alert_manager_loop_detection() {
        let mut manager = AlertManager::new();
        let task_id = TaskId::new();

        // Initialize loop
        manager.init_loop_detection(task_id);
        assert!(manager.loop_states.contains_key(&task_id));

        // Record iterations
        for _ in 0..12 {
            manager.record_loop_iteration(task_id);
        }

        // Record a file change
        manager.record_file_change(task_id);

        // Check alerts - should get warning about approaching max
        let _alerts = manager.check_loop_alerts(task_id);
        // May or may not have alerts depending on cooldown

        // Remove loop state
        manager.remove_loop_state(task_id);
        assert!(!manager.loop_states.contains_key(&task_id));
    }

    #[test]
    fn test_consecutive_failures() {
        let mut manager = AlertManager::new();
        let task_id = TaskId::new();

        // Record 2 failures
        assert_eq!(manager.record_task_failure(task_id), 1);
        assert_eq!(manager.record_task_failure(task_id), 2);

        // Check failure count
        assert_eq!(manager.get_task_failure_count(task_id), 2);

        // Record success - should reset
        manager.record_task_success(task_id);
        assert_eq!(manager.get_task_failure_count(task_id), 0);
    }

    #[test]
    fn test_cost_tracking() {
        let mut manager = AlertManager::new();
        let task_id = TaskId::new();

        // Record costs
        manager.record_task_cost(task_id, 100_000);
        manager.record_task_cost(task_id, 200_000);

        assert_eq!(manager.get_task_cost(task_id), 300_000);
        assert_eq!(manager.get_total_cost(), 300_000);

        // Remove task cost
        manager.remove_task_cost(task_id);
        // Total should still have the cost
        assert_eq!(manager.get_total_cost(), 300_000);
    }

    #[test]
    fn test_policy_violation() {
        let mut manager = AlertManager::new();

        let alert =
            manager.record_policy_violation("rm -rf /", "Destructive command denied", None, None);

        assert!(alert.is_some());
        let alert = alert.unwrap();
        assert_eq!(alert.category, AlertCategory::PolicyViolation);
    }

    #[test]
    fn test_alert_history() {
        let mut manager = AlertManager::new();

        // Create some alerts by triggering conditions
        let task_id = TaskId::new();
        manager.init_loop_detection(task_id);
        for _ in 0..25 {
            manager.record_loop_iteration(task_id);
        }
        manager.record_max_iterations_reached(task_id);

        // Check and collect alerts
        let alerts = manager.check_loop_alerts(task_id);
        for alert in alerts {
            manager.add_alert(alert);
        }

        // Get all alerts
        let all_alerts = manager.get_alerts();
        assert!(!all_alerts.is_empty());

        // Get by category
        let loop_alerts = manager.get_alerts_by_category(AlertCategory::LoopDetection);
        assert!(!loop_alerts.is_empty());

        // Get recent
        let recent = manager.get_recent_alerts(5);
        assert!(!recent.is_empty());

        // Clear alerts
        manager.clear_alerts();
        assert!(manager.get_alerts().is_empty());
    }

    #[test]
    fn test_config_defaults() {
        let config = AlertManagerConfig::default();
        assert_eq!(config.max_alert_history, 100);
        assert_eq!(config.alert_cooldown_secs, 300);

        let loop_config = LoopDetectionConfig::default();
        assert_eq!(loop_config.max_iterations_warning, 10);
        assert_eq!(loop_config.no_progress_threshold, 5);

        let cost_config = CostThresholdConfig::default();
        assert_eq!(cost_config.max_cost_per_task, 500_000);

        let failure_config = ConsecutiveFailureConfig::default();
        assert_eq!(failure_config.warning_threshold, 2);
        assert_eq!(failure_config.critical_threshold, 3);
    }
}
