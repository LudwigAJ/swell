//! Metrics module for tracking task execution metrics.
//!
//! Tracks:
//! - Completion rate: ratio of completed (accepted) tasks to total tasks
//! - Validation pass rate: ratio of tasks that passed validation
//! - Retry rate: average retry count per task
//! - Cost per task: average token cost per task
//! - Agent utilization: ratio of busy vs total agents over time
//!
//! Supports time aggregation and configurable alert thresholds.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use swell_core::ids::TaskId;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Time window for metrics aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricsWindow {
    /// Last hour
    Hour,
    /// Last 24 hours
    Day,
    /// Last 7 days
    Week,
    /// All-time
    AllTime,
}

impl MetricsWindow {
    /// Get the duration this window represents
    pub fn duration(&self) -> Option<Duration> {
        match self {
            MetricsWindow::Hour => Some(Duration::hours(1)),
            MetricsWindow::Day => Some(Duration::days(1)),
            MetricsWindow::Week => Some(Duration::weeks(1)),
            MetricsWindow::AllTime => None,
        }
    }

    /// Check if a timestamp falls within this window
    pub fn contains(&self, timestamp: DateTime<Utc>) -> bool {
        if let Some(duration) = self.duration() {
            let cutoff = Utc::now() - duration;
            timestamp > cutoff
        } else {
            true // AllTime contains everything
        }
    }
}

/// A single metric sample at a point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub timestamp: DateTime<Utc>,
    pub completion_rate: f64,
    pub validation_pass_rate: f64,
    pub avg_retry_rate: f64,
    pub avg_cost_per_task: f64,
    pub agent_utilization: f64,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub total_retries: u32,
    pub total_tokens: u64,
    pub active_agents: usize,
    pub total_agents: usize,
}

/// Aggregated metrics over a time window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedMetrics {
    pub window: MetricsWindow,
    pub completion_rate: f64,
    pub validation_pass_rate: f64,
    pub retry_rate: f64,
    pub cost_per_task: f64,
    pub agent_utilization: f64,
    pub total_tasks: usize,
    pub accepted_tasks: usize,
    pub rejected_tasks: usize,
    pub failed_tasks: usize,
    pub total_retries: u32,
    pub total_cost_tokens: u64,
    pub avg_task_duration_secs: f64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub sampled_at: DateTime<Utc>,
}

impl AggregatedMetrics {
    /// Create from samples, computing weighted average
    pub fn from_samples(samples: &[MetricSample], window: MetricsWindow) -> Self {
        if samples.is_empty() {
            let now = Utc::now();
            return Self {
                window,
                completion_rate: 0.0,
                validation_pass_rate: 0.0,
                retry_rate: 0.0,
                cost_per_task: 0.0,
                agent_utilization: 0.0,
                total_tasks: 0,
                accepted_tasks: 0,
                rejected_tasks: 0,
                failed_tasks: 0,
                total_retries: 0,
                total_cost_tokens: 0,
                avg_task_duration_secs: 0.0,
                window_start: now,
                window_end: now,
                sampled_at: now,
            };
        }

        let total_tasks: usize = samples.iter().map(|s| s.total_tasks).sum();
        let accepted_tasks = samples.iter().map(|s| s.completed_tasks).sum();
        let rejected_tasks = samples.iter().map(|s| s.failed_tasks).sum();
        let total_retries: u32 = samples.iter().map(|s| s.total_retries).sum();
        let total_tokens: u64 = samples.iter().map(|s| s.total_tokens).sum();

        // Weighted averages based on task count
        let completion_rate = if total_tasks > 0 {
            samples
                .iter()
                .map(|s| s.completion_rate * s.total_tasks as f64)
                .sum::<f64>()
                / total_tasks as f64
        } else {
            0.0
        };

        let validation_pass_rate = if total_tasks > 0 {
            samples
                .iter()
                .map(|s| s.validation_pass_rate * s.total_tasks as f64)
                .sum::<f64>()
                / total_tasks as f64
        } else {
            0.0
        };

        let retry_rate = if total_tasks > 0 {
            total_retries as f64 / total_tasks as f64
        } else {
            0.0
        };

        let cost_per_task = if total_tasks > 0 {
            total_tokens as f64 / total_tasks as f64
        } else {
            0.0
        };

        let agent_utilization = if samples.is_empty() {
            0.0
        } else {
            samples.iter().map(|s| s.agent_utilization).sum::<f64>() / samples.len() as f64
        };

        let window_start = samples
            .iter()
            .map(|s| s.timestamp)
            .min()
            .unwrap_or_else(Utc::now);
        let window_end = samples
            .iter()
            .map(|s| s.timestamp)
            .max()
            .unwrap_or_else(Utc::now);

        Self {
            window,
            completion_rate,
            validation_pass_rate,
            retry_rate,
            cost_per_task,
            agent_utilization,
            total_tasks,
            accepted_tasks,
            rejected_tasks,
            failed_tasks: rejected_tasks, // failed = rejected for now
            total_retries,
            total_cost_tokens: total_tokens,
            avg_task_duration_secs: 0.0, // Would need task duration tracking
            window_start,
            window_end,
            sampled_at: Utc::now(),
        }
    }
}

/// Alert threshold configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertThresholds {
    /// Minimum acceptable completion rate (0.0 to 1.0)
    pub min_completion_rate: f64,
    /// Maximum acceptable retry rate
    pub max_retry_rate: f64,
    /// Maximum acceptable cost per task (tokens)
    pub max_cost_per_task: f64,
    /// Minimum acceptable agent utilization (0.0 to 1.0)
    pub min_agent_utilization: f64,
    /// Maximum validation failure rate before alert (0.0 to 1.0)
    pub max_validation_failure_rate: f64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            min_completion_rate: 0.7,
            max_retry_rate: 0.5,
            max_cost_per_task: 500_000.0,
            min_agent_utilization: 0.3,
            max_validation_failure_rate: 0.3,
        }
    }
}

/// An active alert generated by threshold breach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsAlert {
    pub id: Uuid,
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub message: String,
    pub value: f64,
    pub threshold: f64,
    pub triggered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    CompletionRateLow,
    RetryRateHigh,
    CostPerTaskHigh,
    AgentUtilizationLow,
    ValidationFailureRateHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Warning,
    Critical,
}

/// Metrics collector that tracks system metrics over time
pub struct MetricsCollector {
    /// Circular buffer of recent samples (last hour at 1-minute resolution)
    samples: VecDeque<MetricSample>,
    /// Alert thresholds
    thresholds: AlertThresholds,
    /// Task start times for duration tracking
    task_start_times: std::collections::HashMap<TaskId, DateTime<Utc>>,
    /// Task token usage tracking
    task_costs: std::collections::HashMap<TaskId, u64>,
}

impl MetricsCollector {
    /// Create a new metrics collector with default thresholds
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(60), // 60 minutes of 1-minute samples
            thresholds: AlertThresholds::default(),
            task_start_times: std::collections::HashMap::new(),
            task_costs: std::collections::HashMap::new(),
        }
    }

    /// Create with custom thresholds
    pub fn with_thresholds(thresholds: AlertThresholds) -> Self {
        Self {
            samples: VecDeque::with_capacity(60),
            thresholds,
            task_start_times: std::collections::HashMap::new(),
            task_costs: std::collections::HashMap::new(),
        }
    }

    /// Update thresholds at runtime
    pub fn set_thresholds(&mut self, thresholds: AlertThresholds) {
        self.thresholds = thresholds;
    }

    /// Record a task starting
    pub fn record_task_start(&mut self, task_id: TaskId) {
        self.task_start_times.insert(task_id, Utc::now());
        self.task_costs.insert(task_id, 0);
    }

    /// Record token usage for a task
    pub fn record_task_cost(&mut self, task_id: TaskId, tokens: u64) {
        *self.task_costs.entry(task_id).or_insert(0) += tokens;
    }

    /// Record task completion
    pub fn record_task_completed(&mut self, task_id: TaskId, accepted: bool) {
        self.task_start_times.remove(&task_id);
        self.task_costs.remove(&task_id);
        let _ = accepted; // Could be used for additional tracking
    }

    /// Record task rejection/failure
    pub fn record_task_rejected(&mut self, task_id: TaskId) {
        self.task_start_times.remove(&task_id);
        self.task_costs.remove(&task_id);
    }

    /// Sample current metrics state
    pub fn sample(&self, metrics: &OrchestratorMetrics) -> MetricSample {
        MetricSample {
            timestamp: Utc::now(),
            completion_rate: metrics.completion_rate(),
            validation_pass_rate: metrics.validation_pass_rate(),
            avg_retry_rate: metrics.avg_retry_rate(),
            avg_cost_per_task: metrics.avg_cost_per_task(),
            agent_utilization: metrics.agent_utilization(),
            total_tasks: metrics.total_tasks,
            completed_tasks: metrics.accepted_tasks,
            failed_tasks: metrics.rejected_tasks,
            total_retries: metrics.total_retries,
            total_tokens: metrics.total_tokens,
            active_agents: metrics.active_agents,
            total_agents: metrics.total_agents,
        }
    }

    /// Add a sample to the history
    pub fn add_sample(&mut self, sample: MetricSample) {
        // Keep only last 60 minutes of samples
        while self.samples.len() >= 60 {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Get aggregated metrics for a time window
    pub fn get_aggregated(&self, window: MetricsWindow) -> AggregatedMetrics {
        let filtered: Vec<MetricSample> = self
            .samples
            .iter()
            .filter(|s| window.contains(s.timestamp))
            .cloned()
            .collect();

        AggregatedMetrics::from_samples(&filtered, window)
    }

    /// Check for threshold breaches and generate alerts
    pub fn check_thresholds(&self, metrics: &OrchestratorMetrics) -> Vec<MetricsAlert> {
        let mut alerts = Vec::new();

        let completion_rate = metrics.completion_rate();
        if completion_rate < self.thresholds.min_completion_rate {
            alerts.push(MetricsAlert {
                id: Uuid::new_v4(),
                alert_type: AlertType::CompletionRateLow,
                severity: if completion_rate < self.thresholds.min_completion_rate * 0.5 {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                },
                message: format!(
                    "Completion rate {:.1}% below threshold {:.1}%",
                    completion_rate * 100.0,
                    self.thresholds.min_completion_rate * 100.0
                ),
                value: completion_rate,
                threshold: self.thresholds.min_completion_rate,
                triggered_at: Utc::now(),
            });
        }

        let retry_rate = metrics.avg_retry_rate();
        if retry_rate > self.thresholds.max_retry_rate {
            alerts.push(MetricsAlert {
                id: Uuid::new_v4(),
                alert_type: AlertType::RetryRateHigh,
                severity: if retry_rate > self.thresholds.max_retry_rate * 2.0 {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                },
                message: format!(
                    "Retry rate {:.2} above threshold {:.2}",
                    retry_rate, self.thresholds.max_retry_rate
                ),
                value: retry_rate,
                threshold: self.thresholds.max_retry_rate,
                triggered_at: Utc::now(),
            });
        }

        let cost_per_task = metrics.avg_cost_per_task();
        if cost_per_task > self.thresholds.max_cost_per_task {
            alerts.push(MetricsAlert {
                id: Uuid::new_v4(),
                alert_type: AlertType::CostPerTaskHigh,
                severity: if cost_per_task > self.thresholds.max_cost_per_task * 2.0 {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                },
                message: format!(
                    "Cost per task {:.0} tokens above threshold {:.0}",
                    cost_per_task, self.thresholds.max_cost_per_task
                ),
                value: cost_per_task,
                threshold: self.thresholds.max_cost_per_task,
                triggered_at: Utc::now(),
            });
        }

        let utilization = metrics.agent_utilization();
        if utilization < self.thresholds.min_agent_utilization && metrics.total_agents > 0 {
            alerts.push(MetricsAlert {
                id: Uuid::new_v4(),
                alert_type: AlertType::AgentUtilizationLow,
                severity: AlertSeverity::Warning,
                message: format!(
                    "Agent utilization {:.1}% below threshold {:.1}%",
                    utilization * 100.0,
                    self.thresholds.min_agent_utilization * 100.0
                ),
                value: utilization,
                threshold: self.thresholds.min_agent_utilization,
                triggered_at: Utc::now(),
            });
        }

        let validation_failure_rate = 1.0 - metrics.validation_pass_rate();
        if validation_failure_rate > self.thresholds.max_validation_failure_rate {
            alerts.push(MetricsAlert {
                id: Uuid::new_v4(),
                alert_type: AlertType::ValidationFailureRateHigh,
                severity: if validation_failure_rate
                    > self.thresholds.max_validation_failure_rate * 2.0
                {
                    AlertSeverity::Critical
                } else {
                    AlertSeverity::Warning
                },
                message: format!(
                    "Validation failure rate {:.1}% above threshold {:.1}%",
                    validation_failure_rate * 100.0,
                    self.thresholds.max_validation_failure_rate * 100.0
                ),
                value: validation_failure_rate,
                threshold: self.thresholds.max_validation_failure_rate,
                triggered_at: Utc::now(),
            });
        }

        alerts
    }

    /// Get all recent samples
    pub fn get_samples(&self) -> Vec<MetricSample> {
        self.samples.iter().cloned().collect()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Orchestrator metrics snapshot - current state derived from orchestrator data
#[derive(Debug, Clone)]
pub struct OrchestratorMetrics {
    pub total_tasks: usize,
    pub accepted_tasks: usize,
    pub rejected_tasks: usize,
    pub executing_tasks: usize,
    pub pending_tasks: usize,
    pub total_retries: u32,
    pub total_tokens: u64,
    pub active_agents: usize,
    pub total_agents: usize,
}

impl OrchestratorMetrics {
    /// Calculate completion rate (accepted / total)
    pub fn completion_rate(&self) -> f64 {
        if self.total_tasks == 0 {
            0.0
        } else {
            self.accepted_tasks as f64 / self.total_tasks as f64
        }
    }

    /// Calculate validation pass rate (accepted / (accepted + rejected))
    pub fn validation_pass_rate(&self) -> f64 {
        let validated = self.accepted_tasks + self.rejected_tasks;
        if validated == 0 {
            0.0
        } else {
            self.accepted_tasks as f64 / validated as f64
        }
    }

    /// Calculate average retry rate (total retries / accepted tasks)
    pub fn avg_retry_rate(&self) -> f64 {
        if self.accepted_tasks == 0 {
            0.0
        } else {
            self.total_retries as f64 / self.accepted_tasks as f64
        }
    }

    /// Calculate average cost per task (total tokens / total tasks)
    pub fn avg_cost_per_task(&self) -> f64 {
        if self.total_tasks == 0 {
            0.0
        } else {
            self.total_tokens as f64 / self.total_tasks as f64
        }
    }

    /// Calculate agent utilization (active / total)
    pub fn agent_utilization(&self) -> f64 {
        if self.total_agents == 0 {
            0.0
        } else {
            self.active_agents as f64 / self.total_agents as f64
        }
    }
}

/// Thread-safe wrapper for metrics collector
pub type SharedMetricsCollector = Arc<RwLock<MetricsCollector>>;

/// Create a new shared metrics collector
pub fn create_metrics_collector() -> SharedMetricsCollector {
    Arc::new(RwLock::new(MetricsCollector::new()))
}

/// Create with custom thresholds
pub fn create_metrics_collector_with_thresholds(
    thresholds: AlertThresholds,
) -> SharedMetricsCollector {
    Arc::new(RwLock::new(MetricsCollector::with_thresholds(thresholds)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn create_sample(
        timestamp: DateTime<Utc>,
        total: usize,
        completed: usize,
        failed: usize,
        retries: u32,
        tokens: u64,
        active_agents: usize,
        total_agents: usize,
    ) -> MetricSample {
        let success_rate = if total > 0 {
            completed as f64 / total as f64
        } else {
            0.0
        };
        let valid_rate = if completed + failed > 0 {
            completed as f64 / (completed + failed) as f64
        } else {
            0.0
        };
        let retry_rate = if total > 0 {
            retries as f64 / total as f64
        } else {
            0.0
        };
        let cost = if total > 0 {
            tokens as f64 / total as f64
        } else {
            0.0
        };
        let utilization = if total_agents > 0 {
            active_agents as f64 / total_agents as f64
        } else {
            0.0
        };

        MetricSample {
            timestamp,
            completion_rate: success_rate,
            validation_pass_rate: valid_rate,
            avg_retry_rate: retry_rate,
            avg_cost_per_task: cost,
            agent_utilization: utilization,
            total_tasks: total,
            completed_tasks: completed,
            failed_tasks: failed,
            total_retries: retries,
            total_tokens: tokens,
            active_agents,
            total_agents,
        }
    }

    #[test]
    fn test_metrics_window_contains() {
        let now = Utc::now();
        let past_hour = now - Duration::minutes(30);
        let past_day = now - Duration::hours(25);

        assert!(MetricsWindow::Hour.contains(past_hour));
        assert!(!MetricsWindow::Hour.contains(past_day));
        assert!(MetricsWindow::AllTime.contains(past_day));
    }

    #[test]
    fn test_aggregated_metrics_empty() {
        let metrics = AggregatedMetrics::from_samples(&[], MetricsWindow::Hour);

        assert_eq!(metrics.total_tasks, 0);
        assert_eq!(metrics.completion_rate, 0.0);
        assert_eq!(metrics.validation_pass_rate, 0.0);
    }

    #[test]
    fn test_aggregated_metrics_weighted() {
        let samples = vec![
            create_sample(Utc::now(), 10, 8, 2, 3, 100_000, 3, 5),
            create_sample(Utc::now(), 10, 5, 5, 5, 200_000, 4, 5),
        ];

        let metrics = AggregatedMetrics::from_samples(&samples, MetricsWindow::Hour);

        // Weighted average: (0.8*10 + 0.5*10) / 20 = 13/20 = 0.65
        assert!((metrics.completion_rate - 0.65).abs() < 0.001);

        // Total tasks: 20
        assert_eq!(metrics.total_tasks, 20);
    }

    #[test]
    fn test_orchestrator_metrics_calculations() {
        let metrics = OrchestratorMetrics {
            total_tasks: 100,
            accepted_tasks: 70,
            rejected_tasks: 20,
            executing_tasks: 5,
            pending_tasks: 5,
            total_retries: 35,
            total_tokens: 10_000_000,
            active_agents: 3,
            total_agents: 5,
        };

        assert!((metrics.completion_rate() - 0.7).abs() < 0.001);
        assert!((metrics.validation_pass_rate() - 0.778).abs() < 0.01); // 70/90
        assert!((metrics.avg_retry_rate() - 0.5).abs() < 0.001); // 35/70
        assert!((metrics.avg_cost_per_task() - 100_000.0).abs() < 0.001);
        assert!((metrics.agent_utilization() - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_alert_thresholds_default() {
        let thresholds = AlertThresholds::default();

        assert_eq!(thresholds.min_completion_rate, 0.7);
        assert_eq!(thresholds.max_retry_rate, 0.5);
        assert_eq!(thresholds.max_cost_per_task, 500_000.0);
        assert_eq!(thresholds.min_agent_utilization, 0.3);
    }

    #[test]
    fn test_metrics_collector_sample() {
        let collector = MetricsCollector::new();

        let metrics = OrchestratorMetrics {
            total_tasks: 100,
            accepted_tasks: 70,
            rejected_tasks: 20,
            executing_tasks: 5,
            pending_tasks: 5,
            total_retries: 35,
            total_tokens: 10_000_000,
            active_agents: 3,
            total_agents: 5,
        };

        let sample = collector.sample(&metrics);

        assert!((sample.completion_rate - 0.7).abs() < 0.001);
        assert_eq!(sample.total_tasks, 100);
        assert_eq!(sample.active_agents, 3);
    }

    #[test]
    fn test_metrics_collector_add_sample() {
        let mut collector = MetricsCollector::new();

        let sample = create_sample(Utc::now(), 10, 8, 2, 3, 100_000, 3, 5);
        collector.add_sample(sample.clone());

        let samples = collector.get_samples();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].total_tasks, 10);
    }

    #[test]
    fn test_metrics_collector_task_tracking() {
        let mut collector = MetricsCollector::new();
        let task_id = TaskId::new();

        collector.record_task_start(task_id);
        collector.record_task_cost(task_id, 50_000);
        collector.record_task_cost(task_id, 30_000);
        collector.record_task_completed(task_id, true);

        // Task should be removed from tracking
        assert!(!collector.task_start_times.contains_key(&task_id));
        assert!(!collector.task_costs.contains_key(&task_id));
    }

    #[test]
    fn test_check_thresholds_completion_rate() {
        let collector = MetricsCollector::with_thresholds(AlertThresholds {
            min_completion_rate: 0.8,
            ..Default::default()
        });

        let metrics = OrchestratorMetrics {
            total_tasks: 100,
            accepted_tasks: 60, // 60% completion rate - below 80% threshold
            rejected_tasks: 30,
            executing_tasks: 5,
            pending_tasks: 5,
            total_retries: 30,
            total_tokens: 10_000_000,
            active_agents: 3,
            total_agents: 5,
        };

        let alerts = collector.check_thresholds(&metrics);
        assert!(!alerts.is_empty());
        assert!(alerts
            .iter()
            .any(|a| matches!(a.alert_type, AlertType::CompletionRateLow)));
    }

    #[test]
    fn test_check_thresholds_no_alert() {
        let collector = MetricsCollector::with_thresholds(AlertThresholds::default());

        let metrics = OrchestratorMetrics {
            total_tasks: 100,
            accepted_tasks: 90, // 90% - above 70% threshold
            rejected_tasks: 10,
            executing_tasks: 0,
            pending_tasks: 0,
            total_retries: 15,
            total_tokens: 5_000_000, // 50k per task - below 500k threshold
            active_agents: 5,
            total_agents: 5, // 100% utilization - above 30% threshold
        };

        let alerts = collector.check_thresholds(&metrics);
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_get_aggregated_filters_by_window() {
        let mut collector = MetricsCollector::new();

        let now = Utc::now();
        let past_hour = now - Duration::minutes(30);
        let past_day = now - Duration::hours(25);

        // Add samples from different times
        collector.add_sample(create_sample(now, 10, 8, 2, 2, 100_000, 5, 5));
        collector.add_sample(create_sample(past_hour, 10, 9, 1, 1, 80_000, 4, 5));
        collector.add_sample(create_sample(past_day, 10, 7, 3, 5, 120_000, 3, 5));

        // Hour window should only include recent samples
        let hour_metrics = collector.get_aggregated(MetricsWindow::Hour);
        assert_eq!(hour_metrics.total_tasks, 20);

        // AllTime should include all samples
        let all_metrics = collector.get_aggregated(MetricsWindow::AllTime);
        assert_eq!(all_metrics.total_tasks, 30);
    }
}
