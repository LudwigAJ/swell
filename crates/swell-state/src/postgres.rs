//! PostgreSQL-based checkpoint store for production.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use swell_core::{Checkpoint, CheckpointStore, SwellError};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PostgresCheckpointStore {
    pool: PgPool,
}

impl PostgresCheckpointStore {
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        let pool = PgPool::connect(database_url)
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
                id UUID PRIMARY KEY,
                task_id UUID NOT NULL,
                state JSONB NOT NULL,
                snapshot JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                metadata JSONB NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_checkpoints_task_created
            ON checkpoints(task_id, created_at DESC)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for PostgresCheckpointStore {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError> {
        sqlx::query(
            r#"
            INSERT INTO checkpoints (id, task_id, state, snapshot, created_at, metadata)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(checkpoint.id)
        .bind(checkpoint.task_id)
        .bind(serde_json::to_value(checkpoint.state).unwrap())
        .bind(serde_json::to_value(&checkpoint.snapshot).unwrap())
        .bind(checkpoint.created_at)
        .bind(serde_json::to_value(&checkpoint.metadata).unwrap())
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(checkpoint.id)
    }

    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        let row = sqlx::query("SELECT * FROM checkpoints WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn load_latest(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        let row = sqlx::query(
            "SELECT * FROM checkpoints WHERE task_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError> {
        let rows =
            sqlx::query("SELECT * FROM checkpoints WHERE task_id = $1 ORDER BY created_at ASC")
                .bind(task_id)
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
        sqlx::query(
            r#"
            DELETE FROM checkpoints 
            WHERE task_id = $1 
            AND id NOT IN (
                SELECT id FROM checkpoints 
                WHERE task_id = $1 
                ORDER BY created_at DESC 
                LIMIT $2
            )
            "#,
        )
        .bind(task_id)
        .bind(keep as i64)
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

impl PostgresCheckpointStore {
    fn row_to_checkpoint(&self, row: sqlx::postgres::PgRow) -> Result<Checkpoint, SwellError> {
        let id: Uuid = row.get("id");
        let task_id: Uuid = row.get("task_id");
        let state: serde_json::Value = row.get("state");
        let snapshot: serde_json::Value = row.get("snapshot");
        let created_at: DateTime<Utc> = row.get("created_at");
        let metadata: serde_json::Value = row.get("metadata");

        Ok(Checkpoint {
            id,
            task_id,
            state: serde_json::from_value(state)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            snapshot,
            created_at,
            metadata,
        })
    }
}
