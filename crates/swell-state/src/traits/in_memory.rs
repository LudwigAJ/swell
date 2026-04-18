//! In-memory checkpoint store for testing.

use async_trait::async_trait;
use std::collections::HashMap;
use swell_core::{Checkpoint, CheckpointStore, SwellError, TaskId};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug)]
pub struct InMemoryCheckpointStore {
    checkpoints: RwLock<HashMap<Uuid, Checkpoint>>,
    by_task: RwLock<HashMap<Uuid, Vec<Uuid>>>,
}

impl InMemoryCheckpointStore {
    pub fn new() -> Self {
        Self {
            checkpoints: RwLock::new(HashMap::new()),
            by_task: RwLock::new(HashMap::new()),
        }
    }

    pub async fn clear(&self) {
        self.checkpoints.write().await.clear();
        self.by_task.write().await.clear();
    }
}

#[async_trait]
impl CheckpointStore for InMemoryCheckpointStore {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError> {
        let id = checkpoint.id;
        let task_uuid = checkpoint.task_id.as_uuid();

        self.checkpoints
            .write()
            .await
            .insert(id, checkpoint.clone());
        self.by_task
            .write()
            .await
            .entry(task_uuid)
            .or_insert_with(Vec::new)
            .push(id);

        Ok(id)
    }

    async fn load_latest(&self, task_id: TaskId) -> Result<Option<Checkpoint>, SwellError> {
        let ids = self.by_task.read().await.get(&task_id.as_uuid()).cloned();

        match ids {
            Some(ids) if !ids.is_empty() => {
                let latest_id = ids.last().copied().unwrap();
                Ok(self.checkpoints.read().await.get(&latest_id).cloned())
            }
            _ => Ok(None),
        }
    }

    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        Ok(self.checkpoints.read().await.get(&id).cloned())
    }

    async fn list(&self, task_id: TaskId) -> Result<Vec<Checkpoint>, SwellError> {
        let ids = self.by_task.read().await.get(&task_id.as_uuid()).cloned();

        match ids {
            Some(ids) => {
                let checkpoints = self.checkpoints.read().await;
                let result: Vec<Checkpoint> = ids
                    .iter()
                    .filter_map(|id| checkpoints.get(id).cloned())
                    .collect();
                Ok(result)
            }
            None => Ok(Vec::new()),
        }
    }

    async fn prune(&self, task_id: TaskId, keep: usize) -> Result<(), SwellError> {
        let mut by_task = self.by_task.write().await;
        let mut checkpoints = self.checkpoints.write().await;

        if let Some(ids) = by_task.get_mut(&task_id.as_uuid()) {
            if ids.len() > keep {
                let to_remove: Vec<Uuid> = ids.drain(0..ids.len() - keep).collect();
                for id in to_remove {
                    checkpoints.remove(&id);
                }
            }
        }

        Ok(())
    }
}

impl Default for InMemoryCheckpointStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use swell_core::TaskState;

    #[tokio::test]
    async fn test_save_and_load() {
        let store = InMemoryCheckpointStore::new();

        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: TaskId::new(),
            state: TaskState::Created,
            snapshot: serde_json::json!({"key": "value"}),
            created_at: Utc::now(),
            metadata: serde_json::json!({}),
        };

        let id = store.save(checkpoint.clone()).await.unwrap();
        let loaded = store.load(id).await.unwrap().unwrap();

        assert_eq!(loaded.task_id, checkpoint.task_id);
        assert_eq!(loaded.state, TaskState::Created);
    }

    #[tokio::test]
    async fn test_load_latest() {
        let store = InMemoryCheckpointStore::new();
        let task_id = TaskId::new();

        for i in 0..3 {
            let checkpoint = Checkpoint {
                id: Uuid::new_v4(),
                task_id,
                state: TaskState::Created,
                snapshot: serde_json::json!({"index": i}),
                created_at: Utc::now(),
                metadata: serde_json::json!({}),
            };
            store.save(checkpoint).await.unwrap();
        }

        let latest = store.load_latest(task_id).await.unwrap().unwrap();
        assert_eq!(latest.snapshot["index"], 2);
    }

    #[tokio::test]
    async fn test_prune() {
        let store = InMemoryCheckpointStore::new();
        let task_id = TaskId::new();

        for i in 0..5 {
            let checkpoint = Checkpoint {
                id: Uuid::new_v4(),
                task_id,
                state: TaskState::Created,
                snapshot: serde_json::json!({"index": i}),
                created_at: Utc::now(),
                metadata: serde_json::json!({}),
            };
            store.save(checkpoint).await.unwrap();
        }

        store.prune(task_id, 2).await.unwrap();

        let remaining = store.list(task_id).await.unwrap();
        assert_eq!(remaining.len(), 2);
    }
}
