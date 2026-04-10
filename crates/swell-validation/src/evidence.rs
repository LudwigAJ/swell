//! Evidence pack generation for validation results.
//!
//! Bundles all validation signals, test results, and artifacts into
//! an immutable evidence pack suitable for PR review and audit trails.
//!
//! # Evidence Store
//!
//! Use [`EvidenceStore`] trait for immutable storage and retrieval:
//! - [`InMemoryEvidenceStore`] - In-memory store for testing
//! - [`SqliteEvidenceStore`] - SQLite-based store for production

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// An evidence pack containing all validation data for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePack {
    /// Unique identifier for this evidence pack
    pub id: Uuid,
    /// Task ID this evidence is for
    pub task_id: Uuid,
    /// When this evidence was created
    pub created_at: DateTime<Utc>,
    /// Validation outcome summary
    pub outcome: EvidenceOutcome,
    /// Individual gate results
    pub gate_results: Vec<GateEvidence>,
    /// Test results summary
    pub test_summary: TestEvidence,
    /// Coverage data if available
    pub coverage: Option<CoverageEvidence>,
    /// Security scan results
    pub security: SecurityEvidence,
    /// AI review results
    pub ai_review: Option<AiReviewEvidence>,
    /// Confidence score
    pub confidence: ConfidenceEvidence,
    /// Flakiness analysis
    pub flakiness: FlakinessEvidence,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Overall validation outcome
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceOutcome {
    /// All gates passed
    Passed,
    /// Some gates failed
    Failed,
    /// Validation was skipped or inconclusive
    Skipped,
    /// Validation encountered an error
    Error,
}

/// Evidence from a single validation gate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateEvidence {
    /// Gate name
    pub name: String,
    /// Whether the gate passed
    pub passed: bool,
    /// Number of messages by level
    pub message_counts: MessageCounts,
    /// Gate-specific details
    pub details: HashMap<String, String>,
}

/// Count of messages by level
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageCounts {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}

impl MessageCounts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_error(&mut self) {
        self.errors += 1;
    }

    pub fn add_warning(&mut self) {
        self.warnings += 1;
    }

    pub fn add_info(&mut self) {
        self.info += 1;
    }
}

/// Test execution evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvidence {
    /// Total tests run
    pub total: usize,
    /// Tests that passed
    pub passed: usize,
    /// Tests that failed
    pub failed: usize,
    /// Tests that were skipped
    pub skipped: usize,
    /// Total duration in milliseconds
    pub duration_ms: u64,
    /// Individual test results
    pub tests: Vec<TestResult>,
}

/// Individual test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Test name (fully qualified)
    pub name: String,
    /// Whether test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Failure message if applicable
    pub failure_message: Option<String>,
}

/// Coverage evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageEvidence {
    /// Line coverage percentage
    pub line_coverage: f64,
    /// Branch coverage percentage
    pub branch_coverage: f64,
    /// Function coverage percentage
    pub function_coverage: f64,
    /// Coverage report file path
    pub report_path: Option<String>,
}

/// Security scan evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvidence {
    /// Whether scan passed
    pub passed: bool,
    /// Critical findings
    pub critical: usize,
    /// High severity findings
    pub high: usize,
    /// Medium severity findings
    pub medium: usize,
    /// Low severity findings
    pub low: usize,
    /// Findings by category
    pub findings: Vec<SecurityFinding>,
}

/// A security finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    /// Finding ID
    pub id: String,
    /// CWE category
    pub cwe: Option<String>,
    /// Severity
    pub severity: String,
    /// Title
    pub title: String,
    /// File where found
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Description
    pub description: String,
}

/// AI review evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiReviewEvidence {
    /// Whether AI review passed
    pub passed: bool,
    /// Overall confidence score (0-1)
    pub confidence_score: f64,
    /// Number of issues found
    pub issues_found: usize,
    /// Issue categories
    pub issue_categories: Vec<String>,
    /// Review comments
    pub comments: Vec<ReviewComment>,
}

/// A review comment from AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    /// File associated with comment
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Comment text
    pub text: String,
    /// Severity level
    pub severity: String,
}

/// Confidence evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceEvidence {
    /// Overall confidence score (0-1)
    pub score: f64,
    /// Whether eligible for auto-merge
    pub auto_merge: bool,
    /// Individual signal scores
    pub signals: Vec<SignalScore>,
}

/// Individual signal score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalScore {
    /// Signal name
    pub name: String,
    /// Score value (0-1)
    pub score: f64,
    /// Weight of this signal
    pub weight: f64,
}

/// Flakiness evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessEvidence {
    /// Whether any flaky tests detected
    pub has_flaky: bool,
    /// Tests marked as flaky
    pub flaky_tests: Vec<String>,
    /// Tests quarantined
    pub quarantined: Vec<String>,
    /// Flakiness score per test
    pub scores: HashMap<String, f64>,
}

/// Builder for creating evidence packs
#[derive(Debug, Default)]
pub struct EvidencePackBuilder {
    task_id: Option<Uuid>,
    gate_results: Vec<GateEvidence>,
    test_summary: Option<TestEvidence>,
    coverage: Option<CoverageEvidence>,
    security: Option<SecurityEvidence>,
    ai_review: Option<AiReviewEvidence>,
    confidence_score: Option<ConfidenceEvidence>,
    flakiness: Option<FlakinessEvidence>,
    metadata: HashMap<String, String>,
}

impl EvidencePackBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the task ID
    pub fn task_id(mut self, id: Uuid) -> Self {
        self.task_id = Some(id);
        self
    }

    /// Add gate evidence
    pub fn add_gate_result(mut self, result: GateEvidence) -> Self {
        self.gate_results.push(result);
        self
    }

    /// Set test evidence
    pub fn test_summary(mut self, summary: TestEvidence) -> Self {
        self.test_summary = Some(summary);
        self
    }

    /// Set coverage evidence
    pub fn coverage(mut self, coverage: CoverageEvidence) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// Set security evidence
    pub fn security(mut self, security: SecurityEvidence) -> Self {
        self.security = Some(security);
        self
    }

    /// Set AI review evidence
    pub fn ai_review(mut self, review: AiReviewEvidence) -> Self {
        self.ai_review = Some(review);
        self
    }

    /// Set confidence evidence
    pub fn confidence_score(mut self, confidence: ConfidenceEvidence) -> Self {
        self.confidence_score = Some(confidence);
        self
    }

    /// Set flakiness evidence
    pub fn flakiness(mut self, flakiness: FlakinessEvidence) -> Self {
        self.flakiness = Some(flakiness);
        self
    }

    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Build the evidence pack
    pub fn build(self) -> Result<EvidencePack, EvidencePackError> {
        let task_id = self.task_id.ok_or(EvidencePackError::MissingTaskId)?;

        let outcome = self.determine_outcome();

        Ok(EvidencePack {
            id: Uuid::new_v4(),
            task_id,
            created_at: Utc::now(),
            outcome,
            gate_results: self.gate_results,
            test_summary: self.test_summary.unwrap_or(TestEvidence {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                duration_ms: 0,
                tests: vec![],
            }),
            coverage: self.coverage,
            security: self.security.unwrap_or(SecurityEvidence {
                passed: true,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                findings: vec![],
            }),
            ai_review: self.ai_review,
            confidence: self.confidence_score.unwrap_or(ConfidenceEvidence {
                score: 1.0,
                auto_merge: true,
                signals: vec![],
            }),
            flakiness: self.flakiness.unwrap_or(FlakinessEvidence {
                has_flaky: false,
                flaky_tests: vec![],
                quarantined: vec![],
                scores: HashMap::new(),
            }),
            metadata: self.metadata,
        })
    }

    fn determine_outcome(&self) -> EvidenceOutcome {
        // If any gate failed, outcome is failed
        if self.gate_results.iter().any(|g| !g.passed) {
            return EvidenceOutcome::Failed;
        }

        // If test summary shows failures
        if let Some(ref test) = self.test_summary {
            if test.failed > 0 {
                return EvidenceOutcome::Failed;
            }
        }

        // If security has critical issues
        if let Some(ref security) = self.security {
            if security.critical > 0 {
                return EvidenceOutcome::Failed;
            }
        }

        EvidenceOutcome::Passed
    }
}

/// Errors creating evidence pack
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvidencePackError {
    /// Missing required task ID
    MissingTaskId,
    /// Invalid data in builder
    InvalidData(String),
}

impl std::fmt::Display for EvidencePackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvidencePackError::MissingTaskId => write!(f, "Task ID is required"),
            EvidencePackError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for EvidencePackError {}

impl EvidencePack {
    /// Generate a markdown summary suitable for PR comments
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Validation Evidence\n\n");

        // Outcome badge
        let (emoji, status) = match self.outcome {
            EvidenceOutcome::Passed => ("✅", "PASSED"),
            EvidenceOutcome::Failed => ("❌", "FAILED"),
            EvidenceOutcome::Skipped => ("⏭️", "SKIPPED"),
            EvidenceOutcome::Error => ("💥", "ERROR"),
        };
        md.push_str(&format!("## {} {}\n\n", emoji, status));

        // Confidence
        md.push_str("## Confidence\n\n");
        md.push_str(&format!(
            "- **Score**: {:.1}%\n- **Auto-merge**: {}\n\n",
            self.confidence.score * 100.0,
            if self.confidence.auto_merge {
                "Yes"
            } else {
                "No"
            }
        ));

        // Test summary
        md.push_str("## Test Results\n\n");
        let ts = &self.test_summary;
        md.push_str(&format!(
            "- **Total**: {} | **Passed**: {} | **Failed**: {} | **Skipped**: {}\n",
            ts.total, ts.passed, ts.failed, ts.skipped
        ));
        md.push_str(&format!(
            "- **Duration**: {:.2}s\n\n",
            ts.duration_ms as f64 / 1000.0
        ));

        // Security
        md.push_str("## Security\n\n");
        md.push_str(&format!(
            "- **Critical**: {} | **High**: {} | **Medium**: {} | **Low**: {}\n\n",
            self.security.critical, self.security.high, self.security.medium, self.security.low
        ));

        // Gate results
        md.push_str("## Validation Gates\n\n");
        md.push_str("| Gate | Result | Errors | Warnings |\n");
        md.push_str("|------|--------|--------|----------|\n");
        for gate in &self.gate_results {
            let result = if gate.passed { "✅ PASS" } else { "❌ FAIL" };
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                gate.name, result, gate.message_counts.errors, gate.message_counts.warnings
            ));
        }
        md.push('\n');

        // Flaky tests
        if self.flakiness.has_flaky {
            md.push_str("## ⚠️ Flaky Tests Detected\n\n");
            for test in &self.flakiness.flaky_tests {
                md.push_str(&format!("- `{}`\n", test));
            }
            md.push('\n');
        }

        // Metadata
        if !self.metadata.is_empty() {
            md.push_str("## Metadata\n\n");
            for (key, value) in &self.metadata {
                md.push_str(&format!("- **{}**: {}\n", key, value));
            }
            md.push('\n');
        }

        md.push_str(&format!("\n---\n*Evidence pack ID: {}*\n", self.id));

        md
    }

    /// Generate a JSON summary for API responses
    pub fn to_summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id.to_string(),
            "task_id": self.task_id.to_string(),
            "created_at": self.created_at.to_rfc3339(),
            "outcome": match self.outcome {
                EvidenceOutcome::Passed => "passed",
                EvidenceOutcome::Failed => "failed",
                EvidenceOutcome::Skipped => "skipped",
                EvidenceOutcome::Error => "error",
            },
            "confidence": {
                "score": self.confidence.score,
                "auto_merge": self.confidence.auto_merge,
            },
            "test_summary": {
                "total": self.test_summary.total,
                "passed": self.test_summary.passed,
                "failed": self.test_summary.failed,
                "skipped": self.test_summary.skipped,
                "duration_ms": self.test_summary.duration_ms,
            },
            "security": {
                "passed": self.security.passed,
                "critical": self.security.critical,
                "high": self.security.high,
                "medium": self.security.medium,
                "low": self.security.low,
            },
            "flaky_tests": self.flakiness.flaky_tests,
        })
    }
}

// ============================================================================
// Evidence Store - Immutable Storage for Audit
// ============================================================================

/// Trait for storing and retrieving evidence packs immutably.
///
/// Evidence packs, once stored, cannot be modified - they represent
/// a point-in-time snapshot of validation results for audit purposes.
#[async_trait]
pub trait EvidenceStore: Send + Sync {
    /// Store an evidence pack immutably.
    /// Returns the ID of the stored evidence.
    async fn store(&self, evidence: EvidencePack) -> Result<Uuid, EvidenceStoreError>;

    /// Retrieve an evidence pack by its ID.
    async fn get(&self, id: Uuid) -> Result<Option<EvidencePack>, EvidenceStoreError>;

    /// Retrieve all evidence packs for a specific task, newest first.
    async fn get_by_task_id(&self, task_id: Uuid) -> Result<Vec<EvidencePack>, EvidenceStoreError>;

    /// Get the latest evidence pack for a task.
    async fn get_latest(&self, task_id: Uuid) -> Result<Option<EvidencePack>, EvidenceStoreError>;

    /// List all evidence pack IDs for a task (without loading full data).
    async fn list_ids(&self, task_id: Uuid) -> Result<Vec<Uuid>, EvidenceStoreError>;

    /// Get total count of evidence packs for a task.
    async fn count(&self, task_id: Uuid) -> Result<usize, EvidenceStoreError>;

    /// Check if an evidence pack exists.
    async fn exists(&self, id: Uuid) -> Result<bool, EvidenceStoreError>;
}

/// Errors from evidence store operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvidenceStoreError {
    /// Evidence pack not found
    NotFound(Uuid),
    /// Storage error (IO, database, etc.)
    StorageError(String),
    /// Serialization error
    SerializationError(String),
}

impl std::fmt::Display for EvidenceStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvidenceStoreError::NotFound(id) => write!(f, "Evidence pack not found: {}", id),
            EvidenceStoreError::StorageError(msg) => write!(f, "Storage error: {}", msg),
            EvidenceStoreError::SerializationError(msg) => {
                write!(f, "Serialization error: {}", msg)
            }
        }
    }
}

impl std::error::Error for EvidenceStoreError {}

impl From<EvidenceStoreError> for crate::SwellError {
    fn from(err: EvidenceStoreError) -> Self {
        match err {
            EvidenceStoreError::NotFound(_) => crate::SwellError::TaskNotFound(uuid::Uuid::nil()),
            EvidenceStoreError::StorageError(_) => {
                crate::SwellError::DatabaseError(err.to_string())
            }
            EvidenceStoreError::SerializationError(_) => {
                crate::SwellError::ConfigError(err.to_string())
            }
        }
    }
}

// In-memory store implementation for testing
mod mem_store {
    use super::*;

    /// In-memory evidence store for testing.
    ///
    /// Note: This store is NOT truly immutable - it allows deletion
    /// for test cleanup purposes. In production, use SqliteEvidenceStore.
    #[derive(Debug, Default)]
    pub struct InMemoryEvidenceStore {
        evidence: std::sync::RwLock<HashMap<Uuid, EvidencePack>>,
        by_task: std::sync::RwLock<HashMap<Uuid, Vec<Uuid>>>,
    }

    impl InMemoryEvidenceStore {
        /// Create a new in-memory evidence store
        pub fn new() -> Self {
            Self::default()
        }

        /// Create with initial capacity
        pub fn with_capacity(_capacity: usize) -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl EvidenceStore for InMemoryEvidenceStore {
        async fn store(&self, evidence: EvidencePack) -> Result<Uuid, EvidenceStoreError> {
            let id = evidence.id;
            let task_id = evidence.task_id;

            // Store the evidence
            {
                let mut evidence_map = self.evidence.write().unwrap();
                evidence_map.insert(id, evidence);
            }

            // Update task index
            {
                let mut by_task = self.by_task.write().unwrap();
                by_task.entry(task_id).or_default().push(id);
            }

            Ok(id)
        }

        async fn get(&self, id: Uuid) -> Result<Option<EvidencePack>, EvidenceStoreError> {
            let evidence_map = self.evidence.read().unwrap();
            Ok(evidence_map.get(&id).cloned())
        }

        async fn get_by_task_id(
            &self,
            task_id: Uuid,
        ) -> Result<Vec<EvidencePack>, EvidenceStoreError> {
            let evidence_map = self.evidence.read().unwrap();
            let by_task = self.by_task.read().unwrap();

            let ids = by_task.get(&task_id).cloned().unwrap_or_default();

            let mut packs: Vec<EvidencePack> = ids
                .iter()
                .filter_map(|id| evidence_map.get(id).cloned())
                .collect();

            // Sort by created_at descending (newest first)
            packs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

            Ok(packs)
        }

        async fn get_latest(
            &self,
            task_id: Uuid,
        ) -> Result<Option<EvidencePack>, EvidenceStoreError> {
            let packs = self.get_by_task_id(task_id).await?;
            Ok(packs.into_iter().next())
        }

        async fn list_ids(&self, task_id: Uuid) -> Result<Vec<Uuid>, EvidenceStoreError> {
            let by_task = self.by_task.read().unwrap();
            Ok(by_task.get(&task_id).cloned().unwrap_or_default())
        }

        async fn count(&self, task_id: Uuid) -> Result<usize, EvidenceStoreError> {
            let by_task = self.by_task.read().unwrap();
            Ok(by_task.get(&task_id).map(|v| v.len()).unwrap_or(0))
        }

        async fn exists(&self, id: Uuid) -> Result<bool, EvidenceStoreError> {
            let evidence_map = self.evidence.read().unwrap();
            Ok(evidence_map.contains_key(&id))
        }
    }
}

// Re-export the in-memory store
pub use mem_store::InMemoryEvidenceStore;

// SQLite store implementation
pub mod sqlite_store {
    use super::*;

    /// SQLite-based evidence store for production use.
    ///
    /// Stores evidence packs immutably - once stored, evidence cannot be
    /// modified or deleted through this interface.
    #[derive(Debug, Clone)]
    pub struct SqliteEvidenceStore {
        pool: sqlx::SqlitePool,
    }

    impl SqliteEvidenceStore {
        /// Create a new SQLite evidence store with the given database path
        pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, EvidenceStoreError> {
            let database_url = format!("sqlite:{}?mode=rwc", db_path.as_ref().display());

            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&database_url)
                .await
                .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            let store = Self { pool };
            store.init_schema().await?;

            Ok(store)
        }

        /// Create using an existing connection pool
        pub async fn from_pool(pool: sqlx::sqlite::SqlitePool) -> Result<Self, EvidenceStoreError> {
            let store = Self { pool };
            store.init_schema().await?;
            Ok(store)
        }

        /// Create using a connection string (e.g., "sqlite::memory:")
        pub async fn from_connection_string(conn_str: &str) -> Result<Self, EvidenceStoreError> {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect(conn_str)
                .await
                .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            let store = Self { pool };
            store.init_schema().await?;
            Ok(store)
        }

        /// Initialize the database schema
        async fn init_schema(&self) -> Result<(), EvidenceStoreError> {
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS evidence_packs (
                    id TEXT PRIMARY KEY,
                    task_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    data TEXT NOT NULL
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            // Create index on task_id for efficient lookups
            sqlx::query(
                r#"
                CREATE INDEX IF NOT EXISTS idx_evidence_task_id 
                ON evidence_packs(task_id, created_at DESC)
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            Ok(())
        }
    }

    #[async_trait]
    impl EvidenceStore for SqliteEvidenceStore {
        async fn store(&self, evidence: EvidencePack) -> Result<Uuid, EvidenceStoreError> {
            let id = evidence.id.to_string();
            let task_id = evidence.task_id.to_string();
            let created_at = evidence.created_at.to_rfc3339();

            let data = serde_json::to_string(&evidence)
                .map_err(|e| EvidenceStoreError::SerializationError(e.to_string()))?;

            sqlx::query(
                r#"
                INSERT INTO evidence_packs (id, task_id, created_at, data)
                VALUES (?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&task_id)
            .bind(&created_at)
            .bind(&data)
            .execute(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            Ok(evidence.id)
        }

        async fn get(&self, id: Uuid) -> Result<Option<EvidencePack>, EvidenceStoreError> {
            let id_str = id.to_string();

            let row: Option<(String,)> =
                sqlx::query_as(r#"SELECT data FROM evidence_packs WHERE id = ?"#)
                    .bind(&id_str)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            match row {
                Some((data,)) => {
                    let evidence: EvidencePack = serde_json::from_str(&data)
                        .map_err(|e| EvidenceStoreError::SerializationError(e.to_string()))?;
                    Ok(Some(evidence))
                }
                None => Ok(None),
            }
        }

        async fn get_by_task_id(
            &self,
            task_id: Uuid,
        ) -> Result<Vec<EvidencePack>, EvidenceStoreError> {
            let task_id_str = task_id.to_string();

            let rows: Vec<(String,)> = sqlx::query_as(
                r#"
                SELECT data FROM evidence_packs 
                WHERE task_id = ?
                ORDER BY created_at DESC
                "#,
            )
            .bind(&task_id_str)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            let mut packs = Vec::with_capacity(rows.len());
            for (data,) in rows {
                let evidence: EvidencePack = serde_json::from_str(&data)
                    .map_err(|e| EvidenceStoreError::SerializationError(e.to_string()))?;
                packs.push(evidence);
            }

            Ok(packs)
        }

        async fn get_latest(
            &self,
            task_id: Uuid,
        ) -> Result<Option<EvidencePack>, EvidenceStoreError> {
            let task_id_str = task_id.to_string();

            let row: Option<(String,)> = sqlx::query_as(
                r#"
                SELECT data FROM evidence_packs 
                WHERE task_id = ?
                ORDER BY created_at DESC
                LIMIT 1
                "#,
            )
            .bind(&task_id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            match row {
                Some((data,)) => {
                    let evidence: EvidencePack = serde_json::from_str(&data)
                        .map_err(|e| EvidenceStoreError::SerializationError(e.to_string()))?;
                    Ok(Some(evidence))
                }
                None => Ok(None),
            }
        }

        async fn list_ids(&self, task_id: Uuid) -> Result<Vec<Uuid>, EvidenceStoreError> {
            let task_id_str = task_id.to_string();

            let rows: Vec<(String,)> = sqlx::query_as(
                r#"SELECT id FROM evidence_packs WHERE task_id = ? ORDER BY created_at DESC"#,
            )
            .bind(&task_id_str)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            let ids: Vec<Uuid> = rows
                .into_iter()
                .filter_map(|(id_str,)| Uuid::parse_str(&id_str).ok())
                .collect();

            Ok(ids)
        }

        async fn count(&self, task_id: Uuid) -> Result<usize, EvidenceStoreError> {
            let task_id_str = task_id.to_string();

            let row: (i64,) =
                sqlx::query_as(r#"SELECT COUNT(*) FROM evidence_packs WHERE task_id = ?"#)
                    .bind(&task_id_str)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            Ok(row.0 as usize)
        }

        async fn exists(&self, id: Uuid) -> Result<bool, EvidenceStoreError> {
            let id_str = id.to_string();

            let row: (i64,) = sqlx::query_as(r#"SELECT COUNT(*) FROM evidence_packs WHERE id = ?"#)
                .bind(&id_str)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| EvidenceStoreError::StorageError(e.to_string()))?;

            Ok(row.0 > 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_pack_builder() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .add_gate_result(GateEvidence {
                name: "lint".to_string(),
                passed: true,
                message_counts: MessageCounts::new(),
                details: HashMap::new(),
            })
            .add_gate_result(GateEvidence {
                name: "test".to_string(),
                passed: true,
                message_counts: MessageCounts::new(),
                details: HashMap::new(),
            })
            .test_summary(TestEvidence {
                total: 10,
                passed: 10,
                failed: 0,
                skipped: 0,
                duration_ms: 500,
                tests: vec![],
            })
            .security(SecurityEvidence {
                passed: true,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                findings: vec![],
            })
            .confidence_score(ConfidenceEvidence {
                score: 0.9,
                auto_merge: true,
                signals: vec![],
            })
            .build()
            .unwrap();

        assert_eq!(pack.outcome, EvidenceOutcome::Passed);
        assert!(pack.confidence.auto_merge);
    }

    #[test]
    fn test_evidence_pack_failed_outcome() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .add_gate_result(GateEvidence {
                name: "lint".to_string(),
                passed: false,
                message_counts: {
                    let mut c = MessageCounts::new();
                    c.add_error();
                    c
                },
                details: HashMap::new(),
            })
            .build()
            .unwrap();

        assert_eq!(pack.outcome, EvidenceOutcome::Failed);
    }

    #[test]
    fn test_to_markdown() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .test_summary(TestEvidence {
                total: 5,
                passed: 4,
                failed: 1,
                skipped: 0,
                duration_ms: 1000,
                tests: vec![],
            })
            .confidence_score(ConfidenceEvidence {
                score: 0.75,
                auto_merge: false,
                signals: vec![],
            })
            .build()
            .unwrap();

        let md = pack.to_markdown();
        assert!(md.contains("FAILED"));
        assert!(md.contains("75"));
    }

    #[test]
    fn test_to_summary_json() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .test_summary(TestEvidence {
                total: 3,
                passed: 3,
                failed: 0,
                skipped: 0,
                duration_ms: 200,
                tests: vec![],
            })
            .build()
            .unwrap();

        let json = pack.to_summary_json();
        assert_eq!(json["outcome"], "passed");
        assert_eq!(json["test_summary"]["total"], 3);
    }

    #[test]
    fn test_builder_error_no_task_id() {
        let result = EvidencePackBuilder::new().build();
        assert!(result.is_err());
    }
}

// Tests for EvidenceStore implementations
#[cfg(test)]
mod evidence_store_tests {
    use super::*;
    use crate::evidence::sqlite_store::SqliteEvidenceStore;

    /// Helper to create a test evidence pack
    fn create_test_evidence(task_id: Uuid, passed: bool) -> EvidencePack {
        EvidencePackBuilder::new()
            .task_id(task_id)
            .test_summary(TestEvidence {
                total: 10,
                passed: if passed { 10 } else { 8 },
                failed: if passed { 0 } else { 2 },
                skipped: 0,
                duration_ms: 500,
                tests: vec![],
            })
            .security(SecurityEvidence {
                passed: true,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                findings: vec![],
            })
            .confidence_score(ConfidenceEvidence {
                score: if passed { 0.95 } else { 0.7 },
                auto_merge: passed,
                signals: vec![],
            })
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_in_memory_store_basic_operations() {
        let store = InMemoryEvidenceStore::new();
        let task_id = Uuid::new_v4();

        // Store evidence
        let evidence = create_test_evidence(task_id, true);
        let id = evidence.id;
        let stored_id = store.store(evidence).await.unwrap();
        assert_eq!(stored_id, id);

        // Retrieve by ID
        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.task_id, task_id);
        assert_eq!(retrieved.outcome, EvidenceOutcome::Passed);

        // Get by task ID
        let task_evidence = store.get_by_task_id(task_id).await.unwrap();
        assert_eq!(task_evidence.len(), 1);
        assert_eq!(task_evidence[0].id, id);

        // Get latest
        let latest = store.get_latest(task_id).await.unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().id, id);

        // Count
        let count = store.count(task_id).await.unwrap();
        assert_eq!(count, 1);

        // Exists
        let exists = store.exists(id).await.unwrap();
        assert!(exists);

        // Not exists
        let not_exists = store.exists(Uuid::new_v4()).await.unwrap();
        assert!(!not_exists);
    }

    #[tokio::test]
    async fn test_in_memory_store_multiple_evidences_per_task() {
        let store = InMemoryEvidenceStore::new();
        let task_id = Uuid::new_v4();

        // Store multiple evidence packs
        let evidence1 = create_test_evidence(task_id, true);
        let evidence2 = create_test_evidence(task_id, false);
        let evidence3 = create_test_evidence(task_id, true);

        let id1 = evidence1.id;
        let id2 = evidence2.id;
        let id3 = evidence3.id;

        store.store(evidence1).await.unwrap();
        // Small delay to ensure different timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        store.store(evidence2).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        store.store(evidence3).await.unwrap();

        // Get all for task - should be newest first
        let all_evidence = store.get_by_task_id(task_id).await.unwrap();
        assert_eq!(all_evidence.len(), 3);
        // Should be sorted by created_at DESC (newest first)
        assert_eq!(all_evidence[0].id, id3);
        assert_eq!(all_evidence[1].id, id2);
        assert_eq!(all_evidence[2].id, id1);

        // List IDs
        let ids = store.list_ids(task_id).await.unwrap();
        assert_eq!(ids.len(), 3);

        // Count
        let count = store.count(task_id).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_in_memory_store_multiple_tasks() {
        let store = InMemoryEvidenceStore::new();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        let evidence1 = create_test_evidence(task1, true);
        let evidence2 = create_test_evidence(task2, false);

        store.store(evidence1).await.unwrap();
        store.store(evidence2).await.unwrap();

        // Get evidence for task1
        let task1_evidence = store.get_by_task_id(task1).await.unwrap();
        assert_eq!(task1_evidence.len(), 1);
        assert_eq!(task1_evidence[0].task_id, task1);

        // Get evidence for task2
        let task2_evidence = store.get_by_task_id(task2).await.unwrap();
        assert_eq!(task2_evidence.len(), 1);
        assert_eq!(task2_evidence[0].task_id, task2);

        // task3 should have no evidence
        let task3_evidence = store.get_by_task_id(Uuid::new_v4()).await.unwrap();
        assert!(task3_evidence.is_empty());
    }

    #[tokio::test]
    async fn test_in_memory_store_get_nonexistent() {
        let store = InMemoryEvidenceStore::new();
        let result = store.get(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_evidence_pack_immutability_concept() {
        // This test demonstrates the concept: evidence packs are immutable
        // Once created and stored, they cannot be modified
        let store = InMemoryEvidenceStore::new();
        let task_id = Uuid::new_v4();

        let evidence = create_test_evidence(task_id, true);
        let original_id = evidence.id;
        store.store(evidence).await.unwrap();

        // Retrieve the stored evidence
        let retrieved = store.get(original_id).await.unwrap().unwrap();

        // The retrieved evidence is a clone - original is unchanged
        // (EvidencePack is Clone, so this is fine)
        // Note: In a true immutable store, we couldn't do modifications
        // but for testing purposes, we demonstrate the retrieval works
        assert_eq!(retrieved.outcome, EvidenceOutcome::Passed);

        // In a real immutable store, we would NOT be able to update
        // For now, the InMemory store allows this, but SqliteEvidenceStore
        // would prevent modifications (no UPDATE query exists)
    }

    #[tokio::test]
    async fn test_sqlite_evidence_store_basic_operations() {
        // Create a temporary in-memory database for testing
        let store = SqliteEvidenceStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = Uuid::new_v4();
        let evidence = create_test_evidence(task_id, true);
        let id = evidence.id;

        // Store
        let stored_id = store.store(evidence).await.unwrap();
        assert_eq!(stored_id, id);

        // Get
        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().task_id, task_id);

        // Get by task ID
        let task_evidence = store.get_by_task_id(task_id).await.unwrap();
        assert_eq!(task_evidence.len(), 1);

        // Get latest
        let latest = store.get_latest(task_id).await.unwrap();
        assert!(latest.is_some());

        // Count
        let count = store.count(task_id).await.unwrap();
        assert_eq!(count, 1);

        // Exists
        let exists = store.exists(id).await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_sqlite_evidence_store_multiple_evidences() {
        let store = SqliteEvidenceStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = Uuid::new_v4();

        // Store multiple evidences
        let evidence1 = create_test_evidence(task_id, true);
        let evidence2 = create_test_evidence(task_id, false);

        let id1 = evidence1.id;
        let id2 = evidence2.id;

        store.store(evidence1).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        store.store(evidence2).await.unwrap();

        // Verify ordering (newest first)
        let all = store.get_by_task_id(task_id).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, id2); // newest first
        assert_eq!(all[1].id, id1);

        // Count
        let count = store.count(task_id).await.unwrap();
        assert_eq!(count, 2);

        // List IDs
        let ids = store.list_ids(task_id).await.unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn test_sqlite_evidence_store_get_nonexistent() {
        let store = SqliteEvidenceStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let result = store.get(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_evidence_store_persistence() {
        // Test that data persists in the same connection
        let store = SqliteEvidenceStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = Uuid::new_v4();
        let evidence = create_test_evidence(task_id, true);
        let id = evidence.id;

        // Store and verify
        store.store(evidence).await.unwrap();
        let retrieved = store.get(id).await.unwrap();
        assert!(retrieved.is_some());

        // Verify it still exists
        let exists = store.exists(id).await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_evidence_store_trait_object() {
        // Test that we can use the store through the trait
        let mem_store: Box<dyn EvidenceStore> = Box::new(InMemoryEvidenceStore::new());
        let sqlite_store: Box<dyn EvidenceStore> = Box::new(
            SqliteEvidenceStore::from_connection_string("sqlite::memory:")
                .await
                .unwrap(),
        );

        let task_id = Uuid::new_v4();

        // Test in-memory store through trait
        let evidence = create_test_evidence(task_id, true);
        let id = evidence.id;
        mem_store.store(evidence).await.unwrap();
        let retrieved = mem_store.get(id).await.unwrap();
        assert!(retrieved.is_some());

        // Test SQLite store through trait
        let evidence2 = create_test_evidence(task_id, false);
        let id2 = evidence2.id;
        sqlite_store.store(evidence2).await.unwrap();
        let retrieved2 = sqlite_store.get(id2).await.unwrap();
        assert!(retrieved2.is_some());
    }
}
