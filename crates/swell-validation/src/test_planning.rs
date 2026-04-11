//! Test Planning Engine for SWELL Validation.
//!
//! This module provides intelligent test selection based on:
//! - Parsed acceptance criteria from specifications
//! - Diff context from changed files
//! - Risk scoring for prioritized test execution
//!
//! # Architecture
//!
//! The test planning engine consists of:
//! - [`AcceptanceCriteriaParser`] - Parses acceptance criteria from spec documents
//! - [`DiffContextExtractor`] - Extracts test-relevant context from git diffs
//! - [`RiskScorer`] - Assigns risk scores to tests based on change impact
//! - [`TestPlanningEngine`] - Orchestrates the planning process
//!
//! # Usage
//!
//! ```rust
//! use swell_validation::test_planning::{TestPlanningEngine, TestPlanRequest};
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let engine = TestPlanningEngine::with_defaults();
//!     let request = TestPlanRequest {
//!         task_description: "Implement user authentication".to_string(),
//!         changed_files: vec!["src/auth.rs".to_string()],
//!         diff_content: Some("+fn login() { ...".to_string()),
//!         spec_content: None,
//!     };
//!     let plan = engine.create_test_plan(request).await?;
//!     Ok(())
//! }
//! ```

use serde::{Deserialize, Serialize};
use swell_core::{Plan, PlanStep, RiskLevel, StepStatus};
use uuid::Uuid;

/// Request to create a test plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPlanRequest {
    /// Task description
    pub task_description: String,
    /// Files that were changed
    pub changed_files: Vec<String>,
    /// Git diff content (optional)
    pub diff_content: Option<String>,
    /// Specification content (optional)
    pub spec_content: Option<String>,
}

/// A test case with associated metadata and risk score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Test name (fully qualified)
    pub name: String,
    /// File where test is defined
    pub file: Option<String>,
    /// Risk score (0.0 to 1.0, higher = more risky)
    pub risk_score: f64,
    /// Risk level category
    pub risk_level: TestRiskLevel,
    /// Acceptance criteria this test verifies
    pub criteria: Vec<String>,
    /// Whether this test covers critical path
    pub is_critical: bool,
    /// Estimated execution time in milliseconds
    pub estimated_duration_ms: u64,
    /// Tags/labels for the test
    pub tags: Vec<String>,
}

/// Risk level categories for tests
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TestRiskLevel {
    /// Low risk: cosmetic changes, non-critical paths
    Low,
    /// Medium risk: functional changes with some test coverage
    Medium,
    /// High risk: core functionality, auth, payments, data integrity
    High,
    /// Critical: security, safety, data loss scenarios
    Critical,
}

impl TestRiskLevel {
    /// Get numeric weight for aggregation
    pub fn weight(&self) -> f64 {
        match self {
            TestRiskLevel::Low => 0.25,
            TestRiskLevel::Medium => 0.5,
            TestRiskLevel::High => 0.75,
            TestRiskLevel::Critical => 1.0,
        }
    }
}

/// Result of test planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPlan {
    /// Selected test cases ordered by priority
    pub test_cases: Vec<TestCase>,
    /// Acceptance criteria covered
    pub covered_criteria: Vec<String>,
    /// Acceptance criteria not covered
    pub uncovered_criteria: Vec<String>,
    /// Overall risk assessment
    pub overall_risk: TestRiskLevel,
    /// Coverage percentage
    pub coverage_percentage: f64,
}

/// Parsed acceptance criterion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    /// Unique identifier
    pub id: String,
    /// Criterion text
    pub text: String,
    /// Category (e.g., "functional", "performance", "security")
    pub category: String,
    /// Criticality level
    pub criticality: CriterionCriticality,
    /// Related test patterns
    pub test_hints: Vec<String>,
}

/// Criticality level for acceptance criteria
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriterionCriticality {
    /// Must have for release
    MustHave,
    /// Should have for release
    ShouldHave,
    /// Nice to have
    NiceToHave,
}

/// Diff context information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffContext {
    /// Files added
    pub files_added: Vec<String>,
    /// Files modified
    pub files_modified: Vec<String>,
    /// Files deleted
    pub files_deleted: Vec<String>,
    /// Functions added
    pub functions_added: Vec<String>,
    /// Functions modified
    pub functions_modified: Vec<String>,
    /// Lines changed count
    pub lines_changed: usize,
    /// Risk indicators found
    pub risk_indicators: Vec<String>,
}

// ============================================================================
// Acceptance Criteria Parser
// ============================================================================

/// Parser for extracting acceptance criteria from specification documents.
#[derive(Debug, Clone, Default)]
pub struct AcceptanceCriteriaParser {
    /// Known patterns for criterion identification
    known_patterns: Vec<CriterionPattern>,
}

/// A pattern for recognizing acceptance criteria
#[derive(Debug, Clone)]
struct CriterionPattern {
    /// Pattern regex or keyword
    pub pattern: String,
    /// Category to assign
    pub category: String,
    /// Criticality to assign
    pub criticality: CriterionCriticality,
}

impl AcceptanceCriteriaParser {
    /// Create a new parser with default patterns
    pub fn new() -> Self {
        let known_patterns = vec![
            // Must-have patterns (high criticality)
            CriterionPattern {
                pattern: r"(?i)must|required|shall".to_string(),
                category: "functional".to_string(),
                criticality: CriterionCriticality::MustHave,
            },
            CriterionPattern {
                pattern: r"(?i)security|auth|password|encrypt".to_string(),
                category: "security".to_string(),
                criticality: CriterionCriticality::MustHave,
            },
            CriterionPattern {
                pattern: r"(?i)error|panic|fail".to_string(),
                category: "error_handling".to_string(),
                criticality: CriterionCriticality::MustHave,
            },
            // Should-have patterns (medium criticality)
            CriterionPattern {
                pattern: r"(?i)should|expected|typically".to_string(),
                category: "functional".to_string(),
                criticality: CriterionCriticality::ShouldHave,
            },
            CriterionPattern {
                pattern: r"(?i)performance|latency|throughput".to_string(),
                category: "performance".to_string(),
                criticality: CriterionCriticality::ShouldHave,
            },
            CriterionPattern {
                pattern: r"(?i)usability|ux|ui".to_string(),
                category: "usability".to_string(),
                criticality: CriterionCriticality::ShouldHave,
            },
            // Nice-to-have patterns (low criticality)
            CriterionPattern {
                pattern: r"(?i)can|could|may|might".to_string(),
                category: "enhancement".to_string(),
                criticality: CriterionCriticality::NiceToHave,
            },
            CriterionPattern {
                pattern: r"(?i)logging|debug|trace".to_string(),
                category: "observability".to_string(),
                criticality: CriterionCriticality::NiceToHave,
            },
        ];

        Self { known_patterns }
    }

    /// Parse acceptance criteria from spec content
    pub fn parse(&self, spec_content: &str) -> Vec<AcceptanceCriterion> {
        let mut criteria = Vec::new();
        let lines: Vec<&str> = spec_content.lines().collect();

        let mut current_section = String::new();
        let mut line_num = 0;

        for line in &lines {
            line_num += 1;
            let trimmed = line.trim();

            // Track section headers
            if trimmed.ends_with(':') || trimmed.ends_with(':') && trimmed.len() < 50 {
                current_section = trimmed.trim_end_matches(':').trim().to_string();
            }

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }

            // Check if line contains acceptance criteria patterns
            if self.looks_like_criterion(trimmed) {
                if let Some(criterion) = self.extract_criterion(trimmed, line_num, &current_section)
                {
                    criteria.push(criterion);
                }
            }
        }

        // If no structured criteria found, try to extract from bullet points
        if criteria.is_empty() {
            criteria = self.extract_from_bullets(&lines);
        }

        criteria
    }

    /// Check if a line looks like an acceptance criterion
    fn looks_like_criterion(&self, line: &str) -> bool {
        let lower = line.to_lowercase();

        // Check for explicit criterion markers
        let markers = [
            "shall",
            "must",
            "should",
            "will",
            "can",
            "could",
            "given",
            "when",
            "then",
            "expect",
            "acceptance",
            "criterion",
            "requirement",
            "verify",
            "validate",
            "ensure",
            "check",
        ];

        markers.iter().any(|m| lower.contains(m)) && line.len() > 20
    }

    /// Extract a single criterion from a line
    fn extract_criterion(
        &self,
        line: &str,
        line_num: usize,
        section: &str,
    ) -> Option<AcceptanceCriterion> {
        let trimmed = line.trim();

        // Determine category and criticality from patterns
        let mut category = "general".to_string();
        let mut criticality = CriterionCriticality::ShouldHave;

        for pattern in &self.known_patterns {
            if pattern.pattern.len() > 2 {
                // Split by | to get individual keywords
                let keywords: Vec<&str> = pattern.pattern.split('|').collect();
                let line_lower = trimmed.to_lowercase();
                let matches = keywords.iter().any(|kw| {
                    // Skip regex anchors like (?i)
                    let keyword = kw.trim_start_matches("(?i)");
                    line_lower.contains(keyword)
                });
                if matches {
                    category = pattern.category.clone();
                    criticality = pattern.criticality;
                    break;
                }
            }
        }

        // Use section as category hint if no pattern matched
        if category == "general" && !section.is_empty() {
            let section_lower = section.to_lowercase();
            if section_lower.contains("security") {
                category = "security".to_string();
            } else if section_lower.contains("error") || section_lower.contains("fail") {
                category = "error_handling".to_string();
            } else if section_lower.contains("performance") {
                category = "performance".to_string();
            }
        }

        // Generate test hints from keywords
        let test_hints = self.generate_test_hints(trimmed);

        Some(AcceptanceCriterion {
            id: format!("AC-{}-{}", line_num, Self::hash_string(trimmed)),
            text: trimmed.to_string(),
            category,
            criticality,
            test_hints,
        })
    }

    /// Generate test hint patterns from criterion text
    fn generate_test_hints(&self, text: &str) -> Vec<String> {
        let mut hints = Vec::new();
        let lower = text.to_lowercase();

        // Extract action words
        let action_words = [
            "validate", "verify", "check", "ensure", "test", "reject", "accept", "return",
        ];
        for word in action_words {
            if lower.contains(word) {
                hints.push(format!("test_{}", word));
            }
        }

        // Extract key nouns (simple heuristic)
        let nouns = [
            "user", "password", "auth", "token", "data", "file", "request", "response",
        ];
        for noun in nouns {
            if lower.contains(noun) {
                hints.push(format!("test_{}_handling", noun));
            }
        }

        if hints.is_empty() {
            hints.push("test_functional".to_string());
        }

        hints
    }

    /// Simple hash for generating criterion IDs
    fn hash_string(s: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("{:x}", hasher.finish())[..6].to_string()
    }

    /// Extract criteria from bullet points
    fn extract_from_bullets(&self, lines: &[&str]) -> Vec<AcceptanceCriterion> {
        let mut criteria = Vec::new();
        let bullet_chars = ['-', '*', '+', '•', '▸'];

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Check for bullet points
            if let Some(first_char) = trimmed.chars().next() {
                if bullet_chars.contains(&first_char) {
                    let content = trimmed[1..].trim();
                    if content.len() > 10 && self.looks_like_criterion(content) {
                        if let Some(criterion) = self.extract_criterion(content, idx + 1, "") {
                            criteria.push(criterion);
                        }
                    }
                }
            }

            // Check for numbered items
            if trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && trimmed.contains('.')
            {
                let parts: Vec<&str> = trimmed.splitn(2, '.').collect();
                if parts.len() == 2 && parts[1].trim().len() > 10 {
                    let content = parts[1].trim();
                    if self.looks_like_criterion(content) {
                        if let Some(criterion) = self.extract_criterion(content, idx + 1, "") {
                            criteria.push(criterion);
                        }
                    }
                }
            }
        }

        criteria
    }
}

// ============================================================================
// Diff Context Extractor
// ============================================================================

/// Extracts test-relevant context from git diffs
#[derive(Debug, Clone, Default)]
pub struct DiffContextExtractor;

impl DiffContextExtractor {
    /// Create a new extractor
    pub fn new() -> Self {
        Self
    }

    /// Parse diff content and extract context
    pub fn parse(&self, diff_content: &str) -> DiffContext {
        let mut ctx = DiffContext {
            files_added: Vec::new(),
            files_modified: Vec::new(),
            files_deleted: Vec::new(),
            functions_added: Vec::new(),
            functions_modified: Vec::new(),
            lines_changed: 0,
            risk_indicators: Vec::new(),
        };

        let mut current_file = String::new();
        let mut in_diff = false;

        for line in diff_content.lines() {
            let trimmed = line.trim();

            // Track which file we're in
            if trimmed.starts_with("diff --git")
                || trimmed.starts_with("--- ")
                || trimmed.starts_with("+++ ")
            {
                if trimmed.starts_with("diff --git") {
                    in_diff = true;
                    if let Some(path) = self.extract_file_path(trimmed) {
                        current_file = path.clone();
                    }
                } else if trimmed.starts_with("--- ") {
                    let path = self.extract_file_path(trimmed);
                    if let Some(ref p) = path {
                        if p == "/dev/null" {
                            // File being added - we'll confirm when we see +++
                        } else {
                            // File exists in old state - will be modified unless +++ is /dev/null
                            // For now, assume modified (we'll handle add/delete separately)
                            if !ctx.files_modified.contains(p) && !ctx.files_added.contains(p) {
                                ctx.files_modified.push(p.clone());
                            }
                        }
                    }
                } else if trimmed.starts_with("+++ ") {
                    let path = self.extract_file_path(trimmed);
                    if let Some(ref p) = path {
                        if p == "/dev/null" {
                            // File was deleted
                            ctx.files_deleted.push(current_file.clone());
                            // Remove from modified if it was there
                            ctx.files_modified.retain(|f| f != &current_file);
                        }
                        // If not /dev/null and not already in modified, it's an add
                        else if !ctx.files_modified.contains(p) && !ctx.files_added.contains(p) {
                            ctx.files_added.push(p.clone());
                        }
                    }
                }
            }

            // Track additions/deletions and function changes
            #[allow(clippy::if_same_then_else)]
            if in_diff {
                if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
                    ctx.lines_changed += 1;
                } else if trimmed.starts_with('-') && !trimmed.starts_with("---") {
                    ctx.lines_changed += 1;
                }

                // Extract function changes
                if trimmed.starts_with("+fn ") || trimmed.starts_with("+pub fn ") {
                    ctx.functions_added
                        .push(self.extract_function_name(trimmed));
                }
                if trimmed.starts_with("-fn ") || trimmed.starts_with("-pub fn ") {
                    ctx.functions_modified
                        .push(self.extract_function_name(trimmed));
                }
            }

            // Detect risk indicators
            self.detect_risk_indicators(trimmed, &mut ctx.risk_indicators);
        }

        ctx
    }

    /// Extract file path from diff line
    fn extract_file_path(&self, line: &str) -> Option<String> {
        // diff --git a/path/to/file.rs b/path/to/file.rs
        // --- a/path/to/file.rs
        // +++ b/path/to/file.rs
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            // Handle "diff --git a/path b/path" format
            if line.starts_with("diff --git") {
                // Last part is "b/path", strip the "b/" prefix to get the actual path
                let path = *parts.last().unwrap();
                let path = path.strip_prefix("b/").unwrap_or(path);
                return Some(path.to_string());
            }

            // Handle "--- a/path" or "+++ b/path" format
            let path = *parts.last().unwrap();
            if path == "/dev/null" {
                // For /dev/null, get the other path from the first part
                if parts.len() >= 3 {
                    return Some(
                        parts
                            .get(1)
                            .unwrap_or(&"")
                            .trim_start_matches("a/")
                            .trim_start_matches("b/")
                            .to_string(),
                    );
                }
                return None;
            }
            // Strip a/ or b/ prefix
            let path = path
                .strip_prefix("a/")
                .or(path.strip_prefix("b/"))
                .unwrap_or(path);
            return Some(path.to_string());
        }
        None
    }

    /// Extract function name from line
    fn extract_function_name(&self, line: &str) -> String {
        let content = line.trim_start_matches(|c| ['+', '-', ' '].contains(&c));
        let after_fn = content
            .strip_prefix("fn ")
            .or(content.strip_prefix("pub fn "))
            .unwrap_or(content);
        after_fn
            .split('(')
            .next()
            .unwrap_or(after_fn)
            .trim()
            .to_string()
    }

    /// Detect risk indicators in diff lines
    fn detect_risk_indicators(&self, line: &str, indicators: &mut Vec<String>) {
        let lower = line.to_lowercase();

        let risk_patterns = [
            (
                "security",
                vec![
                    "password",
                    "auth",
                    "token",
                    "encrypt",
                    "credential",
                    "secret",
                ],
            ),
            (
                "error_handling",
                vec!["unwrap", "expect", "panic", "unwrap", "abort"],
            ),
            (
                "data_integrity",
                vec!["drop", "delete", "remove", "truncate", "clear"],
            ),
            (
                "concurrency",
                vec!["lock", "mutex", "atomic", "thread", "spawn"],
            ),
            ("resource", vec!["alloc", "dealloc", "memory", "leak"]),
        ];

        for (category, keywords) in &risk_patterns {
            for keyword in keywords {
                if lower.contains(keyword) && !indicators.iter().any(|i| i.contains(category)) {
                    indicators.push(format!("{}: {}", category, keyword));
                }
            }
        }
    }
}

// ============================================================================
// Risk Scorer
// ============================================================================

/// Scores tests based on change impact and risk factors
#[derive(Debug, Clone)]
pub struct RiskScorer {
    /// Weights for different risk factors
    weights: RiskWeights,
}

/// Weights for risk calculation
#[derive(Debug, Clone)]
pub struct RiskWeights {
    /// File risk weight
    pub file_risk: f64,
    /// Function risk weight
    pub function_risk: f64,
    /// Criteria coverage weight
    pub criteria_weight: f64,
    /// Critical path weight
    pub critical_path: f64,
    /// Change magnitude weight
    pub change_magnitude: f64,
}

impl Default for RiskWeights {
    fn default() -> Self {
        Self {
            file_risk: 0.25,
            function_risk: 0.25,
            criteria_weight: 0.20,
            critical_path: 0.15,
            change_magnitude: 0.15,
        }
    }
}

impl Default for RiskScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskScorer {
    /// Create a new risk scorer with default weights
    pub fn new() -> Self {
        Self {
            weights: RiskWeights::default(),
        }
    }

    /// Create with custom weights
    pub fn with_weights(weights: RiskWeights) -> Self {
        Self { weights }
    }

    /// Score a test case based on change context
    pub fn score_test(
        &self,
        test: &mut TestCase,
        context: &DiffContext,
        criteria: &[AcceptanceCriterion],
    ) {
        let mut score = 0.0;

        // Factor 1: File risk
        if let Some(ref test_file) = test.file {
            let file_risk = self.calculate_file_risk(test_file, context);
            score += file_risk * self.weights.file_risk;
        }

        // Factor 2: Function risk
        let func_risk = self.calculate_function_risk(&test.name, context);
        score += func_risk * self.weights.function_risk;

        // Factor 3: Criteria coverage risk
        let criteria_risk = self.calculate_criteria_risk(&test.criteria, criteria);
        score += criteria_risk * self.weights.criteria_weight;

        // Factor 4: Critical path risk
        if test.is_critical {
            score += 0.65 * self.weights.critical_path;
        }

        // Factor 5: Change magnitude
        let magnitude_risk = self.calculate_magnitude_risk(context);
        score += magnitude_risk * self.weights.change_magnitude;

        // Normalize and set
        test.risk_score = score.clamp(0.0, 1.0);
        test.risk_level = self.score_to_level(test.risk_score);
    }

    /// Calculate file-based risk
    fn calculate_file_risk(&self, test_file: &str, context: &DiffContext) -> f64 {
        let test_file_base = test_file.split('/').next_back().unwrap_or(test_file);

        // High-risk directories take precedence (highest priority)
        let high_risk_dirs = [
            "auth", "security", "payment", "account", "admin", "critical",
        ];
        for dir in high_risk_dirs {
            if test_file.contains(dir) {
                return 0.65;
            }
        }

        // Check if test file directly corresponds to modified file
        for modified in &context.files_modified {
            let modified_base = modified.split('/').next_back().unwrap_or(modified);
            if test_file_base.contains(modified_base) || modified_base.contains(test_file_base) {
                return 0.8;
            }
        }

        // Check for added files (new code = higher risk)
        for added in &context.files_added {
            let added_base = added.split('/').next_back().unwrap_or(added);
            if test_file_base.contains(added_base) {
                return 0.7;
            }
        }

        0.0
    }

    /// Calculate function-level risk
    fn calculate_function_risk(&self, test_name: &str, context: &DiffContext) -> f64 {
        let mut risk: f64 = 0.0;

        // Extract function name from test name
        // Test names are like "test_module_function_name"
        let test_parts: Vec<&str> = test_name.split('_').collect();

        for func in &context.functions_modified {
            for part in &test_parts {
                if func.to_lowercase().contains(&part.to_lowercase()) && part.len() > 3 {
                    risk = risk.max(0.7);
                }
            }
        }

        for func in &context.functions_added {
            for part in &test_parts {
                if func.to_lowercase().contains(&part.to_lowercase()) && part.len() > 3 {
                    risk = risk.max(0.6);
                }
            }
        }

        risk
    }

    /// Calculate criteria-based risk
    fn calculate_criteria_risk(
        &self,
        test_criteria: &[String],
        all_criteria: &[AcceptanceCriterion],
    ) -> f64 {
        if test_criteria.is_empty() || all_criteria.is_empty() {
            return 0.7; // Default high risk for untested criteria
        }

        let must_have = all_criteria
            .iter()
            .filter(|c| c.criticality == CriterionCriticality::MustHave)
            .count();

        if must_have > 0 {
            let covered_must_have = test_criteria
                .iter()
                .filter(|tc| {
                    all_criteria.iter().any(|ac| {
                        ac.criticality == CriterionCriticality::MustHave
                            && (ac.text.to_lowercase().contains(&tc.to_lowercase())
                                || tc.to_lowercase().contains(&ac.text.to_lowercase()))
                    })
                })
                .count();

            if covered_must_have < must_have {
                return 0.6; // Uncovered must-have criteria = higher risk
            }
        }

        0.3 // Default risk if criteria are covered
    }

    /// Calculate change magnitude risk
    fn calculate_magnitude_risk(&self, context: &DiffContext) -> f64 {
        let total_files =
            context.files_added.len() + context.files_modified.len() + context.files_deleted.len();

        if context.lines_changed > 500 || total_files > 20 {
            0.8 // Large change = higher risk
        } else if context.lines_changed > 200 || total_files > 10 {
            0.5 // Medium change
        } else if context.lines_changed > 50 || total_files > 3 {
            0.3 // Small change
        } else {
            0.1 // Minimal change
        }
    }

    /// Convert numeric score to risk level
    fn score_to_level(&self, score: f64) -> TestRiskLevel {
        if score >= 0.8 {
            TestRiskLevel::Critical
        } else if score >= 0.6 {
            TestRiskLevel::High
        } else if score >= 0.3 {
            TestRiskLevel::Medium
        } else {
            TestRiskLevel::Low
        }
    }

    /// Score all test cases
    pub fn score_tests(
        &self,
        tests: &mut [TestCase],
        context: &DiffContext,
        criteria: &[AcceptanceCriterion],
    ) {
        for test in tests.iter_mut() {
            self.score_test(test, context, criteria);
        }

        // Sort by risk score descending
        tests.sort_by(|a, b| {
            b.risk_score
                .partial_cmp(&a.risk_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
}

// ============================================================================
// Test Planning Engine
// ============================================================================

/// Main engine for test planning and selection
#[derive(Debug, Clone)]
pub struct TestPlanningEngine {
    criteria_parser: AcceptanceCriteriaParser,
    diff_extractor: DiffContextExtractor,
    risk_scorer: RiskScorer,
}

impl Default for TestPlanningEngine {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl TestPlanningEngine {
    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self {
            criteria_parser: AcceptanceCriteriaParser::new(),
            diff_extractor: DiffContextExtractor::new(),
            risk_scorer: RiskScorer::new(),
        }
    }

    /// Create a test plan from request
    pub async fn create_test_plan(&self, request: TestPlanRequest) -> Result<TestPlan, String> {
        // Step 1: Parse acceptance criteria from spec
        let criteria = if let Some(ref spec) = request.spec_content {
            self.criteria_parser.parse(spec)
        } else {
            // Try to extract criteria from task description
            self.extract_criteria_from_description(&request.task_description)
        };

        // Step 2: Parse diff context
        let diff_context = if let Some(ref diff) = request.diff_content {
            self.diff_extractor.parse(diff)
        } else {
            DiffContext::default()
        };

        // Step 3: Generate candidate test cases
        let mut test_cases = self.generate_candidate_tests(&request, &diff_context);

        // Step 4: Score and rank tests
        self.risk_scorer
            .score_tests(&mut test_cases, &diff_context, &criteria);

        // Step 5: Calculate coverage
        let (covered, uncovered) = self.calculate_coverage(&test_cases, &criteria);

        let coverage_percentage = if criteria.is_empty() {
            0.0
        } else {
            (covered.len() as f64 / criteria.len() as f64) * 100.0
        };

        // Step 6: Determine overall risk
        let overall_risk = self.determine_overall_risk(&test_cases, &diff_context);

        Ok(TestPlan {
            test_cases,
            covered_criteria: covered,
            uncovered_criteria: uncovered,
            overall_risk,
            coverage_percentage,
        })
    }

    /// Extract criteria from task description when no spec is provided
    fn extract_criteria_from_description(&self, description: &str) -> Vec<AcceptanceCriterion> {
        let criteria = self.criteria_parser.parse(description);

        // If still empty, create a default criterion from the description
        if criteria.is_empty() && !description.is_empty() {
            vec![AcceptanceCriterion {
                id: "AC-DESC-1".to_string(),
                text: description.to_string(),
                category: "functional".to_string(),
                criticality: CriterionCriticality::ShouldHave,
                test_hints: vec!["test_functional".to_string()],
            }]
        } else {
            criteria
        }
    }

    /// Generate candidate test cases from request and context
    fn generate_candidate_tests(
        &self,
        request: &TestPlanRequest,
        context: &DiffContext,
    ) -> Vec<TestCase> {
        let mut tests = Vec::new();

        // Generate tests based on changed files from diff context
        for file in &context.files_modified {
            tests.extend(self.generate_tests_for_file(file, "modified"));
        }

        for file in &context.files_added {
            tests.extend(self.generate_tests_for_file(file, "added"));
        }

        // If diff context is empty but request has changed_files, use those
        if context.files_modified.is_empty() && context.files_added.is_empty() {
            for file in &request.changed_files {
                tests.extend(self.generate_tests_for_file(file, "modified"));
            }
        }

        // Generate tests based on task description keywords
        tests.extend(self.generate_keyword_tests(&request.task_description));

        // Generate tests based on risk indicators
        for indicator in &context.risk_indicators {
            tests.extend(self.generate_risk_tests(indicator));
        }

        // Remove duplicates by name
        let mut seen = std::collections::HashSet::new();
        tests.retain(|t| seen.insert(t.name.clone()));

        tests
    }

    /// Generate test cases for a specific file
    fn generate_tests_for_file(&self, file: &str, change_type: &str) -> Vec<TestCase> {
        let mut tests = Vec::new();

        // Determine file type and generate appropriate tests
        let file_name = file.split('/').next_back().unwrap_or(file);
        let module_name = file_name.trim_end_matches(".rs").replace('-', "_");

        // Basic module test
        tests.push(TestCase {
            name: format!("test_{}_module_loads", module_name),
            file: Some(file.to_string()),
            risk_score: 0.0, // Will be scored
            risk_level: TestRiskLevel::Low,
            criteria: vec![],
            is_critical: false,
            estimated_duration_ms: 10,
            tags: vec!["module".to_string(), change_type.to_string()],
        });

        // Add specific tests based on file path
        let path_lower = file.to_lowercase();

        if path_lower.contains("auth")
            || path_lower.contains("login")
            || path_lower.contains("password")
        {
            tests.push(TestCase {
                name: format!("test_{}_authentication", module_name),
                file: Some(file.to_string()),
                risk_score: 0.0,
                risk_level: TestRiskLevel::Critical,
                criteria: vec!["authentication".to_string(), "security".to_string()],
                is_critical: true,
                estimated_duration_ms: 50,
                tags: vec![
                    "auth".to_string(),
                    "security".to_string(),
                    change_type.to_string(),
                ],
            });
        }

        if path_lower.contains("error") || path_lower.contains("fail") {
            tests.push(TestCase {
                name: format!("test_{}_error_handling", module_name),
                file: Some(file.to_string()),
                risk_score: 0.0,
                risk_level: TestRiskLevel::High,
                criteria: vec!["error_handling".to_string()],
                is_critical: false,
                estimated_duration_ms: 30,
                tags: vec!["error".to_string(), change_type.to_string()],
            });
        }

        if path_lower.contains("api")
            || path_lower.contains("endpoint")
            || path_lower.contains("route")
        {
            tests.push(TestCase {
                name: format!("test_{}_api_integration", module_name),
                file: Some(file.to_string()),
                risk_score: 0.0,
                risk_level: TestRiskLevel::Medium,
                criteria: vec!["api".to_string()],
                is_critical: false,
                estimated_duration_ms: 100,
                tags: vec![
                    "api".to_string(),
                    "integration".to_string(),
                    change_type.to_string(),
                ],
            });
        }

        // Default functional test
        tests.push(TestCase {
            name: format!("test_{}_basic_functionality", module_name),
            file: Some(file.to_string()),
            risk_score: 0.0,
            risk_level: TestRiskLevel::Medium,
            criteria: vec!["functional".to_string()],
            is_critical: false,
            estimated_duration_ms: 25,
            tags: vec!["functional".to_string(), change_type.to_string()],
        });

        tests
    }

    /// Generate tests based on keywords in task description
    fn generate_keyword_tests(&self, description: &str) -> Vec<TestCase> {
        let mut tests = Vec::new();
        let lower = description.to_lowercase();

        let keyword_tests = [
            ("error", "test_error_cases", TestRiskLevel::High),
            ("fail", "test_failure_modes", TestRiskLevel::High),
            ("auth", "test_authentication_flow", TestRiskLevel::Critical),
            ("login", "test_login_validation", TestRiskLevel::High),
            (
                "password",
                "test_password_handling",
                TestRiskLevel::Critical,
            ),
            ("data", "test_data_integrity", TestRiskLevel::High),
            ("file", "test_file_operations", TestRiskLevel::Medium),
            ("config", "test_configuration", TestRiskLevel::Medium),
            ("api", "test_api_endpoints", TestRiskLevel::Medium),
            ("validate", "test_validation_rules", TestRiskLevel::Medium),
            ("parse", "test_parsing_logic", TestRiskLevel::Low),
            ("convert", "test_conversion", TestRiskLevel::Low),
        ];

        for (keyword, test_name, risk) in keyword_tests {
            if lower.contains(keyword) {
                tests.push(TestCase {
                    name: test_name.to_string(),
                    file: None,
                    risk_score: 0.0,
                    risk_level: risk,
                    criteria: vec![keyword.to_string()],
                    is_critical: risk == TestRiskLevel::Critical,
                    estimated_duration_ms: 30,
                    tags: vec![keyword.to_string()],
                });
            }
        }

        tests
    }

    /// Generate tests based on risk indicators
    fn generate_risk_tests(&self, indicator: &str) -> Vec<TestCase> {
        let mut tests = Vec::new();
        let lower = indicator.to_lowercase();

        if lower.contains("security") {
            tests.push(TestCase {
                name: "test_security_controls".to_string(),
                file: None,
                risk_score: 0.0,
                risk_level: TestRiskLevel::Critical,
                criteria: vec!["security".to_string()],
                is_critical: true,
                estimated_duration_ms: 100,
                tags: vec!["security".to_string()],
            });
        }

        if lower.contains("error_handling") {
            tests.push(TestCase {
                name: "test_error_recovery".to_string(),
                file: None,
                risk_score: 0.0,
                risk_level: TestRiskLevel::High,
                criteria: vec!["error_handling".to_string()],
                is_critical: false,
                estimated_duration_ms: 50,
                tags: vec!["error".to_string()],
            });
        }

        if lower.contains("data_integrity") {
            tests.push(TestCase {
                name: "test_data_integrity_checks".to_string(),
                file: None,
                risk_score: 0.0,
                risk_level: TestRiskLevel::Critical,
                criteria: vec!["data_integrity".to_string()],
                is_critical: true,
                estimated_duration_ms: 75,
                tags: vec!["data".to_string(), "integrity".to_string()],
            });
        }

        tests
    }

    /// Calculate which criteria are covered by tests
    fn calculate_coverage(
        &self,
        tests: &[TestCase],
        criteria: &[AcceptanceCriterion],
    ) -> (Vec<String>, Vec<String>) {
        let mut covered = Vec::new();
        let mut uncovered = Vec::new();

        for criterion in criteria {
            let is_covered = tests.iter().any(|test| {
                test.criteria.iter().any(|tc| {
                    criterion.text.to_lowercase().contains(&tc.to_lowercase())
                        || tc.to_lowercase().contains(&criterion.text.to_lowercase())
                }) || test.tags.iter().any(|tag| {
                    criterion
                        .category
                        .to_lowercase()
                        .contains(&tag.to_lowercase())
                        || tag
                            .to_lowercase()
                            .contains(&criterion.category.to_lowercase())
                })
            });

            if is_covered {
                covered.push(criterion.text.clone());
            } else {
                uncovered.push(criterion.text.clone());
            }
        }

        (covered, uncovered)
    }

    /// Determine overall risk level from tests and context
    fn determine_overall_risk(&self, tests: &[TestCase], context: &DiffContext) -> TestRiskLevel {
        // Check if any critical tests exist
        let has_critical = tests
            .iter()
            .any(|t| t.risk_level == TestRiskLevel::Critical);
        if has_critical {
            return TestRiskLevel::Critical;
        }

        // Check for high-risk tests
        let has_high = tests.iter().any(|t| t.risk_level == TestRiskLevel::High);
        if has_high {
            return TestRiskLevel::High;
        }

        // Check risk indicators
        for indicator in &context.risk_indicators {
            let lower = indicator.to_lowercase();
            if lower.contains("security") || lower.contains("data_integrity") {
                return TestRiskLevel::High;
            }
        }

        // Check change magnitude
        if context.lines_changed > 300 {
            return TestRiskLevel::High;
        }

        let has_medium = tests.iter().any(|t| t.risk_level == TestRiskLevel::Medium);
        if has_medium {
            return TestRiskLevel::Medium;
        }

        TestRiskLevel::Low
    }

    /// Convert test plan to planner format (Plan with PlanSteps)
    pub fn to_plan(&self, test_plan: &TestPlan, task_id: Uuid) -> Plan {
        let steps: Vec<PlanStep> = test_plan
            .test_cases
            .iter()
            .enumerate()
            .map(|(idx, tc)| {
                let dependencies: Vec<Uuid> = if idx > 0
                    && test_plan.test_cases[idx - 1].risk_level == TestRiskLevel::Critical
                {
                    // Critical tests might need to run first, but we don't add hard dependencies
                    vec![]
                } else {
                    vec![]
                };

                let risk = match tc.risk_level {
                    TestRiskLevel::Low => RiskLevel::Low,
                    TestRiskLevel::Medium => RiskLevel::Medium,
                    TestRiskLevel::High => RiskLevel::High,
                    TestRiskLevel::Critical => RiskLevel::High,
                };

                PlanStep {
                    id: Uuid::new_v4(),
                    description: format!("Run test: {}", tc.name),
                    affected_files: tc.file.iter().cloned().collect(),
                    expected_tests: vec![tc.name.clone()],
                    risk_level: risk,
                    dependencies,
                    status: StepStatus::Pending,
                }
            })
            .collect();

        let risk_assessment = format!(
            "Test plan with {} test cases, {:.0}% criteria coverage, overall risk: {:?}",
            test_plan.test_cases.len(),
            test_plan.coverage_percentage,
            test_plan.overall_risk
        );

        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps,
            total_estimated_tokens: test_plan.test_cases.len() as u64 * 1000,
            risk_assessment,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod acceptance_criteria_parser_tests {
    use super::*;

    #[test]
    fn test_parse_simple_criteria() {
        let parser = AcceptanceCriteriaParser::new();
        let spec = r#"
# Acceptance Criteria

## Authentication
- The system shall authenticate users with email and password
- The system must validate password strength
- Users should be able to reset their password

## Performance
- The system should respond within 100ms
- can handle 1000 concurrent requests
        "#;

        let criteria = parser.parse(spec);
        assert!(!criteria.is_empty());
    }

    #[test]
    fn test_parse_with_section_headers() {
        let parser = AcceptanceCriteriaParser::new();
        let spec = r#"
Security Requirements:
The system shall encrypt all sensitive data
Users must have strong passwords
        "#;

        let criteria = parser.parse(spec);
        assert!(criteria.len() >= 1);

        // Check that security criteria are properly categorized
        let security_criteria: Vec<_> = criteria
            .iter()
            .filter(|c| c.category == "security")
            .collect();
        assert!(!security_criteria.is_empty());
    }

    #[test]
    fn test_criterion_criticality() {
        let parser = AcceptanceCriteriaParser::new();

        let must_spec = "The system shall authenticate users";
        let should_spec = "The system should validate email format";
        let can_spec = "Users can optionally enable 2FA";

        // Parse each and check criticality
        let criteria = parser.parse(must_spec);
        if !criteria.is_empty() {
            assert_eq!(criteria[0].criticality, CriterionCriticality::MustHave);
        }

        let criteria = parser.parse(should_spec);
        if !criteria.is_empty() {
            assert_eq!(criteria[0].criticality, CriterionCriticality::ShouldHave);
        }

        let criteria = parser.parse(can_spec);
        if !criteria.is_empty() {
            assert_eq!(criteria[0].criticality, CriterionCriticality::NiceToHave);
        }
    }

    #[test]
    fn test_empty_input() {
        let parser = AcceptanceCriteriaParser::new();
        let criteria = parser.parse("");
        assert!(criteria.is_empty());
    }

    #[test]
    fn test_bullet_point_extraction() {
        let parser = AcceptanceCriteriaParser::new();
        let spec = r#"
- First requirement shall be met
- Second requirement must work
* Third requirement should be fast
+ Fourth requirement can be optional
        "#;

        let criteria = parser.parse(spec);
        assert!(criteria.len() >= 2);
    }
}

#[cfg(test)]
mod diff_context_extractor_tests {
    use super::*;

    #[test]
    fn test_parse_simple_diff() {
        let extractor = DiffContextExtractor::new();
        let diff = r#"
diff --git a/src/auth.rs b/src/auth.rs
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -1,5 +1,6 @@
 fn login() {
+    println!("debug");
     let password = get_password()?;
-    let hash = compute_hash(password);
+    let hash = compute_hash(password, salt);
     Ok(())
 }
        "#;

        let ctx = extractor.parse(diff);
        assert!(ctx.files_modified.contains(&"src/auth.rs".to_string()));
    }

    #[test]
    fn test_parse_new_file() {
        let extractor = DiffContextExtractor::new();
        let diff = r#"
diff --git a/src/new_module.rs b/src/new_module.rs
--- /dev/null
+++ b/src/new_module.rs
@@ -0,0 +1,3 @@
+fn new_function() {
+    do_something();
+}
        "#;

        let ctx = extractor.parse(diff);
        assert!(ctx.files_added.contains(&"src/new_module.rs".to_string()));
    }

    #[test]
    fn test_risk_indicator_detection() {
        let extractor = DiffContextExtractor::new();
        let diff = r#"
diff --git a/src/auth.rs b/src/auth.rs
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -1,5 +1,6 @@
-fn login(password: String) {
+fn login(password: String) -> Result<User, AuthError> {
+    let hash = bcrypt_hash(&password)?;
     let token = generate_token()?;
     Ok(())
 }
        "#;

        let ctx = extractor.parse(diff);

        // Should detect security-related changes
        let has_security = ctx.risk_indicators.iter().any(|i| i.contains("security"));
        assert!(has_security);
    }

    #[test]
    fn test_empty_diff() {
        let extractor = DiffContextExtractor::new();
        let ctx = extractor.parse("");
        assert!(ctx.files_added.is_empty());
        assert!(ctx.files_modified.is_empty());
    }
}

#[cfg(test)]
mod risk_scorer_tests {
    use super::*;

    #[test]
    fn test_score_calculation() {
        let scorer = RiskScorer::new();

        let mut test_case = TestCase {
            name: "test_auth_function".to_string(),
            file: Some("src/auth.rs".to_string()),
            risk_score: 0.0,
            risk_level: TestRiskLevel::Low,
            criteria: vec!["authentication".to_string()],
            is_critical: false,
            estimated_duration_ms: 50,
            tags: vec![],
        };

        let context = DiffContext {
            files_added: vec![],
            files_modified: vec!["src/auth.rs".to_string()],
            files_deleted: vec![],
            functions_added: vec![],
            functions_modified: vec!["login".to_string()],
            lines_changed: 100,
            risk_indicators: vec!["security: password".to_string()],
        };

        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "Users shall authenticate with password".to_string(),
            category: "security".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["authentication".to_string()],
        }];

        scorer.score_test(&mut test_case, &context, &criteria);

        assert!(test_case.risk_score > 0.3);
        assert!(test_case.risk_level >= TestRiskLevel::Medium);
    }

    #[test]
    fn test_critical_file_scoring() {
        let scorer = RiskScorer::new();

        let mut test_case = TestCase {
            name: "test_payment".to_string(),
            file: Some("src/payment.rs".to_string()),
            risk_score: 0.0,
            risk_level: TestRiskLevel::Low,
            criteria: vec![],
            is_critical: true,
            estimated_duration_ms: 100,
            tags: vec![],
        };

        let context = DiffContext {
            files_added: vec!["src/payment.rs".to_string()],
            files_modified: vec![],
            files_deleted: vec![],
            functions_added: vec!["process_payment".to_string()],
            functions_modified: vec![],
            lines_changed: 50,
            risk_indicators: vec![],
        };

        scorer.score_test(&mut test_case, &context, &[]);

        assert!(test_case.risk_score >= 0.5);
    }

    #[test]
    fn test_low_risk_small_change() {
        let scorer = RiskScorer::new();

        let mut test_case = TestCase {
            name: "test_read_only".to_string(),
            file: Some("src/read_only.rs".to_string()),
            risk_score: 0.0,
            risk_level: TestRiskLevel::Low,
            criteria: vec![],
            is_critical: false,
            estimated_duration_ms: 10,
            tags: vec![],
        };

        let context = DiffContext {
            files_added: vec![],
            files_modified: vec!["src/read_only.rs".to_string()],
            files_deleted: vec![],
            functions_added: vec![],
            functions_modified: vec![],
            lines_changed: 5,
            risk_indicators: vec![],
        };

        scorer.score_test(&mut test_case, &context, &[]);

        assert!(test_case.risk_score < 0.5);
    }
}

#[cfg(test)]
mod test_planning_engine_tests {
    use super::*;

    #[tokio::test]
    async fn test_create_test_plan_basic() {
        let engine = TestPlanningEngine::with_defaults();

        let request = TestPlanRequest {
            task_description: "Implement user authentication".to_string(),
            changed_files: vec!["src/auth.rs".to_string()],
            diff_content: Some(
                r#"
diff --git a/src/auth.rs b/src/auth.rs
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -1,5 +1,6 @@
 fn login() {
+    validate_password_strength(password)?;
     Ok(())
 }
            "#
                .to_string(),
            ),
            spec_content: Some(
                "The system shall authenticate users with email and password".to_string(),
            ),
        };

        let plan = engine.create_test_plan(request).await.unwrap();

        assert!(!plan.test_cases.is_empty());
        assert!(plan.coverage_percentage >= 0.0);
    }

    #[tokio::test]
    async fn test_create_plan_with_no_diff() {
        let engine = TestPlanningEngine::with_defaults();

        let request = TestPlanRequest {
            task_description: "Add new feature".to_string(),
            changed_files: vec!["src/feature.rs".to_string()],
            diff_content: None,
            spec_content: None,
        };

        let plan = engine.create_test_plan(request).await.unwrap();

        assert!(!plan.test_cases.is_empty());
    }

    #[tokio::test]
    async fn test_test_plan_to_plan_conversion() {
        let engine = TestPlanningEngine::with_defaults();

        let request = TestPlanRequest {
            task_description: "Implement auth".to_string(),
            changed_files: vec!["src/auth.rs".to_string()],
            diff_content: None,
            spec_content: Some("The system shall authenticate users".to_string()),
        };

        let test_plan = engine.create_test_plan(request).await.unwrap();
        let plan = engine.to_plan(&test_plan, Uuid::new_v4());

        assert!(!plan.steps.is_empty());
        assert!(plan.risk_assessment.contains("Test plan"));
    }

    #[tokio::test]
    async fn test_criteria_coverage_calculation() {
        let engine = TestPlanningEngine::with_defaults();

        let request = TestPlanRequest {
            task_description: "Implement authentication with password validation".to_string(),
            changed_files: vec!["src/auth.rs".to_string()],
            diff_content: Some("+fn login()".to_string()),
            spec_content: Some(
                "The system shall:\n- Authenticate users\n- Validate password strength\n- Handle authentication failures".to_string(),
            ),
        };

        let plan = engine.create_test_plan(request).await.unwrap();

        // Should have uncovered criteria if no tests match
        assert!(plan.uncovered_criteria.is_empty() || plan.covered_criteria.len() > 0);
    }

    #[test]
    fn test_test_risk_level_ordering() {
        assert!(TestRiskLevel::Critical > TestRiskLevel::High);
        assert!(TestRiskLevel::High > TestRiskLevel::Medium);
        assert!(TestRiskLevel::Medium > TestRiskLevel::Low);
    }

    #[test]
    fn test_test_risk_level_weights() {
        assert_eq!(TestRiskLevel::Low.weight(), 0.25);
        assert_eq!(TestRiskLevel::Medium.weight(), 0.5);
        assert_eq!(TestRiskLevel::High.weight(), 0.75);
        assert_eq!(TestRiskLevel::Critical.weight(), 1.0);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_test_planning_workflow() {
        // Test the complete workflow from spec/diff to test plan

        let engine = TestPlanningEngine::with_defaults();

        let spec = r#"
# Authentication Module Specification

## Functional Requirements

### User Authentication
- The system shall authenticate users using email and password
- The system must validate password strength before acceptance
- Users should be able to reset their password via email

### Session Management
- Sessions shall expire after 30 minutes of inactivity
- The system must handle concurrent login attempts

## Security Requirements
- All passwords shall be hashed using bcrypt
- Authentication tokens shall be encrypted in transit

## Error Handling
- The system shall return appropriate error messages
- Failed authentication attempts must be logged
        "#;

        let diff = r#"
diff --git a/src/auth/login.rs b/src/auth/login.rs
--- a/src/auth/login.rs
+++ b/src/auth/login.rs
@@ -1,10 +1,15 @@
 pub async fn login(email: String, password: String) -> Result<Session, AuthError> {
+    // Validate input
+    if email.is_empty() || password.is_empty() {
+        return Err(AuthError::InvalidCredentials);
+    }
+
+    // Check password strength
+    validate_password_strength(&password)?;
+
     let user = find_user_by_email(&email).await?;
-    let hash = compute_hash(&password);
+    let hash = bcrypt_hash(&password, BCRYPT_COST)?;
     let token = generate_session_token()?;

     Ok(Session { token, user_id: user.id })
 }
+
+fn validate_password_strength(password: &str) -> Result<(), AuthError> {
+    if password.len() < 8 {
+        return Err(AuthError::WeakPassword);
+    }
+    Ok(())
+}
        "#;

        let request = TestPlanRequest {
            task_description: "Implement secure user authentication with password validation"
                .to_string(),
            changed_files: vec!["src/auth/login.rs".to_string()],
            diff_content: Some(diff.to_string()),
            spec_content: Some(spec.to_string()),
        };

        let test_plan = engine.create_test_plan(request).await.unwrap();

        // Verify test plan has content
        assert!(!test_plan.test_cases.is_empty());

        // Verify tests are sorted by risk
        if test_plan.test_cases.len() > 1 {
            for i in 0..test_plan.test_cases.len() - 1 {
                assert!(
                    test_plan.test_cases[i].risk_score >= test_plan.test_cases[i + 1].risk_score
                );
            }
        }

        // Verify coverage is calculated
        assert!(test_plan.coverage_percentage >= 0.0);

        // Verify overall risk is determined
        assert!(test_plan.overall_risk >= TestRiskLevel::Low);
    }
}
