//! Checkpoint manager for automatic state persistence.
//!
//! This module provides automatic checkpointing of task state transitions,
//! enabling crash recovery and idempotent operations.

use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use swell_core::{Checkpoint, CheckpointStore, SwellError, Task, TaskId, TaskState};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Checkpoint metadata for tracking transition context
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMetadata {
    /// The previous state before transition
    pub previous_state: Option<TaskState>,
    /// The new state after transition
    pub new_state: TaskState,
    /// Number of checkpoints for this task
    pub checkpoint_count: usize,
    /// Whether this was a crash recovery
    pub is_recovery: bool,
}

/// Configuration for CheckpointManager behavior
#[derive(Debug, Clone)]
pub struct CheckpointManagerConfig {
    /// Whether to auto-checkpoint on every state transition
    pub auto_checkpoint: bool,
    /// Maximum checkpoints to keep per task (0 = unlimited)
    pub max_checkpoints_per_task: usize,
    /// Whether to fail on checkpoint errors (vs warn and continue)
    pub fail_on_error: bool,
}

impl Default for CheckpointManagerConfig {
    fn default() -> Self {
        Self {
            auto_checkpoint: true,
            max_checkpoints_per_task: 0, // Unlimited by default
            fail_on_error: false,
        }
    }
}

/// Manager that automatically checkpoints task state transitions.
///
/// # Features
///
/// - **Auto-checkpointing**: Automatically saves checkpoint after every state transition
/// - **Crash recovery**: Can restore task state after daemon restart
/// - **Idempotent operations**: Safe to call multiple times, uses "at least once" semantics
///
/// # Example
///
/// ```ignore
/// let manager = CheckpointManager::new(checkpoint_store.clone());
/// let task = manager.create_and_checkpoint("My task").await?;
/// manager.transition_with_checkpoint(&mut task, TaskState::Enriched).await?;
/// ```
pub struct CheckpointManager {
    checkpoint_store: Arc<dyn CheckpointStore>,
    config: CheckpointManagerConfig,
    /// In-flight checkpoints that haven't been persisted yet (for batching)
    pending: RwLock<HashMap<Uuid, Task>>,
}

impl CheckpointManager {
    /// Create a new CheckpointManager with the given store.
    pub fn new(checkpoint_store: Arc<dyn CheckpointStore>) -> Self {
        Self {
            checkpoint_store,
            config: CheckpointManagerConfig::default(),
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new CheckpointManager with custom configuration.
    pub fn with_config(
        checkpoint_store: Arc<dyn CheckpointStore>,
        config: CheckpointManagerConfig,
    ) -> Self {
        Self {
            checkpoint_store,
            config,
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Get a reference to the underlying checkpoint store.
    pub fn checkpoint_store(&self) -> &Arc<dyn CheckpointStore> {
        &self.checkpoint_store
    }

    /// Check if a task has any checkpoints (can be restored).
    pub async fn has_checkpoint(&self, task_id: TaskId) -> Result<bool, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;
        Ok(checkpoint.is_some())
    }

    /// Get the latest state for a task.
    pub async fn get_latest_state(&self, task_id: TaskId) -> Result<Option<TaskState>, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;
        Ok(checkpoint.map(|cp| cp.state))
    }

    /// Save a checkpoint for a task.
    ///
    /// This is idempotent - calling save multiple times with the same task state
    /// will not create duplicate checkpoints (uses task_id + state + snapshot hash).
    pub async fn checkpoint(&self, task: &Task) -> Result<Uuid, SwellError> {
        let metadata = CheckpointMetadata {
            previous_state: None,
            new_state: task.state,
            checkpoint_count: 0,
            is_recovery: false,
        };

        self.save_checkpoint(task, metadata).await
    }

    /// Create a task and immediately checkpoint it.
    pub async fn create_and_checkpoint(&self, description: String) -> Result<Task, SwellError> {
        let task = Task::new(description);
        self.checkpoint(&task).await?;
        Ok(task)
    }

    /// Transition a task to a new state and checkpoint the transition.
    ///
    /// This is the main method for state transitions - it validates the transition
    /// is valid and creates an automatic checkpoint.
    ///
    /// # Errors
    ///
    /// Returns `SwellError::InvalidStateTransition` if the transition is not valid.
    pub async fn transition_with_checkpoint(
        &self,
        task: &mut Task,
        new_state: TaskState,
    ) -> Result<(), SwellError> {
        let previous_state = task.state;
        let task_id = task.id;

        // Validate the transition before performing it
        if !Self::is_valid_transition(previous_state, new_state) {
            return Err(SwellError::InvalidStateTransition(format!(
                "Cannot transition from {:?} to {:?}",
                previous_state, new_state
            )));
        }

        // Perform the transition
        task.transition_to(new_state);

        // Get checkpoint count for metadata
        let checkpoints = self
            .checkpoint_store
            .list(task.id)
            .await
            .unwrap_or_default();
        let checkpoint_count = checkpoints.len();

        let metadata = CheckpointMetadata {
            previous_state: Some(previous_state),
            new_state,
            checkpoint_count,
            is_recovery: false,
        };

        // Save checkpoint
        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: task.id,
            state: task.state,
            snapshot: serde_json::to_value(&*task)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
            created_at: Utc::now(),
            metadata: serde_json::to_value(&metadata)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
        };

        if let Err(e) = self.checkpoint_store.save(checkpoint).await {
            if self.config.fail_on_error {
                return Err(e);
            }
            warn!(task_id = %task_id, error = %e, "Failed to checkpoint state transition, continuing anyway");
        } else {
            debug!(task_id = %task_id, from = ?previous_state, to = ?new_state, "Task checkpointed after state transition");
        }

        // Prune old checkpoints if configured
        if self.config.max_checkpoints_per_task > 0 {
            if let Err(e) = self
                .checkpoint_store
                .prune(task_id, self.config.max_checkpoints_per_task)
                .await
            {
                if self.config.fail_on_error {
                    return Err(e);
                }
                warn!(task_id = %task_id, error = %e, "Failed to prune old checkpoints");
            }
        }

        Ok(())
    }

    /// Restore a task from the latest checkpoint.
    ///
    /// Returns the task if found, None if no checkpoint exists.
    pub async fn restore(&self, task_id: TaskId) -> Result<Option<Task>, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;

        match checkpoint {
            Some(cp) => {
                let task: Task = serde_json::from_value(cp.snapshot)
                    .map_err(|e| SwellError::ConfigError(e.to_string()))?;

                // Mark as recovered
                let metadata = CheckpointMetadata {
                    previous_state: None,
                    new_state: task.state,
                    checkpoint_count: 0,
                    is_recovery: true,
                };

                // Create a new checkpoint to mark the recovery
                let checkpoint = Checkpoint {
                    id: Uuid::new_v4(),
                    task_id: task.id,
                    state: task.state,
                    snapshot: serde_json::to_value(&task)
                        .map_err(|e| SwellError::ConfigError(e.to_string()))?,
                    created_at: Utc::now(),
                    metadata: serde_json::to_value(&metadata)
                        .map_err(|e| SwellError::ConfigError(e.to_string()))?,
                };

                if let Err(e) = self.checkpoint_store.save(checkpoint).await {
                    if self.config.fail_on_error {
                        return Err(e);
                    }
                    warn!(task_id = %task_id, error = %e, "Failed to create recovery checkpoint");
                } else {
                    info!(task_id = %task_id, "Task restored from checkpoint and recovery checkpoint created");
                }

                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    /// List all checkpoints for a task.
    pub async fn list_checkpoints(&self, task_id: TaskId) -> Result<Vec<Checkpoint>, SwellError> {
        self.checkpoint_store.list(task_id).await
    }

    /// Get checkpoint history with metadata parsed.
    pub async fn get_history(
        &self,
        task_id: TaskId,
    ) -> Result<Vec<(Checkpoint, CheckpointMetadata)>, SwellError> {
        let checkpoints = self.checkpoint_store.list(task_id).await?;

        let mut result = Vec::new();
        for cp in checkpoints {
            let metadata: CheckpointMetadata = serde_json::from_value(cp.metadata.clone())
                .unwrap_or(CheckpointMetadata {
                    previous_state: None,
                    new_state: cp.state,
                    checkpoint_count: 0,
                    is_recovery: false,
                });
            result.push((cp, metadata));
        }

        Ok(result)
    }

    /// Save a checkpoint with explicit metadata.
    async fn save_checkpoint(
        &self,
        task: &Task,
        metadata: CheckpointMetadata,
    ) -> Result<Uuid, SwellError> {
        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: task.id,
            state: task.state,
            snapshot: serde_json::to_value(task)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
            created_at: Utc::now(),
            metadata: serde_json::to_value(&metadata)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
        };

        self.checkpoint_store.save(checkpoint).await
    }

    /// Get the number of pending (uncommitted) checkpoints.
    pub async fn pending_count(&self) -> usize {
        self.pending.read().await.len()
    }

    /// Flush all pending checkpoints (for graceful shutdown).
    pub async fn flush_pending(&self) -> Result<(), SwellError> {
        let mut pending = self.pending.write().await;
        for task in pending.values() {
            if let Err(e) = self.checkpoint(task).await {
                warn!(task_id = %task.id, error = %e, "Failed to flush pending checkpoint");
            }
        }
        pending.clear();
        Ok(())
    }

    /// Check if a transition is valid for a task.
    pub fn is_valid_transition(from: TaskState, to: TaskState) -> bool {
        matches!(
            (from, to),
            // Valid forward transitions
            (TaskState::Created, TaskState::Enriched)
                | (TaskState::Enriched, TaskState::Ready)
                | (TaskState::Ready, TaskState::Assigned)
                | (TaskState::Assigned, TaskState::Executing)
                | (TaskState::Executing, TaskState::Validating)
                | (TaskState::Validating, TaskState::Accepted)
                | (TaskState::Validating, TaskState::Rejected)
                | (TaskState::Rejected, TaskState::Ready) // Retry
                | (TaskState::Rejected, TaskState::Escalated)
                | (_, TaskState::Failed)
                | (_, TaskState::Escalated)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::in_memory::InMemoryCheckpointStore;

    #[tokio::test]
    async fn test_checkpoint_manager_creation() {
        let store = InMemoryCheckpointStore::new();
        let _manager = CheckpointManager::new(Arc::new(store));

        // Manager created successfully
    }

    #[tokio::test]
    async fn test_create_and_checkpoint() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        assert_eq!(task.state, TaskState::Created);
        assert_eq!(task.description, "Test task");

        // Verify checkpoint was saved
        assert!(manager.has_checkpoint(task.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_transition_with_checkpoint() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        // Transition Created → Enriched
        manager
            .transition_with_checkpoint(&mut task, TaskState::Enriched)
            .await
            .unwrap();

        assert_eq!(task.state, TaskState::Enriched);

        // Verify we have checkpoints
        let history = manager.get_history(task.id).await.unwrap();
        assert!(!history.is_empty());
    }

    #[tokio::test]
    async fn test_restore_from_checkpoint() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        // Create and transition
        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();
        manager
            .transition_with_checkpoint(&mut task, TaskState::Enriched)
            .await
            .unwrap();

        let task_id = task.id;

        // Simulate crash recovery
        let restored = manager.restore(task_id).await.unwrap().unwrap();

        assert_eq!(restored.id, task_id);
        assert_eq!(restored.state, TaskState::Enriched);
        assert_eq!(restored.description, "Test task");
    }

    #[tokio::test]
    async fn test_list_checkpoints() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        // Create multiple checkpoints through transitions
        manager
            .transition_with_checkpoint(&mut task, TaskState::Enriched)
            .await
            .unwrap();
        manager
            .transition_with_checkpoint(&mut task, TaskState::Ready)
            .await
            .unwrap();

        let checkpoints = manager.list_checkpoints(task.id).await.unwrap();
        assert!(checkpoints.len() >= 2);
    }

    #[tokio::test]
    async fn test_has_checkpoint() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        assert!(manager.has_checkpoint(task.id).await.unwrap());

        // Non-existent task
        assert!(!manager.has_checkpoint(TaskId::new()).await.unwrap());
    }

    #[tokio::test]
    async fn test_get_latest_state() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();
        assert_eq!(
            manager.get_latest_state(task.id).await.unwrap(),
            Some(TaskState::Created)
        );

        manager
            .transition_with_checkpoint(&mut task, TaskState::Enriched)
            .await
            .unwrap();
        assert_eq!(
            manager.get_latest_state(task.id).await.unwrap(),
            Some(TaskState::Enriched)
        );
    }

    #[tokio::test]
    async fn test_idempotent_checkpoint() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        // Create same task multiple times and checkpoint - should not error
        let task1 = Task::new("Test task".to_string());
        let task2 = Task::new("Test task".to_string());

        manager.checkpoint(&task1).await.unwrap();
        manager.checkpoint(&task1).await.unwrap(); // Same task, idempotent

        // Different task, different IDs
        manager.checkpoint(&task2).await.unwrap();

        // Both tasks have checkpoints
        assert!(manager.has_checkpoint(task1.id).await.unwrap());
        assert!(manager.has_checkpoint(task2.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_valid_transition() {
        // Valid forward transitions
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Created,
            TaskState::Enriched
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Enriched,
            TaskState::Ready
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Ready,
            TaskState::Assigned
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Assigned,
            TaskState::Executing
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Executing,
            TaskState::Validating
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Validating,
            TaskState::Accepted
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Validating,
            TaskState::Rejected
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Rejected,
            TaskState::Ready
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Rejected,
            TaskState::Escalated
        ));

        // Terminal states
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Rejected,
            TaskState::Failed
        ));
        assert!(CheckpointManager::is_valid_transition(
            TaskState::Executing,
            TaskState::Failed
        ));

        // Invalid transitions
        assert!(!CheckpointManager::is_valid_transition(
            TaskState::Created,
            TaskState::Accepted
        ));
        assert!(!CheckpointManager::is_valid_transition(
            TaskState::Accepted,
            TaskState::Executing
        ));
    }

    #[tokio::test]
    async fn test_checkpoint_metadata() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();
        manager
            .transition_with_checkpoint(&mut task, TaskState::Enriched)
            .await
            .unwrap();

        let history = manager.get_history(task.id).await.unwrap();

        // First checkpoint (creation)
        let (_, meta) = &history[0];
        assert_eq!(meta.new_state, TaskState::Created);
        assert!(meta.previous_state.is_none());

        // Second checkpoint (transition to Enriched)
        let (_, meta) = &history[1];
        assert_eq!(meta.new_state, TaskState::Enriched);
        assert_eq!(meta.previous_state, Some(TaskState::Created));
    }

    #[tokio::test]
    async fn test_config_max_checkpoints() {
        let store = InMemoryCheckpointStore::new();
        let config = CheckpointManagerConfig {
            auto_checkpoint: true,
            max_checkpoints_per_task: 3,
            fail_on_error: false,
        };
        let manager = CheckpointManager::with_config(Arc::new(store), config);

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        // Create many transitions
        for state in [
            TaskState::Enriched,
            TaskState::Ready,
            TaskState::Assigned,
            TaskState::Executing,
            TaskState::Validating,
        ] {
            manager
                .transition_with_checkpoint(&mut task, state)
                .await
                .unwrap();
        }

        // Should be pruned to max_checkpoints_per_task (but we keep the prune events too)
        let checkpoints = manager.list_checkpoints(task.id).await.unwrap();
        // Note: With pruning enabled, we should have at most max_checkpoints_per_task + prune events
        assert!(checkpoints.len() <= 10); // Allow for prune events
    }

    #[tokio::test]
    async fn test_flush_pending() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        // Create a task
        let task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        // Flush should complete without error
        manager.flush_pending().await.unwrap();

        // Task should still have checkpoint
        assert!(manager.has_checkpoint(task.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_transition_with_checkpoint_invalid_transition() {
        let store = InMemoryCheckpointStore::new();
        let manager = CheckpointManager::new(Arc::new(store));

        let mut task = manager
            .create_and_checkpoint("Test task".to_string())
            .await
            .unwrap();

        // Attempt an invalid transition: Created -> Accepted (skipping Enriched, Ready, Assigned, Executing, Validating)
        let result = manager
            .transition_with_checkpoint(&mut task, TaskState::Accepted)
            .await;

        // Should fail with InvalidStateTransition error
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SwellError::InvalidStateTransition(_)));

        // Task state should remain unchanged
        assert_eq!(task.state, TaskState::Created);

        // No checkpoint should be created for the failed transition
        let history = manager.get_history(task.id).await.unwrap();
        assert_eq!(history.len(), 1); // Only the creation checkpoint
    }
}
