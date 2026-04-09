// recall.rs - Memory recall with BM25 keyword search and temporal queries
//
// This module provides searchable conversation logs with BM25 ranking
// and time-based filtering for agent interactions.

use crate::{SqliteMemoryStore, SwellError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Represents an agent interaction log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub agent_id: Uuid,
    pub agent_role: String,
    pub action: ConversationAction,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl ConversationLog {
    /// Create a new conversation log entry
    pub fn new(
        session_id: Uuid,
        agent_id: Uuid,
        agent_role: String,
        action: ConversationAction,
        content: String,
        task_id: Option<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            task_id,
            agent_id,
            agent_role,
            action,
            content,
            timestamp: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }
}

/// Types of conversation actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationAction {
    /// Agent started
    Start,
    /// Agent finished
    Finish,
    /// Tool was invoked
    ToolCall,
    /// Tool result received
    ToolResult,
    /// LLM prompt sent
    Prompt,
    /// LLM response received
    Response,
    /// Error occurred
    Error,
    /// Observation recorded
    Observation,
    /// Decision made
    Decision,
}

impl ConversationAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConversationAction::Start => "start",
            ConversationAction::Finish => "finish",
            ConversationAction::ToolCall => "tool_call",
            ConversationAction::ToolResult => "tool_result",
            ConversationAction::Prompt => "prompt",
            ConversationAction::Response => "response",
            ConversationAction::Error => "error",
            ConversationAction::Observation => "observation",
            ConversationAction::Decision => "decision",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "start" => Some(ConversationAction::Start),
            "finish" => Some(ConversationAction::Finish),
            "tool_call" => Some(ConversationAction::ToolCall),
            "tool_result" => Some(ConversationAction::ToolResult),
            "prompt" => Some(ConversationAction::Prompt),
            "response" => Some(ConversationAction::Response),
            "error" => Some(ConversationAction::Error),
            "observation" => Some(ConversationAction::Observation),
            "decision" => Some(ConversationAction::Decision),
            _ => None,
        }
    }
}

/// Query for recall search with BM25 parameters and temporal filters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallQuery {
    /// Keywords to search for (BM25 will be applied)
    pub keywords: Vec<String>,
    /// Filter by session ID
    pub session_id: Option<Uuid>,
    /// Filter by task ID
    pub task_id: Option<Uuid>,
    /// Filter by agent role
    pub agent_role: Option<String>,
    /// Filter by action type
    pub action: Option<ConversationAction>,
    /// Start of time range (inclusive)
    pub start_time: Option<DateTime<Utc>>,
    /// End of time range (inclusive)
    pub end_time: Option<DateTime<Utc>>,
    /// Maximum number of results
    pub limit: usize,
    /// Offset for pagination
    pub offset: usize,
    /// BM25 parameters (optional, uses defaults if not set)
    pub bm25_params: Option<Bm25Params>,
}

impl Default for RecallQuery {
    fn default() -> Self {
        Self {
            keywords: Vec::new(),
            session_id: None,
            task_id: None,
            agent_role: None,
            action: None,
            start_time: None,
            end_time: None,
            limit: 10,
            offset: 0,
            bm25_params: None,
        }
    }
}

/// BM25 ranking parameters
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bm25Params {
    /// Term frequency saturation parameter (default: 1.2)
    pub k1: f32,
    /// Document length normalization (default: 1.5)
    pub b: f32,
    /// Average document length (computed from corpus if not set)
    pub avgdl: Option<f32>,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self {
            k1: 1.2,
            b: 0.75,
            avgdl: None,
        }
    }
}

/// Result from recall search with BM25 score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub log: ConversationLog,
    pub score: f32,
}

/// BM25 document representation
#[derive(Debug, Clone)]
struct Bm25Document {
    id: Uuid,
    terms: HashMap<String, f32>, // term -> tf (term frequency)
    #[allow(dead_code)]
    term_count: usize,
}

impl SqliteMemoryStore {
    /// Store a conversation log entry
    pub async fn store_conversation_log(
        &self,
        log: ConversationLog,
    ) -> Result<Uuid, SwellError> {
        let metadata_str = serde_json::to_string(&log.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO conversation_logs (id, session_id, task_id, agent_id, agent_role, action, content, timestamp, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(log.id.to_string())
        .bind(log.session_id.to_string())
        .bind(log.task_id.map(|id| id.to_string()))
        .bind(log.agent_id.to_string())
        .bind(&log.agent_role)
        .bind(log.action.as_str())
        .bind(&log.content)
        .bind(log.timestamp.to_rfc3339())
        .bind(metadata_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(log.id)
    }

    /// Get a conversation log by ID
    pub async fn get_conversation_log(
        &self,
        id: Uuid,
    ) -> Result<Option<ConversationLog>, SwellError> {
        let row = sqlx::query("SELECT * FROM conversation_logs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(self.row_to_conversation_log(&r)?)),
            None => Ok(None),
        }
    }

    /// Get conversation logs by session ID
    pub async fn get_conversation_logs_by_session(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<ConversationLog>, SwellError> {
        let rows = sqlx::query(
            "SELECT * FROM conversation_logs WHERE session_id = ? ORDER BY timestamp ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut logs = Vec::new();
        for row in rows {
            logs.push(self.row_to_conversation_log(&row)?);
        }
        Ok(logs)
    }

    /// Search conversation logs with BM25 keyword search and temporal filtering
    pub async fn recall_search(
        &self,
        query: RecallQuery,
    ) -> Result<Vec<RecallResult>, SwellError> {
        // First, get all candidate logs based on temporal and filter criteria
        let candidates = self.get_candidate_logs(&query).await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // If no keywords, just return candidates sorted by timestamp
        if query.keywords.is_empty() {
            let mut results: Vec<RecallResult> = candidates
                .into_iter()
                .map(|log| RecallResult {
                    log,
                    score: 1.0, // Default score when no keywords
                })
                .collect();
            results.sort_by(|a, b| b.log.timestamp.cmp(&a.log.timestamp));
            results.truncate(query.limit);
            return Ok(results);
        }

        // Tokenize keywords
        let keywords: HashSet<String> = query
            .keywords
            .iter()
            .flat_map(|kw| tokenize(kw))
            .collect();

        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        // Build BM25 documents from candidates
        let documents: Vec<Bm25Document> = candidates
            .iter()
            .map(|log| {
                let terms = extract_terms(&log.content);
                let term_count = terms.len();
                Bm25Document {
                    id: log.id,
                    terms,
                    term_count,
                }
            })
            .collect();

        // Calculate average document length
        let avgdl = if documents.is_empty() {
            0.0
        } else {
            documents.iter().map(|d| d.term_count as f32).sum::<f32>() / documents.len() as f32
        };

        let params = query.bm25_params.unwrap_or_default();
        let avgdl = params.avgdl.unwrap_or(avgdl);

        // Get document frequencies for IDF calculation
        let doc_freqs = calculate_document_frequencies(&documents, &keywords);
        let n = documents.len() as f32;

        // Score each document
        let mut scored_results: Vec<RecallResult> = candidates
            .into_iter()
            .map(|log| {
                let doc = documents.iter().find(|d| d.id == log.id).unwrap();
                let score = calculate_bm25_score(doc, &keywords, &doc_freqs, n, avgdl, params);
                RecallResult { log, score }
            })
            .collect();

        // Sort by score descending, then by timestamp descending
        scored_results.sort_by(|a, b| {
            let score_cmp = b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal);
            if score_cmp != std::cmp::Ordering::Equal {
                score_cmp
            } else {
                b.log.timestamp.cmp(&a.log.timestamp)
            }
        });

        // Filter out zero-score results (documents that don't contain any keywords)
        scored_results.retain(|r| r.score > 0.0);

        // Apply pagination
        scored_results.truncate(query.limit);
        if query.offset > 0 && query.offset < scored_results.len() {
            scored_results = scored_results.into_iter().skip(query.offset).collect();
        }

        Ok(scored_results)
    }

    /// Get candidate logs based on temporal and attribute filters
    async fn get_candidate_logs(
        &self,
        query: &RecallQuery,
    ) -> Result<Vec<ConversationLog>, SwellError> {
        let mut sql = String::from("SELECT * FROM conversation_logs WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(ref session_id) = query.session_id {
            sql.push_str(" AND session_id = ?");
            params.push(session_id.to_string());
        }

        if let Some(ref task_id) = query.task_id {
            sql.push_str(" AND task_id = ?");
            params.push(task_id.to_string());
        }

        if let Some(ref agent_role) = query.agent_role {
            sql.push_str(" AND agent_role = ?");
            params.push(agent_role.clone());
        }

        if let Some(ref action) = query.action {
            sql.push_str(" AND action = ?");
            params.push(action.as_str().to_string());
        }

        if let Some(ref start_time) = query.start_time {
            sql.push_str(" AND timestamp >= ?");
            params.push(start_time.to_rfc3339());
        }

        if let Some(ref end_time) = query.end_time {
            sql.push_str(" AND timestamp <= ?");
            params.push(end_time.to_rfc3339());
        }

        sql.push_str(" ORDER BY timestamp DESC");

        let mut q = sqlx::query(&sql);
        for param in &params {
            q = q.bind(param);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut logs = Vec::new();
        for row in rows {
            logs.push(self.row_to_conversation_log(&row)?);
        }

        Ok(logs)
    }

    /// Convert database row to ConversationLog
    fn row_to_conversation_log(
        &self,
        row: &sqlx::sqlite::SqliteRow,
    ) -> Result<ConversationLog, SwellError> {
        use sqlx::Row;

        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;

        let session_id_str: String = row.get("session_id");
        let session_id = Uuid::parse_str(&session_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid session UUID: {}", e)))?;

        let task_id_str: Option<String> = row.get("task_id");
        let task_id = task_id_str
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| SwellError::DatabaseError(format!("Invalid task UUID: {}", e)))?;

        let agent_id_str: String = row.get("agent_id");
        let agent_id = Uuid::parse_str(&agent_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid agent UUID: {}", e)))?;

        let agent_role: String = row.get("agent_role");
        let action_str: String = row.get("action");
        let action = ConversationAction::from_str(&action_str)
            .ok_or_else(|| SwellError::DatabaseError(format!("Invalid action: {}", action_str)))?;

        let content: String = row.get("content");
        let timestamp_str: String = row.get("timestamp");
        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let metadata_str: String = row.get("metadata");
        let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;

        Ok(ConversationLog {
            id,
            session_id,
            task_id,
            agent_id,
            agent_role,
            action,
            content,
            timestamp,
            metadata,
        })
    }

    /// Initialize the conversation_logs table
    pub async fn init_conversation_logs_schema(
        pool: &sqlx::sqlite::SqlitePool,
    ) -> Result<(), SwellError> {
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
}

// ============================================================================
// BM25 Implementation
// ============================================================================

/// Tokenize text into lowercase words
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Extract terms from content and count frequencies
fn extract_terms(content: &str) -> HashMap<String, f32> {
    let tokens = tokenize(content);
    let mut freq: HashMap<String, f32> = HashMap::new();
    for token in tokens {
        *freq.entry(token).or_insert(0.0) += 1.0;
    }
    freq
}

/// Calculate document frequency for each term
fn calculate_document_frequencies(
    documents: &[Bm25Document],
    keywords: &HashSet<String>,
) -> HashMap<String, f32> {
    let mut doc_freqs: HashMap<String, f32> = HashMap::new();
    for kw in keywords {
        let df = documents.iter().filter(|d| d.terms.contains_key(kw)).count() as f32;
        doc_freqs.insert(kw.clone(), df);
    }
    doc_freqs
}

/// Calculate BM25 score for a single document
fn calculate_bm25_score(
    doc: &Bm25Document,
    keywords: &HashSet<String>,
    doc_freqs: &HashMap<String, f32>,
    n: f32,
    avgdl: f32,
    params: Bm25Params,
) -> f32 {
    let mut score = 0.0f32;
    let k1 = params.k1;
    let b = params.b;

    for kw in keywords {
        let tf = doc.terms.get(kw).copied().unwrap_or(0.0);
        let df = doc_freqs.get(kw).copied().unwrap_or(0.0);

        // Use a modified IDF formula that doesn't go negative
        // Standard BM25 IDF can be negative when df > n/2, but we want to still
        // give some weight to matching terms. We use max(1.0, df) to avoid division issues.
        // Alternative IDF: log((n + 1) / (df + 1)) which is always positive
        let idf = if df > 0.0 {
            ((n + 1.0) / (df + 1.0)).ln()
        } else {
            0.0
        };

        // BM25 term score with saturation
        let numerator = tf * (k1 + 1.0);
        let denominator = tf + k1 * (1.0 - b + b * (doc.term_count as f32) / avgdl.max(1.0));

        score += idf * numerator / denominator;
    }

    score
}

// ============================================================================
// Recall Service
// ============================================================================

/// High-level service for memory recall operations
pub struct RecallService {
    store: SqliteMemoryStore,
}

impl RecallService {
    pub fn new(store: SqliteMemoryStore) -> Self {
        Self { store }
    }

    /// Log an agent interaction
    pub async fn log_interaction(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        agent_role: String,
        action: ConversationAction,
        content: String,
        task_id: Option<Uuid>,
    ) -> Result<Uuid, SwellError> {
        let log = ConversationLog::new(session_id, agent_id, agent_role, action, content, task_id);
        self.store.store_conversation_log(log).await
    }

    /// Search conversation logs with BM25
    pub async fn search(
        &self,
        query: RecallQuery,
    ) -> Result<Vec<RecallResult>, SwellError> {
        self.store.recall_search(query).await
    }

    /// Get all logs for a session
    pub async fn get_session_logs(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<ConversationLog>, SwellError> {
        self.store.get_conversation_logs_by_session(session_id).await
    }

    /// Get logs for a specific task
    pub async fn get_task_logs(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<RecallResult>, SwellError> {
        let query = RecallQuery {
            task_id: Some(task_id),
            ..Default::default()
        };
        self.store.recall_search(query).await
    }

    /// Get logs within a time range
    pub async fn get_logs_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<RecallResult>, SwellError> {
        let query = RecallQuery {
            start_time: Some(start),
            end_time: Some(end),
            ..Default::default()
        };
        self.store.recall_search(query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tokenize() {
        let tokens = tokenize("Hello World! This is a Test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
    }

    #[test]
    fn test_conversation_action_serialization() {
        let action = ConversationAction::ToolCall;
        let serialized = serde_json::to_string(&action).unwrap();
        assert_eq!(serialized, "\"tool_call\"");

        let deserialized: ConversationAction = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, ConversationAction::ToolCall);
    }

    #[tokio::test]
    async fn test_recall_query_default() {
        let query = RecallQuery::default();
        assert!(query.keywords.is_empty());
        assert!(query.session_id.is_none());
        assert!(query.limit == 10);
    }

    #[test]
    fn test_bm25_params_default() {
        let params = Bm25Params::default();
        assert_eq!(params.k1, 1.2);
        assert_eq!(params.b, 0.75);
    }

    #[tokio::test]
    async fn test_recall_search_empty_keywords() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Initialize conversation logs schema
        SqliteMemoryStore::init_conversation_logs_schema(store.pool.as_ref())
            .await
            .unwrap();

        let session_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();

        // Store a conversation log
        let log = ConversationLog::new(
            session_id,
            agent_id,
            "planner".to_string(),
            ConversationAction::Observation,
            "This is a test observation".to_string(),
            None,
        );
        store.store_conversation_log(log.clone()).await.unwrap();

        // Search with no keywords - should still return results
        let query = RecallQuery {
            keywords: vec![],
            session_id: Some(session_id),
            limit: 10,
            ..Default::default()
        };

        let results = store.recall_search(query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log.id, log.id);
    }

    #[tokio::test]
    async fn test_recall_search_with_keywords() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Initialize conversation logs schema
        SqliteMemoryStore::init_conversation_logs_schema(store.pool.as_ref())
            .await
            .unwrap();

        let session_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();

        // Store logs with different content
        let log1 = ConversationLog::new(
            session_id,
            agent_id,
            "planner".to_string(),
            ConversationAction::Observation,
            "Rust is a systems programming language".to_string(),
            None,
        );
        let log2 = ConversationLog::new(
            session_id,
            agent_id,
            "generator".to_string(),
            ConversationAction::ToolCall,
            "Implementing a new feature in Python".to_string(),
            None,
        );
        let log3 = ConversationLog::new(
            session_id,
            agent_id,
            "evaluator".to_string(),
            ConversationAction::Observation,
            "Rust code has excellent performance".to_string(),
            None,
        );

        store.store_conversation_log(log1.clone()).await.unwrap();
        store.store_conversation_log(log2.clone()).await.unwrap();
        store.store_conversation_log(log3.clone()).await.unwrap();

        // Debug: Check that logs are stored
        let all_logs = store.get_conversation_logs_by_session(session_id).await.unwrap();
        eprintln!("DEBUG: Stored {} logs", all_logs.len());
        for log in &all_logs {
            eprintln!("DEBUG: log id={}, content={}", log.id, log.content);
        }

        // Search for "Rust"
        let query = RecallQuery {
            keywords: vec!["Rust".to_string()],
            limit: 10,
            ..Default::default()
        };

        let results = store.recall_search(query).await.unwrap();
        eprintln!("DEBUG: Got {} results", results.len());
        for r in &results {
            eprintln!("DEBUG: result id={}, score={}, content={}", r.log.id, r.score, r.log.content);
        }
        assert_eq!(results.len(), 2);

        // Both Rust logs should be returned (log1 and log3)
        let ids: Vec<Uuid> = results.iter().map(|r| r.log.id).collect();
        assert!(ids.contains(&log1.id));
        assert!(ids.contains(&log3.id));

        // First result should be log1 or log3 (both mention Rust first)
        assert!(results[0].score >= results[1].score);
    }

    #[tokio::test]
    async fn test_recall_search_temporal_filter() {
        use crate::SqliteMemoryStore;
        use chrono::Duration;

        let store = SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Initialize conversation logs schema
        SqliteMemoryStore::init_conversation_logs_schema(store.pool.as_ref())
            .await
            .unwrap();

        let session_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();

        // Create log in the past (2 hours ago)
        let mut log1 = ConversationLog::new(
            session_id,
            agent_id,
            "planner".to_string(),
            ConversationAction::Observation,
            "Old observation".to_string(),
            None,
        );
        let two_hours_ago = Utc::now() - Duration::hours(2);
        log1.timestamp = two_hours_ago;
        store.store_conversation_log(log1.clone()).await.unwrap();

        // Create log now
        let log2 = ConversationLog::new(
            session_id,
            agent_id,
            "planner".to_string(),
            ConversationAction::Observation,
            "Recent observation".to_string(),
            None,
        );
        store.store_conversation_log(log2.clone()).await.unwrap();

        // Use log2's timestamp as reference for the range (it's the most recent)
        let now = log2.timestamp;

        // Search with time range in the last hour
        let query = RecallQuery {
            keywords: vec![],
            start_time: Some(now - Duration::hours(1)),
            end_time: Some(now),
            limit: 10,
            ..Default::default()
        };

        let results = store.recall_search(query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log.id, log2.id);
    }

    #[tokio::test]
    async fn test_recall_service() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Initialize conversation logs schema
        SqliteMemoryStore::init_conversation_logs_schema(store.pool.as_ref())
            .await
            .unwrap();

        let service = RecallService::new(store.clone());
        let session_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();

        // Log an interaction
        let log_id = service
            .log_interaction(
                session_id,
                agent_id,
                "planner".to_string(),
                ConversationAction::Start,
                "Task started".to_string(),
                None,
            )
            .await
            .unwrap();

        // Retrieve the log
        let retrieved = store.get_conversation_log(log_id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.action, ConversationAction::Start);
        assert_eq!(retrieved.content, "Task started");
    }
}
