// pattern_learning.rs - Pattern learning from PR feedback and rejection analysis
//
// This module provides functionality to analyze failed/rejected task executions
// and extract anti-patterns (what NOT to do) as conventions stored in memory blocks.
//
// Unlike skill_extraction which learns from SUCCESSFUL trajectories (what TO do),
// pattern_learning learns from REJECTED trajectories (what NOT to do).

use crate::{SqliteMemoryStore, SwellError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use swell_core::MemoryStore;
use uuid::Uuid;

/// Represents data about a rejection/failure from validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionData {
    pub task_id: Uuid,
    pub task_description: String,
    pub rejection_reason: RejectionReason,
    pub validation_errors: Vec<ValidationError>,
    pub attempted_steps: Vec<RejectedStep>,
    pub files_modified: Vec<String>,
    pub tool_calls: Vec<RejectedToolCall>,
    pub iteration_count: u32,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl RejectionData {
    /// Create new rejection data
    pub fn new(task_id: Uuid, task_description: String, rejection_reason: RejectionReason) -> Self {
        Self {
            task_id,
            task_description,
            rejection_reason,
            validation_errors: Vec::new(),
            attempted_steps: Vec::new(),
            files_modified: Vec::new(),
            tool_calls: Vec::new(),
            iteration_count: 0,
            timestamp: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    /// Add a validation error
    pub fn add_validation_error(&mut self, error: ValidationError) {
        self.validation_errors.push(error);
    }

    /// Add an attempted step
    pub fn add_attempted_step(&mut self, step: RejectedStep) {
        self.attempted_steps.push(step);
    }

    /// Add a tool call
    pub fn add_tool_call(&mut self, tool_call: RejectedToolCall) {
        self.tool_calls.push(tool_call);
    }
}

/// Reason for task rejection
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    ValidationFailure,
    LintFailure,
    TestFailure,
    SecurityIssue,
    AiReviewFailure,
    PolicyViolation,
    Timeout,
    ResourceExceeded,
    Unknown,
}

impl RejectionReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectionReason::ValidationFailure => "validation_failure",
            RejectionReason::LintFailure => "lint_failure",
            RejectionReason::TestFailure => "test_failure",
            RejectionReason::SecurityIssue => "security_issue",
            RejectionReason::AiReviewFailure => "ai_review_failure",
            RejectionReason::PolicyViolation => "policy_violation",
            RejectionReason::Timeout => "timeout",
            RejectionReason::ResourceExceeded => "resource_exceeded",
            RejectionReason::Unknown => "unknown",
        }
    }
}

/// A validation error that caused rejection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub error_type: ValidationErrorType,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorType {
    SyntaxError,
    TypeError,
    MissingImport,
    TestFailed,
    LintWarning,
    SecurityVulnerability,
    FormattingError,
    ImportError,
    UndefinedReference,
    TypeMismatch,
    Other,
}

impl ValidationErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationErrorType::SyntaxError => "syntax_error",
            ValidationErrorType::TypeError => "type_error",
            ValidationErrorType::MissingImport => "missing_import",
            ValidationErrorType::TestFailed => "test_failed",
            ValidationErrorType::LintWarning => "lint_warning",
            ValidationErrorType::SecurityVulnerability => "security_vulnerability",
            ValidationErrorType::FormattingError => "formatting_error",
            ValidationErrorType::ImportError => "import_error",
            ValidationErrorType::UndefinedReference => "undefined_reference",
            ValidationErrorType::TypeMismatch => "type_mismatch",
            ValidationErrorType::Other => "other",
        }
    }
}

/// A step that was attempted but led to rejection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedStep {
    pub step_id: Uuid,
    pub description: String,
    pub affected_files: Vec<String>,
    pub risk_level: String,
    pub error: Option<String>,
}

/// A tool call that was part of a rejected execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub result: String,
    pub was_successful: bool,
    pub error: Option<String>,
}

/// An extracted anti-pattern - something that should NOT be done
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiPattern {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub pattern_type: AntiPatternType,
    pub examples: Vec<AntiPatternExample>,
    pub why_anti: String,
    pub frequency: usize,
    pub confidence: f64,
    /// Number of times this pattern has been confirmed (seen in rejections)
    pub confirmation_count: u32,
    /// Whether this pattern has been promoted to higher retrieval rank
    pub is_promoted: bool,
    pub rejection_reasons: Vec<RejectionReason>,
    pub conventions: Vec<String>,
    pub source_task_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

/// Promotion thresholds for patterns and rules
#[derive(Debug, Clone, Copy)]
pub struct PromotionThresholds {
    /// Pattern promotion: minimum confirmations required
    pub pattern_min_confirmations: u32,
    /// Pattern promotion: minimum confidence required
    pub pattern_min_confidence: f64,
    /// Rule promotion: minimum confirmations required
    pub rule_min_confirmations: u32,
    /// Rule promotion: minimum confidence required
    pub rule_min_confidence: f64,
    /// Boost multiplier for promoted patterns in retrieval
    pub promotion_rank_boost: f64,
}

impl Default for PromotionThresholds {
    fn default() -> Self {
        Self {
            // Pattern promotion: ≥5 confirmations with confidence >0.6
            pattern_min_confirmations: 5,
            pattern_min_confidence: 0.6,
            // Rule promotion: ≥10 confirmations with confidence >0.8
            rule_min_confirmations: 10,
            rule_min_confidence: 0.8,
            // Promoted patterns get 50% boost in retrieval rank
            promotion_rank_boost: 1.5,
        }
    }
}

/// Promotion status for a pattern or rule
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionStatus {
    /// Not yet eligible for promotion
    NotEligible,
    /// Eligible but not yet promoted
    Eligible,
    /// Promoted to higher retrieval rank
    Promoted,
}

impl std::fmt::Display for PromotionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromotionStatus::NotEligible => write!(f, "not_eligible"),
            PromotionStatus::Eligible => write!(f, "eligible"),
            PromotionStatus::Promoted => write!(f, "promoted"),
        }
    }
}

impl AntiPattern {
    /// Create a new anti-pattern
    pub fn new(name: String, pattern_type: AntiPatternType, why_anti: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: String::new(),
            pattern_type,
            examples: Vec::new(),
            why_anti,
            frequency: 0,
            confidence: 0.0,
            confirmation_count: 0,
            is_promoted: false,
            rejection_reasons: Vec::new(),
            conventions: Vec::new(),
            source_task_id: Uuid::nil(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
        }
    }

    /// Add an example of this anti-pattern
    pub fn add_example(&mut self, example: AntiPatternExample) {
        self.examples.push(example);
        self.frequency = self.examples.len();
        self.confirmation_count += 1;
        self.updated_at = Utc::now();
    }

    /// Add a convention (what to do instead)
    pub fn add_convention(&mut self, convention: String) {
        if !self.conventions.contains(&convention) {
            self.conventions.push(convention);
        }
    }

    /// Update confidence based on frequency
    pub fn update_confidence(&mut self, total_rejections: usize) {
        // Confidence increases with frequency, capped at 0.95
        self.confidence = (self.frequency as f64 / total_rejections as f64).min(0.95);
        self.updated_at = Utc::now();
        // Check if promotion thresholds are met
        self.check_promotion();
    }

    /// Check if this pattern meets promotion thresholds and update is_promoted accordingly.
    /// Pattern promotion: ≥5 confirmations with confidence >0.6
    pub fn check_promotion(&mut self) {
        let thresholds = PromotionThresholds::default();
        let should_promote = self.confirmation_count >= thresholds.pattern_min_confirmations
            && self.confidence > thresholds.pattern_min_confidence;

        if should_promote && !self.is_promoted {
            self.is_promoted = true;
            self.updated_at = Utc::now();
        }
    }

    /// Get the current promotion status
    pub fn get_promotion_status(&self) -> PromotionStatus {
        let thresholds = PromotionThresholds::default();
        if self.is_promoted {
            PromotionStatus::Promoted
        } else if self.confirmation_count >= thresholds.pattern_min_confirmations
            && self.confidence > thresholds.pattern_min_confidence
        {
            // Eligible but not yet promoted (edge case - should be auto-promoted)
            PromotionStatus::Eligible
        } else {
            PromotionStatus::NotEligible
        }
    }

    /// Get the retrieval rank boost factor for this pattern.
    /// Promoted patterns get a higher rank boost in retrieval results.
    pub fn get_retrieval_rank_boost(&self, base_score: f64) -> f64 {
        let thresholds = PromotionThresholds::default();
        if self.is_promoted {
            base_score * thresholds.promotion_rank_boost
        } else {
            base_score
        }
    }

    /// Record a confirmation (observed another instance of this anti-pattern)
    pub fn record_confirmation(&mut self) {
        self.confirmation_count += 1;
        self.updated_at = Utc::now();
        self.check_promotion();
    }

    /// Check if this pattern meets promotion criteria
    pub fn meets_promotion_criteria(&self) -> bool {
        let thresholds = PromotionThresholds::default();
        self.confirmation_count >= thresholds.pattern_min_confirmations
            && self.confidence > thresholds.pattern_min_confidence
    }
}

/// Types of anti-patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiPatternType {
    FileOperation,
    GitOperation,
    ShellCommand,
    TestPattern,
    ValidationPattern,
    CodeTransform,
    NamingConvention,
    ImportPattern,
    ErrorHandling,
    TypeUsage,
}

impl AntiPatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AntiPatternType::FileOperation => "file_operation",
            AntiPatternType::GitOperation => "git_operation",
            AntiPatternType::ShellCommand => "shell_command",
            AntiPatternType::TestPattern => "test_pattern",
            AntiPatternType::ValidationPattern => "validation_pattern",
            AntiPatternType::CodeTransform => "code_transform",
            AntiPatternType::NamingConvention => "naming_convention",
            AntiPatternType::ImportPattern => "import_pattern",
            AntiPatternType::ErrorHandling => "error_handling",
            AntiPatternType::TypeUsage => "type_usage",
        }
    }
}

/// An example of an anti-pattern with context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiPatternExample {
    pub task_id: Uuid,
    pub before_code: Option<String>,
    pub after_code: Option<String>,
    pub error_message: String,
    pub rejection_reason: RejectionReason,
    pub timestamp: DateTime<Utc>,
}

/// Conventions extracted from rejection analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Convention {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub convention_type: ConventionType,
    pub pattern: String,
    pub examples: Vec<String>,
    pub source: ConventionSource,
    pub confidence: f64,
    /// Number of times this convention has been confirmed
    pub confirmation_count: u32,
    /// Whether this convention has been promoted to higher retrieval rank
    pub is_promoted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConventionType {
    Naming,
    Import,
    ErrorHandling,
    Testing,
    Documentation,
    Formatting,
    TypeUsage,
    GitCommit,
    CodeStructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConventionSource {
    FromAntiPattern,
    FromSuccessfulTask,
    FromOperatorFeedback,
}

impl Convention {
    /// Create a new convention
    pub fn new(name: String, convention_type: ConventionType, pattern: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: String::new(),
            convention_type,
            pattern,
            examples: Vec::new(),
            source: ConventionSource::FromAntiPattern,
            confidence: 0.0,
            confirmation_count: 0,
            is_promoted: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update confidence based on evidence
    pub fn update_confidence(&mut self, total_observations: usize) {
        // Confidence increases with confirmation rate, capped at 0.95
        self.confidence = (self.confirmation_count as f64 / total_observations as f64).min(0.95);
        self.updated_at = Utc::now();
        // Check if promotion thresholds are met
        self.check_promotion();
    }

    /// Check if this convention meets promotion thresholds and update is_promoted accordingly.
    /// Rule promotion: ≥10 confirmations with confidence >0.8
    pub fn check_promotion(&mut self) {
        let thresholds = PromotionThresholds::default();
        let should_promote = self.confirmation_count >= thresholds.rule_min_confirmations
            && self.confidence > thresholds.rule_min_confidence;

        if should_promote && !self.is_promoted {
            self.is_promoted = true;
            self.updated_at = Utc::now();
        }
    }

    /// Get the current promotion status
    pub fn get_promotion_status(&self) -> PromotionStatus {
        let thresholds = PromotionThresholds::default();
        if self.is_promoted {
            PromotionStatus::Promoted
        } else if self.confirmation_count >= thresholds.rule_min_confirmations
            && self.confidence > thresholds.rule_min_confidence
        {
            PromotionStatus::Eligible
        } else {
            PromotionStatus::NotEligible
        }
    }

    /// Get the retrieval rank boost factor for this convention.
    /// Promoted conventions get a higher rank boost in retrieval results.
    pub fn get_retrieval_rank_boost(&self, base_score: f64) -> f64 {
        let thresholds = PromotionThresholds::default();
        if self.is_promoted {
            base_score * thresholds.promotion_rank_boost
        } else {
            base_score
        }
    }

    /// Record a confirmation (observed another instance of this convention)
    pub fn record_confirmation(&mut self) {
        self.confirmation_count += 1;
        self.updated_at = Utc::now();
        self.check_promotion();
    }

    /// Check if this convention meets promotion criteria
    pub fn meets_promotion_criteria(&self) -> bool {
        let thresholds = PromotionThresholds::default();
        self.confirmation_count >= thresholds.rule_min_confirmations
            && self.confidence > thresholds.rule_min_confidence
    }
}

/// Configuration for pattern learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternLearningConfig {
    pub min_confidence_threshold: f64,
    pub max_anti_patterns_per_rejection: usize,
    pub store_conventions_as_blocks: bool,
    pub convention_block_label: String,
}

impl Default for PatternLearningConfig {
    fn default() -> Self {
        Self {
            min_confidence_threshold: 0.4,
            max_anti_patterns_per_rejection: 10,
            store_conventions_as_blocks: true,
            convention_block_label: "project:conventions".to_string(),
        }
    }
}

/// Result of pattern learning analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternLearningResult {
    pub anti_patterns_extracted: usize,
    pub conventions_extracted: usize,
    pub anti_patterns: Vec<AntiPattern>,
    pub conventions: Vec<Convention>,
    pub memory_blocks_updated: usize,
    pub errors: Vec<String>,
}

/// Analyzer for extracting anti-patterns from rejection data
pub struct PatternLearningAnalyzer {
    config: PatternLearningConfig,
}

impl PatternLearningAnalyzer {
    pub fn new(_store: SqliteMemoryStore, config: PatternLearningConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config(_store: SqliteMemoryStore) -> Self {
        Self {
            config: PatternLearningConfig::default(),
        }
    }

    /// Analyze rejection data and extract anti-patterns
    pub async fn analyze(
        &self,
        rejection: RejectionData,
    ) -> Result<PatternLearningResult, SwellError> {
        let mut anti_patterns = Vec::new();
        let mut conventions = Vec::new();
        let mut errors = Vec::new();

        // Extract anti-patterns based on validation errors
        for error in &rejection.validation_errors {
            match self.extract_error_anti_patterns(error, &rejection) {
                Ok((patterns, convs)) => {
                    anti_patterns.extend(patterns);
                    conventions.extend(convs);
                }
                Err(e) => errors.push(format!("Error extracting patterns: {}", e)),
            }
        }

        // Extract anti-patterns from tool call failures
        for tool_call in &rejection.tool_calls {
            if !tool_call.was_successful {
                if let Ok(patterns) = self.extract_tool_anti_patterns(tool_call, &rejection) {
                    anti_patterns.extend(patterns);
                }
            }
        }

        // Extract naming anti-patterns from file modifications
        let naming_patterns = self.extract_naming_anti_patterns(&rejection);
        anti_patterns.extend(naming_patterns);

        // Deduplicate and filter by confidence
        anti_patterns = self.deduplicate_anti_patterns(anti_patterns);
        anti_patterns.retain(|ap| ap.confidence >= self.config.min_confidence_threshold);
        anti_patterns.truncate(self.config.max_anti_patterns_per_rejection);

        // Generate conventions from anti-patterns
        for anti_pattern in &anti_patterns {
            if let Some(conv) = self.anti_pattern_to_convention(anti_pattern) {
                conventions.push(conv);
            }
        }

        // Deduplicate conventions
        conventions = self.deduplicate_conventions(conventions);

        Ok(PatternLearningResult {
            anti_patterns_extracted: anti_patterns.len(),
            conventions_extracted: conventions.len(),
            anti_patterns,
            conventions,
            memory_blocks_updated: 0,
            errors,
        })
    }

    /// Extract anti-patterns from validation errors
    fn extract_error_anti_patterns(
        &self,
        error: &ValidationError,
        rejection: &RejectionData,
    ) -> Result<(Vec<AntiPattern>, Vec<Convention>), SwellError> {
        let mut patterns = Vec::new();
        let mut conventions = Vec::new();

        match error.error_type {
            ValidationErrorType::SyntaxError => {
                let mut ap = AntiPattern::new(
                    "syntax_error".to_string(),
                    AntiPatternType::CodeTransform,
                    "Syntax errors indicate code that cannot be parsed".to_string(),
                );
                ap.description = "Code with syntax errors was submitted".to_string();
                if error.file.is_some() {
                    ap.add_example(AntiPatternExample {
                        task_id: rejection.task_id,
                        before_code: None,
                        after_code: None,
                        error_message: error.message.clone(),
                        rejection_reason: RejectionReason::ValidationFailure,
                        timestamp: rejection.timestamp,
                    });
                }
                ap.confidence = 0.9;
                ap.rejection_reasons
                    .push(RejectionReason::ValidationFailure);
                ap.add_convention("Always run syntax validation before submitting".to_string());
                patterns.push(ap);
            }
            ValidationErrorType::TestFailed => {
                let mut ap = AntiPattern::new(
                    "test_failure".to_string(),
                    AntiPatternType::TestPattern,
                    "Tests must pass before code can be accepted".to_string(),
                );
                ap.description = "Tests failed after code changes".to_string();
                ap.confidence = 0.95;
                ap.rejection_reasons.push(RejectionReason::TestFailure);
                ap.add_convention("Run tests locally before submitting".to_string());
                ap.add_convention("Ensure all existing tests pass".to_string());
                patterns.push(ap);

                // Add specific convention
                conventions.push(Convention {
                    id: Uuid::new_v4(),
                    name: "test_before_submit".to_string(),
                    description: "Always run full test suite before submission".to_string(),
                    convention_type: ConventionType::Testing,
                    pattern: "cargo test".to_string(),
                    examples: vec!["cargo test --workspace".to_string()],
                    source: ConventionSource::FromAntiPattern,
                    confidence: 0.9,
                    confirmation_count: 0,
                    is_promoted: false,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }
            ValidationErrorType::LintWarning => {
                let mut ap = AntiPattern::new(
                    "lint_violation".to_string(),
                    AntiPatternType::ValidationPattern,
                    "Code should not have lint warnings".to_string(),
                );
                ap.description = "Lint warnings were detected".to_string();
                ap.confidence = 0.85;
                ap.rejection_reasons.push(RejectionReason::LintFailure);
                ap.add_convention("Run clippy and fix warnings before submitting".to_string());
                patterns.push(ap);

                conventions.push(Convention {
                    id: Uuid::new_v4(),
                    name: "lint_before_submit".to_string(),
                    description: "Run clippy and address all warnings".to_string(),
                    convention_type: ConventionType::Formatting,
                    pattern: "cargo clippy".to_string(),
                    examples: vec!["cargo clippy -- -D warnings".to_string()],
                    source: ConventionSource::FromAntiPattern,
                    confidence: 0.85,
                    confirmation_count: 0,
                    is_promoted: false,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }
            ValidationErrorType::MissingImport => {
                let mut ap = AntiPattern::new(
                    "missing_import".to_string(),
                    AntiPatternType::ImportPattern,
                    "All required imports must be present".to_string(),
                );
                ap.description = format!("Missing import: {}", error.message);
                ap.confidence = 0.9;
                ap.rejection_reasons
                    .push(RejectionReason::ValidationFailure);
                ap.add_convention("Check all imports are present and correct".to_string());
                patterns.push(ap);
            }
            ValidationErrorType::TypeError | ValidationErrorType::TypeMismatch => {
                let mut ap = AntiPattern::new(
                    "type_error".to_string(),
                    AntiPatternType::TypeUsage,
                    "Type errors indicate incorrect type usage".to_string(),
                );
                ap.description = error.message.clone();
                ap.confidence = 0.9;
                ap.rejection_reasons
                    .push(RejectionReason::ValidationFailure);
                ap.add_convention("Run type checker before submitting".to_string());
                patterns.push(ap);
            }
            ValidationErrorType::UndefinedReference => {
                let mut ap = AntiPattern::new(
                    "undefined_reference".to_string(),
                    AntiPatternType::ImportPattern,
                    "References to undefined names cause linker/compiler errors".to_string(),
                );
                ap.description = error.message.clone();
                ap.confidence = 0.9;
                ap.rejection_reasons
                    .push(RejectionReason::ValidationFailure);
                ap.add_convention(
                    "Ensure all referenced items are defined or imported".to_string(),
                );
                patterns.push(ap);
            }
            ValidationErrorType::FormattingError => {
                let mut ap = AntiPattern::new(
                    "formatting_error".to_string(),
                    AntiPatternType::NamingConvention,
                    "Code must be properly formatted".to_string(),
                );
                ap.description = "Code formatting does not match project standards".to_string();
                ap.confidence = 0.8;
                ap.rejection_reasons.push(RejectionReason::LintFailure);
                ap.add_convention("Run cargo fmt before submitting".to_string());
                patterns.push(ap);

                conventions.push(Convention {
                    id: Uuid::new_v4(),
                    name: "format_before_submit".to_string(),
                    description: "Format code with cargo fmt".to_string(),
                    convention_type: ConventionType::Formatting,
                    pattern: "cargo fmt".to_string(),
                    examples: vec!["cargo fmt".to_string()],
                    source: ConventionSource::FromAntiPattern,
                    confidence: 0.8,
                    confirmation_count: 0,
                    is_promoted: false,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }
            _ => {
                // Generic pattern for other errors
                let mut ap = AntiPattern::new(
                    format!("error_{}", error.error_type.as_str()),
                    AntiPatternType::CodeTransform,
                    format!("Error type {} should be avoided", error.error_type.as_str()),
                );
                ap.description = error.message.clone();
                ap.confidence = 0.5;
                ap.rejection_reasons
                    .push(RejectionReason::ValidationFailure);
                patterns.push(ap);
            }
        }

        Ok((patterns, conventions))
    }

    /// Extract anti-patterns from failed tool calls
    fn extract_tool_anti_patterns(
        &self,
        tool_call: &RejectedToolCall,
        rejection: &RejectionData,
    ) -> Result<Vec<AntiPattern>, SwellError> {
        let mut patterns = Vec::new();

        // Check for dangerous operations
        match tool_call.tool_name.as_str() {
            "shell" => {
                if let Some(err) = &tool_call.error {
                    if err.contains("permission denied") || err.contains("access denied") {
                        let mut ap = AntiPattern::new(
                            "shell_permission_error".to_string(),
                            AntiPatternType::ShellCommand,
                            "Shell commands requiring elevated permissions should be avoided"
                                .to_string(),
                        );
                        ap.confidence = 0.85;
                        ap.add_example(AntiPatternExample {
                            task_id: rejection.task_id,
                            before_code: None,
                            after_code: None,
                            error_message: err.clone(),
                            rejection_reason: RejectionReason::PolicyViolation,
                            timestamp: rejection.timestamp,
                        });
                        ap.add_convention("Use project-configured tool paths".to_string());
                        patterns.push(ap);
                    }
                }
            }
            "write_file" | "edit_file" => {
                // Check for dangerous file paths
                if let Some(path) = tool_call.arguments.get("path").and_then(|v| v.as_str()) {
                    if path.contains("/etc/") || path.contains("/usr/") || path.contains("/System/")
                    {
                        let mut ap = AntiPattern::new(
                            "system_file_modification".to_string(),
                            AntiPatternType::FileOperation,
                            "Modifying system files is prohibited".to_string(),
                        );
                        ap.confidence = 0.95;
                        ap.add_example(AntiPatternExample {
                            task_id: rejection.task_id,
                            before_code: None,
                            after_code: None,
                            error_message: format!("Attempted to modify system path: {}", path),
                            rejection_reason: RejectionReason::PolicyViolation,
                            timestamp: rejection.timestamp,
                        });
                        ap.add_convention(
                            "Only modify files within the project workspace".to_string(),
                        );
                        patterns.push(ap);
                    }
                }
            }
            _ => {}
        }

        Ok(patterns)
    }

    /// Extract naming anti-patterns from file modifications
    fn extract_naming_anti_patterns(&self, rejection: &RejectionData) -> Vec<AntiPattern> {
        let mut patterns = Vec::new();

        for file in &rejection.files_modified {
            let file_name = std::path::Path::new(file)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Check for snake_case vs PascalCase violations in Rust files
            if file.ends_with(".rs") {
                // If file has uppercase (not in path separators), it might violate snake_case
                let has_uppercase = file_name
                    .chars()
                    .filter(|c| c.is_alphabetic())
                    .any(|c| c.is_uppercase());

                if has_uppercase && !file_name.contains("://") {
                    // Allow PascalCase for certain patterns but flag others
                    if file_name.contains('_') && has_uppercase {
                        let mut ap = AntiPattern::new(
                            "mixed_naming_convention".to_string(),
                            AntiPatternType::NamingConvention,
                            "Rust files should use snake_case naming".to_string(),
                        );
                        ap.description =
                            format!("File '{}' mixes snake_case with PascalCase", file);
                        ap.confidence = 0.7;
                        ap.add_convention("Use snake_case for Rust source files".to_string());
                        ap.add_convention(
                            "Use PascalCase for module names (Rust convention)".to_string(),
                        );
                        patterns.push(ap);
                    }
                }
            }

            // Check for test file naming
            if file.contains("test") && (file.ends_with("_test.rs") || file.ends_with("tests.rs")) {
                let mut ap = AntiPattern::new(
                    "test_file_naming".to_string(),
                    AntiPatternType::TestPattern,
                    "Test files should follow project naming conventions".to_string(),
                );
                ap.description = format!("Test file '{}' may not follow naming conventions", file);
                ap.confidence = 0.5;
                patterns.push(ap);
            }
        }

        patterns
    }

    /// Convert an anti-pattern to a convention
    fn anti_pattern_to_convention(&self, anti_pattern: &AntiPattern) -> Option<Convention> {
        if anti_pattern.conventions.is_empty() {
            return None;
        }

        let convention_text = anti_pattern.conventions.first()?.clone();

        Some(Convention {
            id: Uuid::new_v4(),
            name: format!("do_{}", anti_pattern.name.replace("anti_pattern", "")),
            description: format!("Instead of {}: {}", anti_pattern.name, convention_text),
            convention_type: match anti_pattern.pattern_type {
                AntiPatternType::FileOperation => ConventionType::CodeStructure,
                AntiPatternType::TestPattern => ConventionType::Testing,
                AntiPatternType::NamingConvention => ConventionType::Naming,
                AntiPatternType::ImportPattern => ConventionType::Import,
                AntiPatternType::ValidationPattern => ConventionType::Formatting,
                AntiPatternType::ShellCommand => ConventionType::CodeStructure,
                _ => ConventionType::CodeStructure,
            },
            pattern: convention_text,
            examples: anti_pattern
                .examples
                .iter()
                .map(|e| e.error_message.clone())
                .collect(),
            source: ConventionSource::FromAntiPattern,
            confidence: anti_pattern.confidence,
            confirmation_count: 0,
            is_promoted: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Deduplicate anti-patterns by name and type
    fn deduplicate_anti_patterns(&self, patterns: Vec<AntiPattern>) -> Vec<AntiPattern> {
        let mut seen: HashSet<(AntiPatternType, String)> = HashSet::new();
        let mut result: Vec<AntiPattern> = Vec::new();
        let mut frequency_map: HashMap<String, usize> = HashMap::new();

        // First pass: aggregate frequencies
        for pattern in &patterns {
            *frequency_map.entry(pattern.name.clone()).or_insert(0) += 1;
        }

        // Second pass: build final patterns
        for pattern in patterns {
            let key = (pattern.pattern_type, pattern.name.clone());
            if seen.insert(key) {
                let mut final_pattern = pattern;
                final_pattern.frequency = *frequency_map.get(&final_pattern.name).unwrap_or(&1);
                final_pattern.update_confidence(frequency_map.values().sum());
                result.push(final_pattern);
            }
        }

        result
    }

    /// Deduplicate conventions by name
    fn deduplicate_conventions(&self, conventions: Vec<Convention>) -> Vec<Convention> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut result = Vec::new();

        for conv in conventions {
            if seen.insert(conv.name.clone()) {
                result.push(conv);
            }
        }

        result
    }
}

/// Service for storing and managing learned patterns
pub struct PatternLearningService {
    store: SqliteMemoryStore,
    config: PatternLearningConfig,
    repository: String,
}

impl PatternLearningService {
    pub fn new(
        store: SqliteMemoryStore,
        config: PatternLearningConfig,
        repository: String,
    ) -> Self {
        Self {
            store,
            config,
            repository,
        }
    }

    pub fn with_default_config(store: SqliteMemoryStore, repository: String) -> Self {
        Self {
            store,
            config: PatternLearningConfig::default(),
            repository,
        }
    }

    /// Learn from a rejection event and update memory blocks
    pub async fn learn_from_rejection(
        &self,
        rejection: RejectionData,
    ) -> Result<PatternLearningResult, SwellError> {
        let analyzer = PatternLearningAnalyzer::new(self.store.clone(), self.config.clone());
        let mut result = analyzer.analyze(rejection).await?;

        if self.config.store_conventions_as_blocks {
            match self.update_memory_blocks(&result.conventions).await {
                Ok(count) => result.memory_blocks_updated = count,
                Err(e) => result
                    .errors
                    .push(format!("Failed to update memory blocks: {}", e)),
            }
        }

        Ok(result)
    }

    /// Update memory blocks with extracted conventions
    async fn update_memory_blocks(&self, conventions: &[Convention]) -> Result<usize, SwellError> {
        if conventions.is_empty() {
            return Ok(0);
        }

        let mut _updated_count = 0;

        // Build convention content for the memory block
        let mut content = String::from("# Project Conventions\n\n");
        content.push_str("## Extracted Conventions\n\n");

        let mut by_type: HashMap<ConventionType, Vec<&Convention>> = HashMap::new();
        for conv in conventions {
            by_type.entry(conv.convention_type).or_default().push(conv);
        }

        for (conv_type, convs) in by_type {
            content.push_str(&format!("### {:?}\n\n", conv_type));
            for conv in convs {
                content.push_str(&format!("- **{}**: {}\n", conv.name, conv.description));
                if !conv.examples.is_empty() {
                    content.push_str(&format!("  - Example: `{}`\n", conv.examples[0]));
                }
                content.push('\n');
            }
        }

        // Also add anti-patterns section
        content.push_str("## Anti-Patterns to Avoid\n\n");
        content.push_str("Based on rejection analysis:\n\n");

        // We need to get anti-patterns separately - for now, just use convention names
        for conv in conventions {
            if conv.source == ConventionSource::FromAntiPattern {
                content.push_str(&format!("- Do not use patterns like: {}\n", conv.name));
            }
        }

        // Try to find existing conventions block and update it, or create new one
        let existing = self
            .store
            .get_by_label(
                self.config.convention_block_label.clone(),
                self.repository.clone(),
            )
            .await?;

        if let Some(mut entry) = existing.into_iter().next() {
            // Update existing entry
            entry.content = content;
            entry.updated_at = chrono::Utc::now();
            entry.metadata = serde_json::json!({
                "last_updated": chrono::Utc::now().to_rfc3339(),
                "conventions_count": conventions.len(),
            });
            self.store.update(entry).await?;
            _updated_count = 1;
        } else {
            // Create new entry
            let entry = crate::MemoryEntry {
                id: Uuid::new_v4(),
                block_type: swell_core::MemoryBlockType::Convention,
                label: self.config.convention_block_label.clone(),
                content,
                embedding: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata: serde_json::json!({
                    "last_updated": chrono::Utc::now().to_rfc3339(),
                    "conventions_count": conventions.len(),
                }),
                // Full scope hierarchy
                org: String::new(),
                workspace: String::new(),
                repository: self.repository.clone(),
                language: None,
                framework: None,
                environment: None,
                task_type: None,
                session_id: None,
                last_reinforcement: Some(chrono::Utc::now()),
                is_stale: false,
                source_episode_id: None,
                evidence: None,
                provenance_context: None,
            };
            self.store.store(entry).await?;
            _updated_count = 1;
        }

        Ok(_updated_count)
    }

    /// Get all conventions from memory blocks
    pub async fn get_conventions(&self) -> Result<Vec<Convention>, SwellError> {
        let entries = self
            .store
            .get_by_label(
                self.config.convention_block_label.clone(),
                self.repository.clone(),
            )
            .await?;

        let mut conventions = Vec::new();

        for entry in entries {
            // Parse conventions from content (simplified parsing)
            let parsed = self.parse_conventions_from_content(&entry.content);
            conventions.extend(parsed);
        }

        Ok(conventions)
    }

    /// Parse conventions from memory block content
    fn parse_conventions_from_content(&self, content: &str) -> Vec<Convention> {
        let mut conventions = Vec::new();
        let mut current_type: Option<ConventionType> = None;

        for line in content.lines() {
            let trimmed = line.trim();

            // Check for section headers
            if trimmed.starts_with("### ") {
                let type_str = trimmed.trim_start_matches("### ").trim();
                current_type = match type_str {
                    "Naming" => Some(ConventionType::Naming),
                    "Import" => Some(ConventionType::Import),
                    "ErrorHandling" => Some(ConventionType::ErrorHandling),
                    "Testing" => Some(ConventionType::Testing),
                    "Documentation" => Some(ConventionType::Documentation),
                    "Formatting" => Some(ConventionType::Formatting),
                    "TypeUsage" => Some(ConventionType::TypeUsage),
                    "GitCommit" => Some(ConventionType::GitCommit),
                    "CodeStructure" => Some(ConventionType::CodeStructure),
                    _ => current_type,
                }
            } else if trimmed.starts_with("- **") && current_type.is_some() {
                // Parse convention line: - **name**: description
                if let Some(colon_pos) = trimmed.find("**:") {
                    let name = trimmed[3..colon_pos].trim().to_string();
                    let description = trimmed[colon_pos + 2..].trim().to_string();

                    conventions.push(Convention {
                        id: Uuid::new_v4(),
                        name,
                        description,
                        convention_type: current_type.unwrap(),
                        pattern: String::new(),
                        examples: Vec::new(),
                        source: ConventionSource::FromOperatorFeedback,
                        confidence: 0.5,
                        confirmation_count: 0,
                        is_promoted: false,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }
        }

        conventions
    }

    /// Get anti-patterns related to a specific validation error type
    pub async fn get_anti_patterns_for_error(
        &self,
        error_type: ValidationErrorType,
    ) -> Result<Vec<AntiPattern>, SwellError> {
        // For MVP, return empty - in full implementation would query stored anti-patterns
        // This would require storing anti-patterns separately
        let _ = error_type;
        Ok(Vec::new())
    }
}

/// Create rejection data from a failed task
pub fn create_rejection_data(
    task_id: Uuid,
    task_description: String,
    rejection_reason: RejectionReason,
    validation_errors: Vec<ValidationError>,
) -> RejectionData {
    let mut rejection = RejectionData::new(task_id, task_description, rejection_reason);
    for error in validation_errors {
        rejection.add_validation_error(error);
    }
    rejection
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rejection_data_creation() {
        let task_id = Uuid::new_v4();
        let mut rejection = RejectionData::new(
            task_id,
            "Fix authentication bug".to_string(),
            RejectionReason::TestFailure,
        );

        rejection.add_validation_error(ValidationError {
            error_type: ValidationErrorType::TestFailed,
            message: "Test auth::login failed".to_string(),
            file: Some("src/auth.rs".to_string()),
            line: Some(42),
            column: None,
        });

        assert_eq!(rejection.task_id, task_id);
        assert_eq!(rejection.validation_errors.len(), 1);
        assert_eq!(rejection.rejection_reason, RejectionReason::TestFailure);
    }

    #[tokio::test]
    async fn test_anti_pattern_creation() {
        let mut ap = AntiPattern::new(
            "test_failure".to_string(),
            AntiPatternType::TestPattern,
            "Tests must pass before code can be accepted".to_string(),
        );

        ap.add_example(AntiPatternExample {
            task_id: Uuid::new_v4(),
            before_code: None,
            after_code: None,
            error_message: "Test failed: expected 200, got 404".to_string(),
            rejection_reason: RejectionReason::TestFailure,
            timestamp: Utc::now(),
        });

        ap.add_convention("Run tests locally before submitting".to_string());

        assert_eq!(ap.name, "test_failure");
        assert_eq!(ap.frequency, 1);
        assert!(!ap.conventions.is_empty());
    }

    #[tokio::test]
    async fn test_pattern_learning_config_default() {
        let config = PatternLearningConfig::default();
        assert_eq!(config.min_confidence_threshold, 0.4);
        assert_eq!(config.max_anti_patterns_per_rejection, 10);
        assert!(config.store_conventions_as_blocks);
    }

    #[tokio::test]
    async fn test_pattern_learning_analyzer() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let analyzer = PatternLearningAnalyzer::with_default_config(store);

        let mut rejection = RejectionData::new(
            Uuid::new_v4(),
            "Add new feature".to_string(),
            RejectionReason::TestFailure,
        );

        rejection.add_validation_error(ValidationError {
            error_type: ValidationErrorType::TestFailed,
            message: "Test failed: assertion failed".to_string(),
            file: Some("src/main.rs".to_string()),
            line: Some(10),
            column: None,
        });

        let result = analyzer.analyze(rejection).await.unwrap();
        assert!(result.anti_patterns_extracted > 0);
        assert!(result.conventions_extracted > 0 || result.anti_patterns.len() > 0);
    }

    #[tokio::test]
    async fn test_convention_type_conversion() {
        let conv_type = ConventionType::Testing;
        let json = serde_json::to_string(&conv_type).unwrap();
        assert_eq!(json, "\"testing\"");

        let deserialized: ConventionType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConventionType::Testing);
    }

    #[tokio::test]
    async fn test_rejection_reason_conversion() {
        let reason = RejectionReason::LintFailure;
        assert_eq!(reason.as_str(), "lint_failure");

        let mut rejection = RejectionData::new(
            Uuid::new_v4(),
            "test".to_string(),
            RejectionReason::LintFailure,
        );
        rejection.add_validation_error(ValidationError {
            error_type: ValidationErrorType::LintWarning,
            message: "warning: unused variable".to_string(),
            file: None,
            line: None,
            column: None,
        });

        assert_eq!(
            rejection.validation_errors[0].error_type,
            ValidationErrorType::LintWarning
        );
    }

    #[tokio::test]
    async fn test_validation_error_types() {
        let error_types = vec![
            (ValidationErrorType::SyntaxError, "syntax_error"),
            (ValidationErrorType::TypeError, "type_error"),
            (ValidationErrorType::MissingImport, "missing_import"),
            (ValidationErrorType::TestFailed, "test_failed"),
            (
                ValidationErrorType::SecurityVulnerability,
                "security_vulnerability",
            ),
        ];

        for (error_type, expected_str) in error_types {
            let json = serde_json::to_string(&error_type).unwrap();
            assert!(
                json.contains(expected_str),
                "Expected {} in {}",
                expected_str,
                json
            );
        }
    }

    #[tokio::test]
    async fn test_pattern_learning_result() {
        let result = PatternLearningResult {
            anti_patterns_extracted: 3,
            conventions_extracted: 2,
            anti_patterns: Vec::new(),
            conventions: Vec::new(),
            memory_blocks_updated: 1,
            errors: vec![],
        };

        assert_eq!(result.anti_patterns_extracted, 3);
        assert_eq!(result.conventions_extracted, 2);
        assert_eq!(result.memory_blocks_updated, 1);
    }

    #[tokio::test]
    async fn test_anti_pattern_type_as_str() {
        let types = vec![
            (AntiPatternType::FileOperation, "file_operation"),
            (AntiPatternType::NamingConvention, "naming_convention"),
            (AntiPatternType::ErrorHandling, "error_handling"),
        ];

        for (pattern_type, expected) in types {
            assert_eq!(pattern_type.as_str(), expected);
        }
    }

    #[tokio::test]
    async fn test_convention_source_serialization() {
        let source = ConventionSource::FromAntiPattern;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, "\"from_anti_pattern\"");

        let source2 = ConventionSource::FromOperatorFeedback;
        let json2 = serde_json::to_string(&source2).unwrap();
        assert_eq!(json2, "\"from_operator_feedback\"");
    }

    #[tokio::test]
    async fn test_pattern_learning_service_conventions() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let service = PatternLearningService::with_default_config(store, "test-repo".to_string());

        // Should return empty vec when no conventions stored yet
        let conventions = service.get_conventions().await.unwrap();
        assert!(conventions.is_empty());
    }

    #[tokio::test]
    async fn test_create_rejection_data() {
        let task_id = Uuid::new_v4();
        let errors = vec![ValidationError {
            error_type: ValidationErrorType::TestFailed,
            message: "Failed".to_string(),
            file: None,
            line: None,
            column: None,
        }];

        let rejection = create_rejection_data(
            task_id,
            "Implement feature".to_string(),
            RejectionReason::TestFailure,
            errors,
        );

        assert_eq!(rejection.task_id, task_id);
        assert_eq!(rejection.task_description, "Implement feature");
        assert_eq!(rejection.rejection_reason, RejectionReason::TestFailure);
    }

    #[tokio::test]
    async fn test_anti_pattern_confidence_update() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Don't fail tests".to_string(),
        );

        // Add an example first (represents one observation)
        ap.add_example(AntiPatternExample {
            task_id: Uuid::new_v4(),
            before_code: None,
            after_code: None,
            error_message: "Test failed".to_string(),
            rejection_reason: RejectionReason::TestFailure,
            timestamp: Utc::now(),
        });

        // Simulate 10 total rejections
        ap.update_confidence(10);
        assert!(ap.confidence > 0.0);
        assert!(ap.confidence <= 0.95); // Capped at 0.95
    }

    #[tokio::test]
    async fn test_rejected_tool_call_serialization() {
        let tool_call = RejectedToolCall {
            tool_name: "edit_file".to_string(),
            arguments: serde_json::json!({"path": "src/main.rs", "old_str": "foo", "new_str": "bar"}),
            result: "Edit failed".to_string(),
            was_successful: false,
            error: Some("File not found".to_string()),
        };

        let json = serde_json::to_string(&tool_call).unwrap();
        let deserialized: RejectedToolCall = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tool_name, "edit_file");
        assert!(!deserialized.was_successful);
        assert!(deserialized.error.is_some());
    }

    // =====================================================================
    // Promotion Threshold Tests
    // =====================================================================

    #[test]
    fn test_promotion_thresholds_default() {
        let thresholds = PromotionThresholds::default();
        // Pattern promotion: ≥5 confirmations with confidence >0.6
        assert_eq!(thresholds.pattern_min_confirmations, 5);
        assert_eq!(thresholds.pattern_min_confidence, 0.6);
        // Rule promotion: ≥10 confirmations with confidence >0.8
        assert_eq!(thresholds.rule_min_confirmations, 10);
        assert_eq!(thresholds.rule_min_confidence, 0.8);
        // Promoted patterns get 1.5x boost
        assert_eq!(thresholds.promotion_rank_boost, 1.5);
    }

    #[test]
    fn test_anti_pattern_promotion_not_eligible() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // Initially not eligible
        assert_eq!(ap.get_promotion_status(), PromotionStatus::NotEligible);
        assert!(!ap.is_promoted);
        assert!(!ap.meets_promotion_criteria());

        // Add some examples but not enough for promotion
        for _ in 0..4 {
            ap.add_example(AntiPatternExample {
                task_id: Uuid::new_v4(),
                before_code: None,
                after_code: None,
                error_message: "Test failed".to_string(),
                rejection_reason: RejectionReason::TestFailure,
                timestamp: Utc::now(),
            });
        }

        // Still not eligible (4 confirmations, need 5)
        assert_eq!(ap.get_promotion_status(), PromotionStatus::NotEligible);
        assert!(!ap.is_promoted);
    }

    #[test]
    fn test_anti_pattern_promotion_eligible_but_not_promoted() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // Add 5 examples (this calls check_promotion but confidence is 0.0)
        for i in 0..5 {
            ap.add_example(AntiPatternExample {
                task_id: Uuid::new_v4(),
                before_code: None,
                after_code: None,
                error_message: format!("Test failed {}", i),
                rejection_reason: RejectionReason::TestFailure,
                timestamp: Utc::now(),
            });
        }

        // Set high confidence (>0.6)
        ap.confidence = 0.7;

        // Manually check promotion now that confidence is set
        ap.check_promotion();

        // Now it should be promoted
        assert_eq!(ap.get_promotion_status(), PromotionStatus::Promoted);
        assert!(ap.is_promoted);
        assert!(ap.meets_promotion_criteria());
    }

    #[test]
    fn test_anti_pattern_promotion_confidence_too_low() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // Add 5 examples but with low confidence
        for _ in 0..5 {
            ap.add_example(AntiPatternExample {
                task_id: Uuid::new_v4(),
                before_code: None,
                after_code: None,
                error_message: "Test failed".to_string(),
                rejection_reason: RejectionReason::TestFailure,
                timestamp: Utc::now(),
            });
        }

        // Set low confidence (<=0.6)
        ap.confidence = 0.5;
        ap.check_promotion();

        // Should NOT be promoted (confidence too low)
        assert!(!ap.is_promoted);
        assert!(!ap.meets_promotion_criteria());
    }

    #[test]
    fn test_anti_pattern_record_confirmation() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // Set initial state with confidence > 0.6 for promotion
        ap.confidence = 0.7;
        assert!(!ap.is_promoted);

        // Record confirmations until promotion threshold
        for i in 0..5 {
            ap.record_confirmation();
            if i < 4 {
                assert!(
                    !ap.is_promoted,
                    "Should not be promoted before reaching 5 confirmations"
                );
            }
        }

        // Now should be promoted
        assert!(
            ap.is_promoted,
            "Should be promoted after 5 confirmations with confidence > 0.6"
        );
        assert_eq!(ap.confirmation_count, 5);
    }

    #[test]
    fn test_anti_pattern_retrieval_rank_boost() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        let base_score = 1.0;

        // Not promoted - no boost
        assert_eq!(ap.get_retrieval_rank_boost(base_score), 1.0);

        // Promote the pattern
        ap.confirmation_count = 5;
        ap.confidence = 0.7;
        ap.check_promotion();

        // Now should get boost
        assert_eq!(ap.get_retrieval_rank_boost(base_score), 1.5);
    }

    #[test]
    fn test_convention_promotion_not_eligible() {
        let mut convention = Convention::new(
            "test_convention".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        // Initially not eligible
        assert_eq!(
            convention.get_promotion_status(),
            PromotionStatus::NotEligible
        );
        assert!(!convention.is_promoted);
        assert!(!convention.meets_promotion_criteria());

        // Add some confirmations but not enough for promotion (need 10)
        for _ in 0..9 {
            convention.record_confirmation();
        }

        // Still not eligible (9 confirmations, need 10)
        assert_eq!(
            convention.get_promotion_status(),
            PromotionStatus::NotEligible
        );
        assert!(!convention.is_promoted);
    }

    #[test]
    fn test_convention_promotion_eligible() {
        let mut convention = Convention::new(
            "test_convention".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        // Set high confidence (>0.8)
        convention.confidence = 0.85;

        // Record 10 confirmations (promotion threshold for rules)
        for _ in 0..10 {
            convention.record_confirmation();
        }

        // Now should be promoted
        assert_eq!(convention.get_promotion_status(), PromotionStatus::Promoted);
        assert!(convention.is_promoted);
        assert!(convention.meets_promotion_criteria());
    }

    #[test]
    fn test_convention_promotion_confidence_too_low() {
        let mut convention = Convention::new(
            "test_convention".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        // Set low confidence (<=0.8)
        convention.confidence = 0.7;

        // Record 10 confirmations
        for _ in 0..10 {
            convention.record_confirmation();
        }

        // Should NOT be promoted (confidence too low)
        assert!(!convention.is_promoted);
        assert!(!convention.meets_promotion_criteria());
    }

    #[test]
    fn test_convention_retrieval_rank_boost() {
        let mut convention = Convention::new(
            "test_convention".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        let base_score = 1.0;

        // Not promoted - no boost
        assert_eq!(convention.get_retrieval_rank_boost(base_score), 1.0);

        // Promote the convention (10 confirmations + high confidence)
        convention.confidence = 0.85;
        for _ in 0..10 {
            convention.record_confirmation();
        }

        // Now should get boost
        assert_eq!(convention.get_retrieval_rank_boost(base_score), 1.5);
    }

    #[test]
    fn test_promotion_status_display() {
        assert_eq!(format!("{}", PromotionStatus::NotEligible), "not_eligible");
        assert_eq!(format!("{}", PromotionStatus::Eligible), "eligible");
        assert_eq!(format!("{}", PromotionStatus::Promoted), "promoted");
    }

    #[test]
    fn test_promotion_status_serialization() {
        let status = PromotionStatus::Promoted;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"promoted\"");

        let deserialized: PromotionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, PromotionStatus::Promoted);
    }

    #[test]
    fn test_anti_pattern_promotion_boundary_conditions() {
        // Test exact threshold boundary
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // 5 confirmations with exactly 0.6 confidence - should NOT promote (need >0.6)
        ap.confirmation_count = 5;
        ap.confidence = 0.6;
        ap.check_promotion();
        assert!(
            !ap.is_promoted,
            "Should not promote at exactly 0.6 confidence (need > 0.6)"
        );

        // 5 confirmations with just above 0.6 confidence - should promote
        ap.confidence = 0.601;
        ap.check_promotion();
        assert!(ap.is_promoted, "Should promote at > 0.6 confidence");

        // Reset and test with just 4 confirmations
        let mut ap2 = AntiPattern::new(
            "test_pattern2".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        // 4 confirmations with high confidence - should NOT promote (need >= 5)
        ap2.confirmation_count = 4;
        ap2.confidence = 0.9;
        ap2.check_promotion();
        assert!(
            !ap2.is_promoted,
            "Should not promote with only 4 confirmations"
        );
    }

    #[test]
    fn test_convention_promotion_boundary_conditions() {
        // Test exact threshold boundary
        let mut convention = Convention::new(
            "test_convention".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        // 10 confirmations with exactly 0.8 confidence - should NOT promote (need >0.8)
        convention.confirmation_count = 10;
        convention.confidence = 0.8;
        convention.check_promotion();
        assert!(
            !convention.is_promoted,
            "Should not promote at exactly 0.8 confidence (need > 0.8)"
        );

        // 10 confirmations with just above 0.8 confidence - should promote
        convention.confidence = 0.801;
        convention.check_promotion();
        assert!(convention.is_promoted, "Should promote at > 0.8 confidence");

        // Reset and test with only 9 confirmations
        let mut convention2 = Convention::new(
            "test_convention2".to_string(),
            ConventionType::Testing,
            "cargo test".to_string(),
        );

        // 9 confirmations with high confidence - should NOT promote (need >= 10)
        convention2.confirmation_count = 9;
        convention2.confidence = 0.95;
        convention2.check_promotion();
        assert!(
            !convention2.is_promoted,
            "Should not promote with only 9 confirmations"
        );
    }

    #[test]
    fn test_promotion_updates_timestamp() {
        let mut ap = AntiPattern::new(
            "test_pattern".to_string(),
            AntiPatternType::TestPattern,
            "Test anti-pattern".to_string(),
        );

        let _initial_updated_at = ap.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Promote the pattern
        ap.confirmation_count = 5;
        ap.confidence = 0.7;
        ap.check_promotion();

        assert!(ap.is_promoted);
        // Note: updated_at is updated in check_promotion when promotion occurs
    }
}
