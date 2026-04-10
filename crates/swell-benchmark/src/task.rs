//! Benchmark task definitions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single benchmark task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    /// Unique identifier for this task
    pub id: TaskId,
    /// Category of the task
    pub category: TaskCategory,
    /// Difficulty level
    pub difficulty: TaskDifficulty,
    /// Natural language description
    pub description: String,
    /// Success criteria that define when the task is complete
    pub success_criteria: Vec<String>,
    /// Files that are relevant to this task
    pub affected_files: Vec<String>,
    /// When the task was created
    pub created_at: DateTime<Utc>,
}

impl BenchmarkTask {
    /// Create a new benchmark task
    pub fn new(
        id: impl Into<String>,
        category: TaskCategory,
        difficulty: TaskDifficulty,
        description: impl Into<String>,
        success_criteria: Vec<String>,
        affected_files: Vec<String>,
    ) -> Self {
        Self {
            id: TaskId(id.into()),
            category,
            difficulty,
            description: description.into(),
            success_criteria,
            affected_files,
            created_at: Utc::now(),
        }
    }

    /// Get the number of success criteria
    pub fn criteria_count(&self) -> usize {
        self.success_criteria.len()
    }

    /// Get the number of affected files
    pub fn file_count(&self) -> usize {
        self.affected_files.len()
    }
}

/// Task identifier (slug-style)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Task categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    /// Bug fix tasks
    BugFix,
    /// Feature implementation tasks
    Feature,
    /// Code refactoring tasks
    Refactoring,
    /// Test writing/improvement tasks
    Test,
}

impl std::fmt::Display for TaskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskCategory::BugFix => write!(f, "Bug Fix"),
            TaskCategory::Feature => write!(f, "Feature"),
            TaskCategory::Refactoring => write!(f, "Refactoring"),
            TaskCategory::Test => write!(f, "Test"),
        }
    }
}

/// Task difficulty levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskDifficulty {
    /// Low complexity, well-defined scope
    Low,
    /// Medium complexity, some ambiguity
    Medium,
    /// High complexity, significant challenge
    High,
    /// Very high complexity, major undertaking
    VeryHigh,
}

impl std::fmt::Display for TaskDifficulty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskDifficulty::Low => write!(f, "Low"),
            TaskDifficulty::Medium => write!(f, "Medium"),
            TaskDifficulty::High => write!(f, "High"),
            TaskDifficulty::VeryHigh => write!(f, "Very High"),
        }
    }
}

/// Result of executing a benchmark task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// The task that was executed
    pub task_id: TaskId,
    /// Category of the task
    pub category: super::task::TaskCategory,
    /// Difficulty level
    pub difficulty: super::task::TaskDifficulty,
    /// Outcome of the task
    pub outcome: super::metrics::TaskOutcome,
    /// Time taken to complete
    pub duration_secs: f64,
    /// Number of retries attempted
    pub retries: u32,
    /// Any notes or comments
    pub notes: Option<String>,
    /// Timestamp when execution completed
    pub completed_at: DateTime<Utc>,
}

impl TaskResult {
    /// Create a successful task result
    pub fn success(
        task_id: TaskId,
        category: super::task::TaskCategory,
        difficulty: super::task::TaskDifficulty,
        duration_secs: f64,
    ) -> Self {
        Self {
            task_id,
            category,
            difficulty,
            outcome: super::metrics::TaskOutcome::Completed,
            duration_secs,
            retries: 0,
            notes: None,
            completed_at: Utc::now(),
        }
    }

    /// Create a failed task result
    pub fn failed(
        task_id: TaskId,
        category: super::task::TaskCategory,
        difficulty: super::task::TaskDifficulty,
        duration_secs: f64,
        notes: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            category,
            difficulty,
            outcome: super::metrics::TaskOutcome::Failed,
            duration_secs,
            retries: 0,
            notes: Some(notes.into()),
            completed_at: Utc::now(),
        }
    }

    /// Create a skipped task result
    pub fn skipped(
        task_id: TaskId,
        category: super::task::TaskCategory,
        difficulty: super::task::TaskDifficulty,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            category,
            difficulty,
            outcome: super::metrics::TaskOutcome::Skipped,
            duration_secs: 0.0,
            retries: 0,
            notes: Some(reason.into()),
            completed_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::TaskOutcome;

    #[test]
    fn test_task_creation() {
        let task = BenchmarkTask::new(
            "test_task",
            TaskCategory::BugFix,
            TaskDifficulty::Medium,
            "Test description",
            vec!["Criterion 1".to_string()],
            vec!["file.rs".to_string()],
        );

        assert_eq!(task.id.as_str(), "test_task");
        assert_eq!(task.category, TaskCategory::BugFix);
        assert_eq!(task.difficulty, TaskDifficulty::Medium);
        assert_eq!(task.description, "Test description");
        assert_eq!(task.criteria_count(), 1);
        assert_eq!(task.file_count(), 1);
    }

    #[test]
    fn test_task_result_success() {
        let result = TaskResult::success(
            TaskId("test".to_string()),
            TaskCategory::BugFix,
            TaskDifficulty::Low,
            10.5,
        );

        assert_eq!(result.outcome, TaskOutcome::Completed);
        assert_eq!(result.duration_secs, 10.5);
        assert_eq!(result.retries, 0);
        assert_eq!(result.category, TaskCategory::BugFix);
        assert_eq!(result.difficulty, TaskDifficulty::Low);
        assert!(result.notes.is_none());
    }

    #[test]
    fn test_task_result_failed() {
        let result = TaskResult::failed(
            TaskId("test".to_string()),
            TaskCategory::Feature,
            TaskDifficulty::Medium,
            5.0,
            "Implementation error",
        );

        assert_eq!(result.outcome, TaskOutcome::Failed);
        assert_eq!(result.notes, Some("Implementation error".to_string()));
        assert_eq!(result.category, TaskCategory::Feature);
        assert_eq!(result.difficulty, TaskDifficulty::Medium);
    }
}
