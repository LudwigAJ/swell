//! State management and checkpointing for SWELL.
//!
//! This crate provides persistent state management for tasks,
//! including SQLite/PostgreSQL checkpoint storage.
//!
//! # Architecture
//!
//! The state layer is built on:
//! - [`CheckpointStore`] - trait for persisting task state snapshots
//! - [`StateManager`] - high-level state operations
//! - [`SqliteStore`] - SQLite implementation for MVP
//! - [`PostgresStore`] - PostgreSQL implementation for production

pub mod checkpoint_manager;
pub mod manager;
pub mod postgres;
pub mod session;
pub mod sqlite;
pub mod task_state_file;
pub mod traits;
pub mod workspace_fingerprint;

pub use checkpoint_manager::{CheckpointManager, CheckpointManagerConfig, CheckpointMetadata};
pub use manager::StateManager;
pub use postgres::PostgresCheckpointStore;
pub use session::{
    InMemorySessionStore, SessionError, SessionMetadata, SessionState, SessionStore,
};
pub use sqlite::SqliteCheckpointStore;
pub use task_state_file::{
    read_task_state, read_task_state_sync, write_task_state, write_task_state_sync, TaskStateFile,
    TaskStateFileError,
};
pub use traits::*;
pub use workspace_fingerprint::workspace_fingerprint;

#[cfg(test)]
mod tests {
    use swell_core::CheckpointStore;

    #[tokio::test]
    async fn test_in_memory_store() {
        use crate::traits::in_memory::InMemoryCheckpointStore;
        let store = InMemoryCheckpointStore::new();

        let checkpoint = swell_core::Checkpoint {
            id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            state: swell_core::TaskState::Created,
            snapshot: serde_json::json!({"test": true}),
            created_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        let id = store.save(checkpoint.clone()).await.unwrap();
        let loaded = store.load(id).await.unwrap().unwrap();

        assert_eq!(loaded.task_id, checkpoint.task_id);
    }
}
