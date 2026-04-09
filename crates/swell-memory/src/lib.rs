// swell-memory - Memory layer (SQLite implementation)
//
// This crate provides a SQLite-based implementation of the MemoryStore trait
// for persistent memory storage.

use async_trait::async_trait;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

pub use swell_core::MemoryBlockType;
pub use swell_core::MemoryEntry;
pub use swell_core::MemoryQuery;
pub use swell_core::MemorySearchResult;
pub use swell_core::MemoryStore;
pub use swell_core::SwellError;

/// SQLite-based implementation of the MemoryStore trait
#[derive(Clone)]
pub struct SqliteMemoryStore {
    pool: Arc<SqlitePool>,
}

impl SqliteMemoryStore {
    /// Create a new SqliteMemoryStore with the given database URL (async)
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        Self::create(database_url).await
    }

    /// Create a new pool with the given database URL
    pub async fn create(database_url: &str) -> Result<Self, SwellError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Initialize the schema
        Self::init_schema(&pool).await?;

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Initialize the database schema
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS memory_entries (
                id TEXT PRIMARY KEY,
                block_type TEXT NOT NULL,
                label TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_block_type ON memory_entries(block_type)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_memory_label ON memory_entries(label)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Helper to convert block_type enum to string
    fn block_type_to_string(block_type: MemoryBlockType) -> String {
        match block_type {
            MemoryBlockType::Project => "Project".to_string(),
            MemoryBlockType::User => "User".to_string(),
            MemoryBlockType::Task => "Task".to_string(),
            MemoryBlockType::Skill => "Skill".to_string(),
            MemoryBlockType::Convention => "Convention".to_string(),
        }
    }

    /// Helper to convert string to block_type enum
    fn string_to_block_type(s: &str) -> MemoryBlockType {
        match s {
            "Project" => MemoryBlockType::Project,
            "User" => MemoryBlockType::User,
            "Task" => MemoryBlockType::Task,
            "Skill" => MemoryBlockType::Skill,
            "Convention" => MemoryBlockType::Convention,
            _ => MemoryBlockType::Project,
        }
    }

    /// Helper to serialize embedding to bytes
    fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
        embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// Helper to deserialize embedding from bytes
    fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    /// Helper to convert database row to MemoryEntry
    fn row_to_entry(row: &SqliteRow) -> Result<MemoryEntry, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let block_type_str: String = row.get("block_type");
        let label: String = row.get("label");
        let content: String = row.get("content");
        let embedding_bytes: Option<Vec<u8>> = row.get("embedding");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let metadata_str: String = row.get("metadata");

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;

        let embedding = embedding_bytes.map(|bytes: Vec<u8>| Self::bytes_to_embedding(&bytes));

        Ok(MemoryEntry {
            id,
            block_type: Self::string_to_block_type(&block_type_str),
            label,
            content,
            embedding,
            created_at,
            updated_at,
            metadata,
        })
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    /// Store a new memory entry
    async fn store(&self, entry: MemoryEntry) -> Result<Uuid, SwellError> {
        let block_type_str = Self::block_type_to_string(entry.block_type);
        let embedding_bytes = entry
            .embedding
            .as_ref()
            .map(|e| Self::embedding_to_bytes(e));
        let metadata_str = serde_json::to_string(&entry.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = entry.created_at.to_rfc3339();
        let updated_at_str = entry.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO memory_entries (id, block_type, label, content, embedding, created_at, updated_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry.id.to_string())
        .bind(block_type_str)
        .bind(&entry.label)
        .bind(&entry.content)
        .bind(embedding_bytes)
        .bind(created_at_str)
        .bind(updated_at_str)
        .bind(metadata_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(entry.id)
    }

    /// Retrieve a memory entry by ID
    async fn get(&self, id: Uuid) -> Result<Option<MemoryEntry>, SwellError> {
        let row = sqlx::query("SELECT * FROM memory_entries WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_entry(&r)?)),
            None => Ok(None),
        }
    }

    /// Update an existing memory entry
    async fn update(&self, entry: MemoryEntry) -> Result<(), SwellError> {
        let block_type_str = Self::block_type_to_string(entry.block_type);
        let embedding_bytes = entry
            .embedding
            .as_ref()
            .map(|e| Self::embedding_to_bytes(e));
        let metadata_str = serde_json::to_string(&entry.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let updated_at_str = chrono::Utc::now().to_rfc3339();

        let result = sqlx::query(
            r#"
            UPDATE memory_entries 
            SET block_type = ?, label = ?, content = ?, embedding = ?, updated_at = ?, metadata = ?
            WHERE id = ?
            "#,
        )
        .bind(block_type_str)
        .bind(&entry.label)
        .bind(&entry.content)
        .bind(embedding_bytes)
        .bind(updated_at_str)
        .bind(metadata_str)
        .bind(entry.id.to_string())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "No entry found with id {}",
                entry.id
            )));
        }

        Ok(())
    }

    /// Delete a memory entry by ID
    async fn delete(&self, id: Uuid) -> Result<(), SwellError> {
        let result = sqlx::query("DELETE FROM memory_entries WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "No entry found with id {}",
                id
            )));
        }

        Ok(())
    }

    /// Search memories by query (basic LIKE for MVP, vector search can be stubbed)
    async fn search(&self, query: MemoryQuery) -> Result<Vec<MemorySearchResult>, SwellError> {
        let mut sql = String::from("SELECT * FROM memory_entries WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(ref query_text) = query.query_text {
            sql.push_str(" AND (content LIKE ? OR label LIKE ?)");
            let pattern = format!("%{}%", query_text);
            params.push(pattern.clone());
            params.push(pattern);
        }

        if let Some(ref block_types) = query.block_types {
            if !block_types.is_empty() {
                let placeholders: Vec<String> =
                    block_types.iter().map(|_| "?".to_string()).collect();
                sql.push_str(&format!(" AND block_type IN ({})", placeholders.join(", ")));
                for bt in block_types {
                    params.push(Self::block_type_to_string(*bt));
                }
            }
        }

        if let Some(ref labels) = query.labels {
            if !labels.is_empty() {
                let placeholders: Vec<String> = labels.iter().map(|_| "?".to_string()).collect();
                sql.push_str(&format!(" AND label IN ({})", placeholders.join(", ")));
                params.extend(labels.clone());
            }
        }

        sql.push_str(&format!(" LIMIT {} OFFSET {}", query.limit, query.offset));

        let mut q = sqlx::query(&sql);
        for param in &params {
            q = q.bind(param);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let entry = Self::row_to_entry(&row)?;
            // For MVP, use a simple relevance score based on label match
            let score = if let Some(ref query_text) = query.query_text {
                if entry
                    .label
                    .to_lowercase()
                    .contains(&query_text.to_lowercase())
                {
                    0.9
                } else if entry
                    .content
                    .to_lowercase()
                    .contains(&query_text.to_lowercase())
                {
                    0.7
                } else {
                    0.5
                }
            } else {
                0.5
            };
            results.push(MemorySearchResult { entry, score });
        }

        Ok(results)
    }

    /// Get all memories of a specific type
    async fn get_by_type(
        &self,
        block_type: MemoryBlockType,
    ) -> Result<Vec<MemoryEntry>, SwellError> {
        let block_type_str = Self::block_type_to_string(block_type);
        let rows = sqlx::query("SELECT * FROM memory_entries WHERE block_type = ?")
            .bind(block_type_str)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(Self::row_to_entry(&row)?);
        }

        Ok(entries)
    }

    /// Get all memories with a specific label
    async fn get_by_label(&self, label: String) -> Result<Vec<MemoryEntry>, SwellError> {
        let rows = sqlx::query("SELECT * FROM memory_entries WHERE label = ?")
            .bind(label)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(Self::row_to_entry(&row)?);
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_and_get() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "test-project".to_string(),
            content: "Test content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        let id = store.store(entry.clone()).await.unwrap();
        assert_eq!(id, entry.id);

        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.label, entry.label);
        assert_eq!(retrieved.content, entry.content);
    }

    #[tokio::test]
    async fn test_update_and_delete() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "task-1".to_string(),
            content: "Original content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        store.store(entry.clone()).await.unwrap();

        let mut updated = entry.clone();
        updated.content = "Updated content".to_string();
        store.update(updated.clone()).await.unwrap();

        let retrieved = store.get(entry.id).await.unwrap().unwrap();
        assert_eq!(retrieved.content, "Updated content");

        store.delete(entry.id).await.unwrap();
        let retrieved = store.get(entry.id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_search() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "my-project".to_string(),
            content: "This is about Rust programming".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "my-task".to_string(),
            content: "Another task".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        store.store(entry1.clone()).await.unwrap();
        store.store(entry2.clone()).await.unwrap();

        let results = store
            .search(MemoryQuery {
                query_text: Some("Rust".to_string()),
                block_types: None,
                labels: None,
                limit: 10,
                offset: 0,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, entry1.id);
    }

    #[tokio::test]
    async fn test_get_by_type() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-1".to_string(),
            content: "Project content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "task-1".to_string(),
            content: "Task content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        store.store(entry1.clone()).await.unwrap();
        store.store(entry2.clone()).await.unwrap();

        let projects = store.get_by_type(MemoryBlockType::Project).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, entry1.id);

        let tasks = store.get_by_type(MemoryBlockType::Task).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, entry2.id);
    }

    #[tokio::test]
    async fn test_get_by_label() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "unique-label".to_string(),
            content: "Content 1".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "other-label".to_string(),
            content: "Content 2".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };

        store.store(entry1.clone()).await.unwrap();
        store.store(entry2.clone()).await.unwrap();

        let results = store
            .get_by_label("unique-label".to_string())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, entry1.id);
    }
}
