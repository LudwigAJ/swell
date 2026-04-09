//! Benchmark runner for executing benchmark suites

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::metrics::{BenchmarkMetrics, ProgressTracker, TaskOutcome};
use super::{BenchmarkSuite, BenchmarkTask};
use crate::{TaskId, TaskResult};

/// Configuration for benchmark execution
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Timeout for each task
    pub task_timeout: Duration,
    /// Maximum retries per task
    pub max_retries: u32,
    /// Whether to continue on failure
    pub continue_on_failure: bool,
    /// Number of concurrent tasks (if supported)
    pub concurrency: usize,
    /// Filter by category (None = all)
    pub category_filter: Option<Vec<String>>,
    /// Filter by difficulty (None = all)
    pub difficulty_filter: Option<Vec<String>>,
    /// Task IDs to run (None = all)
    pub task_filter: Option<Vec<String>>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            task_timeout: Duration::from_secs(300), // 5 minutes
            max_retries: 3,
            continue_on_failure: true,
            concurrency: 1,
            category_filter: None,
            difficulty_filter: None,
            task_filter: None,
        }
    }
}

impl RunnerConfig {
    /// Create a config that runs only specific tasks
    pub fn only_tasks(task_ids: Vec<&str>) -> Self {
        Self {
            task_filter: Some(task_ids.into_iter().map(String::from).collect()),
            ..Default::default()
        }
    }

    /// Create a config that excludes specific tasks
    pub fn exclude_tasks(mut self, task_ids: Vec<&str>) -> Self {
        // If we have a task filter, intersect with exclusions
        if let Some(ref mut filter) = self.task_filter {
            let exclusions: std::collections::HashSet<_> = task_ids.into_iter().collect();
            filter.retain(|id| !exclusions.contains(id.as_str()));
        } else {
            // Create a filter that excludes the specified tasks
            let all_tasks: Vec<_> = (0..100).map(|i| format!("task_{}", i)).collect();
            let exclusions: std::collections::HashSet<_> = task_ids.into_iter().collect();
            self.task_filter = Some(
                all_tasks
                    .into_iter()
                    .filter(|id| !exclusions.contains(id.as_str()))
                    .collect(),
            );
        }
        self
    }
}

/// Callback for progress updates
pub trait ProgressCallback: Send + Sync {
    fn on_task_start(&self, task: &BenchmarkTask);
    fn on_task_complete(&self, task_id: &TaskId, result: &TaskResult);
    fn on_progress(&self, progress: &ProgressTracker);
}

/// No-op progress callback
#[allow(dead_code)]
pub struct NoOpCallback;

impl ProgressCallback for NoOpCallback {
    fn on_task_start(&self, _task: &BenchmarkTask) {}
    fn on_task_complete(&self, _task_id: &TaskId, _result: &TaskResult) {}
    fn on_progress(&self, _progress: &ProgressTracker) {}
}

/// The benchmark runner
pub struct BenchmarkRunner {
    suite: BenchmarkSuite,
    config: RunnerConfig,
}

impl BenchmarkRunner {
    /// Create a new runner with the standard benchmark suite
    pub fn standard() -> Self {
        Self::new(BenchmarkSuite::standard())
    }

    /// Create a new runner with a custom suite
    pub fn new(suite: BenchmarkSuite) -> Self {
        Self {
            suite,
            config: RunnerConfig::default(),
        }
    }

    /// Set runner configuration
    pub fn with_config(mut self, config: RunnerConfig) -> Self {
        self.config = config;
        self
    }

    /// Get filtered tasks based on config
    fn filtered_tasks(&self) -> Vec<&BenchmarkTask> {
        self.suite
            .tasks
            .iter()
            .filter(|task| {
                // Filter by specific tasks if configured
                if let Some(ref filter) = self.config.task_filter {
                    if !filter.contains(&task.id.0) {
                        return false;
                    }
                }

                // Filter by category if configured
                if let Some(ref categories) = self.config.category_filter {
                    if !categories.contains(&task.category.to_string()) {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Run all filtered tasks sequentially
    pub async fn run<C: ProgressCallback>(&self, callback: &C) -> BenchmarkMetrics {
        let tasks = self.filtered_tasks();
        let total = tasks.len();
        let mut progress = ProgressTracker::new(total);
        let mut results = Vec::with_capacity(total);

        info!(total_tasks = total, "Starting benchmark run");

        for task in tasks {
            progress.current_task = Some(task.id.clone());
            callback.on_task_start(task);

            let result = self.execute_task(task, &mut progress).await;
            callback.on_task_complete(&task.id, &result);
            callback.on_progress(&progress);

            results.push(result);

            // Check if we should continue
            if !self.config.continue_on_failure {
                let last_result = results.last().unwrap();
                if !last_result.outcome.is_success() {
                    warn!("Stopping early due to failure");
                    break;
                }
            }
        }

        BenchmarkMetrics::from_results(results, total)
    }

    /// Execute a single task
    async fn execute_task(
        &self,
        task: &BenchmarkTask,
        progress: &mut ProgressTracker,
    ) -> TaskResult {
        let start = Instant::now();
        let mut retries = 0u32;

        loop {
            let result = self.run_task_with_timeout(task).await;

            match result {
                Ok(outcome) => {
                    let duration = start.elapsed().as_secs_f64();
                    let task_result = TaskResult {
                        task_id: task.id.clone(),
                        outcome,
                        duration_secs: duration,
                        retries,
                        notes: None,
                        completed_at: chrono::Utc::now(),
                    };
                    progress.record_completion(task.id.clone());
                    return task_result;
                }
                Err(_) => {
                    retries += 1;
                    if retries >= self.config.max_retries {
                        return TaskResult {
                            task_id: task.id.clone(),
                            outcome: TaskOutcome::Timeout,
                            duration_secs: start.elapsed().as_secs_f64(),
                            retries,
                            notes: Some(format!(
                                "Task timed out after {} retries",
                                self.config.max_retries
                            )),
                            completed_at: chrono::Utc::now(),
                        };
                    }
                    warn!(
                        task_id = %task.id,
                        retry = retries,
                        "Task failed, retrying"
                    );
                }
            }
        }
    }

    /// Run a task with timeout
    async fn run_task_with_timeout(
        &self,
        task: &BenchmarkTask,
    ) -> Result<TaskOutcome, ()> {
        tokio::time::timeout(self.config.task_timeout, self.simulate_task(task))
            .await
            .map_err(|_| ())
            .and_then(|r| r.map_err(|_| ()))
    }

    /// Simulate task execution
    /// In a real implementation, this would actually run the task through the orchestrator
    async fn simulate_task(&self, task: &BenchmarkTask) -> Result<TaskOutcome, ()> {
        // Simulate some work
        // In a real implementation, this would:
        // 1. Create a task in the orchestrator
        // 2. Execute the task through the agent pipeline
        // 3. Return the actual outcome

        // For now, we simulate a simple success/failure based on task characteristics
        // This allows the benchmark to be run without a full system setup

        // Simulate processing time based on difficulty
        let base_time = match task.difficulty {
            super::TaskDifficulty::Low => 100,
            super::TaskDifficulty::Medium => 200,
            super::TaskDifficulty::High => 400,
            super::TaskDifficulty::VeryHigh => 800,
        };

        tokio::time::sleep(Duration::from_millis(base_time)).await;

        // For benchmark purposes, tasks with "fix" in id are more likely to be bug fixes
        // which we simulate as having a success rate
        // In reality, this would be determined by actual execution

        // Always return success in simulation mode
        // Real execution would return actual results
        Ok(TaskOutcome::Completed)
    }

    /// Get the benchmark suite
    pub fn suite(&self) -> &BenchmarkSuite {
        &self.suite
    }

    /// Get the runner configuration
    pub fn config(&self) -> &RunnerConfig {
        &self.config
    }
}

/// Async benchmark runner that can be shared
#[allow(dead_code)]
pub struct AsyncBenchmarkRunner {
    inner: Arc<RwLock<BenchmarkRunner>>,
}

#[allow(dead_code)]
impl AsyncBenchmarkRunner {
    /// Create a new async runner
    pub fn new(runner: BenchmarkRunner) -> Self {
        Self {
            inner: Arc::new(RwLock::new(runner)),
        }
    }

    /// Run the benchmark
    pub async fn run<C: ProgressCallback>(&self, callback: &C) -> BenchmarkMetrics {
        let runner = self.inner.read().await;
        runner.run(callback).await
    }

    /// Update configuration
    pub async fn set_config(&self, config: RunnerConfig) {
        let mut runner = self.inner.write().await;
        *runner = BenchmarkRunner {
            suite: runner.suite.clone(),
            config,
        };
    }
}

impl Clone for BenchmarkRunner {
    fn clone(&self) -> Self {
        Self {
            suite: self.suite.clone(),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_creation() {
        let runner = BenchmarkRunner::standard();
        assert_eq!(runner.suite().tasks.len(), 50);
    }

    #[test]
    fn test_config_only_tasks() {
        let config = RunnerConfig::only_tasks(vec!["task1", "task2"]);
        assert_eq!(config.task_filter, Some(vec!["task1".to_string(), "task2".to_string()]));
    }

    #[tokio::test]
    async fn test_run_standard_suite() {
        let runner = BenchmarkRunner::standard();
        let metrics = runner.run(&NoOpCallback).await;

        assert_eq!(metrics.total_tasks, 50);
        // In simulation mode, all tasks should succeed
        assert_eq!(metrics.completed, 50);
    }

    #[tokio::test]
    async fn test_run_with_filter() {
        let runner = BenchmarkRunner::standard()
            .with_config(RunnerConfig::only_tasks(vec!["state_double_transition"]));

        let metrics = runner.run(&NoOpCallback).await;

        assert_eq!(metrics.total_tasks, 1);
    }
}
