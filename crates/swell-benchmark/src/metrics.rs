//! Benchmark metrics and tracking

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::task::TaskId;
use super::TaskResult;

/// Outcome of a task execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    /// Task completed successfully
    Completed,
    /// Task failed during execution
    Failed,
    /// Task was skipped (precondition not met, etc.)
    Skipped,
    /// Task timed out
    Timeout,
}

impl TaskOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, TaskOutcome::Completed)
    }
}

/// Comprehensive benchmark metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    /// Total tasks in the benchmark
    pub total_tasks: usize,
    /// Tasks completed successfully
    pub completed: usize,
    /// Tasks that failed
    pub failed: usize,
    /// Tasks that were skipped
    pub skipped: usize,
    /// Tasks that timed out
    pub timeouts: usize,
    /// Total time spent
    pub total_duration_secs: f64,
    /// Average time per task
    pub avg_duration_secs: f64,
    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,
    /// Completion rate (0.0 to 1.0, includes failed/skipped)
    pub completion_rate: f64,
    /// Results grouped by category
    pub by_category: HashMap<String, CategoryMetrics>,
    /// Results grouped by difficulty
    pub by_difficulty: HashMap<String, DifficultyMetrics>,
    /// Individual task results
    pub task_results: Vec<TaskResult>,
    /// When metrics were recorded
    pub recorded_at: DateTime<Utc>,
}

impl BenchmarkMetrics {
    /// Create new metrics from task results
    pub fn from_results(results: Vec<TaskResult>, total_tasks: usize) -> Self {
        let completed = results
            .iter()
            .filter(|r| r.outcome == TaskOutcome::Completed)
            .count();
        let failed = results
            .iter()
            .filter(|r| r.outcome == TaskOutcome::Failed)
            .count();
        let skipped = results
            .iter()
            .filter(|r| r.outcome == TaskOutcome::Skipped)
            .count();
        let timeouts = results
            .iter()
            .filter(|r| r.outcome == TaskOutcome::Timeout)
            .count();
        let total_duration_secs: f64 = results.iter().map(|r| r.duration_secs).sum();
        let avg_duration_secs = if results.is_empty() {
            0.0
        } else {
            total_duration_secs / results.len() as f64
        };

        let success_rate = if total_tasks == 0 {
            0.0
        } else {
            completed as f64 / total_tasks as f64
        };

        let non_skipped = completed + failed + timeouts;
        let completion_rate = if total_tasks == 0 {
            0.0
        } else {
            non_skipped as f64 / total_tasks as f64
        };

        let by_category = Self::compute_by_category(&results);
        let by_difficulty = Self::compute_by_difficulty(&results);

        Self {
            total_tasks,
            completed,
            failed,
            skipped,
            timeouts,
            total_duration_secs,
            avg_duration_secs,
            success_rate,
            completion_rate,
            by_category,
            by_difficulty,
            task_results: results,
            recorded_at: Utc::now(),
        }
    }

    fn compute_by_category(results: &[TaskResult]) -> HashMap<String, CategoryMetrics> {
        let mut metrics: HashMap<String, CategoryMetrics> = HashMap::new();

        for result in results {
            let cat_key = result.category.to_string();
            let entry = metrics.entry(cat_key).or_default();
            entry.record(result);
        }

        metrics
    }

    fn compute_by_difficulty(results: &[TaskResult]) -> HashMap<String, DifficultyMetrics> {
        let mut metrics: HashMap<String, DifficultyMetrics> = HashMap::new();

        for result in results {
            let diff_key = result.difficulty.to_string();
            let entry = metrics.entry(diff_key).or_default();
            entry.record(result);
        }

        metrics
    }

    /// Generate a summary report
    pub fn summary(&self) -> String {
        format!(
            "Benchmark Results:\n\
             ================\n\
             Total Tasks: {}\n\
             Completed: {} ({:.1}%)\n\
             Failed: {} ({:.1}%)\n\
             Skipped: {}\n\
             Timeouts: {}\n\
             \n\
             Success Rate: {:.1}%\n\
             Completion Rate: {:.1}%\n\
             Total Time: {:.2}s\n\
             Avg Time/Task: {:.2}s\n\
             \n\
             By Category:\n\
             {}\n\
             \n\
             By Difficulty:\n\
             {}",
            self.total_tasks,
            self.completed,
            self.success_rate * 100.0,
            self.failed,
            (self.failed as f64 / self.total_tasks as f64) * 100.0,
            self.skipped,
            self.timeouts,
            self.success_rate * 100.0,
            self.completion_rate * 100.0,
            self.total_duration_secs,
            self.avg_duration_secs,
            self.format_by_category(),
            self.format_by_difficulty()
        )
    }

    fn format_by_category(&self) -> String {
        if self.by_category.is_empty() {
            return "  (none)".to_string();
        }

        self.by_category
            .iter()
            .map(|(cat, metrics)| {
                format!(
                    "  {}: {} completed, {} failed, {:.1}% success",
                    cat,
                    metrics.completed,
                    metrics.failed,
                    metrics.success_rate() * 100.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_by_difficulty(&self) -> String {
        if self.by_difficulty.is_empty() {
            return "  (none)".to_string();
        }

        self.by_difficulty
            .iter()
            .map(|(diff, metrics)| {
                format!(
                    "  {}: {} completed, {} failed, {:.1}% success",
                    diff,
                    metrics.completed,
                    metrics.failed,
                    metrics.success_rate() * 100.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Export metrics as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Metrics aggregated by task category
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryMetrics {
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl CategoryMetrics {
    pub fn record(&mut self, result: &TaskResult) {
        match result.outcome {
            TaskOutcome::Completed => self.completed += 1,
            TaskOutcome::Failed => self.failed += 1,
            TaskOutcome::Skipped => self.skipped += 1,
            TaskOutcome::Timeout => {}
        }
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.completed + self.failed;
        if total == 0 {
            0.0
        } else {
            self.completed as f64 / total as f64
        }
    }
}

/// Metrics aggregated by difficulty level
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DifficultyMetrics {
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl DifficultyMetrics {
    pub fn record(&mut self, result: &TaskResult) {
        match result.outcome {
            TaskOutcome::Completed => self.completed += 1,
            TaskOutcome::Failed => self.failed += 1,
            TaskOutcome::Skipped => self.skipped += 1,
            TaskOutcome::Timeout => {}
        }
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.completed + self.failed;
        if total == 0 {
            0.0
        } else {
            self.completed as f64 / total as f64
        }
    }
}

/// Track progress through a benchmark run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressTracker {
    pub total: usize,
    pub completed: usize,
    pub current_task: Option<TaskId>,
    pub start_time: DateTime<Utc>,
}

impl ProgressTracker {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            completed: 0,
            current_task: None,
            start_time: Utc::now(),
        }
    }

    /// Update progress with a completed task
    pub fn record_completion(&mut self, task_id: TaskId) {
        self.completed += 1;
        self.current_task = Some(task_id);
    }

    /// Get current progress as percentage (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }

    /// Get estimated time remaining in seconds
    pub fn eta_secs(&self) -> Option<f64> {
        if self.completed == 0 {
            return None;
        }

        let elapsed = (Utc::now() - self.start_time).num_seconds() as f64;
        let avg_time = elapsed / self.completed as f64;
        let remaining = self.total - self.completed;

        Some(avg_time * remaining as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{TaskCategory, TaskDifficulty};

    #[test]
    fn test_task_outcome_is_success() {
        assert!(TaskOutcome::Completed.is_success());
        assert!(!TaskOutcome::Failed.is_success());
        assert!(!TaskOutcome::Skipped.is_success());
        assert!(!TaskOutcome::Timeout.is_success());
    }

    #[test]
    fn test_benchmark_metrics_empty() {
        let metrics = BenchmarkMetrics::from_results(vec![], 50);

        assert_eq!(metrics.total_tasks, 50);
        assert_eq!(metrics.completed, 0);
        assert_eq!(metrics.failed, 0);
        assert_eq!(metrics.success_rate, 0.0);
        assert_eq!(metrics.completion_rate, 0.0);
    }

    #[test]
    fn test_benchmark_metrics_all_success() {
        let results = vec![
            TaskResult::success(
                TaskId("task1".to_string()),
                TaskCategory::BugFix,
                TaskDifficulty::Low,
                10.0,
            ),
            TaskResult::success(
                TaskId("task2".to_string()),
                TaskCategory::Feature,
                TaskDifficulty::Medium,
                15.0,
            ),
            TaskResult::success(
                TaskId("task3".to_string()),
                TaskCategory::Refactoring,
                TaskDifficulty::High,
                20.0,
            ),
        ];

        let metrics = BenchmarkMetrics::from_results(results, 3);

        assert_eq!(metrics.completed, 3);
        assert_eq!(metrics.failed, 0);
        assert_eq!(metrics.success_rate, 1.0);
        assert_eq!(metrics.completion_rate, 1.0);
        assert_eq!(metrics.total_duration_secs, 45.0);
        assert_eq!(metrics.avg_duration_secs, 15.0);
    }

    #[test]
    fn test_benchmark_metrics_mixed() {
        let results = vec![
            TaskResult::success(
                TaskId("task1".to_string()),
                TaskCategory::BugFix,
                TaskDifficulty::Low,
                10.0,
            ),
            TaskResult::failed(
                TaskId("task2".to_string()),
                TaskCategory::Feature,
                TaskDifficulty::Medium,
                5.0,
                "Error",
            ),
            TaskResult::success(
                TaskId("task3".to_string()),
                TaskCategory::Test,
                TaskDifficulty::VeryHigh,
                20.0,
            ),
        ];

        let metrics = BenchmarkMetrics::from_results(results, 5);

        assert_eq!(metrics.total_tasks, 5);
        assert_eq!(metrics.completed, 2);
        assert_eq!(metrics.failed, 1);
        assert_eq!(metrics.skipped, 0);
        assert!(metrics.success_rate > 0.0);
    }

    #[test]
    fn test_progress_tracker() {
        let mut tracker = ProgressTracker::new(10);

        assert_eq!(tracker.progress(), 0.0);
        assert!(tracker.eta_secs().is_none());

        tracker.record_completion(TaskId("task1".to_string()));
        assert_eq!(tracker.progress(), 0.1);

        tracker.record_completion(TaskId("task2".to_string()));
        assert_eq!(tracker.progress(), 0.2);
    }

    #[test]
    fn test_category_metrics() {
        let mut metrics = CategoryMetrics::default();

        metrics.record(&TaskResult::success(
            TaskId("t1".to_string()),
            TaskCategory::BugFix,
            TaskDifficulty::Low,
            10.0,
        ));
        metrics.record(&TaskResult::success(
            TaskId("t2".to_string()),
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            10.0,
        ));
        metrics.record(&TaskResult::failed(
            TaskId("t3".to_string()),
            TaskCategory::BugFix,
            TaskDifficulty::High,
            10.0,
            "err",
        ));

        assert_eq!(metrics.completed, 2);
        assert_eq!(metrics.failed, 1);
        assert!((metrics.success_rate() - 2.0 / 3.0).abs() < 0.001);
    }
}
