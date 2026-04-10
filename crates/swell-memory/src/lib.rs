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

// Memory blocks module - Project/User/Task blocks with auto-loading and context assembly
pub mod blocks;

// Event log module - Append-only JSONL event log with schema versioning for immutable audit trail
pub mod event_log;

// Recall module - BM25 keyword search and temporal queries for conversation logs
pub mod recall;

// Skill extraction module - Extracts reusable procedures from successful task trajectories
pub mod skill_extraction;

// Pattern learning module - Learns anti-patterns from rejection feedback and extracts conventions
pub mod pattern_learning;

// Operator feedback module - Parses CLAUDE.md/AGENTS.md with higher trust weight than agent self-learning
pub mod operator_feedback;

pub use operator_feedback::{
    OperatorFeedbackConfig, OperatorFeedbackParser, OperatorFeedbackResult,
    OperatorFeedbackService, OperatorGuidancePattern, OperatorPatternType,
    OPERATOR_FEEDBACK_BASE_CONFIDENCE, OPERATOR_FEEDBACK_MIN_CONFIDENCE,
};

// Semantic memory module - Facts, entities, and relationships stored as graph nodes
// for semantic knowledge representation
pub mod semantic;

pub use semantic::{
    SemanticEntity, SemanticEntityQuery, SemanticEntityType, SemanticRelation,
    SemanticRelationQuery, SemanticRelationResult, SemanticRelationType, SemanticStore,
    SqliteSemanticStore,
};

// Procedural memory module - Strategies, procedures, and action patterns stored with
// Beta posterior distribution for confidence scoring
pub mod procedural;

pub use procedural::{
    BetaPosterior, ConfidenceLevel, ProceduralStore, Procedure, ProcedureQuery, ProcedureResult,
    ProcedureStep, SqliteProceduralStore,
};

// Meta-cognitive memory module - Self-knowledge for model performance tracking,
// prompting strategy storage, and recommendations
pub mod meta_cognitive;

pub use meta_cognitive::{
    AlternativeRecommendation, MetaCognitiveQuery, MetaCognitiveStore, ModelPerformance,
    PromptingStrategy, Recommendation, SqliteMetaCognitiveStore, TaskType,
};

// Time-based decay module - Different decay rates per memory type:
// Procedural (slow): 0.99^(days), Environmental (medium): 0.95^(days), Buffer (fast): 0.90^(days)
pub mod decay;

pub use decay::{
    apply_decay, buffer_decay_rate, calculate_decay, days_since, decay_rate_for_block_type,
    environmental_decay_rate, procedural_decay_rate, DecayRate, DecayedScore,
};

// Deprecation module - Mark memories with confidence <0.3 as deprecated with superseded_by link
pub mod deprecation;

pub use deprecation::{
    apply_confidence_deprecation, check_deprecation, deprecation_score, should_be_deprecated,
    DeprecationCheckResult, DeprecationInfo, DeprecationReason, DeprecationRecommendation,
    DEPRECATION_CONFIDENCE_THRESHOLD,
};

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
                metadata TEXT NOT NULL,
                repository TEXT NOT NULL DEFAULT '',
                language TEXT,
                task_type TEXT
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

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memory_repository ON memory_entries(repository)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_memory_language ON memory_entries(language)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_memory_task_type ON memory_entries(task_type)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Initialize conversation_logs schema for recall functionality
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS conversation_logs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                task_id TEXT,
                agent_id TEXT NOT NULL,
                agent_role TEXT NOT NULL,
                action TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                metadata TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_convlogs_session ON conversation_logs(session_id)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_convlogs_task ON conversation_logs(task_id)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_convlogs_timestamp ON conversation_logs(timestamp)",
        )
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

    /// Compute cosine distance between two embeddings
    /// Returns a value between 0 (identical) and 1 (orthogonal)
    fn cosine_distance(embedding1: &[f32], embedding2: &[f32]) -> f32 {
        if embedding1.len() != embedding2.len() {
            return 1.0; // Different dimensions = maximum distance
        }

        let dot_product: f32 = embedding1
            .iter()
            .zip(embedding2.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm1: f32 = embedding1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = embedding2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 1.0; // Zero vector = maximum distance
        }

        let cosine_similarity = dot_product / (norm1 * norm2);
        // Cosine distance = 1 - cosine similarity
        // Clamp to handle floating point errors
        (1.0 - cosine_similarity).clamp(0.0, 1.0)
    }

    /// Check if entry is too similar to any existing memory in the same repository
    /// Returns Some(existing_id) if similar memory found, None otherwise
    async fn find_similar_memory(
        &self,
        entry: &MemoryEntry,
        max_distance: f32,
    ) -> Result<Option<Uuid>, SwellError> {
        // Only check if the entry has an embedding
        let Some(new_embedding) = &entry.embedding else {
            return Ok(None);
        };

        // Query all entries with embeddings in the same repository
        let rows = sqlx::query(
            r#"
            SELECT * FROM memory_entries 
            WHERE repository = ? AND embedding IS NOT NULL
            "#,
        )
        .bind(&entry.repository)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        for row in rows {
            let existing_entry = Self::row_to_entry(&row)?;

            // Skip the entry itself (for updates)
            if existing_entry.id == entry.id {
                continue;
            }

            if let Some(existing_embedding) = &existing_entry.embedding {
                let distance = Self::cosine_distance(new_embedding, existing_embedding);

                if distance < max_distance {
                    return Ok(Some(existing_entry.id));
                }
            }
        }

        Ok(None)
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
        let repository: String = row.get("repository");
        let language: Option<String> = row.get("language");
        let task_type: Option<String> = row.get("task_type");

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
            repository,
            language,
            task_type,
        })
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    /// Store a new memory entry
    /// Rejects memories with cosine distance < 0.15 (similarity > 0.85) to existing memories
    /// in the same repository to prevent duplication.
    async fn store(&self, entry: MemoryEntry) -> Result<Uuid, SwellError> {
        // Check for similar memories before storing
        // Reject if cosine distance < 0.15 (i.e., similarity > 0.85)
        const SIMILARITY_THRESHOLD: f32 = 0.15;
        if let Some(similar_id) = self
            .find_similar_memory(&entry, SIMILARITY_THRESHOLD)
            .await?
        {
            return Err(SwellError::SimilarMemoryFound(similar_id));
        }

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
            INSERT INTO memory_entries (id, block_type, label, content, embedding, created_at, updated_at, metadata, repository, language, task_type)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(&entry.repository)
        .bind(&entry.language)
        .bind(&entry.task_type)
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
        let mut sql = String::from("SELECT * FROM memory_entries WHERE repository = ?");
        let mut params: Vec<String> = Vec::new();

        // Repository scope is REQUIRED - this ensures cross-repo isolation
        params.push(query.repository.clone());

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

        // Optional language filter
        if let Some(ref language) = query.language {
            sql.push_str(" AND language = ?");
            params.push(language.clone());
        }

        // Optional task_type filter
        if let Some(ref task_type) = query.task_type {
            sql.push_str(" AND task_type = ?");
            params.push(task_type.clone());
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
            repository: "test-repo".to_string(),
            language: Some("rust".to_string()),
            task_type: None,
        };

        let id = store.store(entry.clone()).await.unwrap();
        assert_eq!(id, entry.id);

        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.label, entry.label);
        assert_eq!(retrieved.content, entry.content);
        assert_eq!(retrieved.repository, "test-repo");
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: Some("bugfix".to_string()),
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
            repository: "my-repo".to_string(),
            language: Some("rust".to_string()),
            task_type: None,
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
            repository: "my-repo".to_string(),
            language: None,
            task_type: Some("feature".to_string()),
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
                repository: "my-repo".to_string(),
                language: None,
                task_type: None,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, entry1.id);
    }

    #[tokio::test]
    async fn test_search_by_language_filter() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "rust-project".to_string(),
            content: "Rust project content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: Some("rust".to_string()),
            task_type: None,
        };

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "python-project".to_string(),
            content: "Python project content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: Some("python".to_string()),
            task_type: None,
        };

        store.store(entry1.clone()).await.unwrap();
        store.store(entry2.clone()).await.unwrap();

        // Search with language filter
        let results = store
            .search(MemoryQuery {
                query_text: None,
                block_types: None,
                labels: None,
                limit: 10,
                offset: 0,
                repository: "test-repo".to_string(),
                language: Some("rust".to_string()),
                task_type: None,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.language, Some("rust".to_string()));
    }

    #[tokio::test]
    async fn test_search_cross_repo_isolation() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-a".to_string(),
            content: "Project A content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
        };

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-b".to_string(),
            content: "Project B content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "repo-b".to_string(),
            language: None,
            task_type: None,
        };

        store.store(entry1.clone()).await.unwrap();
        store.store(entry2.clone()).await.unwrap();

        // Search for repo-a - should only find entry1
        let results = store
            .search(MemoryQuery {
                query_text: None,
                block_types: None,
                labels: None,
                limit: 10,
                offset: 0,
                repository: "repo-a".to_string(),
                language: None,
                task_type: None,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.repository, "repo-a");

        // Search for repo-b - should only find entry2
        let results = store
            .search(MemoryQuery {
                query_text: None,
                block_types: None,
                labels: None,
                limit: 10,
                offset: 0,
                repository: "repo-b".to_string(),
                language: None,
                task_type: None,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.repository, "repo-b");
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
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

    #[tokio::test]
    async fn test_similarity_check_rejects_similar_embeddings() {
        use swell_core::SwellError;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create a base embedding
        let base_embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-1".to_string(),
            content: "Project content".to_string(),
            embedding: Some(base_embedding.clone()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Store the first entry - should succeed
        store.store(entry1.clone()).await.unwrap();

        // Create a very similar embedding (distance < 0.15)
        let similar_embedding = vec![0.11, 0.21, 0.31, 0.41, 0.51];

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-2".to_string(),
            content: "Different content".to_string(),
            embedding: Some(similar_embedding),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Try to store similar entry - should be rejected
        let result = store.store(entry2).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            SwellError::SimilarMemoryFound(existing_id) => {
                assert_eq!(existing_id, entry1.id);
            }
            _ => panic!("Expected SimilarMemoryFound error"),
        }
    }

    #[tokio::test]
    async fn test_similarity_check_accepts_different_embeddings() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create a base embedding
        let base_embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];

        let entry1 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-1".to_string(),
            content: "Project content".to_string(),
            embedding: Some(base_embedding),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Store the first entry - should succeed
        store.store(entry1.clone()).await.unwrap();

        // Create a very different embedding (distance > 0.15)
        let different_embedding = vec![0.9, 0.8, 0.7, 0.6, 0.5];

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-2".to_string(),
            content: "Different content".to_string(),
            embedding: Some(different_embedding),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Try to store different entry - should succeed
        let result = store.store(entry2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_similarity_check_allows_no_embedding() {
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
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Store entry without embedding - should succeed
        store.store(entry1.clone()).await.unwrap();

        let entry2 = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-2".to_string(),
            content: "Different content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
        };

        // Store another entry without embedding - should also succeed
        // (no similarity check performed without embeddings)
        let result = store.store(entry2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_similarity_check_is_repository_scoped() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create embedding in repo-a
        let embedding_a = vec![0.1, 0.2, 0.3, 0.4, 0.5];

        let entry_a = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-a".to_string(),
            content: "Project A content".to_string(),
            embedding: Some(embedding_a),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
        };

        store.store(entry_a.clone()).await.unwrap();

        // Create very similar embedding in repo-b
        let similar_embedding = vec![0.11, 0.21, 0.31, 0.41, 0.51];

        let entry_b = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "project-b".to_string(),
            content: "Project B content".to_string(),
            embedding: Some(similar_embedding),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "repo-b".to_string(),
            language: None,
            task_type: None,
        };

        // Should succeed because different repositories are isolated
        let result = store.store(entry_b).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_cosine_distance_identical_embeddings() {
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let distance = SqliteMemoryStore::cosine_distance(&embedding, &embedding);
        assert!(
            distance < 0.001,
            "Identical embeddings should have distance near 0"
        );
    }

    #[test]
    fn test_cosine_distance_orthogonal_embeddings() {
        // [1, 0, 0] and [0, 1, 0] are orthogonal
        let embedding1 = vec![1.0, 0.0, 0.0];
        let embedding2 = vec![0.0, 1.0, 0.0];
        let distance = SqliteMemoryStore::cosine_distance(&embedding1, &embedding2);
        assert!(
            distance > 0.99,
            "Orthogonal embeddings should have distance near 1"
        );
    }

    #[test]
    fn test_cosine_distance_different_lengths() {
        let embedding1 = vec![0.1, 0.2, 0.3];
        let embedding2 = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let distance = SqliteMemoryStore::cosine_distance(&embedding1, &embedding2);
        assert_eq!(
            distance, 1.0,
            "Different length embeddings should have max distance"
        );
    }

    #[test]
    fn test_cosine_distance_zero_vectors() {
        let embedding1 = vec![0.0, 0.0, 0.0];
        let embedding2 = vec![0.0, 0.0, 0.0];
        let distance = SqliteMemoryStore::cosine_distance(&embedding1, &embedding2);
        assert_eq!(distance, 1.0, "Zero vectors should have max distance");
    }
}
