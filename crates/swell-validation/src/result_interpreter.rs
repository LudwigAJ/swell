//! Result Interpreter Module
//!
//! Provides 5-category failure taxonomy with confidence scoring for test result analysis.
//!
//! # Failure Categories
//!
//! - [`FailureCategory::ImplementationBug`] - Actual code bugs or incorrect implementation
//! - [`FailureCategory::TestBug`] - Incorrect test logic, assertions, or test setup
//! - [`FailureCategory::EnvironmentIssue`] - Environment issues (missing deps, compilation errors)
//! - [`FailureCategory::Flaky`] - Non-deterministic test behavior
//! - [`FailureCategory::Unclear`] - Cannot determine cause from available information
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_validation::result_interpreter::{ResultInterpreter, TestResultInfo};
//!
//! let interpreter = ResultInterpreter::default();
//! let info = TestResultInfo {
//!     test_name: "test_example".to_string(),
//!     message: "assertion failed".to_string(),
//!     file: Some("tests/example.rs".to_string()),
//!     line: Some(42),
//!     is_compilation_error: false,
//!     run_history: None,
//! };
//!
//! let result = interpreter.interpret(&info);
//! println!("Category: {:?}", result.category);
//! println!("Confidence: {:.0}%", result.confidence * 100.0);
//! println!("Action: {}", result.recommended_action);
//! ```

use serde::{Deserialize, Serialize};

/// The 5 failure categories for test result analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FailureCategory {
    /// Failure due to actual code bugs or incorrect implementation
    ImplementationBug,
    /// Failure due to incorrect test logic, assertions, or test setup
    TestBug,
    /// Failure due to environment issues (missing deps, compilation errors, infrastructure)
    EnvironmentIssue,
    /// Non-deterministic test behavior (passes sometimes, fails sometimes)
    Flaky,
    /// Cannot determine the cause from available information
    Unclear,
}

impl FailureCategory {
    /// Get the name of this category
    pub fn name(&self) -> &'static str {
        match self {
            FailureCategory::ImplementationBug => "Implementation Bug",
            FailureCategory::TestBug => "Test Bug",
            FailureCategory::EnvironmentIssue => "Environment Issue",
            FailureCategory::Flaky => "Flaky Test",
            FailureCategory::Unclear => "Unclear",
        }
    }

    /// Get a short description of this category
    pub fn description(&self) -> &'static str {
        match self {
            FailureCategory::ImplementationBug => {
                "The test failed due to a bug in the implementation code"
            }
            FailureCategory::TestBug => {
                "The test failed due to a bug in the test itself (wrong assertion, bad setup)"
            }
            FailureCategory::EnvironmentIssue => {
                "The test failed due to environment issues (missing deps, infra problems)"
            }
            FailureCategory::Flaky => {
                "The test shows non-deterministic behavior (sometimes passes, sometimes fails)"
            }
            FailureCategory::Unclear => {
                "Cannot determine the cause of failure from available information"
            }
        }
    }

    /// Get the recommended action for this category
    pub fn recommended_action(&self) -> &'static str {
        match self {
            FailureCategory::ImplementationBug => {
                "Fix the implementation code. Run the failing test after making changes."
            }
            FailureCategory::TestBug => {
                "Fix the test. Review assertions, test setup, and test data. Consider whether the test is testing the right behavior."
            }
            FailureCategory::EnvironmentIssue => {
                "Resolve environment issues. Check dependencies, compilation errors, and infrastructure. Retry after fixing."
            }
            FailureCategory::Flaky => {
                "Investigate flakiness. Check for race conditions, shared state, timing dependencies, or external service issues. Consider adding retries or isolating the test."
            }
            FailureCategory::Unclear => {
                "Collect more information. Run the test in isolation, check logs, and review recent changes to both code and tests."
            }
        }
    }

    /// Get the priority for this category (lower = more urgent)
    pub fn priority(&self) -> u8 {
        match self {
            FailureCategory::ImplementationBug => 1,
            FailureCategory::TestBug => 2,
            FailureCategory::Flaky => 3,
            FailureCategory::EnvironmentIssue => 4,
            FailureCategory::Unclear => 5,
        }
    }
}

impl std::fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name(), self.description())
    }
}

/// Input information about a test result to classify
#[derive(Debug, Clone)]
pub struct TestResultInfo<'a> {
    /// The fully qualified test name
    pub test_name: &'a str,
    /// The failure message
    pub message: &'a str,
    /// File where the failure occurred (if available)
    pub file: Option<&'a str>,
    /// Line number (if available)
    pub line: Option<u32>,
    /// Whether this is a compilation error
    pub is_compilation_error: bool,
    /// Historical run information for flakiness detection (optional)
    pub run_history: Option<&'a TestRunHistory>,
}

/// Historical run information for a test
#[derive(Debug, Clone)]
pub struct TestRunHistory {
    /// Total number of runs
    pub total_runs: usize,
    /// Number of passes
    pub passes: usize,
    /// Number of failures
    pub failures: usize,
    /// Whether the most recent run was a failure
    pub last_run_failed: bool,
}

impl TestRunHistory {
    /// Calculate the failure rate (0.0 to 1.0)
    pub fn failure_rate(&self) -> f64 {
        if self.total_runs == 0 {
            return 0.0;
        }
        self.failures as f64 / self.total_runs as f64
    }

    /// Check if the test shows flaky behavior
    pub fn is_flaky(&self, min_flaky_rate: f64) -> bool {
        if self.total_runs < 3 {
            return false;
        }

        let rate = self.failure_rate();
        // Flaky if failure rate is between 10% and 90%
        rate >= min_flaky_rate && rate <= (1.0 - min_flaky_rate)
    }
}

/// Classification result with category and confidence score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    /// The classified category
    pub category: FailureCategory,
    /// Confidence score (0.0 to 1.0) for this classification
    pub confidence: f64,
    /// Evidence that led to this classification
    pub evidence: Vec<String>,
    /// Recommended action based on the classification
    pub recommended_action: String,
    /// Key phrases found in the error message that support this classification
    pub matched_patterns: Vec<String>,
}

impl ClassificationResult {
    /// Create a new classification result
    fn new(category: FailureCategory, confidence: f64, evidence: Vec<String>) -> Self {
        Self {
            category,
            confidence: confidence.clamp(0.0, 1.0),
            evidence,
            recommended_action: category.recommended_action().to_string(),
            matched_patterns: Vec::new(),
        }
    }

    /// Add a matched pattern to the result
    fn add_pattern(&mut self, pattern: impl Into<String>) {
        self.matched_patterns.push(pattern.into());
    }

    /// Check if this classification is high confidence
    pub fn is_high_confidence(&self) -> bool {
        self.confidence >= 0.8
    }

    /// Check if this classification is low confidence
    pub fn is_low_confidence(&self) -> bool {
        self.confidence < 0.5
    }
}

/// Evidence patterns for classification
#[derive(Debug, Clone)]
struct EvidencePatterns {
    impl_bug: Vec<(&'static str, f64)>,
    test_bug: Vec<(&'static str, f64)>,
    env_issue: Vec<(&'static str, f64)>,
    flaky: Vec<(&'static str, f64)>,
}

impl EvidencePatterns {
    fn new() -> Self {
        // Implementation bug patterns - logic errors, runtime crashes
        let impl_bug = vec![
            ("none", 0.8),                 // Option None unwrap
            ("null", 0.8),                 // Null pointer
            ("index out of bounds", 0.95), // Array bounds
            ("overflow", 0.9),             // Arithmetic overflow
            ("out of bounds", 0.9),        // Collection bounds
            ("deadlock", 0.95),            // Concurrency deadlock
            ("failed to", 0.7),            // Operation failure
            ("panicked at", 0.75),         // Panic without specific type
            ("lock", 0.85),                // Mutex/lock issues
            ("mutex", 0.85),               // Mutex issues
            ("already borrowed", 0.9),     // Borrow checker violation
            ("already mutex", 0.85),       // Mutex already locked
        ];

        // Test bug patterns - assertion failures, wrong expectations
        let test_bug = vec![
            ("assertion", 0.95), // Assertion failure
            ("assert_eq", 0.9),  // assert_eq! failure
            ("assert_ne", 0.9),  // assert_ne! failure
            ("assert!", 0.85),   // assert! failure
            ("expected:", 0.8),  // Expected value mismatch
            ("expected ", 0.7),  // Expected value mismatch
            ("actual:", 0.8),    // Actual value shown
            ("but got", 0.9),    // Expected vs actual
            ("mismatch", 0.85),  // Value mismatch
            ("result was", 0.7), // Result mismatch
            ("wrong", 0.6),      // Wrong value
            ("incorrect", 0.6),  // Incorrect value
        ];

        // Environment issue patterns - compilation, infrastructure, deps
        let env_issue = vec![
            ("cannot find", 0.95),        // Missing file/module
            ("link:", 0.9),               // Linker error
            ("library not found", 0.95),  // Missing library
            ("depends on", 0.85),         // Dependency issue
            ("could not compile", 0.95),  // Compilation error
            ("compilation failed", 0.95), // Compilation error
            ("no such file", 0.9),        // File not found
            ("permission denied", 0.85),  // Permission issue
            ("network", 0.9),             // Network issue
            ("timeout", 0.85),            // Timeout
            ("connection refused", 0.9),  // Connection issue
        ];

        // Flaky patterns - timing, race conditions, non-determinism
        let flaky = vec![
            ("race condition", 0.95),   // Race condition
            ("timing", 0.8),            // Timing issue
            ("intermittently", 0.9),    // Intermittent failure
            ("sometimes", 0.8),         // Non-deterministic
            ("occasionally", 0.85),     // Occasional failure
            ("flaky", 0.95),            // Explicitly called flaky
            ("non-deterministic", 0.9), // Non-deterministic
        ];

        Self {
            impl_bug,
            test_bug,
            env_issue,
            flaky,
        }
    }
}

impl Default for EvidencePatterns {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for the result interpreter
#[derive(Debug, Clone)]
pub struct ResultInterpreterConfig {
    /// Minimum failure rate to consider a test flaky (0.0 to 1.0)
    pub flaky_threshold: f64,
    /// Minimum runs before flakiness is considered
    pub min_runs_for_flakiness: usize,
    /// Use historical data for classification
    pub use_history: bool,
}

impl Default for ResultInterpreterConfig {
    fn default() -> Self {
        Self {
            flaky_threshold: 0.1,
            min_runs_for_flakiness: 3,
            use_history: true,
        }
    }
}

/// Result interpreter that classifies test failures into categories
#[derive(Debug, Clone)]
pub struct ResultInterpreter {
    config: ResultInterpreterConfig,
    patterns: EvidencePatterns,
}

impl Default for ResultInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultInterpreter {
    /// Create a new result interpreter with default configuration
    pub fn new() -> Self {
        Self {
            config: ResultInterpreterConfig::default(),
            patterns: EvidencePatterns::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: ResultInterpreterConfig) -> Self {
        Self {
            config,
            patterns: EvidencePatterns::default(),
        }
    }

    /// Interpret a test result and return a classification
    pub fn interpret<'a>(&self, info: &TestResultInfo<'a>) -> ClassificationResult {
        let msg_lower = info.message.to_lowercase();

        // First check for compilation errors - these are always environment issues
        if info.is_compilation_error {
            return self.classify_as_environment_issue(
                "Compilation error detected",
                vec!["is_compilation_error: true".to_string()],
            );
        }

        // Check for flaky tests first if history is available
        if self.config.use_history {
            if let Some(history) = info.run_history {
                if history.total_runs >= self.config.min_runs_for_flakiness
                    && history.is_flaky(self.config.flaky_threshold)
                {
                    return self.classify_as_flaky(history);
                }
            }
        }

        // Collect scores for each category
        let mut impl_score = self.score_category(&msg_lower, &self.patterns.impl_bug);
        let mut test_score = self.score_category(&msg_lower, &self.patterns.test_bug);
        let mut env_score = self.score_category(&msg_lower, &self.patterns.env_issue);
        let flaky_score = self.score_category(&msg_lower, &self.patterns.flaky);

        // Boost scores based on file location hints
        if let Some(file) = info.file {
            let file_lower = file.to_lowercase();
            // Test files suggest test bug
            if file_lower.contains("test") || file_lower.contains("_test") {
                test_score *= 1.2;
            }
            // Src files suggest implementation bug
            if file_lower.contains("/src/") {
                impl_score *= 1.2;
            }
        }

        // Compilation errors get high env score
        if info.is_compilation_error {
            env_score = 1.0;
        }

        // Determine the category with highest score
        let (category, max_score, evidence) =
            self.determine_category(impl_score, test_score, env_score, flaky_score, &msg_lower);

        // Calculate confidence based on score strength and evidence
        let confidence = self.calculate_confidence(max_score, &evidence);

        let mut result = ClassificationResult::new(category, confidence, evidence);

        // Add matched patterns
        self.add_matched_patterns(&mut result, &msg_lower);

        result
    }

    /// Score a message against patterns for a specific category
    fn score_category(&self, msg_lower: &str, patterns: &[(&str, f64)]) -> f64 {
        let mut score = 0.0;
        let mut count: f64 = 0.0;

        for (pattern, weight) in patterns {
            if msg_lower.contains(*pattern) {
                score += weight;
                count += 1.0;
            }
        }

        if count > 0.0 {
            // Average weighted score
            score / count.max(1.0)
        } else {
            0.0
        }
    }

    /// Determine the category based on scores
    fn determine_category(
        &self,
        impl_score: f64,
        test_score: f64,
        env_score: f64,
        flaky_score: f64,
        _msg_lower: &str,
    ) -> (FailureCategory, f64, Vec<String>) {
        let mut scores = [
            (FailureCategory::ImplementationBug, impl_score),
            (FailureCategory::TestBug, test_score),
            (FailureCategory::EnvironmentIssue, env_score),
            (FailureCategory::Flaky, flaky_score),
        ];

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let (category, max_score) = scores[0];

        // Generate evidence
        let mut evidence = vec![format!(
            "Best match: {:?} with score {:.2}",
            category, max_score
        )];

        if max_score < 0.3 {
            evidence.push("Low pattern match score, classification uncertain".to_string());
            return (FailureCategory::Unclear, max_score * 0.5, evidence);
        }

        // Check for strong second choice
        let second_score = scores[1].1;
        if second_score > 0.7 * max_score && max_score - second_score < 0.2 {
            evidence.push(format!(
                "Close second choice: {:?} with score {:.2}",
                scores[1].0, second_score
            ));
        }

        (category, max_score, evidence)
    }

    /// Calculate confidence based on score strength and evidence
    fn calculate_confidence(&self, max_score: f64, evidence: &[String]) -> f64 {
        // Base confidence from score
        let base_confidence = max_score;

        // Boost for strong evidence
        let evidence_bonus = if evidence.len() > 2 { 0.05 } else { 0.0 };

        // Cap at 0.95 to acknowledge we might be wrong
        (base_confidence + evidence_bonus).min(0.95)
    }

    /// Add matched patterns to the result
    fn add_matched_patterns(&self, result: &mut ClassificationResult, msg_lower: &str) {
        let patterns = match result.category {
            FailureCategory::ImplementationBug => &self.patterns.impl_bug,
            FailureCategory::TestBug => &self.patterns.test_bug,
            FailureCategory::EnvironmentIssue => &self.patterns.env_issue,
            FailureCategory::Flaky => &self.patterns.flaky,
            FailureCategory::Unclear => return,
        };

        for (pattern, _) in patterns {
            if msg_lower.contains(*pattern) {
                result.add_pattern(*pattern);
            }
        }
    }

    /// Classify as environment issue with evidence
    fn classify_as_environment_issue(
        &self,
        reason: &str,
        evidence: Vec<String>,
    ) -> ClassificationResult {
        ClassificationResult {
            category: FailureCategory::EnvironmentIssue,
            confidence: 0.90,
            evidence,
            recommended_action: FailureCategory::EnvironmentIssue
                .recommended_action()
                .to_string(),
            matched_patterns: vec![reason.to_string()],
        }
    }

    /// Classify as flaky based on history
    fn classify_as_flaky(&self, history: &TestRunHistory) -> ClassificationResult {
        let evidence = vec![
            format!(
                "Historical flakiness detected: {}/{} failures ({:.0}%)",
                history.failures,
                history.total_runs,
                history.failure_rate() * 100.0
            ),
            format!(
                "Recent behavior: {}",
                if history.last_run_failed {
                    "last run failed"
                } else {
                    "last run passed"
                }
            ),
        ];

        let confidence = 0.70 + (0.20 * (1.0 - (history.failure_rate() - 0.5).abs() * 2.0));

        ClassificationResult {
            category: FailureCategory::Flaky,
            confidence: confidence.min(0.90),
            evidence,
            recommended_action: FailureCategory::Flaky.recommended_action().to_string(),
            matched_patterns: vec!["flakiness_history".to_string()],
        }
    }

    /// Interpret multiple test results and return aggregated results
    pub fn interpret_batch<'a>(&self, infos: &[TestResultInfo<'a>]) -> BatchClassificationResult {
        let results: Vec<_> = infos.iter().map(|info| self.interpret(info)).collect();

        // Count by category
        let mut category_counts = std::collections::HashMap::new();
        for result in &results {
            *category_counts.entry(result.category).or_insert(0) += 1;
        }

        // Find dominant category
        let dominant_category = category_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(cat, _)| *cat)
            .unwrap_or(FailureCategory::Unclear);

        // Calculate overall confidence (average of all confidences)
        let avg_confidence = if results.is_empty() {
            0.0
        } else {
            let sum: f64 = results.iter().map(|r| r.confidence).sum();
            sum / results.len() as f64
        };

        BatchClassificationResult {
            results,
            dominant_category,
            category_counts,
            overall_confidence: avg_confidence,
        }
    }
}

/// Result of batch classification
#[derive(Debug, Clone)]
pub struct BatchClassificationResult {
    /// Individual classification results
    pub results: Vec<ClassificationResult>,
    /// The most common category
    pub dominant_category: FailureCategory,
    /// Count of results per category
    pub category_counts: std::collections::HashMap<FailureCategory, usize>,
    /// Average confidence across all results
    pub overall_confidence: f64,
}

impl BatchClassificationResult {
    /// Check if all results are high confidence
    pub fn all_high_confidence(&self) -> bool {
        self.results.iter().all(|r| r.is_high_confidence())
    }

    /// Check if any result is low confidence
    pub fn any_low_confidence(&self) -> bool {
        self.results.iter().any(|r| r.is_low_confidence())
    }

    /// Get results filtered by category
    pub fn filter_by_category(&self, category: FailureCategory) -> Vec<&ClassificationResult> {
        self.results
            .iter()
            .filter(|r| r.category == category)
            .collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod result_interpreter_tests {
    use super::*;

    #[test]
    fn test_failure_category_names() {
        assert_eq!(
            FailureCategory::ImplementationBug.name(),
            "Implementation Bug"
        );
        assert_eq!(FailureCategory::TestBug.name(), "Test Bug");
        assert_eq!(
            FailureCategory::EnvironmentIssue.name(),
            "Environment Issue"
        );
        assert_eq!(FailureCategory::Flaky.name(), "Flaky Test");
        assert_eq!(FailureCategory::Unclear.name(), "Unclear");
    }

    #[test]
    fn test_failure_category_priority() {
        assert_eq!(FailureCategory::ImplementationBug.priority(), 1);
        assert_eq!(FailureCategory::TestBug.priority(), 2);
        assert_eq!(FailureCategory::Flaky.priority(), 3);
        assert_eq!(FailureCategory::EnvironmentIssue.priority(), 4);
        assert_eq!(FailureCategory::Unclear.priority(), 5);
    }

    #[test]
    fn test_implementation_bug_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_index_bounds",
            message:
                "thread 'test' panicked at 'index out of bounds: the len is 3 but the index is 99'",
            file: Some("src/array.rs"),
            line: Some(42),
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::ImplementationBug);
        assert!(result.confidence >= 0.7);
        assert!(result
            .matched_patterns
            .contains(&"index out of bounds".to_string()));
    }

    #[test]
    fn test_test_bug_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_add",
            message: "assertion failed: `(left == right)`\n  left: `2`\n right: `3`",
            file: Some("tests/math_test.rs"),
            line: Some(10),
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::TestBug);
        assert!(result.confidence >= 0.6);
    }

    #[test]
    fn test_environment_issue_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_compile",
            message: "error: could not compile `test_crate`\n\nCaused by:\n  cannot find value `FOO` in this scope",
            file: Some("src/lib.rs"),
            line: Some(1),
            is_compilation_error: true,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::EnvironmentIssue);
        assert!(result.confidence >= 0.8);
    }

    #[test]
    fn test_flaky_classification_from_history() {
        let interpreter = ResultInterpreter::default();

        let history = TestRunHistory {
            total_runs: 10,
            passes: 5,
            failures: 5,
            last_run_failed: true,
        };

        let info = TestResultInfo {
            test_name: "test_race",
            message: "thread 'test' panicked at 'lock already held'",
            file: Some("tests/concurrent_test.rs"),
            line: Some(50),
            is_compilation_error: false,
            run_history: Some(&history),
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::Flaky);
        assert!(result.confidence >= 0.7);
    }

    #[test]
    fn test_unclear_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_mystery",
            message: "xyz123 unknown error code 999",
            file: None,
            line: None,
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        // With no pattern match, should be classified as unclear with very low confidence
        assert_eq!(result.category, FailureCategory::Unclear);
        assert!(result.confidence < 0.3);
    }

    #[test]
    fn test_batch_classification() {
        let interpreter = ResultInterpreter::default();

        let infos = vec![
            TestResultInfo {
                test_name: "test1",
                message: "index out of bounds",
                file: Some("src/lib.rs"),
                line: Some(10),
                is_compilation_error: false,
                run_history: None,
            },
            TestResultInfo {
                test_name: "test2",
                message: "assertion failed",
                file: Some("tests/test.rs"),
                line: Some(20),
                is_compilation_error: false,
                run_history: None,
            },
            TestResultInfo {
                test_name: "test3",
                message: "cannot find module",
                file: Some("src/main.rs"),
                line: Some(5),
                is_compilation_error: false,
                run_history: None,
            },
        ];

        let batch_result = interpreter.interpret_batch(&infos);

        assert_eq!(batch_result.results.len(), 3);
        assert!(batch_result
            .category_counts
            .contains_key(&FailureCategory::ImplementationBug));
        assert!(batch_result
            .category_counts
            .contains_key(&FailureCategory::TestBug));
        assert!(batch_result
            .category_counts
            .contains_key(&FailureCategory::EnvironmentIssue));
    }

    #[test]
    fn test_classification_result_high_confidence() {
        let result = ClassificationResult::new(
            FailureCategory::ImplementationBug,
            0.85,
            vec!["strong evidence".to_string()],
        );

        assert!(result.is_high_confidence());
        assert!(!result.is_low_confidence());
    }

    #[test]
    fn test_classification_result_low_confidence() {
        let result = ClassificationResult::new(
            FailureCategory::Unclear,
            0.3,
            vec!["weak evidence".to_string()],
        );

        assert!(!result.is_high_confidence());
        assert!(result.is_low_confidence());
    }

    #[test]
    fn test_test_run_history_failure_rate() {
        let history = TestRunHistory {
            total_runs: 10,
            passes: 7,
            failures: 3,
            last_run_failed: false,
        };

        assert!((history.failure_rate() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_test_run_history_is_flaky() {
        let flaky_history = TestRunHistory {
            total_runs: 10,
            passes: 5,
            failures: 5,
            last_run_failed: true,
        };

        let stable_history = TestRunHistory {
            total_runs: 10,
            passes: 10,
            failures: 0,
            last_run_failed: false,
        };

        assert!(flaky_history.is_flaky(0.1));
        assert!(!stable_history.is_flaky(0.1));
    }

    #[test]
    fn test_test_run_history_not_enough_runs() {
        let history = TestRunHistory {
            total_runs: 2,
            passes: 1,
            failures: 1,
            last_run_failed: true,
        };

        // Should not be flagged as flaky with only 2 runs
        assert!(!history.is_flaky(0.1));
    }

    #[test]
    fn test_interpreter_with_custom_config() {
        let config = ResultInterpreterConfig {
            flaky_threshold: 0.2,
            min_runs_for_flakiness: 5,
            use_history: true,
        };

        let interpreter = ResultInterpreter::with_config(config);

        let history = TestRunHistory {
            total_runs: 5,
            passes: 3,
            failures: 2,
            last_run_failed: true,
        };

        let info = TestResultInfo {
            test_name: "test",
            message: "lock already held",
            file: Some("tests/test.rs"),
            line: Some(10),
            is_compilation_error: false,
            run_history: Some(&history),
        };

        let result = interpreter.interpret(&info);

        // With 5 runs (meeting min_runs_for_flakiness) and 40% failure rate (meeting 0.2 threshold)
        // and mixed results, should be detected as flaky
        assert_eq!(result.category, FailureCategory::Flaky);
    }

    #[test]
    fn test_batch_filter_by_category() {
        let interpreter = ResultInterpreter::default();

        let infos = vec![
            TestResultInfo {
                test_name: "test1",
                message: "index out of bounds",
                file: Some("src/lib.rs"),
                line: Some(10),
                is_compilation_error: false,
                run_history: None,
            },
            TestResultInfo {
                test_name: "test2",
                message: "assertion failed",
                file: Some("tests/test.rs"),
                line: Some(20),
                is_compilation_error: false,
                run_history: None,
            },
            TestResultInfo {
                test_name: "test3",
                message: "index out of bounds",
                file: Some("src/main.rs"),
                line: Some(5),
                is_compilation_error: false,
                run_history: None,
            },
        ];

        let batch_result = interpreter.interpret_batch(&infos);

        let impl_bugs = batch_result.filter_by_category(FailureCategory::ImplementationBug);
        assert_eq!(impl_bugs.len(), 2);
    }

    #[test]
    fn test_all_high_confidence() {
        let batch_result = BatchClassificationResult {
            results: vec![
                ClassificationResult::new(FailureCategory::ImplementationBug, 0.85, vec![]),
                ClassificationResult::new(FailureCategory::TestBug, 0.90, vec![]),
            ],
            dominant_category: FailureCategory::ImplementationBug,
            category_counts: std::collections::HashMap::new(),
            overall_confidence: 0.875,
        };

        assert!(batch_result.all_high_confidence());
    }

    #[test]
    fn test_any_low_confidence() {
        let batch_result = BatchClassificationResult {
            results: vec![
                ClassificationResult::new(FailureCategory::ImplementationBug, 0.85, vec![]),
                ClassificationResult::new(FailureCategory::Unclear, 0.3, vec![]),
            ],
            dominant_category: FailureCategory::ImplementationBug,
            category_counts: std::collections::HashMap::new(),
            overall_confidence: 0.575,
        };

        assert!(batch_result.any_low_confidence());
    }

    #[test]
    fn test_null_pointer_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_null",
            message: "thread 'test' panicked at 'called `Option::unwrap()` on a `None` value'",
            file: Some("src/handler.rs"),
            line: Some(100),
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::ImplementationBug);
        assert!(result.matched_patterns.contains(&"none".to_string()));
    }

    #[test]
    fn test_missing_dependency_classification() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test_dep",
            message:
                "error: cannot find dependency `serde_json`\n\nCaused by:\n  Library not found",
            file: Some("Cargo.toml"),
            line: None,
            is_compilation_error: true,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        assert_eq!(result.category, FailureCategory::EnvironmentIssue);
    }

    #[test]
    fn test_mixed_patterns_impl_vs_test() {
        let interpreter = ResultInterpreter::default();

        // Message contains both impl bug and test bug patterns
        // But file is in src/ so impl bug should win
        let info = TestResultInfo {
            test_name: "test_logic",
            message: "assertion failed: expected index < 10, but got index out of bounds",
            file: Some("src/logic.rs"),
            line: Some(25),
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        // "index out of bounds" is a strong implementation bug indicator
        // Should classify as implementation bug since file is in src/
        assert_eq!(result.category, FailureCategory::ImplementationBug);
    }

    #[test]
    fn test_confidence_bounded() {
        let interpreter = ResultInterpreter::default();

        let info = TestResultInfo {
            test_name: "test",
            message: "index out of bounds: the len is 3 but the index is 99",
            file: Some("src/lib.rs"),
            line: Some(10),
            is_compilation_error: false,
            run_history: None,
        };

        let result = interpreter.interpret(&info);

        // Confidence should be bounded between 0.0 and 1.0
        assert!(result.confidence >= 0.0 && result.confidence <= 1.0);
    }
}
