//! Predictive Test Selection for SWELL Validation.
//!
//! This module provides ML-based intelligent test selection that predicts which tests
//! are most likely to fail based on code changes and historical test outcomes.
//!
//! # Architecture
//!
//! - [`PredictiveModel`] - Machine learning model for test failure prediction
//! - [`ChangeImpactAnalyzer`] - Analyzes code changes to determine test impact
//! - [`TestSubsetSelector`] - Selects optimal test subset based on predictions
//! - [`PredictiveSelectionEngine`] - Orchestrates the prediction and selection process
//!
//! # Prediction Features
//!
//! The model considers:
//! - File change patterns (which files changed)
//! - Function-level impact (which functions were modified)
//! - Historical test failure rates per test
//! - Test coverage relationships (which tests cover which files/functions)
//! - Change size and complexity
//!
//! # Selection Strategies
//!
//! - `RiskBased` - Prioritize high-risk tests first
//! - `CoverageMaximized` - Maximize code coverage with limited tests
//! - `TimeConstrained` - Fit as many tests as possible within time budget
//! - `Hybrid` - Balance risk, coverage, and time constraints

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ============================================================================
// Predictive Model
// ============================================================================

/// A feature vector for test failure prediction
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PredictionFeatures {
    /// Number of files changed
    pub files_changed: usize,
    /// Number of functions modified
    pub functions_modified: usize,
    /// Total lines changed
    pub lines_changed: usize,
    /// Risk score of the change (0.0 to 1.0)
    pub change_risk_score: f64,
    /// Whether changes affect core modules
    pub affects_core: bool,
    /// Whether changes affect test files
    pub affects_tests: bool,
    /// Whether changes are in high-risk categories (auth, data, etc.)
    pub high_risk_category: bool,
    /// Average historical failure rate of related tests
    pub historical_failure_rate: f64,
    /// Time since last test run (in hours)
    pub time_since_last_run: f64,
}

/// Prediction result for a single test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPrediction {
    /// Test name (fully qualified)
    pub test_name: String,
    /// Predicted probability of failure (0.0 to 1.0)
    pub failure_probability: f64,
    /// Confidence in the prediction (0.0 to 1.0)
    pub confidence: f64,
    /// Reasons contributing to the prediction
    pub contributing_factors: Vec<String>,
}

/// Historical test outcome record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestHistoryRecord {
    /// Test name
    pub test_name: String,
    /// Whether the test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Files that were changed in this run
    pub changed_files: Vec<String>,
    /// Functions that were modified
    pub modified_functions: Vec<String>,
}

/// Predictive model for test failure prediction
#[derive(Debug, Clone)]
pub struct PredictiveModel {
    /// Historical test records
    history: Vec<TestHistoryRecord>,
    /// Per-test failure counts
    test_failure_counts: HashMap<String, usize>,
    /// Per-test total counts
    test_total_counts: HashMap<String, usize>,
    /// Per-file change frequency
    file_change_frequency: HashMap<String, usize>,
    /// Per-function change frequency
    function_change_frequency: HashMap<String, usize>,
    /// File-to-test mapping (which tests cover which files)
    file_test_coverage: HashMap<String, HashSet<String>>,
    /// Function-to-test mapping (which tests exercise which functions)
    function_test_coverage: HashMap<String, HashSet<String>>,
    /// Default failure probability when no history
    default_failure_probability: f64,
    /// Decay factor for old history (0.0 to 1.0, lower = more weight on recent)
    #[allow(dead_code)]
    recency_decay: f64,
}

impl Default for PredictiveModel {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            test_failure_counts: HashMap::new(),
            test_total_counts: HashMap::new(),
            file_change_frequency: HashMap::new(),
            function_change_frequency: HashMap::new(),
            file_test_coverage: HashMap::new(),
            function_test_coverage: HashMap::new(),
            default_failure_probability: 0.1,
            recency_decay: 0.9,
        }
    }
}

impl PredictiveModel {
    /// Create a new predictive model with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with custom configuration
    pub fn with_config(default_failure_probability: f64, recency_decay: f64) -> Self {
        Self {
            default_failure_probability,
            recency_decay,
            ..Default::default()
        }
    }

    /// Add a historical test record
    pub fn add_record(&mut self, record: TestHistoryRecord) {
        // Update history
        self.history.push(record.clone());

        // Update failure counts
        let total = self
            .test_total_counts
            .entry(record.test_name.clone())
            .or_insert(0);
        *total += 1;
        if !record.passed {
            let failures = self
                .test_failure_counts
                .entry(record.test_name.clone())
                .or_insert(0);
            *failures += 1;
        }

        // Update file change frequency
        for file in &record.changed_files {
            *self.file_change_frequency.entry(file.clone()).or_insert(0) += 1;
        }

        // Update function change frequency
        for func in &record.modified_functions {
            *self
                .function_change_frequency
                .entry(func.clone())
                .or_insert(0) += 1;
        }
    }

    /// Register a test as covering specific files
    pub fn register_file_coverage(&mut self, test_name: &str, files: &[String]) {
        for file in files {
            let tests = self.file_test_coverage.entry(file.clone()).or_default();
            tests.insert(test_name.to_string());
        }
    }

    /// Register a test as exercising specific functions
    pub fn register_function_coverage(&mut self, test_name: &str, functions: &[String]) {
        for func in functions {
            let tests = self.function_test_coverage.entry(func.clone()).or_default();
            tests.insert(test_name.to_string());
        }
    }

    /// Compute features for a change
    pub fn compute_features(&self, change: &ChangeImpact) -> PredictionFeatures {
        let files_changed = change.modified_files.len() + change.added_files.len();
        let functions_modified = change.modified_functions.len();
        let lines_changed = change.lines_changed;

        // Determine if changes affect core modules
        let core_patterns = ["core", "auth", "payment", "data", "security", "kernel"];
        let affects_core = change
            .modified_files
            .iter()
            .any(|f| core_patterns.iter().any(|p| f.to_lowercase().contains(p)));

        // Determine if changes affect tests
        let affects_tests = change
            .modified_files
            .iter()
            .any(|f| f.ends_with("_test.rs") || f.ends_with(".test.ts") || f.contains("/tests/"));

        // Determine if high-risk category
        let high_risk_patterns = ["auth", "security", "payment", "validation", "crypto"];
        let high_risk_category = change.modified_files.iter().any(|f| {
            high_risk_patterns
                .iter()
                .any(|p| f.to_lowercase().contains(p))
        }) || change.modified_functions.iter().any(|f| {
            high_risk_patterns
                .iter()
                .any(|p| f.to_lowercase().contains(p))
        });

        // Compute historical failure rate for affected tests
        let affected_tests = self.get_affected_tests(change);
        let historical_failure_rate = if affected_tests.is_empty() {
            self.default_failure_probability
        } else {
            let total_failures: usize = affected_tests
                .iter()
                .filter_map(|t| self.test_failure_counts.get(t).copied())
                .sum();
            let total_runs: usize = affected_tests
                .iter()
                .filter_map(|t| self.test_total_counts.get(t).copied())
                .sum();
            if total_runs == 0 {
                self.default_failure_probability
            } else {
                total_failures as f64 / total_runs as f64
            }
        };

        // Compute time since last run (simplified - would need actual timestamps)
        let time_since_last_run = 0.0; // Placeholder

        PredictionFeatures {
            files_changed,
            functions_modified,
            lines_changed,
            change_risk_score: change.risk_score,
            affects_core,
            affects_tests,
            high_risk_category,
            historical_failure_rate,
            time_since_last_run,
        }
    }

    /// Get tests that might be affected by a change
    pub fn get_affected_tests(&self, change: &ChangeImpact) -> Vec<String> {
        let mut affected = HashSet::new();

        // Tests covering changed files
        for file in &change.modified_files {
            if let Some(tests) = self.file_test_coverage.get(file) {
                affected.extend(tests.clone());
            }
            // Also check for partial file matches
            for (cov_file, tests) in &self.file_test_coverage {
                if file.contains(cov_file) || cov_file.contains(file) {
                    affected.extend(tests.clone());
                }
            }
        }

        // Tests exercising modified functions
        for func in &change.modified_functions {
            if let Some(tests) = self.function_test_coverage.get(func) {
                affected.extend(tests.clone());
            }
        }

        // Also add tests for added files
        for file in &change.added_files {
            if let Some(tests) = self.file_test_coverage.get(file) {
                affected.extend(tests.clone());
            }
        }

        affected.into_iter().collect()
    }

    /// Predict failure probability for a test given features
    pub fn predict_failure(&self, features: &PredictionFeatures) -> f64 {
        // Simple linear model (in practice, would use a trained model)
        // Weights learned from historical patterns
        let base_rate = self.default_failure_probability;

        // Increase probability based on change characteristics
        let mut probability = base_rate;

        // More files changed = higher risk
        if features.files_changed > 5 {
            probability += 0.15;
        } else if features.files_changed > 2 {
            probability += 0.1;
        } else if features.files_changed > 0 {
            probability += 0.05;
        }

        // More functions modified = higher risk
        if features.functions_modified > 10 {
            probability += 0.2;
        } else if features.functions_modified > 5 {
            probability += 0.1;
        } else if features.functions_modified > 0 {
            probability += 0.05;
        }

        // More lines changed = higher risk
        if features.lines_changed > 500 {
            probability += 0.15;
        } else if features.lines_changed > 100 {
            probability += 0.1;
        } else if features.lines_changed > 50 {
            probability += 0.05;
        }

        // Core changes are higher risk
        if features.affects_core {
            probability += 0.15;
        }

        // High-risk category changes
        if features.high_risk_category {
            probability += 0.2;
        }

        // Incorporate historical failure rate
        probability = probability * 0.7 + features.historical_failure_rate * 0.3;

        probability.clamp(0.0, 1.0)
    }

    /// Predict failure probability for all tests affected by a change
    pub fn predict_for_change(&self, change: &ChangeImpact) -> Vec<TestPrediction> {
        let features = self.compute_features(change);
        let affected_tests = self.get_affected_tests(change);

        affected_tests
            .into_iter()
            .map(|test_name| {
                // Compute test-specific features
                let mut test_features = features.clone();
                test_features.historical_failure_rate = self
                    .get_test_failure_rate(&test_name)
                    .unwrap_or(self.default_failure_probability);

                let failure_prob = self.predict_failure(&test_features);

                // Compute confidence based on amount of history
                let confidence = self.compute_confidence(&test_name);

                // Generate contributing factors
                let contributing_factors = self.get_contributing_factors(&test_name, &features);

                TestPrediction {
                    test_name,
                    failure_probability: failure_prob,
                    confidence,
                    contributing_factors,
                }
            })
            .collect()
    }

    /// Get failure rate for a specific test
    pub fn get_test_failure_rate(&self, test_name: &str) -> Option<f64> {
        let failures = self
            .test_failure_counts
            .get(test_name)
            .copied()
            .unwrap_or(0);
        let total = self.test_total_counts.get(test_name).copied().unwrap_or(0);
        if total == 0 {
            None
        } else {
            Some(failures as f64 / total as f64)
        }
    }

    /// Compute confidence in prediction based on history amount
    fn compute_confidence(&self, test_name: &str) -> f64 {
        let total = self.test_total_counts.get(test_name).copied().unwrap_or(0);

        if total == 0 {
            0.3 // Low confidence with no history
        } else if total < 5 {
            0.5
        } else if total < 20 {
            0.7
        } else {
            0.9
        }
    }

    /// Get factors contributing to a test's prediction
    fn get_contributing_factors(
        &self,
        test_name: &str,
        features: &PredictionFeatures,
    ) -> Vec<String> {
        let mut factors = Vec::new();

        if features.files_changed > 5 {
            factors.push(format!(
                "High file change count ({})",
                features.files_changed
            ));
        }

        if features.functions_modified > 5 {
            factors.push(format!(
                "Many functions modified ({})",
                features.functions_modified
            ));
        }

        if features.affects_core {
            factors.push("Affects core modules".to_string());
        }

        if features.high_risk_category {
            factors.push("High-risk change category".to_string());
        }

        if let Some(rate) = self.get_test_failure_rate(test_name) {
            if rate > 0.5 {
                factors.push(format!(
                    "High historical failure rate ({:.1}%)",
                    rate * 100.0
                ));
            } else if rate > 0.2 {
                factors.push(format!(
                    "Moderate historical failure rate ({:.1}%)",
                    rate * 100.0
                ));
            }
        }

        if factors.is_empty() {
            factors.push("General code change detected".to_string());
        }

        factors
    }

    /// Get the most frequently changed files
    pub fn get_hotspot_files(&self, limit: usize) -> Vec<(String, usize)> {
        let mut files: Vec<_> = self.file_change_frequency.iter().collect();
        files.sort_by(|a, b| b.1.cmp(a.1));
        files
            .into_iter()
            .take(limit)
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// Get the most frequently failing tests
    pub fn get_flaky_tests(&self, min_runs: usize) -> Vec<(String, f64)> {
        self.test_total_counts
            .iter()
            .filter(|(_, &total)| total >= min_runs)
            .filter_map(|(test, &total)| {
                let failures = self.test_failure_counts.get(test).copied().unwrap_or(0);
                let rate = failures as f64 / total as f64;
                if rate > 0.1 {
                    Some((test.clone(), rate))
                } else {
                    None
                }
            })
            .collect()
    }
}

// ============================================================================
// Change Impact Analysis
// ============================================================================

/// Represents the impact of a code change
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeImpact {
    /// Files that were added
    pub added_files: Vec<String>,
    /// Files that were modified
    pub modified_files: Vec<String>,
    /// Files that were deleted
    pub deleted_files: Vec<String>,
    /// Functions that were added
    pub added_functions: Vec<String>,
    /// Functions that were modified
    pub modified_functions: Vec<String>,
    /// Functions that were deleted
    pub deleted_functions: Vec<String>,
    /// Lines added
    pub lines_added: usize,
    /// Lines deleted
    pub lines_deleted: usize,
    /// Lines modified
    pub lines_modified: usize,
    /// Total lines changed
    pub lines_changed: usize,
    /// Risk score (0.0 to 1.0)
    pub risk_score: f64,
    /// Impact categories affected
    pub impact_categories: Vec<ImpactCategory>,
    /// Detailed change patterns
    pub patterns: Vec<ChangePattern>,
}

/// Categories of impact
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImpactCategory {
    /// Authentication and authorization
    Authentication,
    /// Data handling and storage
    Data,
    /// API contracts
    Api,
    /// Business logic
    BusinessLogic,
    /// User interface
    Ui,
    /// Performance
    Performance,
    /// Security
    Security,
    /// Configuration
    Config,
    /// Infrastructure
    Infrastructure,
    /// Unknown
    Unknown,
}

impl ImpactCategory {
    /// Determine category from file path
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_lowercase();

        if lower.contains("auth") || lower.contains("login") || lower.contains("session") {
            ImpactCategory::Authentication
        } else if lower.contains("data")
            || lower.contains("db")
            || lower.contains("store")
            || lower.contains("model")
        {
            ImpactCategory::Data
        } else if lower.contains("api") || lower.contains("endpoint") || lower.contains("route") {
            ImpactCategory::Api
        } else if lower.contains("service") || lower.contains("logic") || lower.contains("business")
        {
            ImpactCategory::BusinessLogic
        } else if lower.contains("ui")
            || lower.contains("view")
            || lower.contains("component")
            || lower.contains("frontend")
        {
            ImpactCategory::Ui
        } else if lower.contains("perf") || lower.contains("cache") || lower.contains("optim") {
            ImpactCategory::Performance
        } else if lower.contains("security") || lower.contains("crypto") || lower.contains("valid")
        {
            ImpactCategory::Security
        } else if lower.contains("config") || lower.contains("settings") || lower.contains("env") {
            ImpactCategory::Config
        } else if lower.contains("infra") || lower.contains("deploy") || lower.contains("docker") {
            ImpactCategory::Infrastructure
        } else {
            ImpactCategory::Unknown
        }
    }
}

/// Patterns detected in changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangePattern {
    /// Bug fix
    BugFix,
    /// New feature
    Feature,
    /// Refactoring
    Refactor,
    /// Performance improvement
    PerfOptimization,
    /// Security patch
    SecurityPatch,
    /// Documentation update
    Documentation,
    /// Test update
    TestUpdate,
    /// Dependency update
    DependencyUpdate,
    /// Unknown
    Unknown,
}

impl ChangePattern {
    /// Detect pattern from commit message or diff
    pub fn from_message(message: &str) -> Self {
        let lower = message.to_lowercase();

        if lower.contains("fix") || lower.contains("bug") || lower.contains("patch") {
            ChangePattern::BugFix
        } else if lower.contains("feat") || lower.contains("implement") || lower.contains("add") {
            ChangePattern::Feature
        } else if lower.contains("refactor") || lower.contains("restructure") {
            ChangePattern::Refactor
        } else if lower.contains("perf") || lower.contains("optim") || lower.contains("speed") {
            ChangePattern::PerfOptimization
        } else if lower.contains("security") || lower.contains("vuln") || lower.contains("cve") {
            ChangePattern::SecurityPatch
        } else if lower.contains("doc") || lower.contains("readme") || lower.contains("comment") {
            ChangePattern::Documentation
        } else if lower.contains("test") {
            ChangePattern::TestUpdate
        } else if lower.contains("dep") || lower.contains("bump") || lower.contains("upgrade") {
            ChangePattern::DependencyUpdate
        } else {
            ChangePattern::Unknown
        }
    }
}

/// Change impact analyzer
#[derive(Debug, Clone)]
pub struct ChangeImpactAnalyzer {
    /// Known test file patterns
    test_patterns: Vec<String>,
    /// Known source file patterns
    #[allow(dead_code)]
    source_patterns: Vec<String>,
}

impl Default for ChangeImpactAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangeImpactAnalyzer {
    /// Create a new analyzer with default patterns
    pub fn new() -> Self {
        Self {
            test_patterns: vec![
                "_test.rs".to_string(),
                "_tests.rs".to_string(),
                ".test.ts".to_string(),
                ".tests.ts".to_string(),
                "/tests/".to_string(),
                "/test/".to_string(),
                "spec.rs".to_string(),
            ],
            source_patterns: vec![
                ".rs".to_string(),
                ".ts".to_string(),
                ".js".to_string(),
                ".py".to_string(),
                ".go".to_string(),
            ],
        }
    }

    /// Analyze a git diff and extract impact information
    pub fn analyze_diff(&self, diff: &str) -> ChangeImpact {
        let mut impact = ChangeImpact::default();

        let mut current_file = None::<String>;
        let mut _in_hunk = false;
        let mut lines_added = 0;
        let mut lines_deleted = 0;

        for line in diff.lines() {
            // New file diff
            if line.starts_with("diff --git") {
                // Save previous file stats
                if let Some(ref _file) = current_file {
                    impact.lines_changed =
                        impact.lines_added + impact.lines_deleted + lines_added + lines_deleted;
                    impact.lines_added += lines_added;
                    impact.lines_deleted += lines_deleted;
                }

                // Parse new file
                if let Some(path) = line.strip_prefix("diff --git a/") {
                    let path = path.split_whitespace().next().unwrap_or(path);
                    let clean_path = path.trim_start_matches("b/");
                    current_file = Some(clean_path.to_string());

                    // Reset per-file counters
                    lines_added = 0;
                    lines_deleted = 0;
                }
            }
            // File mode changes
            else if line.starts_with("new file mode") {
                if let Some(ref file) = current_file {
                    impact.added_files.push(file.clone());
                }
            }
            // Deleted file
            else if line.starts_with("deleted file mode") {
                if let Some(ref _file) = current_file {
                    impact.deleted_files.push(_file.clone());
                }
            }
            // Hunk header - indicates modifications
            else if line.starts_with("@@") {
                _in_hunk = true;
            }
            // Addition
            else if line.starts_with('+') && !line.starts_with("+++") {
                lines_added += 1;
            }
            // Deletion
            else if line.starts_with('-') && !line.starts_with("---") {
                lines_deleted += 1;
            }
        }

        // Don't forget the last file
        if let Some(ref file) = current_file {
            if !impact.added_files.contains(file) && !impact.deleted_files.contains(file) {
                impact.modified_files.push(file.clone());
            }
            impact.lines_changed += lines_added + lines_deleted;
            impact.lines_added += lines_added;
            impact.lines_deleted += lines_deleted;
        }

        // Compute risk score
        impact.risk_score = self.compute_risk_score(&impact);

        // Determine impact categories
        impact.impact_categories = self.determine_categories(&impact);

        // Detect change patterns
        impact.patterns = vec![ChangePattern::Unknown];

        impact
    }

    /// Analyze changed files (without full diff)
    pub fn analyze_files(&self, files: &[String]) -> ChangeImpact {
        let mut impact = ChangeImpact::default();

        for file in files {
            if file.starts_with("+") || file.starts_with("A ") {
                impact
                    .added_files
                    .push(file.trim_start_matches("+ ").to_string());
            } else if file.starts_with("-") || file.starts_with("D ") {
                impact
                    .deleted_files
                    .push(file.trim_start_matches("- ").to_string());
            } else if file.starts_with("M ") || file.starts_with("M\t") || file.contains(" -> ") {
                // Modified file
                let clean = file
                    .trim_start_matches("M ")
                    .trim_start_matches("M\t")
                    .to_string();
                if let Some(ref arrow_part) = file.split(" -> ").nth(1) {
                    impact.modified_files.push(arrow_part.to_string());
                } else {
                    impact.modified_files.push(clean);
                }
            } else {
                // Assume modified if unknown format
                impact.modified_files.push(file.clone());
            }
        }

        // Estimate lines changed (placeholder - would need actual diff)
        impact.lines_changed = impact.modified_files.len() * 10; // Rough estimate
        impact.lines_added = impact.lines_changed / 2;
        impact.lines_deleted = impact.lines_changed / 2;

        // Compute risk score
        impact.risk_score = self.compute_risk_score(&impact);

        // Determine impact categories
        impact.impact_categories = self.determine_categories(&impact);

        impact
    }

    /// Compute risk score from change impact
    fn compute_risk_score(&self, impact: &ChangeImpact) -> f64 {
        let mut score: f64 = 0.0;

        // More files = higher risk
        let total_files =
            impact.added_files.len() + impact.modified_files.len() + impact.deleted_files.len();
        if total_files > 20 {
            score += 0.3;
        } else if total_files > 10 {
            score += 0.2;
        } else if total_files > 5 {
            score += 0.1;
        }

        // More lines changed = higher risk
        if impact.lines_changed > 1000 {
            score += 0.3;
        } else if impact.lines_changed > 500 {
            score += 0.2;
        } else if impact.lines_changed > 100 {
            score += 0.1;
        }

        // Core file changes are higher risk
        let core_files: HashSet<_> = impact
            .modified_files
            .iter()
            .filter(|f| {
                let lower = f.to_lowercase();
                lower.contains("core")
                    || lower.contains("auth")
                    || lower.contains("payment")
                    || lower.contains("data")
            })
            .collect();

        if !core_files.is_empty() {
            score += 0.25;
        }

        // Test file changes are lower risk
        let test_changes = impact
            .modified_files
            .iter()
            .filter(|f| self.is_test_file(f))
            .count();
        if test_changes > 0 && test_changes == total_files {
            score -= 0.1;
        }

        score.clamp(0.0, 1.0)
    }

    /// Determine impact categories from files
    fn determine_categories(&self, impact: &ChangeImpact) -> Vec<ImpactCategory> {
        let mut categories = HashSet::new();

        for file in &impact.modified_files {
            categories.insert(ImpactCategory::from_path(file));
        }
        for file in &impact.added_files {
            categories.insert(ImpactCategory::from_path(file));
        }

        let mut result: Vec<_> = categories.into_iter().collect();
        result.sort_by(|a, b| {
            let order = |c: &ImpactCategory| match c {
                ImpactCategory::Security => 0,
                ImpactCategory::Authentication => 1,
                ImpactCategory::Data => 2,
                ImpactCategory::Api => 3,
                ImpactCategory::BusinessLogic => 4,
                _ => 5,
            };
            order(a).cmp(&order(b))
        });

        if result.is_empty() {
            result.push(ImpactCategory::Unknown);
        }

        result
    }

    /// Check if a file is a test file
    fn is_test_file(&self, path: &str) -> bool {
        self.test_patterns.iter().any(|p| path.contains(p))
    }
}

// ============================================================================
// Test Subset Selection
// ============================================================================

/// Strategy for test subset selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionStrategy {
    /// Prioritize high-risk tests first
    RiskBased,
    /// Maximize code coverage with limited tests
    CoverageMaximized,
    /// Fit as many tests as possible within time budget
    TimeConstrained,
    /// Balance risk, coverage, and time
    Hybrid,
}

#[allow(clippy::derivable_impls)]
impl Default for SelectionStrategy {
    fn default() -> Self {
        Self::Hybrid
    }
}

/// Selection constraint
#[derive(Debug, Clone, Default)]
pub struct SelectionConstraint {
    /// Maximum number of tests to run
    pub max_tests: Option<usize>,
    /// Maximum total time in milliseconds
    pub max_time_ms: Option<u64>,
    /// Minimum coverage percentage to achieve
    pub min_coverage: Option<f64>,
    /// Required test categories
    pub required_categories: Vec<String>,
}

/// Result of test subset selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedTestSubset {
    /// Selected tests in priority order
    pub selected_tests: Vec<SelectedTest>,
    /// Total estimated time in milliseconds
    pub estimated_time_ms: u64,
    /// Estimated coverage percentage
    pub estimated_coverage: f64,
    /// Tests that were skipped
    pub skipped_tests: Vec<String>,
    /// Selection strategy used
    pub strategy: SelectionStrategy,
    /// Selection rationale
    pub rationale: String,
}

/// A selected test with priority information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedTest {
    /// Test name
    pub name: String,
    /// Priority score (higher = more important)
    pub priority_score: f64,
    /// Estimated duration in milliseconds
    pub estimated_duration_ms: u64,
    /// Files covered by this test
    pub files_covered: Vec<String>,
    /// Priority reason
    pub reason: String,
}

/// Test subset selector
#[derive(Debug, Clone, Default)]
pub struct TestSubsetSelector {
    /// Selection strategy
    strategy: SelectionStrategy,
    /// Known test durations
    test_durations: HashMap<String, u64>,
    /// File coverage map
    file_coverage: HashMap<String, HashSet<String>>,
}

impl TestSubsetSelector {
    /// Create a new selector with default strategy (Hybrid)
    pub fn new() -> Self {
        Self::with_strategy(SelectionStrategy::Hybrid)
    }

    /// Create with specific strategy
    pub fn with_strategy(strategy: SelectionStrategy) -> Self {
        Self {
            strategy,
            test_durations: HashMap::new(),
            file_coverage: HashMap::new(),
        }
    }

    /// Register expected test duration
    pub fn register_duration(&mut self, test_name: &str, duration_ms: u64) {
        self.test_durations
            .insert(test_name.to_string(), duration_ms);
    }

    /// Register file coverage for a test
    pub fn register_coverage(&mut self, test_name: &str, files: &[String]) {
        for file in files {
            let tests = self.file_coverage.entry(file.clone()).or_default();
            tests.insert(test_name.to_string());
        }
    }

    /// Select optimal test subset given predictions and constraints
    pub fn select(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        match self.strategy {
            SelectionStrategy::RiskBased => self.select_by_risk(predictions, constraint),
            SelectionStrategy::CoverageMaximized => {
                self.select_for_coverage(predictions, constraint)
            }
            SelectionStrategy::TimeConstrained => self.select_within_time(predictions, constraint),
            SelectionStrategy::Hybrid => self.select_hybrid(predictions, constraint),
        }
    }

    /// Select tests prioritizing high risk
    fn select_by_risk(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        let mut sorted: Vec<_> = predictions.to_vec();
        sorted.sort_by(|a, b| {
            b.failure_probability
                .partial_cmp(&a.failure_probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected = Vec::new();
        let mut estimated_time = 0u64;
        let mut skipped = Vec::new();

        for pred in &sorted {
            // Check constraints
            if let Some(max) = constraint.max_tests {
                if selected.len() >= max {
                    skipped.push(pred.test_name.clone());
                    continue;
                }
            }

            if let Some(max_time) = constraint.max_time_ms {
                let duration = self
                    .test_durations
                    .get(&pred.test_name)
                    .copied()
                    .unwrap_or(100);
                if estimated_time + duration > max_time {
                    skipped.push(pred.test_name.clone());
                    continue;
                }
                estimated_time += duration;
            }

            let reason = if pred.failure_probability > 0.5 {
                format!(
                    "High failure risk ({:.0}%), confidence {:.0}%",
                    pred.failure_probability * 100.0,
                    pred.confidence * 100.0
                )
            } else {
                format!(
                    "Moderate failure risk ({:.0}%)",
                    pred.failure_probability * 100.0
                )
            };

            selected.push(SelectedTest {
                name: pred.test_name.clone(),
                priority_score: pred.failure_probability,
                estimated_duration_ms: self
                    .test_durations
                    .get(&pred.test_name)
                    .copied()
                    .unwrap_or(100),
                files_covered: Vec::new(),
                reason,
            });
        }

        SelectedTestSubset {
            estimated_time_ms: estimated_time,
            estimated_coverage: self.estimate_coverage(&selected),
            selected_tests: selected,
            skipped_tests: skipped,
            strategy: SelectionStrategy::RiskBased,
            rationale: "Selected tests by predicted failure risk".to_string(),
        }
    }

    /// Select tests maximizing coverage
    fn select_for_coverage(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        let mut selected = Vec::new();
        let mut covered_files = HashSet::new();
        let mut estimated_time = 0u64;
        let mut skipped = Vec::new();

        // Sort by coverage potential (files covered / duration)
        let mut with_coverage: Vec<_> = predictions
            .iter()
            .map(|pred| {
                let files: Vec<_> = self
                    .file_coverage
                    .iter()
                    .filter(|(_, tests)| tests.contains(&pred.test_name))
                    .map(|(f, _)| f.clone())
                    .collect();
                let duration = self
                    .test_durations
                    .get(&pred.test_name)
                    .copied()
                    .unwrap_or(100);
                let coverage_score = if duration > 0 {
                    files.len() as f64 / duration as f64 * 1000.0
                } else {
                    0.0
                };
                (pred, files, duration, coverage_score)
            })
            .collect();

        with_coverage.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

        for (pred, files, duration, _) in with_coverage {
            // Skip if all files already covered - compute new files first
            let mut new_files_count = 0;
            for f in &files {
                if !covered_files.contains(f) {
                    new_files_count += 1;
                    covered_files.insert(f.clone());
                }
            }

            if new_files_count == 0 && constraint.max_tests.is_none() {
                skipped.push(pred.test_name.clone());
                continue;
            }

            // Check constraints
            if let Some(max) = constraint.max_tests {
                if selected.len() >= max {
                    skipped.push(pred.test_name.clone());
                    continue;
                }
            }

            if let Some(max_time) = constraint.max_time_ms {
                if estimated_time + duration > max_time {
                    skipped.push(pred.test_name.clone());
                    continue;
                }
            }

            estimated_time += duration;

            selected.push(SelectedTest {
                name: pred.test_name.clone(),
                priority_score: pred.failure_probability,
                estimated_duration_ms: duration,
                files_covered: files,
                reason: format!("Covers {} new file(s)", new_files_count),
            });
        }

        SelectedTestSubset {
            estimated_time_ms: estimated_time,
            estimated_coverage: self.estimate_coverage(&selected),
            selected_tests: selected,
            skipped_tests: skipped,
            strategy: SelectionStrategy::CoverageMaximized,
            rationale: "Selected tests to maximize code coverage".to_string(),
        }
    }

    /// Select tests within time constraint
    fn select_within_time(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        let max_time = constraint.max_time_ms.unwrap_or(u64::MAX);
        let max_tests = constraint.max_tests.unwrap_or(usize::MAX);

        // Sort by failure probability
        let mut sorted: Vec<_> = predictions.to_vec();
        sorted.sort_by(|a, b| {
            b.failure_probability
                .partial_cmp(&a.failure_probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected = Vec::new();
        let mut estimated_time = 0u64;
        let mut skipped = Vec::new();

        for pred in &sorted {
            let duration = self
                .test_durations
                .get(&pred.test_name)
                .copied()
                .unwrap_or(100);

            // Check if we can fit this test
            if selected.len() >= max_tests {
                skipped.push(pred.test_name.clone());
                continue;
            }

            if estimated_time + duration > max_time {
                skipped.push(pred.test_name.clone());
                continue;
            }

            estimated_time += duration;
            selected.push(SelectedTest {
                name: pred.test_name.clone(),
                priority_score: pred.failure_probability,
                estimated_duration_ms: duration,
                files_covered: Vec::new(),
                reason: format!(
                    "Within time budget, risk {:.0}%",
                    pred.failure_probability * 100.0
                ),
            });
        }

        SelectedTestSubset {
            estimated_time_ms: estimated_time,
            estimated_coverage: self.estimate_coverage(&selected),
            selected_tests: selected,
            skipped_tests: skipped,
            strategy: SelectionStrategy::TimeConstrained,
            rationale: format!("Selected tests to fit within {}ms time budget", max_time),
        }
    }

    /// Hybrid selection balancing risk, coverage, and time
    fn select_hybrid(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        let max_time = constraint.max_time_ms.unwrap_or(u64::MAX);
        let max_tests = constraint.max_tests.unwrap_or(usize::MAX);

        // Compute composite score: risk * 0.5 + coverage * 0.3 + recency * 0.2
        let mut scored: Vec<_> = predictions
            .iter()
            .map(|pred| {
                let risk_score = pred.failure_probability;
                let files: HashSet<_> = self
                    .file_coverage
                    .iter()
                    .filter(|(_, tests)| tests.contains(&pred.test_name))
                    .map(|(f, _)| f.clone())
                    .collect();
                let coverage_score = files.len() as f64 / 100.0; // Normalize
                let duration = self
                    .test_durations
                    .get(&pred.test_name)
                    .copied()
                    .unwrap_or(100);
                let time_score = if max_time > 0 {
                    1.0 - (duration as f64 / max_time as f64).min(1.0)
                } else {
                    0.5
                };

                let composite = risk_score * 0.5 + coverage_score.min(1.0) * 0.3 + time_score * 0.2;

                (pred, composite, files, duration)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected = Vec::new();
        let mut estimated_time = 0u64;
        let mut skipped = Vec::new();

        for (pred, score, files, duration) in scored {
            if selected.len() >= max_tests {
                skipped.push(pred.test_name.clone());
                continue;
            }

            if estimated_time + duration > max_time {
                skipped.push(pred.test_name.clone());
                continue;
            }

            estimated_time += duration;
            selected.push(SelectedTest {
                name: pred.test_name.clone(),
                priority_score: pred.failure_probability,
                estimated_duration_ms: duration,
                files_covered: files.into_iter().collect(),
                reason: format!(
                    "Hybrid score {:.2} (risk {:.0}%)",
                    score,
                    pred.failure_probability * 100.0
                ),
            });
        }

        SelectedTestSubset {
            estimated_time_ms: estimated_time,
            estimated_coverage: self.estimate_coverage(&selected),
            selected_tests: selected,
            skipped_tests: skipped,
            strategy: SelectionStrategy::Hybrid,
            rationale: "Selected tests using hybrid risk-coverage-time strategy".to_string(),
        }
    }

    /// Estimate coverage percentage
    fn estimate_coverage(&self, selected: &[SelectedTest]) -> f64 {
        if self.file_coverage.is_empty() {
            return 0.0;
        }

        let mut covered = HashSet::new();
        for test in selected {
            covered.extend(test.files_covered.iter());
        }

        let total_files = self.file_coverage.len();
        if total_files == 0 {
            0.0
        } else {
            (covered.len() as f64 / total_files as f64) * 100.0
        }
    }
}

// ============================================================================
// Predictive Selection Engine
// ============================================================================

/// Engine that orchestrates predictive test selection
#[derive(Debug, Clone)]
pub struct PredictiveSelectionEngine {
    /// Predictive model
    model: PredictiveModel,
    /// Change impact analyzer
    impact_analyzer: ChangeImpactAnalyzer,
    /// Test subset selector
    selector: TestSubsetSelector,
}

impl Default for PredictiveSelectionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PredictiveSelectionEngine {
    /// Create a new engine with defaults
    pub fn new() -> Self {
        Self {
            model: PredictiveModel::new(),
            impact_analyzer: ChangeImpactAnalyzer::new(),
            selector: TestSubsetSelector::new(),
        }
    }

    /// Create with custom model configuration
    pub fn with_model_config(default_failure_probability: f64, recency_decay: f64) -> Self {
        Self {
            model: PredictiveModel::with_config(default_failure_probability, recency_decay),
            impact_analyzer: ChangeImpactAnalyzer::new(),
            selector: TestSubsetSelector::new(),
        }
    }

    /// Create with custom selection strategy
    pub fn with_strategy(strategy: SelectionStrategy) -> Self {
        Self {
            model: PredictiveModel::new(),
            impact_analyzer: ChangeImpactAnalyzer::new(),
            selector: TestSubsetSelector::with_strategy(strategy),
        }
    }

    /// Add historical test record
    pub fn add_history(&mut self, record: TestHistoryRecord) {
        self.model.add_record(record);
    }

    /// Register file coverage for a test
    pub fn register_coverage(&mut self, test_name: &str, files: &[String]) {
        self.model.register_file_coverage(test_name, files);
        self.selector.register_coverage(test_name, files);
    }

    /// Register function coverage for a test
    pub fn register_function_coverage(&mut self, test_name: &str, functions: &[String]) {
        self.model.register_function_coverage(test_name, functions);
    }

    /// Register test duration
    pub fn register_duration(&mut self, test_name: &str, duration_ms: u64) {
        self.selector.register_duration(test_name, duration_ms);
    }

    /// Analyze change impact from a git diff
    pub fn analyze_impact_from_diff(&self, diff: &str) -> ChangeImpact {
        self.impact_analyzer.analyze_diff(diff)
    }

    /// Analyze change impact from file list
    pub fn analyze_impact_from_files(&self, files: &[String]) -> ChangeImpact {
        self.impact_analyzer.analyze_files(files)
    }

    /// Predict which tests are likely to fail
    pub fn predict_failures(&self, impact: &ChangeImpact) -> Vec<TestPrediction> {
        self.model.predict_for_change(impact)
    }

    /// Select optimal test subset
    pub fn select_tests(
        &self,
        predictions: &[TestPrediction],
        constraint: &SelectionConstraint,
    ) -> SelectedTestSubset {
        self.selector.select(predictions, constraint)
    }

    /// Full pipeline: analyze impact, predict failures, select tests
    pub fn select_for_change(
        &self,
        diff: Option<&str>,
        files: Option<&[String]>,
        constraint: &SelectionConstraint,
    ) -> PredictiveSelectionResult {
        // Analyze impact
        let impact = if let Some(d) = diff {
            self.analyze_impact_from_diff(d)
        } else if let Some(f) = files {
            self.analyze_impact_from_files(f)
        } else {
            ChangeImpact::default()
        };

        // Predict failures
        let predictions = self.predict_failures(&impact);

        // Select tests
        let selection = self.select_tests(&predictions, constraint);

        PredictiveSelectionResult {
            impact,
            predictions,
            selection,
        }
    }

    /// Get hotspot files (most frequently changed)
    pub fn get_hotspot_files(&self, limit: usize) -> Vec<(String, usize)> {
        self.model.get_hotspot_files(limit)
    }

    /// Get flaky tests (frequently failing)
    pub fn get_flaky_tests(&self, min_runs: usize) -> Vec<(String, f64)> {
        self.model.get_flaky_tests(min_runs)
    }
}

/// Result of full predictive selection pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictiveSelectionResult {
    /// Change impact analysis
    pub impact: ChangeImpact,
    /// Predictions for each affected test
    pub predictions: Vec<TestPrediction>,
    /// Selected test subset
    pub selection: SelectedTestSubset,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_predictive_model_default() {
        let model = PredictiveModel::new();
        assert_eq!(model.default_failure_probability, 0.1);
    }

    #[test]
    fn test_predictive_model_with_config() {
        let model = PredictiveModel::with_config(0.2, 0.8);
        assert_eq!(model.default_failure_probability, 0.2);
    }

    #[test]
    fn test_add_history_record() {
        let mut model = PredictiveModel::new();
        model.add_record(TestHistoryRecord {
            test_name: "test_foo".to_string(),
            passed: false,
            duration_ms: 100,
            timestamp: chrono::Utc::now(),
            changed_files: vec!["src/foo.rs".to_string()],
            modified_functions: vec!["foo".to_string()],
        });

        assert_eq!(model.test_total_counts.get("test_foo"), Some(&1));
        assert_eq!(model.test_failure_counts.get("test_foo"), Some(&1));
    }

    #[test]
    fn test_register_coverage() {
        let mut model = PredictiveModel::new();
        model.register_file_coverage("test_foo", &["src/foo.rs".to_string()]);

        let affected = model.get_affected_tests(&ChangeImpact {
            modified_files: vec!["src/foo.rs".to_string()],
            ..Default::default()
        });

        assert!(affected.contains(&"test_foo".to_string()));
    }

    #[test]
    fn test_predict_failure_rate() {
        let mut model = PredictiveModel::new();
        model.register_file_coverage("test_foo", &["src/foo.rs".to_string()]);

        // Add some history
        for _ in 0..5 {
            model.add_record(TestHistoryRecord {
                test_name: "test_foo".to_string(),
                passed: false,
                duration_ms: 100,
                timestamp: chrono::Utc::now(),
                changed_files: vec!["src/foo.rs".to_string()],
                modified_functions: vec![],
            });
        }

        let impact = ChangeImpact {
            modified_files: vec!["src/foo.rs".to_string()],
            modified_functions: vec![],
            lines_changed: 50,
            ..Default::default()
        };

        let predictions = model.predict_for_change(&impact);
        assert!(!predictions.is_empty());

        let pred = predictions.first().unwrap();
        assert!(pred.failure_probability > 0.3); // Should be higher than default due to history
    }

    #[test]
    fn test_change_impact_analyzer_diff() {
        let analyzer = ChangeImpactAnalyzer::new();
        let diff = r#"
diff --git a/src/foo.rs b/src/foo.rs
index 1234567..abcdefg 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,5 +1,7 @@
+use bar;
 fn foo() {
-    println!("old");
+    println!("new");
+    println!("added");
 }
"#;

        let impact = analyzer.analyze_diff(diff);
        assert!(!impact.modified_files.is_empty() || impact.lines_changed != 0);
    }

    #[test]
    fn test_change_impact_analyzer_files() {
        let analyzer = ChangeImpactAnalyzer::new();
        let files = vec!["src/auth.rs".to_string(), "src/user.rs".to_string()];

        let impact = analyzer.analyze_files(&files);
        assert_eq!(impact.modified_files.len(), 2);
    }

    #[test]
    fn test_impact_category_from_path() {
        assert_eq!(
            ImpactCategory::from_path("src/auth/login.rs"),
            ImpactCategory::Authentication
        );
        assert_eq!(
            ImpactCategory::from_path("db/repository.rs"),
            ImpactCategory::Data
        );
        assert_eq!(
            ImpactCategory::from_path("api/endpoint.rs"),
            ImpactCategory::Api
        );
    }

    #[test]
    fn test_change_pattern_from_message() {
        assert_eq!(
            ChangePattern::from_message("fix: resolve null pointer bug"),
            ChangePattern::BugFix
        );
        assert_eq!(
            ChangePattern::from_message("feat: implement user authentication"),
            ChangePattern::Feature
        );
    }

    #[test]
    fn test_selection_strategy_risk_based() {
        let mut selector = TestSubsetSelector::with_strategy(SelectionStrategy::RiskBased);
        selector.register_duration("test_a", 100);
        selector.register_duration("test_b", 100);

        let predictions = vec![
            TestPrediction {
                test_name: "test_a".to_string(),
                failure_probability: 0.3,
                confidence: 0.8,
                contributing_factors: vec![],
            },
            TestPrediction {
                test_name: "test_b".to_string(),
                failure_probability: 0.8,
                confidence: 0.9,
                contributing_factors: vec![],
            },
        ];

        let constraint = SelectionConstraint {
            max_tests: Some(1),
            ..Default::default()
        };

        let result = selector.select(&predictions, &constraint);
        assert_eq!(result.selected_tests.len(), 1);
        assert_eq!(result.selected_tests[0].name, "test_b"); // Higher risk first
    }

    #[test]
    fn test_selection_strategy_time_constrained() {
        let mut selector = TestSubsetSelector::with_strategy(SelectionStrategy::TimeConstrained);
        selector.register_duration("test_a", 100);
        selector.register_duration("test_b", 100);
        selector.register_duration("test_c", 100);

        let predictions = vec![
            TestPrediction {
                test_name: "test_a".to_string(),
                failure_probability: 0.3,
                confidence: 0.8,
                contributing_factors: vec![],
            },
            TestPrediction {
                test_name: "test_b".to_string(),
                failure_probability: 0.5,
                confidence: 0.8,
                contributing_factors: vec![],
            },
            TestPrediction {
                test_name: "test_c".to_string(),
                failure_probability: 0.4,
                confidence: 0.8,
                contributing_factors: vec![],
            },
        ];

        let constraint = SelectionConstraint {
            max_time_ms: Some(200), // Can only run 2 tests
            ..Default::default()
        };

        let result = selector.select(&predictions, &constraint);
        assert!(result.estimated_time_ms <= 200);
    }

    #[test]
    fn test_predictive_selection_engine_full_pipeline() {
        let mut engine = PredictiveSelectionEngine::new();

        // Register coverage
        engine.register_coverage("test_auth", &["src/auth.rs".to_string()]);
        engine.register_coverage("test_user", &["src/user.rs".to_string()]);
        engine.register_duration("test_auth", 150);
        engine.register_duration("test_user", 100);

        // Add history
        engine.add_history(TestHistoryRecord {
            test_name: "test_auth".to_string(),
            passed: false,
            duration_ms: 150,
            timestamp: chrono::Utc::now(),
            changed_files: vec!["src/auth.rs".to_string()],
            modified_functions: vec![],
        });

        let files = vec!["src/auth.rs".to_string()];
        let constraint = SelectionConstraint {
            max_tests: Some(5),
            ..Default::default()
        };

        let result = engine.select_for_change(None, Some(&files), &constraint);

        assert!(!result.predictions.is_empty());
        assert!(!result.selection.selected_tests.is_empty());
        assert_eq!(result.selection.strategy, SelectionStrategy::Hybrid);
    }

    #[test]
    fn test_hotspot_files() {
        let mut model = PredictiveModel::new();
        model.add_record(TestHistoryRecord {
            test_name: "test_a".to_string(),
            passed: true,
            duration_ms: 100,
            timestamp: chrono::Utc::now(),
            changed_files: vec!["src/foo.rs".to_string()],
            modified_functions: vec![],
        });
        model.add_record(TestHistoryRecord {
            test_name: "test_b".to_string(),
            passed: true,
            duration_ms: 100,
            timestamp: chrono::Utc::now(),
            changed_files: vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()],
            modified_functions: vec![],
        });

        let hotspots = model.get_hotspot_files(5);
        assert!(hotspots.iter().any(|(f, _)| f == "src/foo.rs"));
    }

    #[test]
    fn test_flaky_tests() {
        let mut model = PredictiveModel::new();

        // Add records with high failure rate
        for _ in 0..10 {
            model.add_record(TestHistoryRecord {
                test_name: "test_flaky".to_string(),
                passed: false,
                duration_ms: 100,
                timestamp: chrono::Utc::now(),
                changed_files: vec![],
                modified_functions: vec![],
            });
        }
        for _ in 0..2 {
            model.add_record(TestHistoryRecord {
                test_name: "test_flaky".to_string(),
                passed: true,
                duration_ms: 100,
                timestamp: chrono::Utc::now(),
                changed_files: vec![],
                modified_functions: vec![],
            });
        }

        let flaky = model.get_flaky_tests(5);
        assert!(!flaky.is_empty());
    }
}
