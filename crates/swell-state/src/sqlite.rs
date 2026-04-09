//! SQLite-based checkpoint store for MVP.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use swell_core::{Checkpoint, CheckpointStore, SwellError};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SqliteCheckpointStore {
    pool: SqlitePool,
}

impl SqliteCheckpointStore {
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        let pool = SqlitePool::connect(database_url)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<(), SwellError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                state TEXT NOT NULL,
                snapshot TEXT NOT NULL,
                created_at TEXT NOT NULL,
                metadata TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_checkpoints_task_id 
            ON checkpoints(task_id, created_at)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError> {
        let id_str = checkpoint.id.to_string();
        let task_id_str = checkpoint.task_id.to_string();
        let state_str = serde_json::to_string(&checkpoint.state).unwrap();
        let snapshot_str = serde_json::to_string(&checkpoint.snapshot).unwrap();
        let created_at_str = checkpoint.created_at.to_rfc3339();
        let metadata_str = serde_json::to_string(&checkpoint.metadata).unwrap();

        sqlx::query(
            r#"
            INSERT INTO checkpoints (id, task_id, state, snapshot, created_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&task_id_str)
        .bind(&state_str)
        .bind(&snapshot_str)
        .bind(&created_at_str)
        .bind(&metadata_str)
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(checkpoint.id)
    }

    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        let row = sqlx::query("SELECT * FROM checkpoints WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn load_latest(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        let row = sqlx::query(
            "SELECT * FROM checkpoints WHERE task_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError> {
        let rows =
            sqlx::query("SELECT * FROM checkpoints WHERE task_id = ? ORDER BY created_at ASC")
                .bind(task_id.to_string())
                .fetch_all(&self.pool)
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let checkpoints: Vec<Checkpoint> = rows
            .into_iter()
            .filter_map(|r| self.row_to_checkpoint(r).ok())
            .collect();

        Ok(checkpoints)
    }

    async fn prune(&self, task_id: Uuid, keep: usize) -> Result<(), SwellError> {
        // Keep the latest `keep` checkpoints
        sqlx::query(
            r#"
            DELETE FROM checkpoints 
            WHERE task_id = ? 
            AND id NOT IN (
                SELECT id FROM checkpoints 
                WHERE task_id = ? 
                ORDER BY created_at DESC 
                LIMIT ?
            )
            "#,
        )
        .bind(task_id.to_string())
        .bind(task_id.to_string())
        .bind(keep as i64)
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

impl SqliteCheckpointStore {
    fn row_to_checkpoint(&self, row: sqlx::sqlite::SqliteRow) -> Result<Checkpoint, SwellError> {
        let id_str: String = row.get("id");
        let task_id_str: String = row.get("task_id");
        let state_str: String = row.get("state");
        let snapshot_str: String = row.get("snapshot");
        let created_at_str: String = row.get("created_at");
        let metadata_str: String = row.get("metadata");

        Ok(Checkpoint {
            id: Uuid::parse_str(&id_str).map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            task_id: Uuid::parse_str(&task_id_str)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            state: serde_json::from_str(&state_str)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            snapshot: serde_json::from_str(&snapshot_str)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?
                .with_timezone(&Utc),
            metadata: serde_json::from_str(&metadata_str)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::TaskState;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_sqlite_store() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let url = format!("sqlite:{}?mode=rwc", db_path.display());

        let store = SqliteCheckpointStore::new(&url).await.unwrap();

        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            state: TaskState::Created,
            snapshot: serde_json::json!({"test": true}),
            created_at: Utc::now(),
            metadata: serde_json::json!({}),
        };

        let id = store.save(checkpoint.clone()).await.unwrap();
        let loaded = store.load(id).await.unwrap().unwrap();

        assert_eq!(loaded.task_id, checkpoint.task_id);
    }

    #[tokio::test]
    async fn test_load_latest() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let url = format!("sqlite:{}?mode=rwc", db_path.display());

        let store = SqliteCheckpointStore::new(&url).await.unwrap();
        let task_id = Uuid::new_v4();

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
}
