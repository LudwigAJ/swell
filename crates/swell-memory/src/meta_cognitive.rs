// meta_cognitive.rs - Meta-cognitive memory for self-knowledge
//
// This module provides meta-cognitive memory capabilities that enable the system
// to learn from its own experiences. It tracks:
//
// 1. Model performance by task type - which models perform best on which tasks
// 2. Effective prompting strategies - successful prompt templates and approaches
// 3. Recommendations - suggestions for model/strategy per new task
//
// This self-knowledge enables the system to make better decisions over time
// rather than relying solely on static configurations.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use swell_core::SwellError;

/// Task types for model performance tracking
///
/// These align with the LLM router's task types but are defined here to allow
/// the memory system to evolve independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Complex code generation, refactoring, debugging
    Coding,
    /// Task decomposition, architectural decisions, planning
    Planning,
    /// Quick lookups, simple transformations, fast responses
    Fast,
    /// Code review, feedback, critique
    Review,
    /// General purpose tasks
    #[default]
    Default,
}

impl TaskType {
    /// Returns a display name for this task type
    pub fn display_name(&self) -> &'static str {
        match self {
            TaskType::Coding => "coding",
            TaskType::Planning => "planning",
            TaskType::Fast => "fast",
            TaskType::Review => "review",
            TaskType::Default => "default",
        }
    }

    /// Returns the default cost tolerance (0.0 to 1.0)
    pub fn cost_tolerance(&self) -> f64 {
        match self {
            TaskType::Fast => 0.2,
            TaskType::Default => 0.5,
            TaskType::Review => 0.6,
            TaskType::Coding => 0.7,
            TaskType::Planning => 0.9,
        }
    }

    /// Returns true if this task type benefits from longer context
    pub fn needs_long_context(&self) -> bool {
        matches!(self, TaskType::Planning | TaskType::Coding)
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Model performance record tracking how well a specific model performs on a task type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPerformance {
    pub id: Uuid,
    pub model_name: String,
    pub task_type: TaskType,
    /// Number of successful completions
    pub success_count: u32,
    /// Number of failed completions
    pub failure_count: u32,
    /// Total tokens consumed
    pub total_tokens: u64,
    /// Total latency in milliseconds
    pub total_latency_ms: u64,
    /// Average confidence score from validation (0.0 to 1.0)
    pub avg_confidence: f64,
    /// First time this model was used for this task type
    pub first_used: DateTime<Utc>,
    /// Most recent usage
    pub last_used: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ModelPerformance {
    /// Create a new model performance record
    pub fn new(model_name: String, task_type: TaskType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            model_name,
            task_type,
            success_count: 0,
            failure_count: 0,
            total_tokens: 0,
            total_latency_ms: 0,
            avg_confidence: 0.0,
            first_used: now,
            last_used: now,
            created_at: now,
            updated_at: now,
        }
    }

    /// Record a task completion with the given metrics
    pub fn record_completion(
        &mut self,
        success: bool,
        tokens_used: u64,
        latency_ms: u64,
        confidence: f64,
    ) {
        if success {
            self.success_count += 1;
        } else {
            self.failure_count += 1;
        }

        self.total_tokens += tokens_used;
        self.total_latency_ms += latency_ms;

        // Update running average of confidence
        let n = (self.success_count + self.failure_count) as f64;
        self.avg_confidence = (self.avg_confidence * (n - 1.0) + confidence) / n;

        self.last_used = Utc::now();
        self.updated_at = Utc::now();
    }

    /// Get the success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.success_count as f64 / total as f64
    }

    /// Get the average tokens per completion
    pub fn avg_tokens_per_completion(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.total_tokens as f64 / total as f64
    }

    /// Get the average latency per completion in milliseconds
    pub fn avg_latency_ms(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 0.0;
        }
        self.total_latency_ms as f64 / total as f64
    }

    /// Get a composite score for ranking (higher is better)
    /// Combines success rate, efficiency (tokens), and speed
    pub fn composite_score(&self, task_type: TaskType) -> f64 {
        let success_rate = self.success_rate();

        // Efficiency score: lower tokens is better, normalized to 0-1
        // Assume 10k tokens is good, 100k is poor
        let efficiency = 1.0 - (self.avg_tokens_per_completion() / 100_000.0).min(1.0);

        // Speed score: lower latency is better, normalized to 0-1
        // Assume 1s is good, 60s is poor
        let speed = 1.0 - (self.avg_latency_ms() / 60_000.0).min(1.0);

        // Weight factors based on task type
        let (w_success, w_efficiency, w_speed) = match task_type {
            TaskType::Fast => (0.3, 0.2, 0.5), // Speed matters most for fast tasks
            TaskType::Coding => (0.5, 0.3, 0.2), // Success and efficiency matter for coding
            TaskType::Planning => (0.4, 0.3, 0.3), // Balanced for planning
            TaskType::Review => (0.5, 0.3, 0.2), // Success matters for review
            TaskType::Default => (0.4, 0.3, 0.3), // Balanced default
        };

        w_success * success_rate + w_efficiency * efficiency + w_speed * speed
    }
}

/// A prompting strategy that has been learned to be effective
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptingStrategy {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Task types this strategy is effective for
    pub task_types: Vec<TaskType>,
    /// The actual prompt template
    pub prompt_template: String,
    /// Number of times this strategy was used
    pub usage_count: u32,
    /// Number of times this strategy succeeded
    pub success_count: u32,
    /// Average user rating if available (0.0 to 1.0)
    pub avg_rating: f64,
    /// First time this strategy was used
    pub first_used: DateTime<Utc>,
    /// Most recent usage
    pub last_used: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Optional metadata about the strategy
    pub metadata: serde_json::Value,
}

impl PromptingStrategy {
    /// Create a new prompting strategy
    pub fn new(
        name: String,
        description: String,
        task_types: Vec<TaskType>,
        prompt_template: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description,
            task_types,
            prompt_template,
            usage_count: 0,
            success_count: 0,
            avg_rating: 0.0,
            first_used: now,
            last_used: now,
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
        }
    }

    /// Record a usage of this strategy
    pub fn record_usage(&mut self, success: bool, rating: Option<f64>) {
        self.usage_count += 1;
        if success {
            self.success_count += 1;
        }

        // Update running average of rating
        if let Some(r) = rating {
            let n = self.usage_count as f64;
            self.avg_rating = (self.avg_rating * (n - 1.0) + r) / n;
        }

        self.last_used = Utc::now();
        self.updated_at = Utc::now();
    }

    /// Get the success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.usage_count == 0 {
            return 0.0;
        }
        self.success_count as f64 / self.usage_count as f64
    }

    /// Get an effectiveness score combining success rate and rating
    pub fn effectiveness_score(&self) -> f64 {
        let success_weight = 0.7;
        let rating_weight = 0.3;
        success_weight * self.success_rate() + rating_weight * self.avg_rating
    }

    /// Check if this strategy is applicable to a task type
    pub fn is_applicable_to(&self, task_type: TaskType) -> bool {
        self.task_types.contains(&task_type)
    }
}

/// A recommendation for model and/or strategy to use for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub recommended_model: Option<String>,
    pub recommended_strategy_id: Option<Uuid>,
    pub confidence: f64,
    pub reasoning: String,
    pub alternatives: Vec<AlternativeRecommendation>,
}

/// An alternative recommendation with lower confidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeRecommendation {
    pub model_name: Option<String>,
    pub strategy_id: Option<Uuid>,
    pub confidence: f64,
    pub reasoning: String,
}

/// Query for meta-cognitive search
#[derive(Debug, Clone, Default)]
pub struct MetaCognitiveQuery {
    pub task_type: Option<TaskType>,
    pub model_name: Option<String>,
    pub min_success_rate: Option<f64>,
    pub min_confidence: Option<f64>,
    pub limit: usize,
    pub offset: usize,
}

impl MetaCognitiveQuery {
    pub fn new() -> Self {
        Self {
            task_type: None,
            model_name: None,
            min_success_rate: None,
            min_confidence: None,
            limit: 100,
            offset: 0,
        }
    }

    pub fn with_task_type(mut self, task_type: TaskType) -> Self {
        self.task_type = Some(task_type);
        self
    }

    pub fn with_model_name(mut self, model_name: String) -> Self {
        self.model_name = Some(model_name);
        self
    }

    pub fn with_min_success_rate(mut self, rate: f64) -> Self {
        self.min_success_rate = Some(rate);
        self
    }

    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = Some(confidence);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// SQLite-based meta-cognitive memory store
#[derive(Clone)]
pub struct SqliteMetaCognitiveStore {
    pool: Arc<SqlitePool>,
}

impl SqliteMetaCognitiveStore {
    /// Create a new SqliteMetaCognitiveStore with the given database URL
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        Self::create(database_url).await
    }

    /// Create a new pool with the given database URL
    pub async fn create(database_url: &str) -> Result<Self, SwellError> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Self::init_schema(&pool).await?;

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Initialize the database schema for meta-cognitive memory
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        // Model performance table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS model_performance (
                id TEXT PRIMARY KEY,
                model_name TEXT NOT NULL,
                task_type TEXT NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                total_latency_ms INTEGER NOT NULL DEFAULT 0,
                avg_confidence REAL NOT NULL DEFAULT 0.0,
                first_used TEXT NOT NULL,
                last_used TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(model_name, task_type)
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_model_perf_task ON model_performance(task_type)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_model_perf_model ON model_performance(model_name)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Prompting strategies table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS prompting_strategies (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                task_types TEXT NOT NULL,
                prompt_template TEXT NOT NULL,
                usage_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                avg_rating REAL NOT NULL DEFAULT 0.0,
                first_used TEXT NOT NULL,
                last_used TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_strategies_name ON prompting_strategies(name)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Helper to convert TaskType to string
    fn task_type_to_string(task_type: TaskType) -> String {
        task_type.display_name().to_string()
    }

    /// Helper to convert string to TaskType
    fn string_to_task_type(s: &str) -> TaskType {
        match s {
            "coding" => TaskType::Coding,
            "planning" => TaskType::Planning,
            "fast" => TaskType::Fast,
            "review" => TaskType::Review,
            _ => TaskType::Default,
        }
    }

    /// Convert database row to ModelPerformance
    fn row_to_model_performance(&self, row: &SqliteRow) -> Result<ModelPerformance, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let model_name: String = row.get("model_name");
        let task_type_str: String = row.get("task_type");
        let success_count: u32 = row.get("success_count");
        let failure_count: u32 = row.get("failure_count");
        let total_tokens: i64 = row.get("total_tokens");
        let total_latency_ms: i64 = row.get("total_latency_ms");
        let avg_confidence: f64 = row.get("avg_confidence");
        let first_used_str: String = row.get("first_used");
        let last_used_str: String = row.get("last_used");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");

        let first_used = chrono::DateTime::parse_from_rfc3339(&first_used_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let last_used = chrono::DateTime::parse_from_rfc3339(&last_used_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        Ok(ModelPerformance {
            id,
            model_name,
            task_type: Self::string_to_task_type(&task_type_str),
            success_count,
            failure_count,
            total_tokens: total_tokens as u64,
            total_latency_ms: total_latency_ms as u64,
            avg_confidence,
            first_used,
            last_used,
            created_at,
            updated_at,
        })
    }

    /// Convert database row to PromptingStrategy
    fn row_to_prompting_strategy(&self, row: &SqliteRow) -> Result<PromptingStrategy, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let name: String = row.get("name");
        let description: String = row.get("description");
        let task_types_str: String = row.get("task_types");
        let prompt_template: String = row.get("prompt_template");
        let usage_count: u32 = row.get("usage_count");
        let success_count: u32 = row.get("success_count");
        let avg_rating: f64 = row.get("avg_rating");
        let first_used_str: String = row.get("first_used");
        let last_used_str: String = row.get("last_used");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let metadata_str: String = row.get("metadata");

        let task_types: Vec<TaskType> = task_types_str
            .split(',')
            .filter_map(|s| {
                let t = s.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(Self::string_to_task_type(t))
                }
            })
            .collect();

        let first_used = chrono::DateTime::parse_from_rfc3339(&first_used_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let last_used = chrono::DateTime::parse_from_rfc3339(&last_used_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;

        Ok(PromptingStrategy {
            id,
            name,
            description,
            task_types,
            prompt_template,
            usage_count,
            success_count,
            avg_rating,
            first_used,
            last_used,
            created_at,
            updated_at,
            metadata,
        })
    }
}

#[async_trait]
impl MetaCognitiveStore for SqliteMetaCognitiveStore {
    /// Record model performance for a task
    async fn record_model_performance(
        &self,
        model_name: String,
        task_type: TaskType,
        success: bool,
        tokens_used: u64,
        latency_ms: u64,
        confidence: f64,
    ) -> Result<(), SwellError> {
        // Try to get existing record
        let existing = self.get_model_performance(&model_name, task_type).await?;

        let mut performance =
            existing.unwrap_or_else(|| ModelPerformance::new(model_name.clone(), task_type));
        performance.record_completion(success, tokens_used, latency_ms, confidence);

        // Upsert
        sqlx::query(
            r#"
            INSERT INTO model_performance (id, model_name, task_type, success_count, failure_count, total_tokens, total_latency_ms, avg_confidence, first_used, last_used, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(model_name, task_type) DO UPDATE SET
                success_count = excluded.success_count,
                failure_count = excluded.failure_count,
                total_tokens = excluded.total_tokens,
                total_latency_ms = excluded.total_latency_ms,
                avg_confidence = excluded.avg_confidence,
                last_used = excluded.last_used,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(performance.id.to_string())
        .bind(&performance.model_name)
        .bind(Self::task_type_to_string(performance.task_type))
        .bind(performance.success_count)
        .bind(performance.failure_count)
        .bind(performance.total_tokens as i64)
        .bind(performance.total_latency_ms as i64)
        .bind(performance.avg_confidence)
        .bind(performance.first_used.to_rfc3339())
        .bind(performance.last_used.to_rfc3339())
        .bind(performance.created_at.to_rfc3339())
        .bind(performance.updated_at.to_rfc3339())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Get model performance for a specific model and task type
    async fn get_model_performance(
        &self,
        model_name: &str,
        task_type: TaskType,
    ) -> Result<Option<ModelPerformance>, SwellError> {
        let row =
            sqlx::query("SELECT * FROM model_performance WHERE model_name = ? AND task_type = ?")
                .bind(model_name)
                .bind(Self::task_type_to_string(task_type))
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(self.row_to_model_performance(&r)?)),
            None => Ok(None),
        }
    }

    /// Find best performing models for a task type
    async fn find_best_models(
        &self,
        task_type: TaskType,
        limit: usize,
    ) -> Result<Vec<ModelPerformance>, SwellError> {
        let rows = sqlx::query(
            "SELECT * FROM model_performance WHERE task_type = ? ORDER BY success_count DESC LIMIT ?",
        )
        .bind(Self::task_type_to_string(task_type))
        .bind(limit as i64)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(self.row_to_model_performance(&row)?);
        }

        Ok(results)
    }

    /// Store a prompting strategy
    async fn store_strategy(&self, strategy: PromptingStrategy) -> Result<Uuid, SwellError> {
        let task_types_str = strategy
            .task_types
            .iter()
            .map(|t| Self::task_type_to_string(*t))
            .collect::<Vec<_>>()
            .join(",");
        let metadata_str = serde_json::to_string(&strategy.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO prompting_strategies (id, name, description, task_types, prompt_template, usage_count, success_count, avg_rating, first_used, last_used, created_at, updated_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(strategy.id.to_string())
        .bind(&strategy.name)
        .bind(&strategy.description)
        .bind(&task_types_str)
        .bind(&strategy.prompt_template)
        .bind(strategy.usage_count)
        .bind(strategy.success_count)
        .bind(strategy.avg_rating)
        .bind(strategy.first_used.to_rfc3339())
        .bind(strategy.last_used.to_rfc3339())
        .bind(strategy.created_at.to_rfc3339())
        .bind(strategy.updated_at.to_rfc3339())
        .bind(&metadata_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(strategy.id)
    }

    /// Get a prompting strategy by ID
    async fn get_strategy(&self, id: Uuid) -> Result<Option<PromptingStrategy>, SwellError> {
        let row = sqlx::query("SELECT * FROM prompting_strategies WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(self.row_to_prompting_strategy(&r)?)),
            None => Ok(None),
        }
    }

    /// Record usage of a prompting strategy
    async fn record_strategy_usage(
        &self,
        id: Uuid,
        success: bool,
        rating: Option<f64>,
    ) -> Result<(), SwellError> {
        let strategy = self
            .get_strategy(id)
            .await?
            .ok_or_else(|| SwellError::DatabaseError(format!("Strategy not found: {}", id)))?;

        let mut updated = strategy;
        updated.record_usage(success, rating);

        let _task_types_str = updated
            .task_types
            .iter()
            .map(|t| Self::task_type_to_string(*t))
            .collect::<Vec<_>>()
            .join(",");
        let metadata_str = serde_json::to_string(&updated.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            UPDATE prompting_strategies
            SET usage_count = ?, success_count = ?, avg_rating = ?, last_used = ?, updated_at = ?, metadata = ?
            WHERE id = ?
            "#,
        )
        .bind(updated.usage_count)
        .bind(updated.success_count)
        .bind(updated.avg_rating)
        .bind(updated.last_used.to_rfc3339())
        .bind(updated.updated_at.to_rfc3339())
        .bind(&metadata_str)
        .bind(id.to_string())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Find strategies applicable to a task type
    async fn find_strategies_for_task(
        &self,
        task_type: TaskType,
        limit: usize,
    ) -> Result<Vec<PromptingStrategy>, SwellError> {
        // Use LIKE for task_types search since it's stored as comma-separated
        let pattern = format!("%{}%", Self::task_type_to_string(task_type));

        let rows = sqlx::query(
            "SELECT * FROM prompting_strategies WHERE task_types LIKE ? ORDER BY success_count DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit as i64)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(self.row_to_prompting_strategy(&row)?);
        }

        Ok(results)
    }

    /// Get a recommendation for a task
    async fn get_recommendation(
        &self,
        task_type: TaskType,
        _task_description: &str,
    ) -> Result<Recommendation, SwellError> {
        // Find best models for this task type
        let best_models = self.find_best_models(task_type, 5).await?;

        // Find best strategies for this task type
        let best_strategies = self.find_strategies_for_task(task_type, 5).await?;

        // Build recommendation
        let (recommended_model, model_confidence) = if let Some(best) = best_models.first() {
            let score = best.composite_score(task_type);
            (Some(best.model_name.clone()), score.min(1.0))
        } else {
            (None, 0.0)
        };

        let (recommended_strategy_id, strategy_confidence) =
            if let Some(best) = best_strategies.first() {
                let score = best.effectiveness_score();
                (Some(best.id), score.min(1.0))
            } else {
                (None, 0.0)
            };

        // Overall confidence is weighted average
        let confidence = 0.6 * model_confidence + 0.4 * strategy_confidence;

        // Build reasoning
        let mut reasoning = String::new();
        if let Some(ref model) = recommended_model {
            reasoning.push_str(&format!(
                "Model '{}' has {}% success rate for {} tasks. ",
                model,
                (model_confidence * 100.0) as u32,
                task_type
            ));
        }
        if recommended_strategy_id.is_some() {
            reasoning.push_str(&format!("Effective {} strategy available.", task_type));
        }
        if reasoning.is_empty() {
            reasoning.push_str("No historical data available. Using default model selection.");
        }

        // Build alternatives
        let alternatives: Vec<AlternativeRecommendation> = best_models
            .iter()
            .skip(1)
            .take(3)
            .map(|m| AlternativeRecommendation {
                model_name: Some(m.model_name.clone()),
                strategy_id: None,
                confidence: m.composite_score(task_type).min(1.0) * 0.8,
                reasoning: format!(
                    "Alternative model with {:.0}% success rate",
                    m.success_rate() * 100.0
                ),
            })
            .collect();

        Ok(Recommendation {
            recommended_model,
            recommended_strategy_id,
            confidence,
            reasoning,
            alternatives,
        })
    }

    /// Delete a strategy
    async fn delete_strategy(&self, id: Uuid) -> Result<(), SwellError> {
        let result = sqlx::query("DELETE FROM prompting_strategies WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "Strategy not found: {}",
                id
            )));
        }

        Ok(())
    }
}

/// Trait for meta-cognitive memory storage operations
#[async_trait]
pub trait MetaCognitiveStore: Send + Sync {
    /// Record model performance for a task
    async fn record_model_performance(
        &self,
        model_name: String,
        task_type: TaskType,
        success: bool,
        tokens_used: u64,
        latency_ms: u64,
        confidence: f64,
    ) -> Result<(), SwellError>;

    /// Get model performance for a specific model and task type
    async fn get_model_performance(
        &self,
        model_name: &str,
        task_type: TaskType,
    ) -> Result<Option<ModelPerformance>, SwellError>;

    /// Find best performing models for a task type
    async fn find_best_models(
        &self,
        task_type: TaskType,
        limit: usize,
    ) -> Result<Vec<ModelPerformance>, SwellError>;

    /// Store a prompting strategy
    async fn store_strategy(&self, strategy: PromptingStrategy) -> Result<Uuid, SwellError>;

    /// Get a prompting strategy by ID
    async fn get_strategy(&self, id: Uuid) -> Result<Option<PromptingStrategy>, SwellError>;

    /// Record usage of a prompting strategy
    async fn record_strategy_usage(
        &self,
        id: Uuid,
        success: bool,
        rating: Option<f64>,
    ) -> Result<(), SwellError>;

    /// Find strategies applicable to a task type
    async fn find_strategies_for_task(
        &self,
        task_type: TaskType,
        limit: usize,
    ) -> Result<Vec<PromptingStrategy>, SwellError>;

    /// Get a recommendation for a task
    async fn get_recommendation(
        &self,
        task_type: TaskType,
        task_description: &str,
    ) -> Result<Recommendation, SwellError>;

    /// Delete a strategy
    async fn delete_strategy(&self, id: Uuid) -> Result<(), SwellError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_type_display_name() {
        assert_eq!(TaskType::Coding.display_name(), "coding");
        assert_eq!(TaskType::Planning.display_name(), "planning");
        assert_eq!(TaskType::Fast.display_name(), "fast");
        assert_eq!(TaskType::Review.display_name(), "review");
        assert_eq!(TaskType::Default.display_name(), "default");
    }

    #[test]
    fn test_task_type_cost_tolerance() {
        assert_eq!(TaskType::Fast.cost_tolerance(), 0.2);
        assert_eq!(TaskType::Default.cost_tolerance(), 0.5);
        assert_eq!(TaskType::Review.cost_tolerance(), 0.6);
        assert_eq!(TaskType::Coding.cost_tolerance(), 0.7);
        assert_eq!(TaskType::Planning.cost_tolerance(), 0.9);
    }

    #[test]
    fn test_task_type_needs_long_context() {
        assert!(TaskType::Planning.needs_long_context());
        assert!(TaskType::Coding.needs_long_context());
        assert!(!TaskType::Fast.needs_long_context());
        assert!(!TaskType::Review.needs_long_context());
        assert!(!TaskType::Default.needs_long_context());
    }

    #[test]
    fn test_model_performance_new() {
        let mp = ModelPerformance::new("claude-sonnet".to_string(), TaskType::Coding);
        assert_eq!(mp.model_name, "claude-sonnet");
        assert_eq!(mp.task_type, TaskType::Coding);
        assert_eq!(mp.success_count, 0);
        assert_eq!(mp.failure_count, 0);
        assert_eq!(mp.success_rate(), 0.0);
    }

    #[test]
    fn test_model_performance_record_completion() {
        let mut mp = ModelPerformance::new("claude-sonnet".to_string(), TaskType::Coding);

        // Record some completions
        mp.record_completion(true, 1000, 5000, 0.9);
        assert_eq!(mp.success_count, 1);
        assert_eq!(mp.failure_count, 0);
        assert!((mp.avg_confidence - 0.9).abs() < 0.001);

        mp.record_completion(true, 1200, 4500, 0.85);
        assert_eq!(mp.success_count, 2);
        assert_eq!(mp.avg_tokens_per_completion(), 1100.0);

        mp.record_completion(false, 800, 3000, 0.3);
        assert_eq!(mp.failure_count, 1);
        assert!((mp.avg_confidence - 0.683).abs() < 0.01);
    }

    #[test]
    fn test_model_performance_success_rate() {
        let mut mp = ModelPerformance::new("claude-sonnet".to_string(), TaskType::Coding);

        assert_eq!(mp.success_rate(), 0.0);

        mp.record_completion(true, 1000, 5000, 0.9);
        assert!((mp.success_rate() - 1.0).abs() < 0.001);

        mp.record_completion(false, 1000, 5000, 0.3);
        assert!((mp.success_rate() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_model_performance_composite_score() {
        let mut mp = ModelPerformance::new("claude-sonnet".to_string(), TaskType::Coding);

        // Perfect record
        for _ in 0..10 {
            mp.record_completion(true, 5000, 10000, 0.9);
        }

        let score = mp.composite_score(TaskType::Coding);
        assert!(score > 0.5, "Score should be high for good performance");

        // Check that different task types weight differently
        let fast_score = mp.composite_score(TaskType::Fast);
        assert!(fast_score >= 0.0 && fast_score <= 1.0);
    }

    #[test]
    fn test_prompting_strategy_new() {
        let strategy = PromptingStrategy::new(
            "step_by_step".to_string(),
            "Break down complex tasks".to_string(),
            vec![TaskType::Planning, TaskType::Coding],
            "Let's break this down step by step:\n".to_string(),
        );

        assert_eq!(strategy.name, "step_by_step");
        assert_eq!(strategy.task_types.len(), 2);
        assert_eq!(strategy.usage_count, 0);
        assert_eq!(strategy.success_rate(), 0.0);
    }

    #[test]
    fn test_prompting_strategy_record_usage() {
        let mut strategy = PromptingStrategy::new(
            "code_review".to_string(),
            "Structured code review".to_string(),
            vec![TaskType::Review],
            "Review checklist:\n".to_string(),
        );

        strategy.record_usage(true, Some(0.9));
        assert_eq!(strategy.usage_count, 1);
        assert_eq!(strategy.success_count, 1);
        assert!((strategy.avg_rating - 0.9).abs() < 0.001);

        strategy.record_usage(false, Some(0.5));
        assert_eq!(strategy.usage_count, 2);
        assert_eq!(strategy.success_count, 1);
        assert!((strategy.avg_rating - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_prompting_strategy_effectiveness_score() {
        let mut strategy = PromptingStrategy::new(
            "test".to_string(),
            "Test strategy".to_string(),
            vec![TaskType::Coding],
            "Template".to_string(),
        );

        // 3 successes, 1 failure = 75% success rate
        for _ in 0..3 {
            strategy.record_usage(true, Some(0.8));
        }
        strategy.record_usage(false, Some(0.6));

        let score = strategy.effectiveness_score();
        // 0.7 * 0.75 + 0.3 * 0.75 = 0.525 + 0.225 = 0.75
        assert!((score - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_prompting_strategy_is_applicable_to() {
        let strategy = PromptingStrategy::new(
            "multi".to_string(),
            "Multi-purpose".to_string(),
            vec![TaskType::Coding, TaskType::Review],
            "Template".to_string(),
        );

        assert!(strategy.is_applicable_to(TaskType::Coding));
        assert!(strategy.is_applicable_to(TaskType::Review));
        assert!(!strategy.is_applicable_to(TaskType::Fast));
    }

    #[test]
    fn test_meta_cognitive_query_builder() {
        let query = MetaCognitiveQuery::new()
            .with_task_type(TaskType::Coding)
            .with_model_name("claude-sonnet".to_string())
            .with_min_success_rate(0.7)
            .with_min_confidence(0.5)
            .with_limit(50);

        assert_eq!(query.task_type, Some(TaskType::Coding));
        assert_eq!(query.model_name, Some("claude-sonnet".to_string()));
        assert_eq!(query.min_success_rate, Some(0.7));
        assert_eq!(query.min_confidence, Some(0.5));
        assert_eq!(query.limit, 50);
    }

    #[tokio::test]
    async fn test_record_and_get_model_performance() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Record some performance
        store
            .record_model_performance(
                "claude-sonnet".to_string(),
                TaskType::Coding,
                true,
                5000,
                10000,
                0.9,
            )
            .await
            .unwrap();

        store
            .record_model_performance(
                "claude-sonnet".to_string(),
                TaskType::Coding,
                true,
                4500,
                9000,
                0.85,
            )
            .await
            .unwrap();

        // Get performance
        let perf = store
            .get_model_performance("claude-sonnet", TaskType::Coding)
            .await
            .unwrap();

        assert!(perf.is_some());
        let perf = perf.unwrap();
        assert_eq!(perf.model_name, "claude-sonnet");
        assert_eq!(perf.task_type, TaskType::Coding);
        assert_eq!(perf.success_count, 2);
        assert_eq!(perf.failure_count, 0);
        assert_eq!(perf.total_tokens, 9500);
    }

    #[tokio::test]
    async fn test_find_best_models() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Add multiple models
        store
            .record_model_performance(
                "claude-haiku".to_string(),
                TaskType::Fast,
                true,
                500,
                1000,
                0.8,
            )
            .await
            .unwrap();

        store
            .record_model_performance(
                "claude-sonnet".to_string(),
                TaskType::Fast,
                true,
                2000,
                3000,
                0.9,
            )
            .await
            .unwrap();

        store
            .record_model_performance(
                "claude-opus".to_string(),
                TaskType::Fast,
                false,
                5000,
                8000,
                0.6,
            )
            .await
            .unwrap();

        let best = store.find_best_models(TaskType::Fast, 10).await.unwrap();

        assert_eq!(best.len(), 3);
        // First should be the one with most successes
        assert_eq!(best[0].model_name, "claude-haiku");
        assert_eq!(best[0].success_count, 1);
    }

    #[tokio::test]
    async fn test_store_and_get_strategy() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        let strategy = PromptingStrategy::new(
            "test_strategy".to_string(),
            "A test strategy".to_string(),
            vec![TaskType::Coding, TaskType::Planning],
            "Test template: {task}".to_string(),
        );

        let id = store.store_strategy(strategy.clone()).await.unwrap();
        assert_eq!(id, strategy.id);

        let retrieved = store.get_strategy(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "test_strategy");
        assert_eq!(retrieved.task_types.len(), 2);
    }

    #[tokio::test]
    async fn test_record_strategy_usage() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        let strategy = PromptingStrategy::new(
            "usage_test".to_string(),
            "Test usage tracking".to_string(),
            vec![TaskType::Review],
            "Review: {code}".to_string(),
        );

        let id = store.store_strategy(strategy).await.unwrap();

        store
            .record_strategy_usage(id, true, Some(0.9))
            .await
            .unwrap();
        store
            .record_strategy_usage(id, true, Some(0.85))
            .await
            .unwrap();
        store
            .record_strategy_usage(id, false, Some(0.4))
            .await
            .unwrap();

        let updated = store.get_strategy(id).await.unwrap().unwrap();
        assert_eq!(updated.usage_count, 3);
        assert_eq!(updated.success_count, 2);
        assert!((updated.avg_rating - 0.716).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_find_strategies_for_task() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Add strategies for different task types
        let strategy1 = PromptingStrategy::new(
            "coding_strategy".to_string(),
            "For coding".to_string(),
            vec![TaskType::Coding],
            "Code template".to_string(),
        );
        store.store_strategy(strategy1).await.unwrap();

        let strategy2 = PromptingStrategy::new(
            "planning_strategy".to_string(),
            "For planning".to_string(),
            vec![TaskType::Planning],
            "Plan template".to_string(),
        );
        store.store_strategy(strategy2).await.unwrap();

        let strategy3 = PromptingStrategy::new(
            "multi_strategy".to_string(),
            "For coding and planning".to_string(),
            vec![TaskType::Coding, TaskType::Planning],
            "Multi template".to_string(),
        );
        let id3 = store.store_strategy(strategy3).await.unwrap();

        // Record more successes for multi_strategy
        store
            .record_strategy_usage(id3, true, Some(0.9))
            .await
            .unwrap();
        store
            .record_strategy_usage(id3, true, Some(0.9))
            .await
            .unwrap();

        let coding_strategies = store
            .find_strategies_for_task(TaskType::Coding, 10)
            .await
            .unwrap();

        assert_eq!(coding_strategies.len(), 2);
        // multi_strategy should be first due to more successes
        assert_eq!(coding_strategies[0].name, "multi_strategy");
    }

    #[tokio::test]
    async fn test_get_recommendation() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Record some performance data
        store
            .record_model_performance(
                "claude-sonnet".to_string(),
                TaskType::Coding,
                true,
                5000,
                10000,
                0.9,
            )
            .await
            .unwrap();

        store
            .record_model_performance(
                "claude-opus".to_string(),
                TaskType::Coding,
                true,
                8000,
                15000,
                0.95,
            )
            .await
            .unwrap();

        // Store a strategy
        let strategy = PromptingStrategy::new(
            "effective_coding".to_string(),
            "An effective coding strategy".to_string(),
            vec![TaskType::Coding],
            "Let's code this step by step:\n".to_string(),
        );
        let strategy_id = store.store_strategy(strategy).await.unwrap();
        store
            .record_strategy_usage(strategy_id, true, Some(0.9))
            .await
            .unwrap();

        // Get recommendation
        let rec = store
            .get_recommendation(TaskType::Coding, "Implement a function")
            .await
            .unwrap();

        assert!(rec.recommended_model.is_some());
        assert!(rec.recommended_strategy_id.is_some());
        assert!(rec.confidence > 0.0);
        assert!(!rec.reasoning.is_empty());
    }

    #[tokio::test]
    async fn test_get_recommendation_no_data() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Get recommendation with no data
        let rec = store
            .get_recommendation(TaskType::Coding, "Implement a function")
            .await
            .unwrap();

        assert!(rec.recommended_model.is_none());
        assert!(rec.recommended_strategy_id.is_none());
        assert_eq!(rec.confidence, 0.0);
        assert!(rec.reasoning.contains("No historical data"));
    }

    #[tokio::test]
    async fn test_delete_strategy() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        let strategy = PromptingStrategy::new(
            "to_delete".to_string(),
            "Will be deleted".to_string(),
            vec![TaskType::Default],
            "Delete me".to_string(),
        );

        let id = store.store_strategy(strategy).await.unwrap();
        store.delete_strategy(id).await.unwrap();

        let retrieved = store.get_strategy(id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_update_model_performance() {
        let store = SqliteMetaCognitiveStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Record initial performance
        store
            .record_model_performance(
                "test-model".to_string(),
                TaskType::Fast,
                true,
                1000,
                2000,
                0.8,
            )
            .await
            .unwrap();

        let initial = store
            .get_model_performance("test-model", TaskType::Fast)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(initial.success_count, 1);

        // Record another completion - should update, not create new
        store
            .record_model_performance(
                "test-model".to_string(),
                TaskType::Fast,
                false,
                500,
                1000,
                0.4,
            )
            .await
            .unwrap();

        let updated = store
            .get_model_performance("test-model", TaskType::Fast)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.success_count, 1);
        assert_eq!(updated.failure_count, 1);
        assert_eq!(updated.total_tokens, 1500);
    }

    #[test]
    fn test_task_type_default() {
        let tt: TaskType = Default::default();
        assert_eq!(tt, TaskType::Default);
    }

    #[test]
    fn test_task_type_display() {
        assert_eq!(format!("{}", TaskType::Coding), "coding");
        assert_eq!(format!("{}", TaskType::Planning), "planning");
    }

    #[test]
    fn test_model_performance_avg_latency() {
        let mut mp = ModelPerformance::new("test".to_string(), TaskType::Default);

        mp.record_completion(true, 1000, 10000, 0.9);
        mp.record_completion(true, 1000, 20000, 0.9);

        assert!((mp.avg_latency_ms() - 15000.0).abs() < 0.001);
    }

    #[test]
    fn test_model_performance_avg_tokens() {
        let mut mp = ModelPerformance::new("test".to_string(), TaskType::Default);

        mp.record_completion(true, 5000, 1000, 0.9);
        mp.record_completion(true, 15000, 1000, 0.9);

        assert!((mp.avg_tokens_per_completion() - 10000.0).abs() < 0.001);
    }
}
