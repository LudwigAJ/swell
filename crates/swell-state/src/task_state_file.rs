//! Task state file observability for external monitoring.
//!
//! This module provides atomic writes of task state to `.swell/task-state.json`
//! on each state transition, enabling external observability tools to monitor
//! task progress without requiring database access.
//!
//! The atomic write pattern (write-to-temp + rename) ensures no partial writes
//! are observable even under concurrent access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use swell_core::{SwellError, TaskState};
use uuid::Uuid;

/// Represents the task state file content written on each state transition.
///
/// This file is written atomically (write-to-temp + rename) so external
/// observability tools can monitor task progress by reading this file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStateFile {
    /// The task ID this state belongs to
    pub task_id: Uuid,
    /// The current state after transition
    pub state: TaskState,
    /// ISO 8601 timestamp of when the transition occurred
    pub timestamp: DateTime<Utc>,
    /// Number of iterations completed for this task
    pub iteration_count: u32,
}

/// Error type for task state file operations
#[derive(Debug, thiserror::Error)]
pub enum TaskStateFileError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Swell error: {0}")]
    Swell(#[from] SwellError),
}

/// Atomically writes task state to `.swell/task-state.json`.
///
/// Uses the write-to-temp + rename pattern to ensure:
/// - No partial writes are ever observable
/// - File always contains valid, complete JSON
/// - Safe for concurrent access
///
/// # Arguments
///
/// * `swell_dir` - Path to the `.swell/` directory
/// * `task_id` - The task ID
/// * `state` - The new state after transition
/// * `iteration_count` - Current iteration count for the task
pub async fn write_task_state(
    swell_dir: &Path,
    task_id: Uuid,
    state: TaskState,
    iteration_count: u32,
) -> Result<(), TaskStateFileError> {
    let task_state = TaskStateFile {
        task_id,
        state,
        timestamp: Utc::now(),
        iteration_count,
    };

    let json = serde_json::to_string_pretty(&task_state)?;
    let path = swell_dir.join("task-state.json");
    let temp_path = swell_dir.join("task-state.json.tmp");

    // Write to temp file first (not atomic yet)
    tokio::fs::write(&temp_path, json).await?;

    // Atomic rename (OS guarantees atomicity for rename over existing file)
    tokio::fs::rename(&temp_path, &path).await?;

    Ok(())
}

/// Reads the current task state from `.swell/task-state.json`.
///
/// Returns `None` if the file doesn't exist.
pub async fn read_task_state(
    swell_dir: &Path,
) -> Result<Option<TaskStateFile>, TaskStateFileError> {
    let path = swell_dir.join("task-state.json");

    if !path.exists() {
        return Ok(None);
    }

    let content = tokio::fs::read_to_string(&path).await?;
    let state: TaskStateFile = serde_json::from_str(&content)?;

    Ok(Some(state))
}

/// Synchronous version of [`write_task_state`] using std::fs.
///
/// This is useful for contexts where async I/O is not available.
pub fn write_task_state_sync(
    swell_dir: &Path,
    task_id: Uuid,
    state: TaskState,
    iteration_count: u32,
) -> Result<(), TaskStateFileError> {
    let task_state = TaskStateFile {
        task_id,
        state,
        timestamp: Utc::now(),
        iteration_count,
    };

    let json = serde_json::to_string_pretty(&task_state)?;
    let path = swell_dir.join("task-state.json");
    let temp_path = swell_dir.join("task-state.json.tmp");

    // Write to temp file first
    std::fs::write(&temp_path, json)?;

    // Atomic rename
    std::fs::rename(&temp_path, &path)?;

    Ok(())
}

/// Synchronous version of [`read_task_state`] using std::fs.
pub fn read_task_state_sync(swell_dir: &Path) -> Result<Option<TaskStateFile>, TaskStateFileError> {
    let path = swell_dir.join("task-state.json");

    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)?;
    let state: TaskStateFile = serde_json::from_str(&content)?;

    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_and_read_task_state() {
        let temp_dir = TempDir::new().unwrap();
        let swell_dir = temp_dir.path();

        let task_id = Uuid::new_v4();
        let state = TaskState::Executing;
        let iteration_count = 5u32;

        // Write state
        write_task_state(swell_dir, task_id, state, iteration_count)
            .await
            .unwrap();

        // Read back
        let read_state = read_task_state(swell_dir).await.unwrap().unwrap();

        assert_eq!(read_state.task_id, task_id);
        assert_eq!(read_state.state, TaskState::Executing);
        assert_eq!(read_state.iteration_count, 5);
        // Timestamp should be recent (within a minute)
        let now = Utc::now();
        let diff = now.signed_duration_since(read_state.timestamp);
        assert!(diff.num_seconds() < 60);
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let swell_dir = temp_dir.path();

        let result = read_task_state(swell_dir).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_atomic_write_no_partial_content() {
        let temp_dir = TempDir::new().unwrap();
        let swell_dir = temp_dir.path();

        let task_id = Uuid::new_v4();
        let state = TaskState::Executing;

        // Write state
        write_task_state(swell_dir, task_id, state, 1)
            .await
            .unwrap();

        // Read the file content
        let path = swell_dir.join("task-state.json");
        let content = tokio::fs::read_to_string(&path).await.unwrap();

        // Verify it's valid JSON
        let parsed: TaskStateFile = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.state, TaskState::Executing);

        // Verify no temp file remains
        let temp_path = swell_dir.join("task-state.json.tmp");
        assert!(!temp_path.exists());
    }

    #[test]
    fn test_rapid_sequential_writes() {
        // Test rapid sequential writes - this verifies the atomic write pattern
        // works correctly even when writes happen in quick succession
        let temp_dir = tempfile::TempDir::new().unwrap();
        let swell_dir = temp_dir.path().to_path_buf();

        let task_id = Uuid::new_v4();
        let state = TaskState::Executing;

        // Rapid sequential writes
        for i in 0..10 {
            write_task_state_sync(&swell_dir, task_id, state, i).unwrap();
        }

        // File should still be valid JSON with complete data
        let final_state = read_task_state_sync(&swell_dir).unwrap().unwrap();
        assert_eq!(final_state.task_id, task_id);
        assert_eq!(final_state.state, TaskState::Executing);
        assert!(final_state.iteration_count < 10);
    }

    #[test]
    fn test_write_and_read_sync() {
        let temp_dir = TempDir::new().unwrap();
        let swell_dir = temp_dir.path();

        let task_id = Uuid::new_v4();
        let state = TaskState::Validating;
        let iteration_count = 3u32;

        // Write state synchronously
        write_task_state_sync(swell_dir, task_id, state, iteration_count).unwrap();

        // Read back
        let read_state = read_task_state_sync(swell_dir).unwrap().unwrap();

        assert_eq!(read_state.task_id, task_id);
        assert_eq!(read_state.state, TaskState::Validating);
        assert_eq!(read_state.iteration_count, 3);
    }

    #[test]
    fn test_sync_no_partial_content() {
        let temp_dir = TempDir::new().unwrap();
        let swell_dir = temp_dir.path();

        let task_id = Uuid::new_v4();
        let state = TaskState::Accepted;

        write_task_state_sync(swell_dir, task_id, state, 1).unwrap();

        let path = swell_dir.join("task-state.json");
        let content = std::fs::read_to_string(&path).unwrap();

        // Verify it's valid JSON
        let parsed: TaskStateFile = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.state, TaskState::Accepted);

        // Verify no temp file remains
        let temp_path = swell_dir.join("task-state.json.tmp");
        assert!(!temp_path.exists());
    }

    #[tokio::test]
    async fn test_task_state_file_serde() {
        let task_id = Uuid::new_v4();
        let original = TaskStateFile {
            task_id,
            state: TaskState::Enriched,
            timestamp: Utc::now(),
            iteration_count: 7,
        };

        // Round-trip through JSON
        let json = serde_json::to_string(&original).unwrap();
        let parsed: TaskStateFile = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.task_id, original.task_id);
        assert_eq!(parsed.state, original.state);
        assert_eq!(parsed.iteration_count, original.iteration_count);
    }
}
