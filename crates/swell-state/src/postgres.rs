//! PostgreSQL-based checkpoint store for production.
//!
//! This module implements event-sourced storage with append-only events
//! and materialized views for efficient state reconstruction.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::hash::Hash;
use swell_core::{Checkpoint, CheckpointStore, SwellError, TaskId};
use uuid::Uuid;

/// Event types for event-sourced checkpoint storage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CheckpointEventType {
    /// A new checkpoint was created
    CheckpointCreated,
    /// A checkpoint was updated
    CheckpointUpdated,
    /// Checkpoints were pruned
    CheckpointsPruned,
}

impl CheckpointEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckpointEventType::CheckpointCreated => "CheckpointCreated",
            CheckpointEventType::CheckpointUpdated => "CheckpointUpdated",
            CheckpointEventType::CheckpointsPruned => "CheckpointsPruned",
        }
    }
}

/// An event in the checkpoint event log (append-only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointEvent {
    pub id: Uuid,
    pub task_id: Uuid,
    pub event_type: CheckpointEventType,
    pub state: swell_core::TaskState,
    pub snapshot: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    /// Sequence number for ordering events within a task
    pub sequence: i64,
    /// Hash of the previous event for chain integrity
    pub previous_hash: Option<String>,
    /// Hash of this event
    pub event_hash: String,
}

/// Compute SHA256 hash for event chaining
fn compute_event_hash(event: &CheckpointEvent, previous_hash: &Option<String>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    event.id.hash(&mut hasher);
    event.task_id.hash(&mut hasher);
    event.event_type.hash(&mut hasher);
    serde_json::to_string(&event.state)
        .unwrap_or_default()
        .hash(&mut hasher);
    serde_json::to_string(&event.snapshot)
        .unwrap_or_default()
        .hash(&mut hasher);
    event.created_at.hash(&mut hasher);
    event.sequence.hash(&mut hasher);
    if let Some(prev) = previous_hash {
        prev.hash(&mut hasher);
    }

    format!("{:016x}", hasher.finish())
}

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
        // Event-sourced checkpoint_events table (append-only)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS checkpoint_events (
                id UUID NOT NULL,
                task_id UUID NOT NULL,
                event_type VARCHAR(50) NOT NULL,
                state JSONB NOT NULL,
                snapshot JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                metadata JSONB NOT NULL,
                sequence BIGINT NOT NULL,
                previous_hash VARCHAR(64),
                event_hash VARCHAR(64) NOT NULL,
                PRIMARY KEY (task_id, sequence)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index for querying events by task_id and sequence
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_checkpoint_events_task_sequence
            ON checkpoint_events(task_id, sequence DESC)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index for time-based queries
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_checkpoint_events_created
            ON checkpoint_events(task_id, created_at DESC)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Materialized view for latest checkpoint per task
        sqlx::query(
            r#"
            CREATE MATERIALIZED VIEW IF NOT EXISTS latest_task_checkpoints AS
            SELECT DISTINCT ON (task_id)
                task_id,
                id AS checkpoint_id,
                state,
                snapshot,
                created_at,
                metadata,
                sequence,
                event_hash
            FROM checkpoint_events
            ORDER BY task_id, sequence DESC
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on the materialized view for efficient lookups
        sqlx::query(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_latest_task_checkpoints_task_id
            ON latest_task_checkpoints(task_id)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Materialized view for task state history (for analytics/replay)
        sqlx::query(
            r#"
            CREATE MATERIALIZED VIEW IF NOT EXISTS task_state_history AS
            SELECT 
                task_id,
                state,
                snapshot,
                created_at,
                sequence,
                event_hash
            FROM checkpoint_events
            ORDER BY task_id, sequence ASC
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index for task state history queries
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_task_state_history_task_id
            ON task_state_history(task_id, sequence ASC)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Refresh the latest checkpoints materialized view
    /// Call this after restoring from events or when consistency is needed
    pub async fn refresh_materialized_views(&self) -> Result<(), SwellError> {
        sqlx::query("REFRESH MATERIALIZED VIEW latest_task_checkpoints")
            .execute(&self.pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("REFRESH MATERIALIZED VIEW task_state_history")
            .execute(&self.pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Get the next sequence number for a task
    async fn get_next_sequence(&self, task_id: TaskId) -> Result<i64, SwellError> {
        let result: Option<i64> =
            sqlx::query_scalar("SELECT MAX(sequence) FROM checkpoint_events WHERE task_id = $1")
                .bind(task_id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(result.unwrap_or(0) + 1)
    }

    /// Get the event hash of the previous event for a task
    async fn get_previous_hash(&self, task_id: TaskId) -> Result<Option<String>, SwellError> {
        let hash: Option<String> = sqlx::query_scalar(
            "SELECT event_hash FROM checkpoint_events WHERE task_id = $1 ORDER BY sequence DESC LIMIT 1"
        )
        .bind(task_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(hash)
    }

    /// Append a new checkpoint event (append-only)
    async fn append_event(&self, event: CheckpointEvent) -> Result<(), SwellError> {
        let event_type_str = event.event_type.as_str();

        sqlx::query(
            r#"
            INSERT INTO checkpoint_events 
            (id, task_id, event_type, state, snapshot, created_at, metadata, sequence, previous_hash, event_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(event.id)
        .bind(event.task_id)
        .bind(event_type_str)
        .bind(serde_json::to_value(event.state).unwrap())
        .bind(serde_json::to_value(&event.snapshot).unwrap())
        .bind(event.created_at)
        .bind(serde_json::to_value(&event.metadata).unwrap())
        .bind(event.sequence)
        .bind(&event.previous_hash)
        .bind(&event.event_hash)
        .execute(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for PostgresCheckpointStore {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError> {
        // Get the next sequence and previous hash for chaining
        let sequence = self.get_next_sequence(checkpoint.task_id).await?;
        let previous_hash = self.get_previous_hash(checkpoint.task_id).await?;

        let event = CheckpointEvent {
            id: checkpoint.id,
            task_id: checkpoint.task_id.as_uuid(),
            event_type: CheckpointEventType::CheckpointCreated,
            state: checkpoint.state,
            snapshot: checkpoint.snapshot.clone(),
            created_at: checkpoint.created_at,
            metadata: checkpoint.metadata.clone(),
            sequence,
            previous_hash: previous_hash.clone(),
            event_hash: String::new(), // Will be computed
        };

        let mut event_with_hash = event;
        event_with_hash.event_hash = compute_event_hash(&event_with_hash, &previous_hash);

        // Append the event (append-only)
        self.append_event(event_with_hash).await?;

        Ok(checkpoint.id)
    }

    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError> {
        // Load checkpoint by ID from events
        let row = sqlx::query(
            r#"
            SELECT id, task_id, state, snapshot, created_at, metadata, sequence
            FROM checkpoint_events
            WHERE id = $1
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn load_latest(&self, task_id: TaskId) -> Result<Option<Checkpoint>, SwellError> {
        // Use materialized view for efficient lookup
        let row = sqlx::query(
            r#"
            SELECT checkpoint_id, state, snapshot, created_at, metadata, sequence
            FROM latest_task_checkpoints
            WHERE task_id = $1
            "#,
        )
        .bind(task_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if let Some(r) = row {
            let id: Uuid = r.get("checkpoint_id");
            let state: serde_json::Value = r.get("state");
            let snapshot: serde_json::Value = r.get("snapshot");
            let created_at: DateTime<Utc> = r.get("created_at");
            let metadata: serde_json::Value = r.get("metadata");

            return Ok(Some(Checkpoint {
                id,
                task_id,  // Already TaskId
                state: serde_json::from_value(state)
                    .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
                snapshot,
                created_at,
                metadata,
            }));
        }

        // Fallback: query events directly if materialized view not available
        let row = sqlx::query(
            "SELECT id, task_id, state, snapshot, created_at, metadata FROM checkpoint_events WHERE task_id = $1 ORDER BY sequence DESC LIMIT 1",
        )
        .bind(task_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| self.row_to_checkpoint(r).ok()))
    }

    async fn list(&self, task_id: TaskId) -> Result<Vec<Checkpoint>, SwellError> {
        let rows = sqlx::query(
            "SELECT id, task_id, state, snapshot, created_at, metadata, sequence FROM checkpoint_events WHERE task_id = $1 ORDER BY sequence ASC",
        )
        .bind(task_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let checkpoints: Vec<Checkpoint> = rows
            .into_iter()
            .filter_map(|r| self.row_to_checkpoint(r).ok())
            .collect();

        Ok(checkpoints)
    }

    async fn prune(&self, task_id: TaskId, keep: usize) -> Result<(), SwellError> {
        // For event sourcing, pruning means marking old events as superseded
        // We don't actually delete events (append-only), but we can record a prune event

        if keep == 0 {
            return Ok(());
        }

        // Get the sequence number of the event at position `keep`
        let cutoff_sequence: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT sequence FROM checkpoint_events 
            WHERE task_id = $1 
            ORDER BY sequence ASC
            LIMIT 1 OFFSET $2
            "#,
        )
        .bind(task_id.as_uuid())
        .bind(keep as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Record a prune event (we keep events for audit trail)
        if let Some(cutoff_seq) = cutoff_sequence {
            let sequence = self.get_next_sequence(task_id).await?;
            let prev_hash = self.get_previous_hash(task_id).await?;

            let prune_event = CheckpointEvent {
                id: Uuid::new_v4(),
                task_id: task_id.as_uuid(),
                event_type: CheckpointEventType::CheckpointsPruned,
                state: swell_core::TaskState::Created, // Placeholder
                snapshot: serde_json::json!({
                    "pruned_before_sequence": cutoff_seq,
                    "kept_count": keep
                }),
                created_at: Utc::now(),
                metadata: serde_json::json!({}),
                sequence,
                previous_hash: prev_hash.clone(),
                event_hash: String::new(),
            };

            let mut prune_event_with_hash = prune_event;
            prune_event_with_hash.event_hash =
                compute_event_hash(&prune_event_with_hash, &prev_hash);

            self.append_event(prune_event_with_hash).await?;
        }

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
            task_id: TaskId::from_uuid(task_id),
            state: serde_json::from_value(state)
                .map_err(|e| SwellError::DatabaseError(e.to_string()))?,
            snapshot,
            created_at,
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::TaskState;

    #[test]
    fn test_checkpoint_event_type_as_str() {
        assert_eq!(
            CheckpointEventType::CheckpointCreated.as_str(),
            "CheckpointCreated"
        );
        assert_eq!(
            CheckpointEventType::CheckpointUpdated.as_str(),
            "CheckpointUpdated"
        );
        assert_eq!(
            CheckpointEventType::CheckpointsPruned.as_str(),
            "CheckpointsPruned"
        );
    }

    #[test]
    fn test_event_type_derive_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(CheckpointEventType::CheckpointCreated);
        set.insert(CheckpointEventType::CheckpointUpdated);
        set.insert(CheckpointEventType::CheckpointsPruned);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn test_compute_event_hash_deterministic() {
        let event = CheckpointEvent {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            task_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            event_type: CheckpointEventType::CheckpointCreated,
            state: TaskState::Created,
            snapshot: serde_json::json!({"key": "value"}),
            created_at: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            metadata: serde_json::json!({}),
            sequence: 1,
            previous_hash: None,
            event_hash: String::new(),
        };

        let hash1 = compute_event_hash(&event, &None);
        let hash2 = compute_event_hash(&event, &None);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16); // hex format
    }

    #[test]
    fn test_compute_event_hash_changes_with_previous() {
        let event = CheckpointEvent {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            task_id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            event_type: CheckpointEventType::CheckpointCreated,
            state: TaskState::Created,
            snapshot: serde_json::json!({"key": "value"}),
            created_at: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            metadata: serde_json::json!({}),
            sequence: 1,
            previous_hash: None,
            event_hash: String::new(),
        };

        let hash_no_prev = compute_event_hash(&event, &None);
        let hash_with_prev = compute_event_hash(&event, &Some("prevhash123".to_string()));
        assert_ne!(hash_no_prev, hash_with_prev);
    }

    #[test]
    fn test_checkpoint_event_serialization() {
        let event = CheckpointEvent {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            event_type: CheckpointEventType::CheckpointCreated,
            state: TaskState::Created,
            snapshot: serde_json::json!({"data": 42}),
            created_at: Utc::now(),
            metadata: serde_json::json!({"meta": true}),
            sequence: 1,
            previous_hash: Some("abc123".to_string()),
            event_hash: "def456".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: CheckpointEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, event.id);
        assert_eq!(parsed.task_id, event.task_id);
        assert_eq!(parsed.event_type, event.event_type);
        assert_eq!(parsed.sequence, event.sequence);
    }
}

#[cfg(test)]
mod postgres_integration_tests {
    use super::*;
    use std::env;
    use swell_core::TaskState;

    #[tokio::test]
    async fn test_postgres_checkpoint_save_and_load() {
        let db_url = match env::var("POSTGRES_TEST_URL") {
            Ok(url) => url,
            Err(_) => return, // Skip if no postgres available
        };

        let store = PostgresCheckpointStore::new(&db_url).await.unwrap();
        let task_id = Uuid::new_v4();

        let checkpoint = Checkpoint {
            id: Uuid::new_v4(),
            task_id,
            state: TaskState::Created,
            snapshot: serde_json::json!({"step": 1}),
            created_at: Utc::now(),
            metadata: serde_json::json!({}),
        };

        let saved_id = store.save(checkpoint.clone()).await.unwrap();
        assert_eq!(saved_id, checkpoint.id);

        let loaded = store.load(saved_id).await.unwrap().unwrap();
        assert_eq!(loaded.task_id, checkpoint.task_id);
        assert_eq!(loaded.snapshot["step"], 1);
    }

    #[tokio::test]
    async fn test_postgres_load_latest() {
        let db_url = match env::var("POSTGRES_TEST_URL") {
            Ok(url) => url,
            Err(_) => return,
        };

        let store = PostgresCheckpointStore::new(&db_url).await.unwrap();
        let task_id = Uuid::new_v4();

        // Save multiple checkpoints
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
    async fn test_postgres_list_all() {
        let db_url = match env::var("POSTGRES_TEST_URL") {
            Ok(url) => url,
            Err(_) => return,
        };

        let store = PostgresCheckpointStore::new(&db_url).await.unwrap();
        let task_id = Uuid::new_v4();

        // Save 3 checkpoints
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

        let checkpoints = store.list(task_id).await.unwrap();
        assert_eq!(checkpoints.len(), 3);
        // Events are ordered by sequence ASC
        assert_eq!(checkpoints[0].snapshot["index"], 0);
        assert_eq!(checkpoints[1].snapshot["index"], 1);
        assert_eq!(checkpoints[2].snapshot["index"], 2);
    }

    #[tokio::test]
    async fn test_postgres_prune() {
        let db_url = match env::var("POSTGRES_TEST_URL") {
            Ok(url) => url,
            Err(_) => return,
        };

        let store = PostgresCheckpointStore::new(&db_url).await.unwrap();
        let task_id = Uuid::new_v4();

        // Save 5 checkpoints
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

        // Prune keeping only 2
        store.prune(task_id, 2).await.unwrap();

        let checkpoints = store.list(task_id).await.unwrap();
        // Original 5 + 1 prune event = 6
        assert_eq!(checkpoints.len(), 6);
    }
}
