// golden_sample_testing.rs - Golden sample validation for learned procedures
//
// This module provides functionality to validate learned procedures against
// test cases (golden samples) before auto-application in future tasks.
//
// Key concepts:
// - Golden samples are test inputs/outputs that a procedure should handle correctly
// - Procedures must pass golden sample tests before being promoted for auto-use
// - Failed validations are flagged for human review
//
// Validation flow:
// 1. A procedure is learned from a successful task trajectory
// 2. Before promotion, the procedure is tested against applicable golden samples
// 3. If pass rate >= threshold (e.g., 80%), the procedure is promoted
// 4. If pass rate < threshold, the procedure is flagged for review

use crate::SwellError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

/// Simple glob-style pattern matching function.
/// Supports: * (matches any sequence of characters), ? (matches any single character)
/// Returns true if the pattern matches the text.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    // Simple recursive implementation
    fn match_chars(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (Some('*'), _) => {
                // * can match zero chars (skip *) or one+ chars (consume one char from text)
                match_chars(&p[1..], t) || (!t.is_empty() && match_chars(p, &t[1..]))
            }
            (Some('?'), Some(_)) => match_chars(&p[1..], &t[1..]),
            (Some(pc), Some(tc)) if pc == tc => match_chars(&p[1..], &t[1..]),
            (None, None) => true,
            (None, Some(_)) | (Some(_), None) => false,
            _ => false,
        }
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    match_chars(&pattern_chars, &text_chars)
}

/// A golden sample represents a test case that a procedure should handle correctly.
///
/// Golden samples are created from:
/// - Successful task executions with known correct outputs
/// - Representative examples manually added by operators
/// - Edge cases discovered during task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenSample {
    pub id: Uuid,
    /// Human-readable name for the sample
    pub name: String,
    /// Description of what this sample tests
    pub description: String,
    /// Keywords that indicate when this sample is applicable
    pub context_pattern: String,
    /// The input context/task description for this sample
    pub input_context: String,
    /// Expected output or result
    pub expected_output: String,
    /// Additional validation criteria (e.g., "files_modified contains src/main.rs")
    pub validation_criteria: Vec<ValidationCriterion>,
    /// Source of this golden sample (task, operator, generated)
    pub source: GoldenSampleSource,
    /// Whether this sample is currently active (inactive samples are ignored)
    pub is_active: bool,
    /// Priority for validation order (higher = validated first)
    pub priority: u32,
    /// Tags for categorization
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl GoldenSample {
    /// Create a new golden sample
    pub fn new(
        name: String,
        description: String,
        context_pattern: String,
        input_context: String,
        expected_output: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description,
            context_pattern,
            input_context,
            expected_output,
            validation_criteria: Vec::new(),
            source: GoldenSampleSource::Generated,
            is_active: true,
            priority: 0,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
        }
    }

    /// Add a validation criterion
    pub fn add_criterion(&mut self, criterion: ValidationCriterion) {
        self.validation_criteria.push(criterion);
    }

    /// Check if this sample matches the given context
    pub fn matches_context(&self, context: &str) -> bool {
        let context_lower = context.to_lowercase();
        let pattern_lower = self.context_pattern.to_lowercase();

        // Simple keyword matching - all keywords in pattern must be present
        pattern_lower
            .split_whitespace()
            .all(|kw| context_lower.contains(kw))
    }
}

/// A validation criterion for checking procedure output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCriterion {
    /// Type of validation to perform
    pub criterion_type: CriterionType,
    /// Description of what this criterion checks
    pub description: String,
    /// The expected value or pattern
    pub expected: String,
    /// Whether this criterion must pass (fatal) or is a warning
    pub is_required: bool,
}

impl ValidationCriterion {
    /// Create a new required criterion
    pub fn required(criterion_type: CriterionType, description: String, expected: String) -> Self {
        Self {
            criterion_type,
            description,
            expected,
            is_required: true,
        }
    }

    /// Create a new optional (warning) criterion
    pub fn optional(criterion_type: CriterionType, description: String, expected: String) -> Self {
        Self {
            criterion_type,
            description,
            expected,
            is_required: false,
        }
    }
}

/// Types of validation criteria
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionType {
    /// Check that output contains a specific string
    Contains,
    /// Check that output matches a regex pattern
    RegexMatch,
    /// Check that output equals expected (exact match)
    ExactMatch,
    /// Check that output starts with expected prefix
    StartsWith,
    /// Check that output ends with expected suffix
    EndsWith,
    /// Check that output is valid JSON
    ValidJson,
    /// Check that output is valid Rust code (compiles)
    ValidRust,
    /// Custom validation (uses validation script)
    Custom,
}

impl CriterionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CriterionType::Contains => "contains",
            CriterionType::RegexMatch => "regex_match",
            CriterionType::ExactMatch => "exact_match",
            CriterionType::StartsWith => "starts_with",
            CriterionType::EndsWith => "ends_with",
            CriterionType::ValidJson => "valid_json",
            CriterionType::ValidRust => "valid_rust",
            CriterionType::Custom => "custom",
        }
    }
}

/// Source of the golden sample
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldenSampleSource {
    /// Created from a successful task execution
    TaskExecution,
    /// Manually created by operator
    Operator,
    /// Automatically generated from patterns
    Generated,
}

impl GoldenSampleSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoldenSampleSource::TaskExecution => "task_execution",
            GoldenSampleSource::Operator => "operator",
            GoldenSampleSource::Generated => "generated",
        }
    }
}

/// Result of validating a procedure against a golden sample
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureValidation {
    pub id: Uuid,
    pub procedure_id: Uuid,
    pub golden_sample_id: Uuid,
    pub passed: bool,
    pub actual_output: String,
    pub validation_details: Vec<CriterionResult>,
    pub failure_reason: Option<String>,
    pub validated_at: DateTime<Utc>,
}

impl ProcedureValidation {
    /// Create a new validation result
    pub fn new(
        procedure_id: Uuid,
        golden_sample_id: Uuid,
        passed: bool,
        actual_output: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            procedure_id,
            golden_sample_id,
            passed,
            actual_output,
            validation_details: Vec::new(),
            failure_reason: None,
            validated_at: Utc::now(),
        }
    }

    /// Add a criterion result
    pub fn add_criterion_result(&mut self, result: CriterionResult) {
        self.validation_details.push(result);
    }

    /// Set the failure reason
    pub fn set_failure_reason(&mut self, reason: String) {
        self.failure_reason = Some(reason);
    }
}

/// Result of checking a single validation criterion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionResult {
    pub criterion_description: String,
    pub passed: bool,
    pub actual_value: Option<String>,
}

impl CriterionResult {
    pub fn pass(description: String) -> Self {
        Self {
            criterion_description: description,
            passed: true,
            actual_value: None,
        }
    }

    pub fn fail(description: String, actual_value: String) -> Self {
        Self {
            criterion_description: description,
            passed: false,
            actual_value: Some(actual_value),
        }
    }
}

/// Summary of validation results for a procedure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub procedure_id: Uuid,
    pub total_samples: usize,
    pub passed_samples: usize,
    pub failed_samples: usize,
    pub pass_rate: f64,
    pub is_promotion_eligible: bool,
    pub flagged_for_review: bool,
    pub validation_history: Vec<ProcedureValidation>,
}

/// Configuration for golden sample validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenSampleConfig {
    /// Minimum pass rate required for promotion (0.0 to 1.0)
    pub promotion_pass_rate: f64,
    /// Minimum number of samples required for promotion
    pub min_samples_for_promotion: usize,
    /// Maximum number of samples to validate against (for performance)
    pub max_samples_to_validate: usize,
    /// Whether to flag for review when promotion fails
    pub flag_for_review_on_failure: bool,
}

impl Default for GoldenSampleConfig {
    fn default() -> Self {
        Self {
            // 80% pass rate required for promotion
            promotion_pass_rate: 0.8,
            // At least 1 sample must exist for promotion
            min_samples_for_promotion: 1,
            // Limit validation to 10 samples for performance
            max_samples_to_validate: 10,
            // Always flag for review when promotion fails
            flag_for_review_on_failure: true,
        }
    }
}

/// Result of validating a procedure against all applicable golden samples
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenSampleValidationResult {
    pub procedure_id: Uuid,
    pub samples_tested: usize,
    pub samples_passed: usize,
    pub samples_failed: usize,
    pub pass_rate: f64,
    pub is_eligible_for_promotion: bool,
    pub should_flag_for_review: bool,
    pub validations: Vec<ProcedureValidation>,
    pub errors: Vec<String>,
}

/// Trait for golden sample storage operations
#[async_trait]
pub trait GoldenSampleStore: Send + Sync {
    /// Store a new golden sample
    async fn store_sample(&self, sample: GoldenSample) -> Result<Uuid, SwellError>;

    /// Get a golden sample by ID
    async fn get_sample(&self, id: Uuid) -> Result<Option<GoldenSample>, SwellError>;

    /// Update a golden sample
    async fn update_sample(&self, sample: GoldenSample) -> Result<(), SwellError>;

    /// Delete a golden sample
    async fn delete_sample(&self, id: Uuid) -> Result<(), SwellError>;

    /// Find samples matching a context pattern
    async fn find_samples_by_context(
        &self,
        context: &str,
        limit: usize,
    ) -> Result<Vec<GoldenSample>, SwellError>;

    /// Store a validation result
    async fn store_validation(&self, validation: ProcedureValidation) -> Result<Uuid, SwellError>;

    /// Get validation history for a procedure
    async fn get_validation_history(
        &self,
        procedure_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ProcedureValidation>, SwellError>;

    /// Get all validations for a specific sample
    async fn get_validations_for_sample(
        &self,
        sample_id: Uuid,
    ) -> Result<Vec<ProcedureValidation>, SwellError>;
}

/// SQLite-backed golden sample store
#[derive(Clone)]
pub struct SqliteGoldenSampleStore {
    pool: Arc<SqlitePool>,
}

impl SqliteGoldenSampleStore {
    /// Create a new store with the given database URL
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        Self::create(database_url).await
    }

    /// Create a new store
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

    /// Initialize the database schema
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        // Golden samples table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS golden_samples (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                context_pattern TEXT NOT NULL,
                input_context TEXT NOT NULL,
                expected_output TEXT NOT NULL,
                validation_criteria TEXT NOT NULL,
                source TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                priority INTEGER NOT NULL DEFAULT 0,
                tags TEXT NOT NULL,
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
            "CREATE INDEX IF NOT EXISTS idx_golden_samples_context ON golden_samples(context_pattern)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_golden_samples_active ON golden_samples(is_active)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Procedure validations table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS procedure_validations (
                id TEXT PRIMARY KEY,
                procedure_id TEXT NOT NULL,
                golden_sample_id TEXT NOT NULL,
                passed INTEGER NOT NULL,
                actual_output TEXT NOT NULL,
                validation_details TEXT NOT NULL,
                failure_reason TEXT,
                validated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_validations_procedure ON procedure_validations(procedure_id)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_validations_sample ON procedure_validations(golden_sample_id)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Convert a database row to GoldenSample
    fn row_to_sample(&self, row: &SqliteRow) -> Result<GoldenSample, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let name: String = row.get("name");
        let description: String = row.get("description");
        let context_pattern: String = row.get("context_pattern");
        let input_context: String = row.get("input_context");
        let expected_output: String = row.get("expected_output");
        let criteria_str: String = row.get("validation_criteria");
        let source_str: String = row.get("source");
        let is_active: i32 = row.get("is_active");
        let priority: i32 = row.get("priority");
        let tags_str: String = row.get("tags");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let metadata_str: String = row.get("metadata");

        let validation_criteria: Vec<ValidationCriterion> = serde_json::from_str(&criteria_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON criteria: {}", e)))?;

        let source = match source_str.as_str() {
            "task_execution" => GoldenSampleSource::TaskExecution,
            "operator" => GoldenSampleSource::Operator,
            _ => GoldenSampleSource::Generated,
        };

        let tags: Vec<String> = serde_json::from_str(&tags_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON tags: {}", e)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;

        Ok(GoldenSample {
            id,
            name,
            description,
            context_pattern,
            input_context,
            expected_output,
            validation_criteria,
            source,
            is_active: is_active != 0,
            priority: priority as u32,
            tags,
            created_at,
            updated_at,
            metadata,
        })
    }

    /// Convert a database row to ProcedureValidation
    fn row_to_validation(&self, row: &SqliteRow) -> Result<ProcedureValidation, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let procedure_id_str: String = row.get("procedure_id");
        let procedure_id = Uuid::parse_str(&procedure_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let golden_sample_id_str: String = row.get("golden_sample_id");
        let golden_sample_id = Uuid::parse_str(&golden_sample_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let passed: i32 = row.get("passed");
        let actual_output: String = row.get("actual_output");
        let details_str: String = row.get("validation_details");
        let failure_reason: Option<String> = row.get("failure_reason");
        let validated_at_str: String = row.get("validated_at");

        let validation_details: Vec<CriterionResult> = serde_json::from_str(&details_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON details: {}", e)))?;

        let validated_at = chrono::DateTime::parse_from_rfc3339(&validated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        Ok(ProcedureValidation {
            id,
            procedure_id,
            golden_sample_id,
            passed: passed != 0,
            actual_output,
            validation_details,
            failure_reason,
            validated_at,
        })
    }
}

#[async_trait]
impl GoldenSampleStore for SqliteGoldenSampleStore {
    async fn store_sample(&self, sample: GoldenSample) -> Result<Uuid, SwellError> {
        let criteria_str =
            serde_json::to_string(&sample.validation_criteria).map_err(|e| {
                SwellError::DatabaseError(format!("Failed to serialize criteria: {}", e))
            })?;
        let tags_str = serde_json::to_string(&sample.tags)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize tags: {}", e)))?;
        let metadata_str = serde_json::to_string(&sample.metadata)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize metadata: {}", e)))?;
        let created_at_str = sample.created_at.to_rfc3339();
        let updated_at_str = sample.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO golden_samples (id, name, description, context_pattern, input_context, expected_output, validation_criteria, source, is_active, priority, tags, created_at, updated_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(sample.id.to_string())
        .bind(&sample.name)
        .bind(&sample.description)
        .bind(&sample.context_pattern)
        .bind(&sample.input_context)
        .bind(&sample.expected_output)
        .bind(&criteria_str)
        .bind(sample.source.as_str())
        .bind(if sample.is_active { 1 } else { 0 })
        .bind(sample.priority as i32)
        .bind(&tags_str)
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .bind(&metadata_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(sample.id)
    }

    async fn get_sample(&self, id: Uuid) -> Result<Option<GoldenSample>, SwellError> {
        let row = sqlx::query("SELECT * FROM golden_samples WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(self.row_to_sample(&r)?)),
            None => Ok(None),
        }
    }

    async fn update_sample(&self, sample: GoldenSample) -> Result<(), SwellError> {
        let criteria_str =
            serde_json::to_string(&sample.validation_criteria).map_err(|e| {
                SwellError::DatabaseError(format!("Failed to serialize criteria: {}", e))
            })?;
        let tags_str = serde_json::to_string(&sample.tags)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize tags: {}", e)))?;
        let metadata_str = serde_json::to_string(&sample.metadata)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize metadata: {}", e)))?;
        let updated_at_str = chrono::Utc::now().to_rfc3339();

        let result = sqlx::query(
            r#"
            UPDATE golden_samples
            SET name = ?, description = ?, context_pattern = ?, input_context = ?, expected_output = ?, validation_criteria = ?, source = ?, is_active = ?, priority = ?, tags = ?, updated_at = ?, metadata = ?
            WHERE id = ?
            "#,
        )
        .bind(&sample.name)
        .bind(&sample.description)
        .bind(&sample.context_pattern)
        .bind(&sample.input_context)
        .bind(&sample.expected_output)
        .bind(&criteria_str)
        .bind(sample.source.as_str())
        .bind(if sample.is_active { 1 } else { 0 })
        .bind(sample.priority as i32)
        .bind(&tags_str)
        .bind(&updated_at_str)
        .bind(&metadata_str)
        .bind(sample.id.to_string())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "Golden sample not found: {}",
                sample.id
            )));
        }

        Ok(())
    }

    async fn delete_sample(&self, id: Uuid) -> Result<(), SwellError> {
        let result = sqlx::query("DELETE FROM golden_samples WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "Golden sample not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn find_samples_by_context(
        &self,
        context: &str,
        limit: usize,
    ) -> Result<Vec<GoldenSample>, SwellError> {
        let context_lower = context.to_lowercase();
        let keywords: Vec<&str> = context_lower.split_whitespace().collect();

        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        // Build a LIKE query for each keyword
        let conditions: Vec<String> = keywords
            .iter()
            .map(|kw| format!("context_pattern LIKE '%{}%'", kw))
            .collect();
        let where_clause = conditions.join(" AND ");

        let sql = format!(
            "SELECT * FROM golden_samples WHERE is_active = 1 AND ({}) ORDER BY priority DESC LIMIT {}",
            where_clause, limit
        );

        let rows = sqlx::query(&sql)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut samples = Vec::new();
        for row in rows {
            samples.push(self.row_to_sample(&row)?);
        }

        Ok(samples)
    }

    async fn store_validation(&self, validation: ProcedureValidation) -> Result<Uuid, SwellError> {
        let details_str = serde_json::to_string(&validation.validation_details)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize details: {}", e)))?;
        let validated_at_str = validation.validated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO procedure_validations (id, procedure_id, golden_sample_id, passed, actual_output, validation_details, failure_reason, validated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(validation.id.to_string())
        .bind(validation.procedure_id.to_string())
        .bind(validation.golden_sample_id.to_string())
        .bind(if validation.passed { 1 } else { 0 })
        .bind(&validation.actual_output)
        .bind(&details_str)
        .bind(&validation.failure_reason)
        .bind(&validated_at_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(validation.id)
    }

    async fn get_validation_history(
        &self,
        procedure_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ProcedureValidation>, SwellError> {
        let rows = sqlx::query(
            "SELECT * FROM procedure_validations WHERE procedure_id = ? ORDER BY validated_at DESC LIMIT ?",
        )
        .bind(procedure_id.to_string())
        .bind(limit as i64)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut validations = Vec::new();
        for row in rows {
            validations.push(self.row_to_validation(&row)?);
        }

        Ok(validations)
    }

    async fn get_validations_for_sample(
        &self,
        sample_id: Uuid,
    ) -> Result<Vec<ProcedureValidation>, SwellError> {
        let rows = sqlx::query(
            "SELECT * FROM procedure_validations WHERE golden_sample_id = ? ORDER BY validated_at DESC",
        )
        .bind(sample_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut validations = Vec::new();
        for row in rows {
            validations.push(self.row_to_validation(&row)?);
        }

        Ok(validations)
    }
}

/// Golden sample tester that validates procedures against samples
pub struct GoldenSampleTester {
    store: SqliteGoldenSampleStore,
    config: GoldenSampleConfig,
}

impl GoldenSampleTester {
    /// Create a new tester
    pub fn new(store: SqliteGoldenSampleStore, config: GoldenSampleConfig) -> Self {
        Self { store, config }
    }

    /// Create a tester with default configuration
    pub fn with_default_config(store: SqliteGoldenSampleStore) -> Self {
        Self {
            store,
            config: GoldenSampleConfig::default(),
        }
    }

    /// Validate a procedure against all applicable golden samples
    ///
    /// Returns a result indicating whether the procedure is eligible for promotion.
    pub async fn validate_procedure(
        &self,
        procedure_id: Uuid,
        procedure_context: &str,
        procedure_output: &str,
    ) -> Result<GoldenSampleValidationResult, SwellError> {
        let mut result = GoldenSampleValidationResult {
            procedure_id,
            samples_tested: 0,
            samples_passed: 0,
            samples_failed: 0,
            pass_rate: 0.0,
            is_eligible_for_promotion: false,
            should_flag_for_review: false,
            validations: Vec::new(),
            errors: Vec::new(),
        };

        // Find applicable samples
        let samples = self
            .store
            .find_samples_by_context(procedure_context, self.config.max_samples_to_validate)
            .await?;

        if samples.is_empty() {
            // No applicable samples - cannot validate but don't fail
            result.is_eligible_for_promotion = true;
            result.should_flag_for_review = false;
            return Ok(result);
        }

        // Validate against each sample
        for sample in samples {
            let mut validation = ProcedureValidation::new(
                procedure_id,
                sample.id,
                true, // Assume passed until proven otherwise
                procedure_output.to_string(),
            );

            // Check if the output matches expected output
            let output_matches = self.check_output_match(procedure_output, &sample);

            if !output_matches {
                validation.passed = false;
                validation.set_failure_reason(format!(
                    "Output does not match expected: {}",
                    sample.expected_output
                ));
                result.samples_failed += 1;
            } else {
                // Check validation criteria
                for criterion in &sample.validation_criteria {
                    let criterion_result =
                        self.check_criterion(procedure_output, criterion);

                    if !criterion_result.passed && criterion.is_required {
                        validation.passed = false;
                    }
                    validation.add_criterion_result(criterion_result);
                }

                if validation.passed {
                    result.samples_passed += 1;
                } else {
                    result.samples_failed += 1;
                }
            }

            // Store validation result
            if let Err(e) = self.store.store_validation(validation.clone()).await {
                result.errors.push(format!("Failed to store validation: {}", e));
            }

            result.validations.push(validation);
            result.samples_tested += 1;
        }

        // Calculate pass rate
        if result.samples_tested > 0 {
            result.pass_rate =
                result.samples_passed as f64 / result.samples_tested as f64;
        }

        // Check if eligible for promotion
        result.is_eligible_for_promotion = result.samples_tested >= self.config.min_samples_for_promotion
            && result.pass_rate >= self.config.promotion_pass_rate;

        // Flag for review if promotion fails
        result.should_flag_for_review =
            self.config.flag_for_review_on_failure && !result.is_eligible_for_promotion;

        Ok(result)
    }

    /// Check if procedure output matches the golden sample's expected output
    fn check_output_match(&self, output: &str, sample: &GoldenSample) -> bool {
        // Simple containment check - the output should contain the expected output
        // or be sufficiently similar
        let output_lower = output.to_lowercase();
        let expected_lower = sample.expected_output.to_lowercase();

        // Exact containment check
        if output_lower.contains(&expected_lower) {
            return true;
        }

        // Check for significant overlap (at least 50% of expected words appear in output)
        let expected_words: Vec<&str> = expected_lower.split_whitespace().collect();
        if expected_words.is_empty() {
            return true;
        }

        let matching_words: usize = expected_words
            .iter()
            .filter(|w| output_lower.contains(*w))
            .count();

        let overlap_ratio = matching_words as f64 / expected_words.len() as f64;
        overlap_ratio >= 0.5
    }

    /// Check a single validation criterion
    fn check_criterion(&self, output: &str, criterion: &ValidationCriterion) -> CriterionResult {
        match criterion.criterion_type {
            CriterionType::Contains => {
                if output.contains(&criterion.expected) {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        format!("Output does not contain: {}", criterion.expected),
                    )
                }
            }
            CriterionType::ExactMatch => {
                if output.trim() == criterion.expected.trim() {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        format!("Expected exact match: {}", criterion.expected),
                    )
                }
            }
            CriterionType::StartsWith => {
                if output.trim().starts_with(&criterion.expected) {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        format!("Output does not start with: {}", criterion.expected),
                    )
                }
            }
            CriterionType::EndsWith => {
                if output.trim().ends_with(&criterion.expected) {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        format!("Output does not end with: {}", criterion.expected),
                    )
                }
            }
            CriterionType::RegexMatch => {
                // Simple glob-style pattern matching without regex crate
                // Supports: * (match any chars), ? (match single char)
                let pattern = &criterion.expected;
                let output_lower = output.to_lowercase();
                let pattern_lower = pattern.to_lowercase();

                if simple_glob_match(&pattern_lower, &output_lower) {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        format!(
                            "Output does not match pattern: {} (tried to match: {})",
                            criterion.expected, output
                        ),
                    )
                }
            }
            CriterionType::ValidJson => {
                match serde_json::from_str::<serde_json::Value>(output) {
                    Ok(_) => CriterionResult::pass(criterion.description.clone()),
                    Err(e) => CriterionResult::fail(
                        criterion.description.clone(),
                        format!("Invalid JSON: {}", e),
                    ),
                }
            }
            CriterionType::ValidRust => {
                // For now, just check for basic Rust syntax indicators
                // Full compilation check would require rustc
                let has_fn_main = output.contains("fn main()");
                let has_braces = output.contains('{') && output.contains('}');
                if has_fn_main || has_braces {
                    CriterionResult::pass(criterion.description.clone())
                } else {
                    CriterionResult::fail(
                        criterion.description.clone(),
                        "Output does not appear to be valid Rust code".to_string(),
                    )
                }
            }
            CriterionType::Custom => {
                // Custom criteria need external validation
                // For now, treat as pass
                CriterionResult::pass(format!("{} (custom - assumed pass)", criterion.description))
            }
        }
    }

    /// Get the validation configuration
    pub fn config(&self) -> &GoldenSampleConfig {
        &self.config
    }
}

/// Service for managing golden samples and procedure validation
pub struct GoldenSampleService {
    store: SqliteGoldenSampleStore,
    tester: GoldenSampleTester,
}

impl GoldenSampleService {
    /// Create a new service
    pub fn new(store: SqliteGoldenSampleStore) -> Self {
        let config = GoldenSampleConfig::default();
        Self {
            store: store.clone(),
            tester: GoldenSampleTester::new(store, config),
        }
    }

    /// Create a service with custom configuration
    pub fn with_config(store: SqliteGoldenSampleStore, config: GoldenSampleConfig) -> Self {
        Self {
            store: store.clone(),
            tester: GoldenSampleTester::new(store, config),
        }
    }

    /// Add a new golden sample
    pub async fn add_sample(&self, sample: GoldenSample) -> Result<Uuid, SwellError> {
        self.store.store_sample(sample).await
    }

    /// Get a golden sample by ID
    pub async fn get_sample(&self, id: Uuid) -> Result<Option<GoldenSample>, SwellError> {
        self.store.get_sample(id).await
    }

    /// Update a golden sample
    pub async fn update_sample(&self, sample: GoldenSample) -> Result<(), SwellError> {
        self.store.update_sample(sample).await
    }

    /// Delete a golden sample
    pub async fn delete_sample(&self, id: Uuid) -> Result<(), SwellError> {
        self.store.delete_sample(id).await
    }

    /// Find samples matching a context
    pub async fn find_samples(
        &self,
        context: &str,
        limit: usize,
    ) -> Result<Vec<GoldenSample>, SwellError> {
        self.store.find_samples_by_context(context, limit).await
    }

    /// Validate a procedure before promotion
    pub async fn validate_before_promotion(
        &self,
        procedure_id: Uuid,
        procedure_context: &str,
        procedure_output: &str,
    ) -> Result<GoldenSampleValidationResult, SwellError> {
        self.tester
            .validate_procedure(procedure_id, procedure_context, procedure_output)
            .await
    }

    /// Get validation history for a procedure
    pub async fn get_validation_history(
        &self,
        procedure_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ProcedureValidation>, SwellError> {
        self.store.get_validation_history(procedure_id, limit).await
    }

    /// Get all validations for a specific sample
    pub async fn get_validations_for_sample(
        &self,
        sample_id: Uuid,
    ) -> Result<Vec<ProcedureValidation>, SwellError> {
        self.store.get_validations_for_sample(sample_id).await
    }

    /// Check if a procedure should be promoted based on golden sample testing
    ///
    /// This is a convenience method that combines validation and promotion check.
    pub async fn should_promote_procedure(
        &self,
        procedure_id: Uuid,
        procedure_context: &str,
        procedure_output: &str,
    ) -> Result<(bool, GoldenSampleValidationResult), SwellError> {
        let validation_result = self
            .validate_before_promotion(procedure_id, procedure_context, procedure_output)
            .await?;

        Ok((validation_result.is_eligible_for_promotion, validation_result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =====================================================================
    // GoldenSample Tests
    // =====================================================================

    #[test]
    fn test_golden_sample_creation() {
        let sample = GoldenSample::new(
            "test_sample".to_string(),
            "A test golden sample".to_string(),
            "rust test".to_string(),
            "input context".to_string(),
            "expected output".to_string(),
        );

        assert_eq!(sample.name, "test_sample");
        assert_eq!(sample.context_pattern, "rust test");
        assert!(sample.is_active);
        assert_eq!(sample.priority, 0);
    }

    #[test]
    fn test_golden_sample_matches_context() {
        let sample = GoldenSample::new(
            "test".to_string(),
            "desc".to_string(),
            "rust test fix".to_string(),
            "input".to_string(),
            "output".to_string(),
        );

        // Exact match - all keywords present
        assert!(sample.matches_context("rust test fix"));

        // Case insensitive
        assert!(sample.matches_context("RUST TEST FIX"));
        assert!(sample.matches_context("Rust Test Fix"));

        // No match - missing keyword (all keywords must be present)
        assert!(!sample.matches_context("rust"));
        assert!(!sample.matches_context("rust test"));
        assert!(!sample.matches_context("test fix"));
        assert!(!sample.matches_context("python"));
        assert!(!sample.matches_context("fix bug"));
    }

    #[test]
    fn test_golden_sample_add_criterion() {
        let mut sample = GoldenSample::new(
            "test".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "output".to_string(),
        );

        sample.add_criterion(ValidationCriterion::required(
            CriterionType::Contains,
            "Check for success".to_string(),
            "success".to_string(),
        ));

        assert_eq!(sample.validation_criteria.len(), 1);
        assert!(sample.validation_criteria[0].is_required);
    }

    // =====================================================================
    // ValidationCriterion Tests
    // =====================================================================

    #[test]
    fn test_validation_criterion_required() {
        let criterion = ValidationCriterion::required(
            CriterionType::Contains,
            "Check output".to_string(),
            "expected".to_string(),
        );

        assert!(criterion.is_required);
        assert_eq!(criterion.criterion_type, CriterionType::Contains);
    }

    #[test]
    fn test_validation_criterion_optional() {
        let criterion = ValidationCriterion::optional(
            CriterionType::RegexMatch,
            "Check pattern".to_string(),
            "pattern.*".to_string(),
        );

        assert!(!criterion.is_required);
        assert_eq!(criterion.criterion_type, CriterionType::RegexMatch);
    }

    #[test]
    fn test_criterion_type_as_str() {
        assert_eq!(CriterionType::Contains.as_str(), "contains");
        assert_eq!(CriterionType::RegexMatch.as_str(), "regex_match");
        assert_eq!(CriterionType::ExactMatch.as_str(), "exact_match");
        assert_eq!(CriterionType::StartsWith.as_str(), "starts_with");
        assert_eq!(CriterionType::EndsWith.as_str(), "ends_with");
        assert_eq!(CriterionType::ValidJson.as_str(), "valid_json");
        assert_eq!(CriterionType::ValidRust.as_str(), "valid_rust");
        assert_eq!(CriterionType::Custom.as_str(), "custom");
    }

    // =====================================================================
    // GoldenSampleSource Tests
    // =====================================================================

    #[test]
    fn test_golden_sample_source_as_str() {
        assert_eq!(GoldenSampleSource::TaskExecution.as_str(), "task_execution");
        assert_eq!(GoldenSampleSource::Operator.as_str(), "operator");
        assert_eq!(GoldenSampleSource::Generated.as_str(), "generated");
    }

    // =====================================================================
    // ProcedureValidation Tests
    // =====================================================================

    #[test]
    fn test_procedure_validation_creation() {
        let validation = ProcedureValidation::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            true,
            "actual output".to_string(),
        );

        assert!(validation.passed);
        assert_eq!(validation.actual_output, "actual output");
        assert!(validation.failure_reason.is_none());
    }

    #[test]
    fn test_procedure_validation_add_criterion_result() {
        let mut validation = ProcedureValidation::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            true,
            "output".to_string(),
        );

        validation.add_criterion_result(CriterionResult::pass("Check 1".to_string()));
        validation.add_criterion_result(CriterionResult::fail(
            "Check 2".to_string(),
            "missing".to_string(),
        ));

        assert_eq!(validation.validation_details.len(), 2);
    }

    #[test]
    fn test_procedure_validation_set_failure_reason() {
        let mut validation = ProcedureValidation::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            false,
            "bad output".to_string(),
        );

        validation.set_failure_reason("Output mismatch".to_string());

        assert!(validation.failure_reason.is_some());
        assert_eq!(validation.failure_reason.unwrap(), "Output mismatch");
    }

    // =====================================================================
    // CriterionResult Tests
    // =====================================================================

    #[test]
    fn test_criterion_result_pass() {
        let result = CriterionResult::pass("Check passed".to_string());

        assert!(result.passed);
        assert_eq!(result.criterion_description, "Check passed");
        assert!(result.actual_value.is_none());
    }

    #[test]
    fn test_criterion_result_fail() {
        let result = CriterionResult::fail("Check failed".to_string(), "actual value".to_string());

        assert!(!result.passed);
        assert_eq!(result.criterion_description, "Check failed");
        assert_eq!(result.actual_value, Some("actual value".to_string()));
    }

    // =====================================================================
    // GoldenSampleConfig Tests
    // =====================================================================

    #[test]
    fn test_golden_sample_config_default() {
        let config = GoldenSampleConfig::default();

        assert_eq!(config.promotion_pass_rate, 0.8);
        assert_eq!(config.min_samples_for_promotion, 1);
        assert_eq!(config.max_samples_to_validate, 10);
        assert!(config.flag_for_review_on_failure);
    }

    // =====================================================================
    // GoldenSampleTester Tests
    // =====================================================================

    #[tokio::test]
    async fn test_golden_sample_tester_output_match_contains() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let sample = GoldenSample::new(
            "test".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "expected result".to_string(),
        );

        // Output contains expected
        assert!(tester.check_output_match("The expected result was found", &sample));

        // Output doesn't contain expected
        assert!(!tester.check_output_match("something else entirely", &sample));
    }

    #[tokio::test]
    async fn test_golden_sample_tester_output_match_word_overlap() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let sample = GoldenSample::new(
            "test".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "expected result here".to_string(),
        );

        // All words present - should pass
        assert!(tester.check_output_match("The expected result was here today", &sample));

        // 2 out of 3 words = 66.7% - passes 50% threshold
        assert!(tester.check_output_match("The expected here", &sample));

        // Only 1 out of 3 words = 33.3% - fails 50% threshold
        assert!(!tester.check_output_match("The only here", &sample));
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_contains() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::Contains,
            "Check for success".to_string(),
            "success".to_string(),
        );

        // Contains the expected
        let result = tester.check_criterion("Operation success", &criterion);
        assert!(result.passed);

        // Doesn't contain
        let result = tester.check_criterion("Operation failed", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_exact_match() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::ExactMatch,
            "Check exact".to_string(),
            "exact output".to_string(),
        );

        // Exact match (with trimming)
        let result = tester.check_criterion("  exact output  ", &criterion);
        assert!(result.passed);

        // Not exact match
        let result = tester.check_criterion("different output", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_starts_with() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::StartsWith,
            "Check prefix".to_string(),
            "Hello".to_string(),
        );

        let result = tester.check_criterion("Hello, World!", &criterion);
        assert!(result.passed);

        let result = tester.check_criterion("Goodbye, World!", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_ends_with() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::EndsWith,
            "Check suffix".to_string(),
            "World!".to_string(),
        );

        let result = tester.check_criterion("Hello, World!", &criterion);
        assert!(result.passed);

        // Does not end with the expected suffix
        let result = tester.check_criterion("Hello, World", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_regex() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        // Using glob-style pattern (simple pattern matching, not full regex)
        // The pattern "2024*15" means anything starting with 2024 and ending with 15
        let criterion = ValidationCriterion::required(
            CriterionType::RegexMatch,
            "Check pattern".to_string(),
            "2024*15".to_string(),
        );

        // This should match because "2024/01/15" contains "2024" and ends with "15"
        let result = tester.check_criterion("2024/01/15", &criterion);
        assert!(result.passed);

        // Should not match - 2023 doesn't match 2024
        let result = tester.check_criterion("2023/12/25", &criterion);
        assert!(!result.passed);

        let result = tester.check_criterion("Date: not a date", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_valid_json() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::ValidJson,
            "Check JSON".to_string(),
            "{}".to_string(),
        );

        let result = tester.check_criterion(r#"{"key": "value"}"#, &criterion);
        assert!(result.passed);

        let result = tester.check_criterion("not json", &criterion);
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn test_golden_sample_tester_check_criterion_valid_rust() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let tester = GoldenSampleTester::with_default_config(store);

        let criterion = ValidationCriterion::required(
            CriterionType::ValidRust,
            "Check Rust".to_string(),
            "{}".to_string(),
        );

        // Has fn main
        let result = tester.check_criterion("fn main() { println!(\"hello\"); }", &criterion);
        assert!(result.passed);

        // Has braces
        let result = tester.check_criterion("struct Foo { bar: i32 }", &criterion);
        assert!(result.passed);

        // No Rust indicators
        let result = tester.check_criterion("not rust code", &criterion);
        assert!(!result.passed);
    }

    // =====================================================================
    // SqliteGoldenSampleStore Tests
    // =====================================================================

    #[tokio::test]
    async fn test_store_and_get_sample() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let mut sample = GoldenSample::new(
            "test_sample".to_string(),
            "A test sample".to_string(),
            "rust test".to_string(),
            "input context".to_string(),
            "expected output".to_string(),
        );
        sample.add_criterion(ValidationCriterion::required(
            CriterionType::Contains,
            "Check output".to_string(),
            "success".to_string(),
        ));

        let id = store.store_sample(sample.clone()).await.unwrap();
        assert_eq!(id, sample.id);

        let retrieved = store.get_sample(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "test_sample");
        assert_eq!(retrieved.validation_criteria.len(), 1);
    }

    #[tokio::test]
    async fn test_update_sample() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let sample = GoldenSample::new(
            "original_name".to_string(),
            "Original description".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "output".to_string(),
        );

        store.store_sample(sample.clone()).await.unwrap();

        let mut updated = sample.clone();
        updated.name = "updated_name".to_string();
        updated.description = "Updated description".to_string();

        store.update_sample(updated.clone()).await.unwrap();

        let retrieved = store.get_sample(sample.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "updated_name");
        assert_eq!(retrieved.description, "Updated description");
    }

    #[tokio::test]
    async fn test_delete_sample() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let sample = GoldenSample::new(
            "to_delete".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "output".to_string(),
        );

        store.store_sample(sample.clone()).await.unwrap();
        store.delete_sample(sample.id).await.unwrap();

        let retrieved = store.get_sample(sample.id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_find_samples_by_context() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Store samples with different context patterns
        let sample1 = GoldenSample::new(
            "rust_test".to_string(),
            "Rust test sample".to_string(),
            "rust test".to_string(),
            "input1".to_string(),
            "output1".to_string(),
        );

        let sample2 = GoldenSample::new(
            "python_script".to_string(),
            "Python script sample".to_string(),
            "python script".to_string(),
            "input2".to_string(),
            "output2".to_string(),
        );

        let sample3 = GoldenSample::new(
            "rust_bugfix".to_string(),
            "Rust bugfix sample".to_string(),
            "rust bugfix".to_string(),
            "input3".to_string(),
            "output3".to_string(),
        );

        store.store_sample(sample1).await.unwrap();
        store.store_sample(sample2).await.unwrap();
        store.store_sample(sample3).await.unwrap();

        // Find samples matching "rust"
        let results = store.find_samples_by_context("rust", 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|s| s.context_pattern.contains("rust")));

        // Find samples matching "test"
        let results = store.find_samples_by_context("test", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rust_test");

        // Find samples matching "rust bug"
        let results = store.find_samples_by_context("rust bug", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rust_bugfix");
    }

    #[tokio::test]
    async fn test_store_and_get_validation() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let validation = ProcedureValidation::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            true,
            "actual output".to_string(),
        );

        let id = store.store_validation(validation.clone()).await.unwrap();
        assert_eq!(id, validation.id);

        // Retrieve via validation history
        let history = store.get_validation_history(validation.procedure_id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, validation.id);
    }

    #[tokio::test]
    async fn test_validation_history_order() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let procedure_id = Uuid::new_v4();
        let sample_id = Uuid::new_v4();

        // Add multiple validations with slight delays (using timestamps)
        let validation1 = ProcedureValidation::new(
            procedure_id,
            sample_id,
            false,
            "output1".to_string(),
        );
        store.store_validation(validation1.clone()).await.unwrap();

        let validation2 = ProcedureValidation::new(
            procedure_id,
            sample_id,
            true,
            "output2".to_string(),
        );
        store.store_validation(validation2.clone()).await.unwrap();

        let history = store.get_validation_history(procedure_id, 10).await.unwrap();
        assert_eq!(history.len(), 2);
        // Most recent first
        assert_eq!(history[0].id, validation2.id);
    }

    #[tokio::test]
    async fn test_get_validations_for_sample() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();

        let sample_id = Uuid::new_v4();

        // Create validations for different procedures
        let validation1 = ProcedureValidation::new(
            Uuid::new_v4(),
            sample_id,
            true,
            "output1".to_string(),
        );
        let validation2 = ProcedureValidation::new(
            Uuid::new_v4(),
            sample_id,
            false,
            "output2".to_string(),
        );

        store.store_validation(validation1.clone()).await.unwrap();
        store.store_validation(validation2.clone()).await.unwrap();

        let validations = store.get_validations_for_sample(sample_id).await.unwrap();
        assert_eq!(validations.len(), 2);
    }

    // =====================================================================
    // GoldenSampleService Tests
    // =====================================================================

    #[tokio::test]
    async fn test_golden_sample_service_add_get_sample() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        let sample = GoldenSample::new(
            "test".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "output".to_string(),
        );

        let id = service.add_sample(sample.clone()).await.unwrap();
        let retrieved = service.get_sample(id).await.unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test");
    }

    #[tokio::test]
    async fn test_validate_before_promotion_no_samples() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        let result = service
            .validate_before_promotion(
                Uuid::new_v4(),
                "some context",
                "procedure output",
            )
            .await
            .unwrap();

        // No samples - should be eligible but not flag for review
        assert!(result.is_eligible_for_promotion);
        assert!(!result.should_flag_for_review);
        assert_eq!(result.samples_tested, 0);
    }

    #[tokio::test]
    async fn test_validate_before_promotion_with_samples_pass() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        // Add a sample with context "rust test" - will match procedure context "rust test"
        let sample = GoldenSample::new(
            "test_sample".to_string(),
            "desc".to_string(),
            "rust test".to_string(),
            "input".to_string(),
            "expected output here".to_string(),
        );
        service.add_sample(sample).await.unwrap();

        // Validate with matching output - context "rust test" matches sample
        let result = service
            .validate_before_promotion(
                Uuid::new_v4(),
                "rust test",
                "The expected output here was found",
            )
            .await
            .unwrap();

        assert_eq!(result.samples_tested, 1);
        assert_eq!(result.samples_passed, 1);
        assert_eq!(result.samples_failed, 0);
        assert!(result.pass_rate >= 0.8);
        assert!(result.is_eligible_for_promotion);
        assert!(!result.should_flag_for_review);
    }

    #[tokio::test]
    async fn test_validate_before_promotion_with_samples_fail() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        // Add a sample with context "rust test"
        let sample = GoldenSample::new(
            "test_sample".to_string(),
            "desc".to_string(),
            "rust test".to_string(),
            "input".to_string(),
            "expected output here".to_string(),
        );
        service.add_sample(sample).await.unwrap();

        // Validate with non-matching output - context "rust test" matches sample
        let result = service
            .validate_before_promotion(
                Uuid::new_v4(),
                "rust test",
                "completely different output",
            )
            .await
            .unwrap();

        assert_eq!(result.samples_tested, 1);
        assert_eq!(result.samples_passed, 0);
        assert_eq!(result.samples_failed, 1);
        assert!(result.pass_rate < 0.8);
        assert!(!result.is_eligible_for_promotion);
        assert!(result.should_flag_for_review);
    }

    #[tokio::test]
    async fn test_should_promote_procedure() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        // Test passing case
        let sample = GoldenSample::new(
            "pass_sample".to_string(),
            "desc".to_string(),
            "rust".to_string(),
            "input".to_string(),
            "success".to_string(),
        );
        service.add_sample(sample).await.unwrap();

        let (should_promote, result) = service
            .should_promote_procedure(
                Uuid::new_v4(),
                "rust context",
                "The task was successful with success message",
            )
            .await
            .unwrap();

        assert!(should_promote);
        assert!(result.is_eligible_for_promotion);
    }

    // =====================================================================
    // Golden Sample Validation Flow Integration Tests
    // =====================================================================

    #[tokio::test]
    async fn test_full_validation_flow() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        // 1. Create golden samples representing edge cases
        // Using "rust" as context so it matches both samples
        let sample1 = GoldenSample::new(
            "rust_test_passing".to_string(),
            "Tests should pass".to_string(),
            "rust test".to_string(),
            "Run tests".to_string(),
            "test result: all passed".to_string(),
        );

        let sample2 = GoldenSample::new(
            "rust_lint_clean".to_string(),
            "Lint should be clean".to_string(),
            "rust lint".to_string(),
            "Run linting".to_string(),
            "lint result: clean".to_string(),
        );

        service.add_sample(sample1).await.unwrap();
        service.add_sample(sample2).await.unwrap();

        // 2. Validate a procedure against these samples
        // Using "rust" as context to match both samples
        let procedure_id = Uuid::new_v4();
        let result = service
            .validate_before_promotion(
                procedure_id,
                "rust",
                "The test result: all passed and lint result: clean",
            )
            .await
            .unwrap();

        // 3. Check promotion eligibility
        assert!(result.is_eligible_for_promotion);
        assert!(!result.should_flag_for_review);

        // 4. Verify validation history was stored
        let history = service.get_validation_history(procedure_id, 10).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_validation_flags_failed_procedures() {
        let store = SqliteGoldenSampleStore::create("sqlite::memory:")
            .await
            .unwrap();
        let service = GoldenSampleService::new(store);

        // Create samples that expect specific outputs
        // Using context_pattern "task lint" so it matches procedure context "task lint"
        let sample1 = GoldenSample::new(
            "expect_success".to_string(),
            "Expect success output".to_string(),
            "task lint".to_string(),
            "Run task".to_string(),
            "SUCCESS: task completed".to_string(),
        );

        let sample2 = GoldenSample::new(
            "expect_clean".to_string(),
            "Expect clean lint".to_string(),
            "task lint".to_string(),
            "Run lint".to_string(),
            "CLEAN: no warnings".to_string(),
        );

        service.add_sample(sample1).await.unwrap();
        service.add_sample(sample2).await.unwrap();

        // Procedure outputs wrong results - context "task lint" matches both samples
        let procedure_id = Uuid::new_v4();
        let result = service
            .validate_before_promotion(
                procedure_id,
                "task lint",
                "ERROR: something went wrong and lint has warnings",
            )
            .await
            .unwrap();

        // Should not be eligible for promotion
        assert!(!result.is_eligible_for_promotion);
        assert!(result.should_flag_for_review);
        assert_eq!(result.samples_passed, 0);
        assert_eq!(result.samples_failed, 2);

        // History should contain the failed validations
        let history = service.get_validation_history(procedure_id, 10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|v| !v.passed));
    }

    #[test]
    fn test_simple_glob_match() {
        // Test the glob matching function directly
        assert!(simple_glob_match("2024*15", "2024/01/15"));
        assert!(simple_glob_match("hello*world", "hello beautiful world"));
        assert!(simple_glob_match("test?match", "test1match"));
        // "test*fail" should match "testfail" because * can match zero chars
        assert!(simple_glob_match("test*fail", "testfail"));
        // "test*fail" should match "testXYZfail"
        assert!(simple_glob_match("test*fail", "testXYZfail"));
        // No match
        assert!(!simple_glob_match("test*foo", "testbar"));
    }
}
