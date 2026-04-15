// procedural.rs - Procedural memory with Beta posterior distribution
//
// This module provides procedural memory for storing strategies, procedures,
// and action patterns with Bayesian confidence scoring using Beta posterior
// distribution for effectiveness tracking.
//
// Unlike skill_extraction which learns from SUCCESSFUL trajectories only,
// procedural memory tracks the EFFECTIVENESS of procedures over time using
// Bayesian inference to update confidence scores based on outcomes.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use swell_core::SwellError;

/// Approximation of the error function using Abramowitz and Stegun formula.
/// Valid for |x| <= 1.65, beyond that returns 1 or -1.
fn erf_approx(x: f64) -> f64 {
    let abs_x = x.abs();
    if abs_x > 1.65 {
        return if x > 0.0 { 1.0 } else { -1.0 };
    }

    // Approximation formula from Abramowitz and Stegun (13.4.3)
    let t = 1.0 / (1.0 + 0.5 * abs_x);
    let tau = t
        * (-abs_x * abs_x - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886807
                                + t * (-1.13520398
                                    + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))));
    let sign = if x > 0.0 { 1.0 } else { -1.0 };
    sign * tau
}

/// Standard normal CDF approximation (valid for all real x)
fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf_approx(x / std::f64::consts::SQRT_2))
}

/// A precondition that must be met for a procedure to be applicable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedurePrecondition {
    /// The type of precondition (tool_required, file_exists, context_var, etc.)
    pub precondition_type: PreconditionType,
    /// The specific value required (e.g., tool name, file path, variable name)
    pub value: String,
    /// Whether this precondition is strict (must have) or soft (should have)
    pub is_strict: bool,
    /// Confidence in this precondition based on contrastive analysis
    pub confidence: f64,
    /// Evidence: which differentiating factor produced this precondition
    pub source_factor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreconditionType {
    /// Tool must be used in this procedure
    ToolRequired,
    /// Tool must NOT be used (differentiating negative)
    ToolForbidden,
    /// File must exist before procedure
    FileExists,
    /// File must NOT be modified (differentiating negative)
    FileUntouched,
    /// Context variable must be set
    ContextVarRequired,
    /// Context variable must NOT be certain value
    ContextVarForbidden,
    /// Sequence of steps must be in order
    StepOrderRequired,
    /// Risk level threshold
    MaxRiskLevel,
}

impl PreconditionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PreconditionType::ToolRequired => "tool_required",
            PreconditionType::ToolForbidden => "tool_forbidden",
            PreconditionType::FileExists => "file_exists",
            PreconditionType::FileUntouched => "file_untouched",
            PreconditionType::ContextVarRequired => "context_var_required",
            PreconditionType::ContextVarForbidden => "context_var_forbidden",
            PreconditionType::StepOrderRequired => "step_order_required",
            PreconditionType::MaxRiskLevel => "max_risk_level",
        }
    }
}

/// A procedure representing a reusable strategy or action pattern with effectiveness tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Keywords that indicate when this procedure is applicable
    pub context_pattern: String,
    pub steps: Vec<ProcedureStep>,
    pub effectiveness: BetaPosterior,
    pub usage_count: u32,
    pub last_used: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
    /// Preconditions derived from contrastive learning analysis
    /// These tighten the procedure's applicability criteria based on
    /// what distinguishes successful executions from failed ones
    pub preconditions: Vec<ProcedurePrecondition>,
}

impl Procedure {
    /// Create a new procedure with uniform Beta prior (Beta(1,1) = uniform)
    pub fn new(name: String, description: String, context_pattern: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description,
            context_pattern,
            steps: Vec::new(),
            effectiveness: BetaPosterior::new(1.0, 1.0),
            usage_count: 0,
            last_used: None,
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
            preconditions: Vec::new(),
        }
    }

    /// Add a precondition derived from contrastive learning analysis
    /// If a precondition of the same type and value already exists, it updates
    /// the confidence (only increases, never decreases) and source factor
    pub fn add_precondition(&mut self, precondition: ProcedurePrecondition) {
        // Check if we already have this precondition
        if let Some(existing) = self.preconditions.iter_mut().find(|p| {
            p.precondition_type == precondition.precondition_type && p.value == precondition.value
        }) {
            // Update confidence (only increase, never decrease)
            if precondition.confidence > existing.confidence {
                existing.confidence = precondition.confidence;
            }
            // Update source factor if provided
            if let Some(ref source) = precondition.source_factor {
                if existing.source_factor.is_none() {
                    existing.source_factor = Some(source.clone());
                }
            }
            // Update strictness - if any version says it's strict, it becomes strict
            if precondition.is_strict {
                existing.is_strict = true;
            }
        } else {
            // Add new precondition
            self.preconditions.push(precondition);
        }
        self.updated_at = Utc::now();
    }

    /// Add a tool-required precondition (tool must be used in this procedure)
    pub fn require_tool(&mut self, tool_name: &str, confidence: f64, source: Option<String>) {
        self.add_precondition(ProcedurePrecondition {
            precondition_type: PreconditionType::ToolRequired,
            value: tool_name.to_string(),
            is_strict: false,
            confidence,
            source_factor: source,
        });
    }

    /// Add a tool-forbidden precondition (tool must NOT be used)
    pub fn forbid_tool(&mut self, tool_name: &str, confidence: f64, source: Option<String>) {
        self.add_precondition(ProcedurePrecondition {
            precondition_type: PreconditionType::ToolForbidden,
            value: tool_name.to_string(),
            is_strict: true, // Forbidden tools are strict
            confidence,
            source_factor: source,
        });
    }

    /// Add a file-untouched precondition (file must NOT be modified)
    pub fn preserve_file(&mut self, file_path: &str, confidence: f64, source: Option<String>) {
        self.add_precondition(ProcedurePrecondition {
            precondition_type: PreconditionType::FileUntouched,
            value: file_path.to_string(),
            is_strict: true, // Preserved files are strict
            confidence,
            source_factor: source,
        });
    }

    /// Get preconditions that are strict (must be met for procedure to apply)
    pub fn strict_preconditions(&self) -> Vec<&ProcedurePrecondition> {
        self.preconditions.iter().filter(|p| p.is_strict).collect()
    }

    /// Get preconditions that are soft (should be met for better results)
    pub fn soft_preconditions(&self) -> Vec<&ProcedurePrecondition> {
        self.preconditions.iter().filter(|p| !p.is_strict).collect()
    }

    /// Get the highest confidence among all preconditions
    pub fn highest_precondition_confidence(&self) -> f64 {
        self.preconditions
            .iter()
            .map(|p| p.confidence)
            .fold(0.0, f64::max)
    }

    /// Update the effectiveness based on an outcome (success/failure)
    pub fn record_outcome(&mut self, success: bool) {
        self.effectiveness.update(success);
        self.usage_count += 1;
        self.last_used = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Get the expected success probability (mean of Beta distribution)
    pub fn expected_success_rate(&self) -> f64 {
        self.effectiveness.mean()
    }

    /// Get the confidence score (HDI width indicator, lower is better)
    pub fn confidence_score(&self) -> f64 {
        self.effectiveness.confidence_score()
    }

    /// Get the confidence level based on expected success rate
    ///
    /// Thresholds:
    /// - <0.3: deprecated
    /// - 0.3-0.6: uncertain
    /// - 0.6-0.8: probable
    /// - >0.8: confident
    pub fn confidence_level(&self) -> ConfidenceLevel {
        self.effectiveness.confidence_level()
    }
}

/// A single step within a procedure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureStep {
    pub order: usize,
    pub description: String,
    pub tool_sequence: Vec<String>,
    pub validation_check: Option<String>,
}

/// Confidence level based on expected probability thresholds
///
/// Thresholds:
/// - <0.3: deprecated (low confidence, should not be used)
/// - 0.3-0.6: uncertain (needs more evidence)
/// - 0.6-0.8: probable (likely effective)
/// - >0.8: confident (highly reliable)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    /// Confidence < 0.3 - deprecated, should not be used
    Deprecated,
    /// Confidence 0.3-0.6 - uncertain, needs more evidence
    Uncertain,
    /// Confidence 0.6-0.8 - probable, likely effective
    Probable,
    /// Confidence > 0.8 - confident, highly reliable
    Confident,
}

impl ConfidenceLevel {
    /// Threshold for uncertain zone (lower bound)
    pub const UNCERTAIN_THRESHOLD: f64 = 0.3;
    /// Threshold for probable zone (lower bound)
    pub const PROBABLE_THRESHOLD: f64 = 0.6;
    /// Threshold for confident zone (lower bound)
    pub const CONFIDENT_THRESHOLD: f64 = 0.8;

    /// Get the confidence level from a probability value
    pub fn from_probability(probability: f64) -> Self {
        if probability < Self::UNCERTAIN_THRESHOLD {
            ConfidenceLevel::Deprecated
        } else if probability < Self::PROBABLE_THRESHOLD {
            ConfidenceLevel::Uncertain
        } else if probability < Self::CONFIDENT_THRESHOLD {
            ConfidenceLevel::Probable
        } else {
            ConfidenceLevel::Confident
        }
    }

    /// Get the minimum probability for this confidence level
    pub fn min_probability(&self) -> f64 {
        match self {
            ConfidenceLevel::Deprecated => 0.0,
            ConfidenceLevel::Uncertain => Self::UNCERTAIN_THRESHOLD,
            ConfidenceLevel::Probable => Self::PROBABLE_THRESHOLD,
            ConfidenceLevel::Confident => Self::CONFIDENT_THRESHOLD,
        }
    }

    /// Get the maximum probability for this confidence level
    pub fn max_probability(&self) -> f64 {
        match self {
            ConfidenceLevel::Deprecated => Self::UNCERTAIN_THRESHOLD,
            ConfidenceLevel::Uncertain => Self::PROBABLE_THRESHOLD,
            ConfidenceLevel::Probable => Self::CONFIDENT_THRESHOLD,
            ConfidenceLevel::Confident => 1.0,
        }
    }

    /// Check if a probability meets the minimum threshold for this level
    pub fn is_met_by(&self, probability: f64) -> bool {
        match self {
            ConfidenceLevel::Deprecated => probability < Self::UNCERTAIN_THRESHOLD,
            ConfidenceLevel::Uncertain => {
                (Self::UNCERTAIN_THRESHOLD..Self::PROBABLE_THRESHOLD).contains(&probability)
            }
            ConfidenceLevel::Probable => {
                (Self::PROBABLE_THRESHOLD..Self::CONFIDENT_THRESHOLD).contains(&probability)
            }
            ConfidenceLevel::Confident => probability >= Self::CONFIDENT_THRESHOLD,
        }
    }

    /// Human-readable description of the confidence level
    pub fn description(&self) -> &'static str {
        match self {
            ConfidenceLevel::Deprecated => "deprecated (< 0.3) - should not be used",
            ConfidenceLevel::Uncertain => "uncertain (0.3-0.6) - needs more evidence",
            ConfidenceLevel::Probable => "probable (0.6-0.8) - likely effective",
            ConfidenceLevel::Confident => "confident (> 0.8) - highly reliable",
        }
    }
}

impl std::fmt::Display for ConfidenceLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            ConfidenceLevel::Deprecated => "deprecated",
            ConfidenceLevel::Uncertain => "uncertain",
            ConfidenceLevel::Probable => "probable",
            ConfidenceLevel::Confident => "confident",
        };
        write!(f, "{}", label)
    }
}

/// Beta posterior distribution for tracking effectiveness
///
/// Uses Bayesian inference with Beta conjugate prior:
/// - Prior: Beta(α₀, β₀) representing initial belief
/// - After observing s successes and f failures:
///   Posterior: Beta(α₀ + s, β₀ + f)
/// - Expected probability: E[p | data] = α / (α + β)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetaPosterior {
    /// Alpha parameter (successes + prior successes)
    pub alpha: f64,
    /// Beta parameter (failures + prior failures)
    pub beta: f64,
    /// Number of observed successes (likelihood)
    pub successes: u32,
    /// Number of observed failures (likelihood)
    pub failures: u32,
}

impl BetaPosterior {
    /// Create a new Beta posterior with given parameters
    ///
    /// Beta(1, 1) is the uniform prior (no prior knowledge)
    /// Beta(α, β) where α = successes + 1, β = failures + 1
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha,
            beta,
            successes: 0,
            failures: 0,
        }
    }

    /// Create from observed successes and failures
    ///
    /// Starts with uniform prior Beta(1,1) and updates with observations
    pub fn from_observations(successes: u32, failures: u32) -> Self {
        let alpha = 1.0 + successes as f64;
        let beta = 1.0 + failures as f64;
        Self {
            alpha,
            beta,
            successes,
            failures,
        }
    }

    /// Update the posterior with a new observation
    pub fn update(&mut self, success: bool) {
        if success {
            self.successes += 1;
            self.alpha += 1.0;
        } else {
            self.failures += 1;
            self.beta += 1.0;
        }
    }

    /// Compute the mean of the Beta distribution
    ///
    /// This is the expected probability of success given the observed data
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Compute the variance of the Beta distribution
    pub fn variance(&self) -> f64 {
        let denom = (self.alpha + self.beta).powi(2) * (self.alpha + self.beta + 1.0);
        if denom == 0.0 {
            return 0.0;
        }
        (self.alpha * self.beta) / denom
    }

    /// Compute standard deviation
    pub fn std(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Compute confidence score based on HDI width
    ///
    /// Returns a score from 0.0 to 1.0 where:
    /// - 1.0 = high confidence (narrow HDI)
    /// - 0.0 = low confidence (wide HDI or no data)
    ///
    /// Uses 95% highest density interval approximation
    pub fn confidence_score(&self) -> f64 {
        let total = self.alpha + self.beta;

        if total < 4.0 {
            // Not enough data for meaningful confidence
            return 0.0;
        }

        // Approximate 95% HDI using normal approximation when alpha, beta > 5
        if self.alpha > 5.0 && self.beta > 5.0 {
            let _mean = self.mean();
            let std = self.std();
            let hdi_width = 2.0 * 1.96 * std; // Approximate 95% HDI width

            // Normalize: confidence = 1 when HDI is narrow (practically 0)
            // and decreases as HDI widens
            // Max possible width for any probability is 1.0
            let max_width = 1.0;
            let normalized = 1.0 - (hdi_width / max_width).min(1.0);
            normalized.max(0.0)
        } else {
            // Not enough data for normal approximation
            // Scale confidence based on sample size
            (total / 100.0).min(1.0)
        }
    }

    /// Get the 95% highest density interval (approximation)
    ///
    /// Returns (lower_bound, upper_bound) for the success probability
    pub fn hdi_95(&self) -> (f64, f64) {
        let total = self.alpha + self.beta;

        if total < 4.0 {
            // Not enough data, return uninformative interval
            return (0.0, 1.0);
        }

        // Normal approximation when we have enough data
        if self.alpha > 5.0 && self.beta > 5.0 {
            let mean = self.mean();
            let std = self.std();
            let z = 1.96; // 95% CI
            let lower = (mean - z * std).max(0.0);
            let upper = (mean + z * std).min(1.0);
            (lower, upper)
        } else {
            // Jeffreys interval approximation for small samples
            let alpha_adj = self.alpha + 0.5;
            let beta_adj = self.beta + 0.5;
            let total_adj = alpha_adj + beta_adj;
            let mean = alpha_adj / total_adj;
            let se = ((alpha_adj * beta_adj) / (total_adj.powi(2) * (total_adj + 1.0))).sqrt();
            let lower = (mean - 1.96 * se).max(0.0);
            let upper = (mean + 1.96 * se).min(1.0);
            (lower, upper)
        }
    }

    /// Probability that p > threshold (using Beta CDF)
    ///
    /// For efficiency, uses approximation for common thresholds
    pub fn prob_greater_than(&self, threshold: f64) -> f64 {
        if threshold <= 0.0 {
            return 1.0;
        }
        if threshold >= 1.0 {
            return 0.0;
        }

        // For well-behaved Beta distributions, approximate using normal CDF
        if self.alpha > 3.0 && self.beta > 3.0 {
            let z = (self.mean() - threshold) / self.std();
            let cdf = normal_cdf(z);
            cdf.clamp(0.0, 1.0)
        } else {
            // For small samples, return conservative estimate
            if self.mean() > threshold {
                (self.alpha / (self.alpha + self.beta)).max(0.5)
            } else {
                1.0 - (self.alpha / (self.alpha + self.beta)).max(0.5)
            }
        }
    }

    /// Get the confidence level based on the expected probability
    ///
    /// Uses threshold zones:
    /// - <0.3: deprecated
    /// - 0.3-0.6: uncertain
    /// - 0.6-0.8: probable
    /// - >0.8: confident
    pub fn confidence_level(&self) -> ConfidenceLevel {
        ConfidenceLevel::from_probability(self.mean())
    }

    /// Check if this posterior has enough data for meaningful inference
    ///
    /// Returns true if total observations >= 4
    pub fn has_sufficient_data(&self) -> bool {
        self.alpha + self.beta >= 4.0
    }

    /// Get total number of observations
    pub fn total_observations(&self) -> u32 {
        self.successes + self.failures
    }
}

/// Query for finding relevant procedures by context
#[derive(Debug, Clone, Default)]
pub struct ProcedureQuery {
    /// Keywords to match against context_pattern
    pub keywords: Option<String>,
    /// Minimum confidence score (0.0 to 1.0)
    pub min_confidence: Option<f64>,
    /// Minimum number of uses
    pub min_uses: Option<u32>,
    /// Limit results
    pub limit: usize,
    pub offset: usize,
}

impl ProcedureQuery {
    pub fn new() -> Self {
        Self {
            keywords: None,
            min_confidence: None,
            min_uses: None,
            limit: 100,
            offset: 0,
        }
    }

    pub fn with_keywords(mut self, keywords: String) -> Self {
        self.keywords = Some(keywords);
        self
    }

    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = Some(confidence);
        self
    }

    pub fn with_min_uses(mut self, uses: u32) -> Self {
        self.min_uses = Some(uses);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Result from a procedure query with relevance scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureResult {
    pub procedure: Procedure,
    pub relevance_score: f64,
}

/// SQLite-based procedural memory store
#[derive(Clone)]
pub struct SqliteProceduralStore {
    pool: Arc<SqlitePool>,
}

impl SqliteProceduralStore {
    /// Create a new SqliteProceduralStore with the given database URL
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

    /// Initialize the database schema for procedural memory
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS procedures (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                context_pattern TEXT NOT NULL,
                steps TEXT NOT NULL,
                alpha REAL NOT NULL,
                beta REAL NOT NULL,
                successes INTEGER NOT NULL,
                failures INTEGER NOT NULL,
                usage_count INTEGER NOT NULL,
                last_used TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT NOT NULL,
                preconditions TEXT NOT NULL DEFAULT '[]'
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Add preconditions column if it doesn't exist (migration for existing databases)
        let _ = sqlx::query(
            "ALTER TABLE procedures ADD COLUMN preconditions TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()));

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_procedures_name ON procedures(name)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_procedures_context ON procedures(context_pattern)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Convert database row to Procedure
    fn row_to_procedure(&self, row: &SqliteRow) -> Result<Procedure, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let name: String = row.get("name");
        let description: String = row.get("description");
        let context_pattern: String = row.get("context_pattern");
        let steps_str: String = row.get("steps");
        let alpha: f64 = row.get("alpha");
        let beta: f64 = row.get("beta");
        let successes: u32 = row.get("successes");
        let failures: u32 = row.get("failures");
        let usage_count: u32 = row.get("usage_count");
        let last_used_str: Option<String> = row.get("last_used");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let metadata_str: String = row.get("metadata");
        let preconditions_str: String = row.get("preconditions");

        let steps: Vec<ProcedureStep> = serde_json::from_str(&steps_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON steps: {}", e)))?;

        let last_used = last_used_str
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&chrono::Utc))
            })
            .transpose()
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;
        let preconditions: Vec<ProcedurePrecondition> = serde_json::from_str(&preconditions_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON preconditions: {}", e)))?;

        let effectiveness = BetaPosterior {
            alpha,
            beta,
            successes,
            failures,
        };

        Ok(Procedure {
            id,
            name,
            description,
            context_pattern,
            steps,
            effectiveness,
            usage_count,
            last_used,
            created_at,
            updated_at,
            metadata,
            preconditions,
        })
    }
}

#[async_trait]
impl ProceduralStore for SqliteProceduralStore {
    /// Store a new procedure
    async fn store_procedure(&self, procedure: Procedure) -> Result<Uuid, SwellError> {
        let steps_str = serde_json::to_string(&procedure.steps)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let last_used_str = procedure.last_used.map(|dt| dt.to_rfc3339());
        let metadata_str = serde_json::to_string(&procedure.metadata)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let preconditions_str = serde_json::to_string(&procedure.preconditions)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = procedure.created_at.to_rfc3339();
        let updated_at_str = procedure.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO procedures (id, name, description, context_pattern, steps, alpha, beta, successes, failures, usage_count, last_used, created_at, updated_at, metadata, preconditions)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(procedure.id.to_string())
        .bind(&procedure.name)
        .bind(&procedure.description)
        .bind(&procedure.context_pattern)
        .bind(&steps_str)
        .bind(procedure.effectiveness.alpha)
        .bind(procedure.effectiveness.beta)
        .bind(procedure.effectiveness.successes)
        .bind(procedure.effectiveness.failures)
        .bind(procedure.usage_count)
        .bind(last_used_str)
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .bind(&metadata_str)
        .bind(&preconditions_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(procedure.id)
    }

    /// Retrieve a procedure by ID
    async fn get_procedure(&self, id: Uuid) -> Result<Option<Procedure>, SwellError> {
        let row = sqlx::query("SELECT * FROM procedures WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(self.row_to_procedure(&r)?)),
            None => Ok(None),
        }
    }

    /// Update a procedure's effectiveness with a new outcome
    async fn record_outcome(&self, id: Uuid, success: bool) -> Result<(), SwellError> {
        let procedure = self
            .get_procedure(id)
            .await?
            .ok_or_else(|| SwellError::DatabaseError(format!("Procedure not found: {}", id)))?;

        let mut updated = procedure;
        updated.record_outcome(success);

        sqlx::query(
            r#"
            UPDATE procedures
            SET alpha = ?, beta = ?, successes = ?, failures = ?, usage_count = ?, last_used = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(updated.effectiveness.alpha)
        .bind(updated.effectiveness.beta)
        .bind(updated.effectiveness.successes)
        .bind(updated.effectiveness.failures)
        .bind(updated.usage_count)
        .bind(updated.last_used.map(|dt| dt.to_rfc3339()))
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Search procedures by context similarity
    async fn find_by_context(
        &self,
        query: ProcedureQuery,
    ) -> Result<Vec<ProcedureResult>, SwellError> {
        let mut sql = String::from("SELECT * FROM procedures WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(ref keywords) = query.keywords {
            // Use LIKE for context pattern matching
            sql.push_str(" AND context_pattern LIKE ?");
            params.push(format!("%{}%", keywords.to_lowercase()));
        }

        if let Some(min_conf) = query.min_confidence {
            // Filter by confidence score (computed from alpha, beta)
            // confidence_score threshold approximation
            let min_total = ((1.0 - min_conf) * 100.0).max(4.0) as u32;
            sql.push_str(&format!(" AND (alpha + beta) >= {}", min_total));
        }

        if let Some(min_uses) = query.min_uses {
            sql.push_str(&format!(" AND usage_count >= {}", min_uses));
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
            let procedure = self.row_to_procedure(&row)?;

            // Deprioritize procedures with confidence < 0.3 (Deprecated)
            // These are filtered out by default unless explicitly included via min_confidence
            if procedure.effectiveness.confidence_level() == ConfidenceLevel::Deprecated {
                // Skip deprecated procedures - they should not be used
                continue;
            }

            let relevance = if let Some(ref keywords) = query.keywords {
                self.calculate_relevance(&procedure, keywords)
            } else {
                1.0 // Default relevance if no keywords
            };

            results.push(ProcedureResult {
                procedure,
                relevance_score: relevance,
            });
        }

        // Sort by relevance score descending
        results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Delete a procedure
    async fn delete_procedure(&self, id: Uuid) -> Result<(), SwellError> {
        let result = sqlx::query("DELETE FROM procedures WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "Procedure not found: {}",
                id
            )));
        }

        Ok(())
    }
}

impl SqliteProceduralStore {
    /// Calculate relevance score based on keyword match
    fn calculate_relevance(&self, procedure: &Procedure, keywords: &str) -> f64 {
        let context_lower = procedure.context_pattern.to_lowercase();
        let kw_lower = keywords.to_lowercase();

        // Tokenize keywords
        let kw_tokens: Vec<&str> = kw_lower.split_whitespace().collect();

        // Count matching tokens
        let matches: usize = kw_tokens
            .iter()
            .filter(|kw| context_lower.contains(*kw))
            .count();

        if matches == 0 {
            return 0.1; // Low base relevance
        }

        // Calculate combined score: keyword match ratio + confidence weight
        let match_ratio = matches as f64 / kw_tokens.len() as f64;
        let confidence_weight = procedure.confidence_score();

        // Higher weight for confidence + keyword match
        (match_ratio * 0.7 + confidence_weight * 0.3).min(1.0)
    }
}

/// Trait for procedural memory storage operations
#[async_trait]
pub trait ProceduralStore: Send + Sync {
    /// Store a new procedure
    async fn store_procedure(&self, procedure: Procedure) -> Result<Uuid, SwellError>;

    /// Retrieve a procedure by ID
    async fn get_procedure(&self, id: Uuid) -> Result<Option<Procedure>, SwellError>;

    /// Record an outcome for a procedure (success/failure)
    async fn record_outcome(&self, id: Uuid, success: bool) -> Result<(), SwellError>;

    /// Search procedures by context similarity
    async fn find_by_context(
        &self,
        query: ProcedureQuery,
    ) -> Result<Vec<ProcedureResult>, SwellError>;

    /// Delete a procedure
    async fn delete_procedure(&self, id: Uuid) -> Result<(), SwellError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beta_posterior_new() {
        let bp = BetaPosterior::new(1.0, 1.0);
        assert_eq!(bp.alpha, 1.0);
        assert_eq!(bp.beta, 1.0);
        assert_eq!(bp.successes, 0);
        assert_eq!(bp.failures, 0);
    }

    #[test]
    fn test_beta_posterior_from_observations() {
        let bp = BetaPosterior::from_observations(5, 3);
        assert_eq!(bp.alpha, 6.0); // 1 + 5
        assert_eq!(bp.beta, 4.0); // 1 + 3
        assert_eq!(bp.successes, 5);
        assert_eq!(bp.failures, 3);
    }

    #[test]
    fn test_beta_posterior_update() {
        let mut bp = BetaPosterior::new(1.0, 1.0);
        bp.update(true);
        assert_eq!(bp.alpha, 2.0);
        assert_eq!(bp.successes, 1);

        bp.update(false);
        assert_eq!(bp.beta, 2.0);
        assert_eq!(bp.failures, 1);
    }

    #[test]
    fn test_beta_posterior_mean() {
        // Beta(6, 4) should have mean 6/(6+4) = 0.6
        let bp = BetaPosterior::from_observations(5, 3);
        assert!((bp.mean() - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_beta_posterior_mean_perfect_success() {
        // All successes: Beta(n+1, 1) has mean (n+1)/(n+2)
        let mut bp = BetaPosterior::new(1.0, 1.0);
        for _ in 0..10 {
            bp.update(true);
        }
        // After 10 successes: α = 1 + 10 = 11, β = 1 + 0 = 1
        // Mean = α / (α + β) = 11 / 12 = 0.9167
        assert!((bp.mean() - 11.0 / 12.0).abs() < 0.001);
    }

    #[test]
    fn test_beta_posterior_mean_perfect_failure() {
        // All failures: Beta(1, n+1) has mean 1/(n+1)
        let mut bp = BetaPosterior::new(1.0, 1.0);
        for _ in 0..10 {
            bp.update(false);
        }
        assert!((bp.mean() - 1.0 / 12.0).abs() < 0.001);
    }

    #[test]
    fn test_beta_posterior_variance() {
        let bp = BetaPosterior::from_observations(10, 10);
        let var = bp.variance();
        assert!(var > 0.0);
        assert!(var < 0.25); // Should be relatively small
    }

    #[test]
    fn test_beta_posterior_confidence_score_no_data() {
        // With no data, confidence should be 0
        let bp = BetaPosterior::new(1.0, 1.0);
        assert_eq!(bp.confidence_score(), 0.0);
    }

    #[test]
    fn test_beta_posterior_confidence_score_some_data() {
        // With some data, confidence should increase
        let mut bp = BetaPosterior::new(1.0, 1.0);
        for _ in 0..10 {
            bp.update(true);
        }
        let conf = bp.confidence_score();
        assert!(conf > 0.0);
    }

    #[test]
    fn test_beta_posterior_confidence_score_more_data() {
        // With more data, confidence should increase further
        let mut bp_low = BetaPosterior::new(1.0, 1.0);
        for _ in 0..5 {
            bp_low.update(true);
        }

        let mut bp_high = BetaPosterior::new(1.0, 1.0);
        for _ in 0..50 {
            bp_high.update(true);
        }

        assert!(bp_high.confidence_score() > bp_low.confidence_score());
    }

    #[test]
    fn test_beta_posterior_hdi_95() {
        let bp = BetaPosterior::from_observations(20, 10);
        let (lower, upper) = bp.hdi_95();
        assert!(lower >= 0.0);
        assert!(upper <= 1.0);
        assert!(lower < upper);
        // Mean is 20/31 ≈ 0.645, should be within interval
        assert!(bp.mean() >= lower);
        assert!(bp.mean() <= upper);
    }

    #[test]
    fn test_beta_posterior_hdi_95_no_data() {
        let bp = BetaPosterior::new(1.0, 1.0);
        let (lower, upper) = bp.hdi_95();
        assert_eq!(lower, 0.0);
        assert_eq!(upper, 1.0);
    }

    #[test]
    fn test_beta_posterior_prob_greater_than() {
        let bp = BetaPosterior::from_observations(8, 2);
        // Beta(9, 3) has mean 9/12 = 0.75, which is > 0.5
        // So prob > 0.5 should be greater than 0.5 (conservative estimate from mean comparison)
        let prob = bp.prob_greater_than(0.5);
        assert!(prob > 0.5, "Expected prob > 0.5, got {}", prob);
    }

    #[test]
    fn test_procedure_new() {
        let procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        assert_eq!(procedure.name, "test_procedure");
        assert_eq!(procedure.effectiveness.alpha, 1.0);
        assert_eq!(procedure.effectiveness.beta, 1.0);
        assert_eq!(procedure.usage_count, 0);
    }

    #[test]
    fn test_procedure_record_outcome() {
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        procedure.record_outcome(true);
        assert_eq!(procedure.usage_count, 1);
        assert_eq!(procedure.effectiveness.successes, 1);

        procedure.record_outcome(false);
        assert_eq!(procedure.usage_count, 2);
        assert_eq!(procedure.effectiveness.failures, 1);
    }

    #[test]
    fn test_procedure_expected_success_rate() {
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // 3 successes, 1 failure => expected rate ≈ 3/5 = 0.6
        procedure.record_outcome(true);
        procedure.record_outcome(true);
        procedure.record_outcome(true);
        procedure.record_outcome(false);

        let rate = procedure.expected_success_rate();
        assert!((rate - 0.6).abs() < 0.1);
    }

    #[test]
    fn test_procedure_confidence_score() {
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Initially no confidence
        assert_eq!(procedure.confidence_score(), 0.0);

        // Add many observations
        for _ in 0..20 {
            procedure.record_outcome(true);
        }

        // Should have some confidence now
        assert!(procedure.confidence_score() > 0.0);
    }

    #[test]
    fn test_procedure_step_serialization() {
        let step = ProcedureStep {
            order: 1,
            description: "Test step".to_string(),
            tool_sequence: vec!["read_file".to_string(), "edit_file".to_string()],
            validation_check: Some("cargo build".to_string()),
        };

        let json = serde_json::to_string(&step).unwrap();
        let deserialized: ProcedureStep = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.order, 1);
        assert_eq!(deserialized.tool_sequence.len(), 2);
    }

    #[test]
    fn test_procedure_serialization() {
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        procedure.steps.push(ProcedureStep {
            order: 1,
            description: "Test step".to_string(),
            tool_sequence: vec!["read_file".to_string()],
            validation_check: None,
        });

        procedure.record_outcome(true);
        procedure.record_outcome(true);
        procedure.record_outcome(false);

        let json = serde_json::to_string_pretty(&procedure).unwrap();
        let deserialized: Procedure = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "test_procedure");
        assert_eq!(deserialized.steps.len(), 1);
        assert_eq!(deserialized.usage_count, 3);
        assert_eq!(deserialized.effectiveness.successes, 2);
        assert_eq!(deserialized.effectiveness.failures, 1);
    }

    #[tokio::test]
    async fn test_store_and_get_procedure() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        let procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context keywords".to_string(),
        );

        let id = store.store_procedure(procedure.clone()).await.unwrap();
        assert_eq!(id, procedure.id);

        let retrieved = store.get_procedure(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "test_procedure");
        assert_eq!(retrieved.effectiveness.alpha, 1.0);
        assert_eq!(retrieved.effectiveness.beta, 1.0);
    }

    #[tokio::test]
    async fn test_record_outcome() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );
        procedure.steps.push(ProcedureStep {
            order: 1,
            description: "Step 1".to_string(),
            tool_sequence: vec!["shell".to_string()],
            validation_check: None,
        });

        let id = store.store_procedure(procedure.clone()).await.unwrap();

        // Record successful outcomes
        store.record_outcome(id, true).await.unwrap();
        store.record_outcome(id, true).await.unwrap();
        store.record_outcome(id, false).await.unwrap();

        let updated = store.get_procedure(id).await.unwrap().unwrap();
        assert_eq!(updated.usage_count, 3);
        assert_eq!(updated.effectiveness.successes, 2);
        assert_eq!(updated.effectiveness.failures, 1);
        assert!(updated.last_used.is_some());
    }

    #[tokio::test]
    async fn test_find_by_context_keywords() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Store procedures with different context patterns
        let proc1 = Procedure::new(
            "rust_fix".to_string(),
            "Fix Rust compilation errors".to_string(),
            "rust compilation error fix".to_string(),
        );
        let proc2 = Procedure::new(
            "test_write".to_string(),
            "Write unit tests".to_string(),
            "test unit testing write".to_string(),
        );
        let proc3 = Procedure::new(
            "api_impl".to_string(),
            "Implement API endpoint".to_string(),
            "api endpoint http rest".to_string(),
        );

        store.store_procedure(proc1).await.unwrap();
        store.store_procedure(proc2).await.unwrap();
        store.store_procedure(proc3).await.unwrap();

        // Search for "rust"
        let query = ProcedureQuery::new().with_keywords("rust".to_string());
        let results = store.find_by_context(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].procedure.name, "rust_fix");
        assert!(results[0].relevance_score > 0.5);
    }

    #[tokio::test]
    async fn test_find_by_context_min_uses() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        let mut proc1 = Procedure::new(
            "popular_procedure".to_string(),
            "A popular procedure".to_string(),
            "popular context".to_string(),
        );
        proc1.usage_count = 10;
        proc1.effectiveness = BetaPosterior::from_observations(8, 2);

        let proc2 = Procedure::new(
            "unpopular_procedure".to_string(),
            "An unpopular procedure".to_string(),
            "unpopular context".to_string(),
        );

        store.store_procedure(proc1).await.unwrap();
        store.store_procedure(proc2).await.unwrap();

        // Search with min_uses = 5
        let query = ProcedureQuery::new().with_min_uses(5);
        let results = store.find_by_context(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].procedure.name, "popular_procedure");
    }

    #[tokio::test]
    async fn test_find_by_context_min_confidence() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        // High confidence procedure - 99 successes (beta will be 1 from the update)
        // This gives alpha=100, beta=2, total=102 which >= 70 for 0.3 min_confidence threshold
        // min_total = (1.0 - 0.3) * 100.0 = 70, so total must be >= 70
        let mut high_conf = Procedure::new(
            "high_confidence".to_string(),
            "A high confidence procedure".to_string(),
            "high confidence context".to_string(),
        );
        for _ in 0..99 {
            high_conf.record_outcome(true);
        }

        // Low confidence procedure
        let low_conf = Procedure::new(
            "low_confidence".to_string(),
            "A low confidence procedure".to_string(),
            "low confidence context".to_string(),
        );

        store.store_procedure(high_conf).await.unwrap();
        store.store_procedure(low_conf).await.unwrap();

        // Search with min_confidence = 0.3
        let query = ProcedureQuery::new().with_min_confidence(0.3);
        let results = store.find_by_context(query).await.unwrap();

        // Should get at least the high confidence one
        assert!(results
            .iter()
            .any(|r| r.procedure.name == "high_confidence"));
    }

    #[tokio::test]
    async fn test_delete_procedure() {
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        let procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        let id = store.store_procedure(procedure).await.unwrap();
        store.delete_procedure(id).await.unwrap();

        let retrieved = store.get_procedure(id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_procedure_query_builder() {
        let query = ProcedureQuery::new()
            .with_keywords("rust test".to_string())
            .with_min_confidence(0.5)
            .with_min_uses(3)
            .with_limit(50);

        assert_eq!(query.keywords, Some("rust test".to_string()));
        assert_eq!(query.min_confidence, Some(0.5));
        assert_eq!(query.min_uses, Some(3));
        assert_eq!(query.limit, 50);
    }

    #[test]
    fn test_procedure_query_default() {
        let query = ProcedureQuery::default();
        assert!(query.keywords.is_none());
        assert!(query.min_confidence.is_none());
        assert!(query.min_uses.is_none());
        assert_eq!(query.limit, 0); // Default gives 0, use ProcedureQuery::new() for safe defaults
        assert_eq!(query.offset, 0);
    }

    #[test]
    fn test_procedure_result_serialization() {
        let procedure = Procedure::new(
            "test".to_string(),
            "desc".to_string(),
            "context".to_string(),
        );
        let result = ProcedureResult {
            procedure,
            relevance_score: 0.85,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ProcedureResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.relevance_score, 0.85);
    }

    #[test]
    fn test_beta_posterior_sequential_updates() {
        let mut bp = BetaPosterior::new(1.0, 1.0);

        // Sequence: T, T, T, F, T
        bp.update(true);
        bp.update(true);
        bp.update(true);
        bp.update(false);
        bp.update(true);

        // Final state: 4 successes, 1 failure
        // Start with Beta(1,1), after 4 successes: α = 1+4 = 5, after 1 failure: β = 1+1 = 2
        assert_eq!(bp.successes, 4);
        assert_eq!(bp.failures, 1);
        assert_eq!(bp.alpha, 5.0);
        assert_eq!(bp.beta, 2.0);

        // Mean = α / (α + β) = 5 / 7 ≈ 0.714
        assert!((bp.mean() - 5.0 / 7.0).abs() < 0.001);
    }

    #[test]
    fn test_beta_posterior_consistency_with_binomial() {
        // If we observe k successes in n trials, the posterior mean
        // should match the empirical frequency as n -> infinity
        let mut bp = BetaPosterior::new(1.0, 1.0);

        // 70% success rate
        for i in 0..100 {
            bp.update(i < 70);
        }

        let empirical = 70.0 / 100.0;
        let posterior_mean = bp.mean();

        // Should be close to empirical rate
        assert!((empirical - posterior_mean).abs() < 0.1);
    }

    // =====================================================================
    // Bayesian confidence level tests
    // =====================================================================

    #[test]
    fn test_confidence_level_thresholds() {
        // Test boundary at 0.3 - deprecated vs uncertain
        assert_eq!(
            ConfidenceLevel::from_probability(0.0),
            ConfidenceLevel::Deprecated
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.29),
            ConfidenceLevel::Deprecated
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.3),
            ConfidenceLevel::Uncertain
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.5),
            ConfidenceLevel::Uncertain
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.59),
            ConfidenceLevel::Uncertain
        );

        // Test boundary at 0.6 - uncertain vs probable
        assert_eq!(
            ConfidenceLevel::from_probability(0.6),
            ConfidenceLevel::Probable
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.7),
            ConfidenceLevel::Probable
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.79),
            ConfidenceLevel::Probable
        );

        // Test boundary at 0.8 - probable vs confident
        assert_eq!(
            ConfidenceLevel::from_probability(0.8),
            ConfidenceLevel::Confident
        );
        assert_eq!(
            ConfidenceLevel::from_probability(0.9),
            ConfidenceLevel::Confident
        );
        assert_eq!(
            ConfidenceLevel::from_probability(1.0),
            ConfidenceLevel::Confident
        );
    }

    #[test]
    fn test_confidence_level_description() {
        assert!(ConfidenceLevel::Deprecated
            .description()
            .contains("deprecated"));
        assert!(ConfidenceLevel::Uncertain
            .description()
            .contains("uncertain"));
        assert!(ConfidenceLevel::Probable.description().contains("probable"));
        assert!(ConfidenceLevel::Confident
            .description()
            .contains("confident"));
    }

    #[test]
    fn test_confidence_level_display() {
        assert_eq!(format!("{}", ConfidenceLevel::Deprecated), "deprecated");
        assert_eq!(format!("{}", ConfidenceLevel::Uncertain), "uncertain");
        assert_eq!(format!("{}", ConfidenceLevel::Probable), "probable");
        assert_eq!(format!("{}", ConfidenceLevel::Confident), "confident");
    }

    #[test]
    fn test_confidence_level_min_max_probability() {
        assert_eq!(ConfidenceLevel::Deprecated.min_probability(), 0.0);
        assert_eq!(ConfidenceLevel::Deprecated.max_probability(), 0.3);

        assert_eq!(ConfidenceLevel::Uncertain.min_probability(), 0.3);
        assert_eq!(ConfidenceLevel::Uncertain.max_probability(), 0.6);

        assert_eq!(ConfidenceLevel::Probable.min_probability(), 0.6);
        assert_eq!(ConfidenceLevel::Probable.max_probability(), 0.8);

        assert_eq!(ConfidenceLevel::Confident.min_probability(), 0.8);
        assert_eq!(ConfidenceLevel::Confident.max_probability(), 1.0);
    }

    #[test]
    fn test_confidence_level_is_met_by() {
        // Deprecated
        assert!(ConfidenceLevel::Deprecated.is_met_by(0.0));
        assert!(ConfidenceLevel::Deprecated.is_met_by(0.29));
        assert!(!ConfidenceLevel::Deprecated.is_met_by(0.3));

        // Uncertain
        assert!(ConfidenceLevel::Uncertain.is_met_by(0.3));
        assert!(ConfidenceLevel::Uncertain.is_met_by(0.5));
        assert!(!ConfidenceLevel::Uncertain.is_met_by(0.6));

        // Probable
        assert!(ConfidenceLevel::Probable.is_met_by(0.6));
        assert!(ConfidenceLevel::Probable.is_met_by(0.7));
        assert!(!ConfidenceLevel::Probable.is_met_by(0.8));

        // Confident
        assert!(ConfidenceLevel::Confident.is_met_by(0.8));
        assert!(ConfidenceLevel::Confident.is_met_by(0.9));
        assert!(ConfidenceLevel::Confident.is_met_by(1.0));
    }

    #[test]
    fn test_beta_posterior_confidence_level() {
        // Beta(1,1) uniform prior - mean = 0.5 = Uncertain
        let bp = BetaPosterior::new(1.0, 1.0);
        assert_eq!(bp.confidence_level(), ConfidenceLevel::Uncertain);

        // After many successes: mean approaches 1.0 = Confident
        let mut high_success = BetaPosterior::new(1.0, 1.0);
        for _ in 0..100 {
            high_success.update(true);
        }
        // With 100 successes and 0 failures: mean = 101/102 ≈ 0.99
        assert_eq!(high_success.confidence_level(), ConfidenceLevel::Confident);

        // After many failures: mean approaches 0 = Deprecated
        let mut high_failure = BetaPosterior::new(1.0, 1.0);
        for _ in 0..100 {
            high_failure.update(false);
        }
        // With 0 successes and 100 failures: mean = 1/102 ≈ 0.01
        assert_eq!(high_failure.confidence_level(), ConfidenceLevel::Deprecated);

        // Mixed: 15 successes, 5 failures -> mean = 16/22 ≈ 0.727 = Probable
        let mixed = BetaPosterior::from_observations(15, 5);
        assert_eq!(mixed.confidence_level(), ConfidenceLevel::Probable);
    }

    #[test]
    fn test_beta_posterior_confidence_level_zone_transitions() {
        // Track transitions through confidence zones as observations accumulate
        let mut bp = BetaPosterior::new(1.0, 1.0);

        // Start: Beta(1,1) -> mean 0.5 = Uncertain
        assert_eq!(bp.confidence_level(), ConfidenceLevel::Uncertain);

        // Add successes to move toward Confident
        // After 8 successes: Beta(9,1) -> mean = 9/10 = 0.9 = Confident
        for _ in 0..8 {
            bp.update(true);
        }
        assert_eq!(bp.confidence_level(), ConfidenceLevel::Confident);

        // Add failures to move toward Deprecated
        // After 10 total failures starting from Beta(9,1):
        // 8 successes, 10 failures -> Beta(9, 11) -> mean = 9/20 = 0.45 = Uncertain
        for _ in 0..10 {
            bp.update(false);
        }
        assert_eq!(bp.confidence_level(), ConfidenceLevel::Uncertain);
    }

    #[test]
    fn test_beta_posterior_has_sufficient_data() {
        let mut bp = BetaPosterior::new(1.0, 1.0);
        assert!(!bp.has_sufficient_data());

        // Need at least 4 total observations (alpha + beta >= 4)
        bp.update(true); // Beta(2,1) -> total = 3
        assert!(!bp.has_sufficient_data());

        bp.update(true); // Beta(3,1) -> total = 4
        assert!(bp.has_sufficient_data());

        bp.update(false); // Beta(3,2) -> total = 5
        assert!(bp.has_sufficient_data());
    }

    #[test]
    fn test_beta_posterior_total_observations() {
        let bp = BetaPosterior::from_observations(7, 3);
        assert_eq!(bp.total_observations(), 10);
    }

    #[test]
    fn test_procedure_confidence_level() {
        let mut procedure = Procedure::new(
            "test".to_string(),
            "desc".to_string(),
            "context".to_string(),
        );

        // Initially uncertain (mean = 0.5)
        assert_eq!(procedure.confidence_level(), ConfidenceLevel::Uncertain);

        // After many successes -> confident
        for _ in 0..50 {
            procedure.record_outcome(true);
        }
        assert_eq!(procedure.confidence_level(), ConfidenceLevel::Confident);

        // After failures -> drop to uncertain
        for _ in 0..30 {
            procedure.record_outcome(false);
        }
        // 50 successes, 30 failures: mean = 51/(51+31) = 51/82 ≈ 0.62 = Probable
        assert_eq!(procedure.confidence_level(), ConfidenceLevel::Probable);
    }

    #[test]
    fn test_confidence_level_serialization() {
        let level = ConfidenceLevel::Probable;
        let json = serde_json::to_string(&level).unwrap();
        let deserialized: ConfidenceLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, level);
    }

    // =====================================================================
    // Beta posterior tracking tests (mem-procedural-beta feature)
    // =====================================================================

    #[test]
    fn test_beta_posterior_new_procedure_starts_with_uniform_prior() {
        // New procedures should start with α=1, β=1 (mean=0.5)
        let procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Check initial Beta posterior state
        assert_eq!(procedure.effectiveness.alpha, 1.0);
        assert_eq!(procedure.effectiveness.beta, 1.0);
        assert_eq!(procedure.effectiveness.successes, 0);
        assert_eq!(procedure.effectiveness.failures, 0);

        // Mean should be 0.5 (α / (α + β) = 1 / 2)
        let mean = procedure.expected_success_rate();
        assert!(
            (mean - 0.5).abs() < 0.001,
            "Expected mean 0.5, got {}",
            mean
        );
    }

    #[test]
    fn test_beta_posterior_success_increments_alpha() {
        // On success, α should increment
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Record 3 successes
        procedure.record_outcome(true);
        procedure.record_outcome(true);
        procedure.record_outcome(true);

        // α = 1 + 3 = 4
        assert_eq!(procedure.effectiveness.alpha, 4.0);
        assert_eq!(procedure.effectiveness.successes, 3);

        // β should still be 1
        assert_eq!(procedure.effectiveness.beta, 1.0);
        assert_eq!(procedure.effectiveness.failures, 0);

        // Mean = α / (α + β) = 4 / 5 = 0.8
        let mean = procedure.expected_success_rate();
        assert!(
            (mean - 0.8).abs() < 0.001,
            "Expected mean 0.8, got {}",
            mean
        );
    }

    #[test]
    fn test_beta_posterior_failure_increments_beta() {
        // On failure, β should increment
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Record 2 failures
        procedure.record_outcome(false);
        procedure.record_outcome(false);

        // β = 1 + 2 = 3
        assert_eq!(procedure.effectiveness.beta, 3.0);
        assert_eq!(procedure.effectiveness.failures, 2);

        // α should still be 1
        assert_eq!(procedure.effectiveness.alpha, 1.0);
        assert_eq!(procedure.effectiveness.successes, 0);

        // Mean = α / (α + β) = 1 / 4 = 0.25
        let mean = procedure.expected_success_rate();
        assert!(
            (mean - 0.25).abs() < 0.001,
            "Expected mean 0.25, got {}",
            mean
        );
    }

    #[test]
    fn test_beta_posterior_mixed_success_and_failure() {
        // Mixed outcomes: 7 successes, 3 failures
        let mut procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        // Record 7 successes
        for _ in 0..7 {
            procedure.record_outcome(true);
        }
        // Record 3 failures
        for _ in 0..3 {
            procedure.record_outcome(false);
        }

        // Final state: α = 1 + 7 = 8, β = 1 + 3 = 4
        assert_eq!(procedure.effectiveness.alpha, 8.0);
        assert_eq!(procedure.effectiveness.beta, 4.0);
        assert_eq!(procedure.effectiveness.successes, 7);
        assert_eq!(procedure.effectiveness.failures, 3);

        // Mean = α / (α + β) = 8 / 12 = 0.667
        let mean = procedure.expected_success_rate();
        assert!(
            (mean - 0.667).abs() < 0.01,
            "Expected mean ~0.667, got {}",
            mean
        );
    }

    #[test]
    fn test_beta_posterior_mean_computation() {
        // Verify mean = α / (α + β) formula
        // Start with Beta(3, 2) which means α=3, β=2
        let bp = BetaPosterior::new(3.0, 2.0);

        // Mean should be 3 / (3 + 2) = 0.6
        let mean = bp.mean();
        assert!(
            (mean - 0.6).abs() < 0.001,
            "Expected mean 0.6, got {}",
            mean
        );
    }

    #[test]
    fn test_confidence_below_03_is_deprecated() {
        // Procedures with mean confidence < 0.3 should be Deprecated
        // This is the threshold for deprioritization

        // Beta(1, 5) -> mean = 1/6 ≈ 0.167 (< 0.3 = Deprecated)
        let mut procedure = Procedure::new(
            "deprecated_procedure".to_string(),
            "A deprecated procedure".to_string(),
            "test context".to_string(),
        );

        // Record 4 more failures to get Beta(1, 5)
        for _ in 0..4 {
            procedure.record_outcome(false);
        }

        // Mean = 1 / (1 + 5) = 1/6 ≈ 0.167
        let mean = procedure.expected_success_rate();
        assert!(mean < 0.3, "Mean {} should be < 0.3", mean);
        assert_eq!(procedure.confidence_level(), ConfidenceLevel::Deprecated);
    }

    #[test]
    fn test_confidence_above_03_is_not_deprecated() {
        // Procedures with mean confidence >= 0.3 should NOT be Deprecated

        // Beta(5, 1) -> mean = 5/6 ≈ 0.833 (>= 0.3 = Confident)
        let mut procedure = Procedure::new(
            "confident_procedure".to_string(),
            "A confident procedure".to_string(),
            "test context".to_string(),
        );

        // Record 4 more successes to get Beta(5, 1)
        for _ in 0..4 {
            procedure.record_outcome(true);
        }

        // Mean = 5 / (5 + 1) = 5/6 ≈ 0.833
        let mean = procedure.expected_success_rate();
        assert!(mean >= 0.3, "Mean {} should be >= 0.3", mean);
        assert_ne!(procedure.confidence_level(), ConfidenceLevel::Deprecated);
    }

    #[tokio::test]
    async fn test_find_by_context_deprioritizes_low_confidence() {
        // Procedures with confidence < 0.3 should be excluded from retrieval
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create a low-confidence procedure (mean < 0.3)
        // Beta(1, 5) -> mean = 1/6 ≈ 0.167 (< 0.3 = Deprecated)
        let mut low_confidence = Procedure::new(
            "low_confidence".to_string(),
            "A low confidence procedure".to_string(),
            "test context".to_string(),
        );
        for _ in 0..4 {
            low_confidence.record_outcome(false);
        }

        // Create a normal-confidence procedure (mean >= 0.3)
        let normal_confidence = Procedure::new(
            "normal_confidence".to_string(),
            "A normal confidence procedure".to_string(),
            "test context".to_string(),
        );

        store.store_procedure(low_confidence.clone()).await.unwrap();
        store
            .store_procedure(normal_confidence.clone())
            .await
            .unwrap();

        // Query should NOT return the low-confidence procedure
        let query = ProcedureQuery::new().with_keywords("test".to_string());
        let results = store.find_by_context(query).await.unwrap();

        // Low confidence procedure should be deprioritized (excluded)
        let found_low_confidence = results.iter().any(|r| r.procedure.name == "low_confidence");
        assert!(
            !found_low_confidence,
            "Low confidence procedure should be excluded from results"
        );

        // Normal confidence procedure should be found
        let found_normal = results
            .iter()
            .any(|r| r.procedure.name == "normal_confidence");
        assert!(
            found_normal,
            "Normal confidence procedure should be in results"
        );
    }

    #[tokio::test]
    async fn test_find_by_context_includes_uncertain_confidence() {
        // Procedures with confidence 0.3-0.6 (Uncertain) should be included
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create a new procedure (starts with Beta(1,1) -> mean = 0.5 = Uncertain)
        let uncertain_procedure = Procedure::new(
            "uncertain_procedure".to_string(),
            "An uncertain procedure".to_string(),
            "test context".to_string(),
        );

        store
            .store_procedure(uncertain_procedure.clone())
            .await
            .unwrap();

        // Query should return the uncertain procedure
        let query = ProcedureQuery::new().with_keywords("test".to_string());
        let results = store.find_by_context(query).await.unwrap();

        // Uncertain procedure should be included (mean 0.5 >= 0.3)
        let found = results
            .iter()
            .any(|r| r.procedure.name == "uncertain_procedure");
        assert!(found, "Uncertain procedure (mean 0.5) should be in results");
    }

    #[tokio::test]
    async fn test_procedure_retrieval_ranking_by_confidence() {
        // Higher confidence procedures should rank higher when keyword matches are equal
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create a high confidence procedure (Beta(51, 1) -> mean ≈ 0.98)
        let mut high_confidence = Procedure::new(
            "high_conf".to_string(),
            "High confidence".to_string(),
            "rust fix".to_string(),
        );
        for _ in 0..50 {
            high_confidence.record_outcome(true);
        }

        // Create a medium confidence procedure (Beta(3, 2) -> mean = 0.6)
        let mut medium_confidence = Procedure::new(
            "medium_conf".to_string(),
            "Medium confidence".to_string(),
            "rust fix".to_string(),
        );
        medium_confidence.record_outcome(true);
        medium_confidence.record_outcome(true);
        medium_confidence.record_outcome(true);
        medium_confidence.record_outcome(false);
        medium_confidence.record_outcome(false);

        store
            .store_procedure(high_confidence.clone())
            .await
            .unwrap();
        store
            .store_procedure(medium_confidence.clone())
            .await
            .unwrap();

        // Query for "rust" - both match equally, but high confidence should rank higher
        let query = ProcedureQuery::new().with_keywords("rust".to_string());
        let results = store.find_by_context(query).await.unwrap();

        assert_eq!(results.len(), 2);
        // High confidence should be first
        assert_eq!(results[0].procedure.name, "high_conf");
        assert_eq!(results[1].procedure.name, "medium_conf");
    }

    #[tokio::test]
    async fn test_record_outcome_updates_alpha_beta() {
        // Verify that record_outcome properly updates alpha (success) and beta (failure)
        let store = SqliteProceduralStore::create("sqlite::memory:")
            .await
            .unwrap();

        let procedure = Procedure::new(
            "test_procedure".to_string(),
            "A test procedure".to_string(),
            "test context".to_string(),
        );

        let id = store.store_procedure(procedure.clone()).await.unwrap();

        // Record outcomes: success, success, failure
        store.record_outcome(id, true).await.unwrap();
        store.record_outcome(id, true).await.unwrap();
        store.record_outcome(id, false).await.unwrap();

        // Retrieve and verify
        let updated = store.get_procedure(id).await.unwrap().unwrap();

        // α = 1 + 2 = 3 (2 successes)
        assert_eq!(updated.effectiveness.alpha, 3.0);
        assert_eq!(updated.effectiveness.successes, 2);

        // β = 1 + 1 = 2 (1 failure)
        assert_eq!(updated.effectiveness.beta, 2.0);
        assert_eq!(updated.effectiveness.failures, 1);

        // Mean = 3 / (3 + 2) = 0.6
        let mean = updated.expected_success_rate();
        assert!(
            (mean - 0.6).abs() < 0.001,
            "Expected mean 0.6, got {}",
            mean
        );

        // Usage count should be 3
        assert_eq!(updated.usage_count, 3);
    }
}
