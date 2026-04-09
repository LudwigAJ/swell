//! High-level state management.

use chrono::Utc;
use std::sync::Arc;
use swell_core::{Checkpoint, CheckpointStore, SwellError, Task};
use uuid::Uuid;

/// High-level state manager for orchestrating task state
pub struct StateManager {
    checkpoint_store: Arc<dyn CheckpointStore>,
}

impl StateManager {
    pub fn new(checkpoint_store: Arc<dyn CheckpointStore>) -> Self {
        Self { checkpoint_store }
    }

    /// Save a task snapshot
    pub async fn save_task(&self, task: &Task) -> Result<Uuid, SwellError> {
        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: task.id,
            state: task.state,
            snapshot: serde_json::to_value(task)
                .map_err(|e| SwellError::ConfigError(e.to_string()))?,
            created_at: Utc::now(),
            metadata: serde_json::json!({}),
        };

        self.checkpoint_store.save(checkpoint).await
    }

    /// Restore a task from the latest checkpoint
    pub async fn restore_task(&self, task_id: Uuid) -> Result<Option<Task>, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;

        match checkpoint {
            Some(cp) => {
                let task: Task = serde_json::from_value(cp.snapshot)
                    .map_err(|e| SwellError::ConfigError(e.to_string()))?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    /// Check if a task has a checkpoint
    pub async fn has_checkpoint(&self, task_id: Uuid) -> Result<bool, SwellError> {
        let checkpoint = self.checkpoint_store.load_latest(task_id).await?;
        Ok(checkpoint.is_some())
    }

    /// Get checkpoint history for a task
    pub async fn get_history(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError> {
        self.checkpoint_store.list(task_id).await
    }

    /// Prune old checkpoints, keeping only the latest N
    pub async fn prune_history(&self, task_id: Uuid, keep: usize) -> Result<(), SwellError> {
        self.checkpoint_store.prune(task_id, keep).await
    }
}
