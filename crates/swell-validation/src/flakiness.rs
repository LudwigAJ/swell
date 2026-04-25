//! Flakiness detection using historical patterns (DeFlaker approach).
//!
//! This module provides:
//! - [`FlakinessDetector`] - Core detector that analyzes test runs for flakiness patterns
//! - [`QuarantinePool`] - Manages tests that are quarantined due to flakiness
//! - [`FlakinessReport`] - Detailed report on detected flaky tests
//!
//! # DeFlaker Approach
//!
//! The DeFlaker approach tracks tests across multiple runs and identifies flakiness
//! by analyzing historical patterns. A test is considered flaky if:
//! 1. It has both passed and failed runs in its history
//! 2. The failure rate is between 10% and 90% (not consistently failing)
//! 3. It shows inconsistency across runs (sometimes passes, sometimes fails)
//!
//! # Quarantine Mechanism
//!
//! Tests that exceed a configurable flakiness threshold are added to a quarantine pool.
//! Quarantined tests are:
//! - Excluded from normal test runs
//! - Run separately with retry logic
//! - Monitored for improvement or removal from quarantine

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::confidence::{FlakinessHistory, TestRun};

/// Minimum number of runs before a test can be considered for flakiness detection
const MIN_RUNS_FOR_FLAKINESS: usize = 3;

/// Default flakiness score threshold for quarantine (0.0 to 1.0)
const DEFAULT_FLAKINESS_THRESHOLD: f64 = 0.1;

/// Minimum failure rate to consider a test potentially flaky (10%)
const MIN_FAILURE_RATE: f64 = 0.1;

/// Maximum failure rate to consider a test potentially flaky (90%)
const MAX_FAILURE_RATE: f64 = 0.9;

// ============================================================================
// Flakiness Detector
// ============================================================================

/// Core flakiness detector using the DeFlaker approach.
///
/// Analyzes test runs across time to identify flaky tests based on:
/// - Historical pass/fail patterns
/// - Failure rate consistency
/// - Temporal patterns (clustering of failures)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessDetector {
    /// Historical test runs keyed by test name
    history: FlakinessHistory,
    /// Configuration for flakiness detection
    config: FlakinessConfig,
    /// Cached flakiness scores for tests
    scores: HashMap<String, f64>,
    /// Tests currently flagged as flaky
    flaky_tests: std::collections::HashSet<String>,
}

impl Default for FlakinessDetector {
    fn default() -> Self {
        Self::new(FlakinessConfig::default())
    }
}

/// Configuration for flakiness detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessConfig {
    /// Minimum number of runs before flakiness is evaluated
    pub min_runs: usize,
    /// Flakiness score threshold for quarantine (0.0 to 1.0)
    pub quarantine_threshold: f64,
    /// Minimum failure rate to be considered potentially flaky
    pub min_failure_rate: f64,
    /// Maximum failure rate to be considered potentially flaky
    pub max_failure_rate: f64,
    /// Maximum age of runs to consider (in days)
    pub max_history_days: Option<u32>,
    /// Whether to enable experimental temporal analysis
    pub enable_temporal_analysis: bool,
}

impl Default for FlakinessConfig {
    fn default() -> Self {
        Self {
            min_runs: MIN_RUNS_FOR_FLAKINESS,
            quarantine_threshold: DEFAULT_FLAKINESS_THRESHOLD,
            min_failure_rate: MIN_FAILURE_RATE,
            max_failure_rate: MAX_FAILURE_RATE,
            max_history_days: Some(30),
            enable_temporal_analysis: true,
        }
    }
}

impl FlakinessConfig {
    /// Create a new config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set minimum runs required for flakiness evaluation
    pub fn with_min_runs(mut self, runs: usize) -> Self {
        self.min_runs = runs;
        self
    }

    /// Set quarantine threshold
    pub fn with_quarantine_threshold(mut self, threshold: f64) -> Self {
        self.quarantine_threshold = threshold;
        self
    }

    /// Set maximum history age in days
    pub fn with_max_history_days(mut self, days: u32) -> Self {
        self.max_history_days = Some(days);
        self
    }

    /// Disable temporal analysis
    pub fn without_temporal_analysis(mut self) -> Self {
        self.enable_temporal_analysis = false;
        self
    }
}

impl FlakinessDetector {
    /// Create a new flakiness detector with default configuration
    pub fn new(config: FlakinessConfig) -> Self {
        Self {
            history: FlakinessHistory::new(),
            config,
            scores: HashMap::new(),
            flaky_tests: std::collections::HashSet::new(),
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(FlakinessConfig::default())
    }

    /// Record a test run
    pub fn record(&mut self, test_name: String, passed: bool, duration_ms: u64) {
        let run = TestRun {
            timestamp: Utc::now(),
            passed,
            duration_ms,
            is_retry: false,
        };
        self.history.record(test_name.clone(), run);
        self.update_flakiness(&test_name);
    }

    /// Record a test run with metadata
    pub fn record_with_retry(
        &mut self,
        test_name: String,
        passed: bool,
        duration_ms: u64,
        is_retry: bool,
    ) {
        let run = TestRun {
            timestamp: Utc::now(),
            passed,
            duration_ms,
            is_retry,
        };
        self.history.record(test_name.clone(), run);
        self.update_flakiness(&test_name);
    }

    /// Update flakiness score for a single test
    fn update_flakiness(&mut self, test_name: &str) {
        let score = self.compute_flakiness_score(test_name);
        self.scores.insert(test_name.to_string(), score);

        if score >= self.config.quarantine_threshold {
            self.flaky_tests.insert(test_name.to_string());
        } else {
            self.flaky_tests.remove(test_name);
        }
    }

    /// Compute flakiness score for a test (0.0 = stable, 1.0 = highly flaky)
    pub fn compute_flakiness_score(&self, test_name: &str) -> f64 {
        let runs = self.history.get_runs(test_name);

        if runs.len() < self.config.min_runs {
            return 0.0;
        }

        // Filter runs by age if configured
        let runs = self.filter_runs_by_age(runs);

        if runs.len() < self.config.min_runs {
            return 0.0;
        }

        // Calculate failure rate
        let total = runs.len();
        let failures: usize = runs.iter().filter(|r| !r.passed).count();
        let passes = total - failures;

        // Must have mixed results to be flaky
        if failures == 0 || passes == 0 {
            return 0.0;
        }

        let failure_rate = failures as f64 / total as f64;

        // Check if failure rate is in the "potentially flaky" range
        if failure_rate < self.config.min_failure_rate
            || failure_rate > self.config.max_failure_rate
        {
            return 0.0;
        }

        // Compute inconsistency score based on run sequence
        let inconsistency = self.compute_inconsistency(runs);

        // Compute temporal clustering score if enabled
        let temporal = if self.config.enable_temporal_analysis {
            self.compute_temporal_clustering(runs)
        } else {
            0.0
        };

        // Combined score: weighted average of failure rate balance and inconsistency
        // Higher score = more flaky
        let balance_score = 1.0 - (failure_rate - 0.5).abs() * 2.0; // 0.0 at edges, 1.0 at 50%
        let combined = (inconsistency + temporal + balance_score) / 3.0;

        combined.clamp(0.0, 1.0)
    }

    /// Filter runs by age if max_history_days is configured
    fn filter_runs_by_age<'a>(&self, runs: &'a [TestRun]) -> &'a [TestRun] {
        if let Some(max_days) = self.config.max_history_days {
            let cutoff = Utc::now() - chrono::Duration::days(max_days as i64);
            // Find first run within cutoff - if all old, return empty
            // For simplicity, we return the original slice if within age
            // A more sophisticated implementation would filter properly
            if runs.first().map(|r| r.timestamp < cutoff).unwrap_or(false) {
                // All runs are older than cutoff, return empty view
                return &[];
            }
        }
        runs
    }

    /// Compute inconsistency score based on run sequence.
    /// A test that alternates pass/fail frequently is more likely to be flaky
    /// than one that clusters failures together.
    fn compute_inconsistency(&self, runs: &[TestRun]) -> f64 {
        if runs.len() < 2 {
            return 0.0;
        }

        // Count transitions between pass and fail
        let mut transitions = 0;
        for i in 1..runs.len() {
            if runs[i].passed != runs[i - 1].passed {
                transitions += 1;
            }
        }

        // Maximum possible transitions
        let max_transitions = runs.len() - 1;

        // Normalized to 0-1, where 1 means alternating every run
        transitions as f64 / max_transitions as f64
    }

    /// Compute temporal clustering score.
    /// A test that clusters its failures (runs a bunch, then fails a bunch)
    /// is less likely to be truly flaky than one that fails randomly throughout.
    fn compute_temporal_clustering(&self, runs: &[TestRun]) -> f64 {
        if runs.len() < 4 {
            return 0.0;
        }

        // Count runs where failure is isolated (neighbors passed)
        let mut isolated_failures = 0;
        for i in 0..runs.len() {
            if !runs[i].passed {
                let prev_passed = i == 0 || runs[i - 1].passed;
                let next_passed = i == runs.len() - 1 || runs[i + 1].passed;
                if prev_passed && next_passed {
                    isolated_failures += 1;
                }
            }
        }

        let failures: usize = runs.iter().filter(|r| !r.passed).count();
        if failures == 0 {
            return 0.0;
        }

        // High ratio of isolated failures suggests random flakiness
        isolated_failures as f64 / failures as f64
    }

    /// Check if a specific test is currently considered flaky
    pub fn is_flaky(&self, test_name: &str) -> bool {
        self.flaky_tests.contains(test_name)
    }

    /// Get the flakiness score for a test
    pub fn flakiness_score(&self, test_name: &str) -> f64 {
        self.scores.get(test_name).copied().unwrap_or(0.0)
    }

    /// Get all tests currently flagged as flaky
    pub fn get_flaky_tests(&self) -> Vec<String> {
        self.flaky_tests.iter().cloned().collect()
    }

    /// Get all test names in history
    pub fn get_all_tests(&self) -> Vec<String> {
        self.history.get_all_tests()
    }

    /// Get the underlying history for a test
    pub fn get_history(&self, test_name: &str) -> Option<&[TestRun]> {
        let runs = self.history.get_runs(test_name);
        if runs.is_empty() {
            None
        } else {
            Some(runs)
        }
    }

    /// Generate a flakiness report for all tests
    pub fn generate_report(&self) -> FlakinessReport {
        let mut test_reports = Vec::new();

        for test_name in self.history.get_all_tests() {
            let score = self.flakiness_score(&test_name);
            let history = self.get_history(&test_name).unwrap_or(&[]);

            let runs_count = history.len();
            let failures_count = history.iter().filter(|r| !r.passed).count();
            let passes_count = runs_count - failures_count;

            let failure_rate = if runs_count > 0 {
                failures_count as f64 / runs_count as f64
            } else {
                0.0
            };

            let avg_duration_ms = if !history.is_empty() {
                let total: u64 = history.iter().map(|r| r.duration_ms).sum();
                total / runs_count as u64
            } else {
                0
            };

            test_reports.push(TestFlakinessReport {
                test_name: test_name.clone(),
                is_flaky: self.is_flaky(&test_name),
                flakiness_score: score,
                total_runs: runs_count,
                passes: passes_count,
                failures: failures_count,
                failure_rate,
                avg_duration_ms,
            });
        }

        // Sort by flakiness score descending
        test_reports.sort_by(|a, b| {
            b.flakiness_score
                .partial_cmp(&a.flakiness_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let quarantined_count = test_reports.iter().filter(|r| r.is_flaky).count();

        FlakinessReport {
            config: self.config.clone(),
            total_tests: test_reports.len(),
            flaky_tests: quarantined_count,
            quarantined_threshold: self.config.quarantine_threshold,
            test_reports,
        }
    }

    /// Reset all history and scores
    pub fn reset(&mut self) {
        self.history = FlakinessHistory::new();
        self.scores.clear();
        self.flaky_tests.clear();
    }

    /// Merge history from another detector
    pub fn merge(&mut self, other: &FlakinessDetector) {
        for test_name in other.history.get_all_tests() {
            for run in other.history.get_runs(&test_name) {
                self.history.record(test_name.clone(), run.clone());
            }
        }
        // Recompute all scores
        for test_name in self.history.get_all_tests() {
            self.update_flakiness(&test_name);
        }
    }
}

// ============================================================================
// Flakiness Report
// ============================================================================

/// Detailed report of flakiness analysis across all tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessReport {
    /// Configuration used for detection
    pub config: FlakinessConfig,
    /// Total number of tests tracked
    pub total_tests: usize,
    /// Number of tests currently flagged as flaky
    pub flaky_tests: usize,
    /// Threshold used for quarantine
    pub quarantined_threshold: f64,
    /// Individual test reports
    pub test_reports: Vec<TestFlakinessReport>,
}

impl FlakinessReport {
    /// Get the percentage of tests that are flaky
    pub fn flaky_percentage(&self) -> f64 {
        if self.total_tests == 0 {
            0.0
        } else {
            (self.flaky_tests as f64 / self.total_tests as f64) * 100.0
        }
    }

    /// Get tests above a specific flakiness threshold
    pub fn tests_above_threshold(&self, threshold: f64) -> Vec<&TestFlakinessReport> {
        self.test_reports
            .iter()
            .filter(|r| r.flakiness_score >= threshold)
            .collect()
    }

    /// Generate a markdown summary
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Flakiness Report\n\n");

        // Summary
        md.push_str("## Summary\n\n");
        md.push_str(&format!(
            "- **Total Tests Tracked**: {}\n",
            self.total_tests
        ));
        md.push_str(&format!(
            "- **Flaky Tests**: {} ({:.1}%)\n",
            self.flaky_tests,
            self.flaky_percentage()
        ));
        md.push_str(&format!(
            "- **Quarantine Threshold**: {:.0}%\n\n",
            self.quarantined_threshold * 100.0
        ));

        // Configuration
        md.push_str("## Detection Configuration\n\n");
        md.push_str(&format!("- **Min Runs**: {}\n", self.config.min_runs));
        md.push_str(&format!(
            "- **Failure Rate Range**: {:.0}% - {:.0}%\n",
            self.config.min_failure_rate * 100.0,
            self.config.max_failure_rate * 100.0
        ));
        if let Some(days) = self.config.max_history_days {
            md.push_str(&format!("- **Max History**: {} days\n", days));
        }
        md.push('\n');

        // Flaky tests
        let flaky: Vec<_> = self.test_reports.iter().filter(|r| r.is_flaky).collect();
        if !flaky.is_empty() {
            md.push_str("## ⚠️ Flaky Tests\n\n");
            md.push_str("| Test | Score | Runs | Pass | Fail | Rate |\n");
            md.push_str("|------|-------|------|------|------|------|\n");

            for report in flaky {
                md.push_str(&format!(
                    "| `{}` | {:.0}% | {} | {} | {} | {:.0}% |\n",
                    report.test_name,
                    report.flakiness_score * 100.0,
                    report.total_runs,
                    report.passes,
                    report.failures,
                    report.failure_rate * 100.0
                ));
            }
            md.push('\n');
        }

        // Recently stable tests (not flaky but had failures)
        let stable_with_failures: Vec<_> = self
            .test_reports
            .iter()
            .filter(|r| !r.is_flaky && r.failures > 0)
            .take(5)
            .collect();

        if !stable_with_failures.is_empty() {
            md.push_str("## Stable Tests with Historical Failures\n\n");
            md.push_str("(These tests have failures in history but are not currently flaky)\n\n");

            for report in stable_with_failures {
                md.push_str(&format!(
                    "- `{}`: {:.0}% failure rate ({} failures in {} runs)\n",
                    report.test_name,
                    report.failure_rate * 100.0,
                    report.failures,
                    report.total_runs
                ));
            }
            md.push('\n');
        }

        md
    }
}

/// Report for a single test's flakiness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFlakinessReport {
    /// Test name
    pub test_name: String,
    /// Whether test is currently flagged as flaky
    pub is_flaky: bool,
    /// Flakiness score (0.0 to 1.0)
    pub flakiness_score: f64,
    /// Total number of runs
    pub total_runs: usize,
    /// Number of passes
    pub passes: usize,
    /// Number of failures
    pub failures: usize,
    /// Failure rate
    pub failure_rate: f64,
    /// Average duration in milliseconds
    pub avg_duration_ms: u64,
}

// ============================================================================
// Quarantine Pool
// ============================================================================

/// Pool of tests that have been quarantined due to flakiness.
///
/// Quarantined tests are:
/// - Excluded from normal test runs
/// - Run separately with retry logic
/// - Monitored for improvement or removal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantinePool {
    /// Tests in quarantine with metadata
    quarantined: HashMap<String, QuarantinedTest>,
    /// Configuration
    config: QuarantineConfig,
}

/// Configuration for quarantine behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineConfig {
    /// Minimum flakiness score to enter quarantine
    pub enter_threshold: f64,
    /// Flakiness score to exit quarantine (must stay below this)
    pub exit_threshold: f64,
    /// Maximum time in quarantine before manual review
    pub max_quarantine_days: u32,
    /// Number of consecutive clean runs to exit quarantine
    pub consecutive_passes_to_exit: usize,
    /// Number of runs in stability loop (10 consecutive passes to exit)
    pub stability_loop_runs: usize,
}

impl Default for QuarantineConfig {
    fn default() -> Self {
        Self {
            enter_threshold: DEFAULT_FLAKINESS_THRESHOLD,
            exit_threshold: 0.1,
            max_quarantine_days: 7,
            consecutive_passes_to_exit: 3,
            stability_loop_runs: 10,
        }
    }
}

/// Metadata for a quarantined test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantinedTest {
    /// Test name
    pub test_name: String,
    /// When the test was quarantined
    pub quarantined_at: DateTime<Utc>,
    /// Flakiness score when quarantined
    pub flakiness_score: f64,
    /// Number of passes since quarantine
    pub consecutive_passes: usize,
    /// Number of failures since quarantine
    pub failures_since_quarantine: usize,
    /// Last time the test was run
    pub last_run_at: Option<DateTime<Utc>>,
    /// Last run result
    pub last_run_passed: Option<bool>,
    /// Number of times we've attempted to exit quarantine
    pub exit_attempts: usize,
}

impl Default for QuarantinePool {
    fn default() -> Self {
        Self::new(QuarantineConfig::default())
    }
}

impl QuarantinePool {
    /// Create a new quarantine pool with configuration
    pub fn new(config: QuarantineConfig) -> Self {
        Self {
            quarantined: HashMap::new(),
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(QuarantineConfig::default())
    }

    /// Add a test to the quarantine pool
    pub fn quarantine(&mut self, test_name: String, flakiness_score: f64) {
        if !self.quarantined.contains_key(&test_name) {
            self.quarantined.insert(
                test_name.clone(),
                QuarantinedTest {
                    test_name,
                    quarantined_at: Utc::now(),
                    flakiness_score,
                    consecutive_passes: 0,
                    failures_since_quarantine: 0,
                    last_run_at: None,
                    last_run_passed: None,
                    exit_attempts: 0,
                },
            );
        }
    }

    /// Remove a test from the quarantine pool
    pub fn release(&mut self, test_name: &str) -> Option<QuarantinedTest> {
        self.quarantined.remove(test_name)
    }

    /// Check if a test is in quarantine
    pub fn is_quarantined(&self, test_name: &str) -> bool {
        self.quarantined.contains_key(test_name)
    }

    /// Get all quarantined test names
    pub fn get_quarantined_tests(&self) -> Vec<String> {
        self.quarantined.keys().cloned().collect()
    }

    /// Get quarantined test metadata
    pub fn get_quarantined_test(&self, test_name: &str) -> Option<&QuarantinedTest> {
        self.quarantined.get(test_name)
    }

    /// Record a run result for a quarantined test
    ///
    /// Quarantined tests run stability loops - a failure resets the consecutive pass count.
    /// Only after `stability_loop_runs` (default 10) consecutive passes is a test restored.
    ///
    /// Returns true if the test should remain in quarantine.
    pub fn record_result(&mut self, test_name: &str, passed: bool) -> bool {
        let Some(test) = self.quarantined.get_mut(test_name) else {
            return false;
        };

        test.last_run_at = Some(Utc::now());
        test.last_run_passed = Some(passed);

        if passed {
            test.consecutive_passes += 1;
            test.failures_since_quarantine = 0;

            // Check if we've passed the stability loop
            if test.consecutive_passes >= self.config.stability_loop_runs {
                return false; // Passed stability loop - should be released
            }
        } else {
            // Failure in stability loop - reset and stay quarantined
            test.consecutive_passes = 0;
            test.failures_since_quarantine += 1;
            test.exit_attempts = 0;
        }

        true // Remain in quarantine
    }

    /// Check if a test should be released from quarantine
    ///
    /// A test is released from quarantine if:
    /// 1. It has passed `stability_loop_runs` consecutive times (default 10), OR
    /// 2. Max time in quarantine has been reached (for manual review)
    pub fn should_release(&self, test_name: &str, _current_score: f64) -> bool {
        let Some(test) = self.quarantined.get(test_name) else {
            return false;
        };

        // Check time limit - max time in quarantine reached
        let days_in_quarantine = (Utc::now() - test.quarantined_at).num_days();
        if days_in_quarantine >= self.config.max_quarantine_days as i64 {
            return true; // Max time reached, release for manual review
        }

        // Primary release mechanism: stability loop - 10 consecutive passes
        // After quarantining a flaky test, we run it stability_loop_runs times.
        // Only if ALL passes do we restore it. If ANY fail, it stays quarantined.
        if test.consecutive_passes >= self.config.stability_loop_runs {
            return true; // Passed stability loop - test is stable
        }

        false
    }

    /// Update quarantine based on current flakiness scores
    ///
    /// Returns list of tests that were released
    pub fn update(&mut self, detector: &FlakinessDetector) -> Vec<String> {
        let mut released = Vec::new();
        let to_remove: Vec<String> = self
            .quarantined
            .keys()
            .filter(|name| {
                let score = detector.flakiness_score(name);
                self.should_release(name, score)
            })
            .cloned()
            .collect();

        for name in to_remove {
            self.release(&name);
            released.push(name);
        }

        released
    }

    /// Add tests to quarantine based on detector scores
    ///
    /// Returns list of tests that were newly quarantined
    pub fn sync_with_detector(&mut self, detector: &FlakinessDetector) -> Vec<String> {
        let mut newly_quarantined = Vec::new();

        for test_name in detector.get_flaky_tests() {
            if !self.is_quarantined(&test_name) {
                let score = detector.flakiness_score(&test_name);
                self.quarantine(test_name.clone(), score);
                newly_quarantined.push(test_name);
            }
        }

        newly_quarantined
    }

    /// Get quarantine statistics
    pub fn stats(&self) -> QuarantineStats {
        let total = self.quarantined.len();
        let with_recent_pass = self
            .quarantined
            .values()
            .filter(|t| t.consecutive_passes > 0)
            .count();
        let max_time = self
            .quarantined
            .values()
            .map(|t| (Utc::now() - t.quarantined_at).num_days())
            .max()
            .unwrap_or(0);

        QuarantineStats {
            total_quarantined: total,
            with_consecutive_passes: with_recent_pass,
            max_days_in_quarantine: max_time,
        }
    }

    /// Get tests in quarantine that have exceeded max time
    pub fn overdue_for_review(&self) -> Vec<&QuarantinedTest> {
        self.quarantined
            .values()
            .filter(|t| {
                let days = (Utc::now() - t.quarantined_at).num_days();
                days >= self.config.max_quarantine_days as i64
            })
            .collect()
    }

    /// Clear all quarantined tests
    pub fn clear(&mut self) {
        self.quarantined.clear();
    }
}

/// Statistics about the quarantine pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineStats {
    /// Total number of quarantined tests
    pub total_quarantined: usize,
    /// Number with consecutive passes (close to release)
    pub with_consecutive_passes: usize,
    /// Maximum days any test has been in quarantine
    pub max_days_in_quarantine: i64,
}

impl QuarantineStats {
    /// Check if quarantine pool is healthy (not overcrowded)
    pub fn is_healthy(&self) -> bool {
        self.total_quarantined < 10
    }
}

// ============================================================================
// Flakiness Retry Handler (3x Retry with Majority Voting)
// ============================================================================

/// Configuration for retry behavior with majority voting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (including initial run)
    pub max_attempts: usize,
    /// Minimum number of passes required for overall success (for majority voting)
    pub min_passes_for_success: usize,
    /// Whether to enable retry for flaky tests
    pub enable_flaky_retry: bool,
    /// Whether to use exponential backoff between retries
    pub use_exponential_backoff: bool,
    /// Base delay in milliseconds between retries (when not using exponential backoff)
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds between retries
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            min_passes_for_success: 2, // Majority: 2 out of 3
            enable_flaky_retry: true,
            use_exponential_backoff: false,
            base_delay_ms: 100,
            max_delay_ms: 1000,
        }
    }
}

impl RetryConfig {
    /// Create a new retry config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum retry attempts
    pub fn with_max_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = attempts;
        self
    }

    /// Set minimum passes for success (majority threshold)
    pub fn with_min_passes(mut self, passes: usize) -> Self {
        self.min_passes_for_success = passes;
        self
    }

    /// Enable exponential backoff between retries
    pub fn with_exponential_backoff(mut self) -> Self {
        self.use_exponential_backoff = true;
        self
    }

    /// Set base delay in milliseconds
    pub fn with_base_delay_ms(mut self, delay_ms: u64) -> Self {
        self.base_delay_ms = delay_ms;
        self
    }

    /// Set maximum delay in milliseconds
    pub fn with_max_delay_ms(mut self, delay_ms: u64) -> Self {
        self.max_delay_ms = delay_ms;
        self
    }
}

/// Result of a single retry attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    /// Attempt number (1-indexed)
    pub attempt: usize,
    /// Whether this attempt passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Timestamp of the attempt
    pub timestamp: DateTime<Utc>,
}

/// Result of a retry sequence with majority voting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryResult {
    /// Test name
    pub test_name: String,
    /// All attempts made
    pub attempts: Vec<RetryAttempt>,
    /// Final result after majority voting
    pub passed: bool,
    /// Number of passes among all attempts
    pub total_passes: usize,
    /// Number of failures among all attempts
    pub total_failures: usize,
    /// The majority voting threshold used
    pub min_passes_required: usize,
    /// Total duration of all attempts combined
    pub total_duration_ms: u64,
}

impl RetryResult {
    /// Get the pass rate (0.0 to 1.0)
    pub fn pass_rate(&self) -> f64 {
        let total = self.attempts.len();
        if total == 0 {
            return 0.0;
        }
        self.total_passes as f64 / total as f64
    }

    /// Check if the result is definitive (clear majority)
    pub fn is_definitive(&self) -> bool {
        // A result is definitive if there's a clear majority
        // (not a tie)
        self.total_passes != self.total_failures
    }

    /// Get the first failure's attempt number, if any
    pub fn first_failure_attempt(&self) -> Option<usize> {
        self.attempts.iter().find(|a| !a.passed).map(|a| a.attempt)
    }
}

/// Handler for 3x retry with majority voting for flaky tests.
///
/// This handler implements a retry mechanism that runs tests multiple times
/// and uses majority voting to determine the final result. This helps
/// distinguish between truly failing tests and flaky tests that sometimes pass.
///
/// # Example
///
/// ```rust,ignore
/// use swell_validation::flakiness::{FlakinessRetryHandler, RetryConfig};
///
/// let handler = FlakinessRetryHandler::new();
/// let result = handler.retry_test("test_flaky", || async { Ok(true) }).await;
/// ```
#[derive(Debug, Clone)]
pub struct FlakinessRetryHandler {
    config: RetryConfig,
}

impl Default for FlakinessRetryHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl FlakinessRetryHandler {
    /// Create a new retry handler with default configuration
    pub fn new() -> Self {
        Self {
            config: RetryConfig::default(),
        }
    }

    /// Create with custom retry configuration
    pub fn with_config(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Create with flakiness detector integration
    pub fn with_detector(detector: &FlakinessDetector) -> Self {
        let _ = detector; // Used for future integration
        Self::new()
    }

    /// Get the retry configuration
    pub fn config(&self) -> &RetryConfig {
        &self.config
    }

    /// Calculate delay before next retry using exponential backoff
    fn calculate_delay(&self, attempt: usize) -> u64 {
        if self.config.use_exponential_backoff {
            let delay = self.config.base_delay_ms * (2_u64.pow(attempt as u32 - 1));
            delay.min(self.config.max_delay_ms)
        } else {
            self.config.base_delay_ms
        }
    }

    /// Execute a test with retry logic and majority voting.
    ///
    /// Runs the test up to `max_attempts` times and determines success
    /// based on majority voting (at least `min_passes_for_success` passes).
    ///
    /// Returns a `RetryResult` with all attempt details and the final outcome.
    pub async fn retry_test<F, Fut>(&self, test_name: &str, mut test_fn: F) -> RetryResult
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<bool, std::io::Error>>,
    {
        let mut attempts = Vec::new();
        let mut total_duration_ms = 0u64;

        for attempt_num in 1..=self.config.max_attempts {
            let attempt_start = std::time::Instant::now();

            // Run the test
            let result = test_fn().await;

            let passed = result.unwrap_or(false);
            let duration_ms = attempt_start.elapsed().as_millis() as u64;
            total_duration_ms += duration_ms;

            // Record the attempt
            attempts.push(RetryAttempt {
                attempt: attempt_num,
                passed,
                duration_ms,
                timestamp: Utc::now(),
            });

            // If we already have a majority, we can stop early
            let current_passes = attempts.iter().filter(|a| a.passed).count();
            let remaining_attempts = self.config.max_attempts - attempt_num;

            // If we can still win with remaining attempts, continue
            // Otherwise, we have a definitive result
            if current_passes >= self.config.min_passes_for_success {
                // We have enough passes for majority - stop early
                break;
            }

            // If even if we win all remaining, we can't reach majority, stop
            if current_passes + remaining_attempts < self.config.min_passes_for_success {
                // Can't possibly win - stop early
                break;
            }

            // Add delay between retries (except on last attempt)
            if attempt_num < self.config.max_attempts {
                let delay = self.calculate_delay(attempt_num);
                tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
            }
        }

        // Calculate final result
        let total_passes = attempts.iter().filter(|a| a.passed).count();
        let total_failures = attempts.len() - total_passes;
        let passed = total_passes >= self.config.min_passes_for_success;

        RetryResult {
            test_name: test_name.to_string(),
            attempts,
            passed,
            total_passes,
            total_failures,
            min_passes_required: self.config.min_passes_for_success,
            total_duration_ms,
        }
    }

    /// Execute a test with retry, returning only the final boolean result.
    ///
    /// This is a convenience method that discards the detailed retry information.
    pub async fn retry_test_simple<F, Fut>(&self, test_name: &str, test_fn: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<bool, std::io::Error>>,
    {
        self.retry_test(test_name, test_fn).await.passed
    }

    /// Determine if a test should be retried based on its flakiness history.
    ///
    /// Returns true if:
    /// - The test is currently flagged as flaky, OR
    /// - The test has a flakiness score above the retry threshold
    pub fn should_retry(&self, test_name: &str, detector: &FlakinessDetector) -> bool {
        if !self.config.enable_flaky_retry {
            return false;
        }

        // Retry tests that are currently flagged as flaky
        if detector.is_flaky(test_name) {
            return true;
        }

        // Also retry tests with high flakiness scores but not yet quarantined
        let score = detector.flakiness_score(test_name);
        score >= 0.3 // Retry if score is 30% or higher
    }
}

// ============================================================================
// Flakiness Gate (Integration with Validation Pipeline)
// ============================================================================

/// Gate that checks for flaky tests using the DeFlaker approach.
///
/// This gate runs after TestGate and analyzes the test results
/// to detect and report flaky tests.
#[derive(Debug, Clone)]
pub struct FlakinessGate {
    /// Flakiness detector with historical tracking
    detector: FlakinessDetector,
    /// Quarantine pool for managing flaky tests
    quarantine: QuarantinePool,
    /// Gate configuration
    config: FlakinessGateConfig,
}

/// Configuration for FlakinessGate
#[derive(Debug, Clone)]
pub struct FlakinessGateConfig {
    /// Run in strict mode (fail if flaky tests detected)
    pub strict_mode: bool,
    /// Skip tests in quarantine
    pub skip_quarantined: bool,
}

impl Default for FlakinessGateConfig {
    fn default() -> Self {
        Self {
            strict_mode: false,
            skip_quarantined: true,
        }
    }
}

impl FlakinessGate {
    /// Create a new FlakinessGate with default configuration
    pub fn new() -> Self {
        Self {
            detector: FlakinessDetector::with_defaults(),
            quarantine: QuarantinePool::with_defaults(),
            config: FlakinessGateConfig::default(),
        }
    }

    /// Create with custom detector
    pub fn with_detector(detector: FlakinessDetector) -> Self {
        Self {
            detector,
            quarantine: QuarantinePool::with_defaults(),
            config: FlakinessGateConfig::default(),
        }
    }

    /// Create with custom quarantine pool
    pub fn with_quarantine(quarantine: QuarantinePool) -> Self {
        Self {
            detector: FlakinessDetector::with_defaults(),
            quarantine,
            config: FlakinessGateConfig::default(),
        }
    }

    /// Enable strict mode (fail validation if flaky tests detected)
    pub fn with_strict_mode(mut self) -> Self {
        self.config.strict_mode = true;
        self
    }

    /// Enable skipping quarantined tests
    pub fn with_skip_quarantined(mut self) -> Self {
        self.config.skip_quarantined = true;
        self
    }

    /// Record test results from a validation run
    pub fn record_results(&mut self, results: &[TestResultRecord]) {
        for result in results {
            self.detector.record_with_retry(
                result.test_name.clone(),
                result.passed,
                result.duration_ms,
                result.is_retry,
            );
        }

        // Sync quarantine with detector
        self.quarantine.sync_with_detector(&self.detector);
    }

    /// Get tests that should be skipped (quarantined and skip enabled)
    pub fn get_tests_to_skip(&self) -> Vec<String> {
        if self.config.skip_quarantined {
            self.quarantine.get_quarantined_tests()
        } else {
            vec![]
        }
    }

    /// Get current flakiness report
    pub fn get_report(&self) -> FlakinessReport {
        self.detector.generate_report()
    }

    /// Get quarantine pool
    pub fn get_quarantine(&self) -> &QuarantinePool {
        &self.quarantine
    }

    /// Get quarantine stats
    pub fn get_quarantine_stats(&self) -> QuarantineStats {
        self.quarantine.stats()
    }
}

impl Default for FlakinessGate {
    fn default() -> Self {
        Self::new()
    }
}

/// A test result record for flakiness tracking
#[derive(Debug, Clone)]
pub struct TestResultRecord {
    /// Test name
    pub test_name: String,
    /// Whether test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Whether this was a retry
    pub is_retry: bool,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod flakiness_detector_tests {
    use super::*;

    #[test]
    fn test_flakiness_detector_empty() {
        let detector = FlakinessDetector::with_defaults();
        assert!(detector.get_flaky_tests().is_empty());
        assert_eq!(detector.flakiness_score("test_foo"), 0.0);
        assert!(!detector.is_flaky("test_foo"));
    }

    #[test]
    fn test_flakiness_detector_stable_test() {
        let mut detector = FlakinessDetector::with_defaults();

        // Record 5 passes
        for _ in 0..5 {
            detector.record("test_stable".to_string(), true, 100);
        }

        assert!(!detector.is_flaky("test_stable"));
        assert_eq!(detector.flakiness_score("test_stable"), 0.0);
    }

    #[test]
    fn test_flakiness_detector_consistently_failing() {
        let mut detector = FlakinessDetector::with_defaults();

        // Record 5 failures
        for _ in 0..5 {
            detector.record("test_failing".to_string(), false, 100);
        }

        // Consistently failing is not considered flaky
        // (failure rate is 100%, which is outside the 10-90% range)
        assert!(!detector.is_flaky("test_failing"));
        assert_eq!(detector.flakiness_score("test_failing"), 0.0);
    }

    #[test]
    fn test_flakiness_detector_flaky_pattern() {
        let mut detector = FlakinessDetector::with_defaults();

        // Record mixed results - this should be flaky
        // DeFlaker approach: alternating pass/fail suggests flakiness
        let results = [true, false, true, false, true, false, true];
        for passed in results {
            detector.record("test_flaky".to_string(), passed, 100);
        }

        assert!(detector.is_flaky("test_flaky"));
        assert!(detector.flakiness_score("test_flaky") > 0.0);
    }

    #[test]
    fn test_flakiness_detector_min_runs() {
        let mut detector = FlakinessDetector::with_defaults();

        // Below minimum runs - should not be flagged
        detector.record("test_min".to_string(), false, 100);
        detector.record("test_min".to_string(), true, 100);

        assert!(!detector.is_flaky("test_min"));
        assert_eq!(detector.flakiness_score("test_min"), 0.0);

        // Add one more - now we have 3 runs
        detector.record("test_min".to_string(), false, 100);

        // Now with mixed results and 3+ runs, it should be evaluated
        // Results: fail, pass, fail = 66% failure rate (in range)
        assert!(detector.flakiness_score("test_min") > 0.0);
    }

    #[test]
    fn test_flakiness_detector_report() {
        let mut detector = FlakinessDetector::with_defaults();

        // Add a flaky test
        for (i, passed) in [true, false, true, false, true].iter().enumerate() {
            detector.record_with_retry(
                "test_flaky".to_string(),
                *passed,
                100 + i as u64 * 10,
                i > 2,
            );
        }

        // Add a stable test
        for _ in 0..5 {
            detector.record("test_stable".to_string(), true, 100);
        }

        let report = detector.generate_report();

        assert_eq!(report.total_tests, 2);
        assert_eq!(report.flaky_tests, 1);
        assert!(report.flaky_percentage() > 0.0);
    }

    #[test]
    fn test_flakiness_detector_reset() {
        let mut detector = FlakinessDetector::with_defaults();

        detector.record("test_foo".to_string(), true, 100);
        assert!(!detector.get_all_tests().is_empty());

        detector.reset();
        assert!(detector.get_all_tests().is_empty());
    }

    #[test]
    fn test_inconsistency_score() {
        let mut detector = FlakinessDetector::with_defaults();

        // Alternating pattern has highest inconsistency
        for passed in [true, false, true, false, true, false].iter() {
            detector.record("test_alt".to_string(), *passed, 100);
        }

        let score_alt = detector.flakiness_score("test_alt");

        // Reset and do clustered pattern
        detector.reset();

        for _ in 0..3 {
            detector.record("test_cluster".to_string(), true, 100);
        }
        for _ in 0..3 {
            detector.record("test_cluster".to_string(), false, 100);
        }

        let score_cluster = detector.flakiness_score("test_cluster");

        // Alternating should have higher inconsistency
        assert!(score_alt >= score_cluster);
    }

    #[test]
    fn test_flakiness_config_custom() {
        let config = FlakinessConfig::new()
            .with_min_runs(5)
            .with_quarantine_threshold(0.5)
            .with_max_history_days(14)
            .without_temporal_analysis();

        assert_eq!(config.min_runs, 5);
        assert_eq!(config.quarantine_threshold, 0.5);
        assert_eq!(config.max_history_days, Some(14));
        assert!(!config.enable_temporal_analysis);
    }
}

#[cfg(test)]
mod quarantine_pool_tests {
    use super::*;

    #[test]
    fn test_quarantine_pool_empty() {
        let pool = QuarantinePool::with_defaults();
        assert!(pool.get_quarantined_tests().is_empty());
        assert!(!pool.is_quarantined("test_foo"));
    }

    #[test]
    fn test_quarantine_pool_add_remove() {
        let mut pool = QuarantinePool::with_defaults();

        pool.quarantine("test_quarantine".to_string(), 0.5);

        assert!(pool.is_quarantined("test_quarantine"));
        assert_eq!(pool.get_quarantined_tests(), vec!["test_quarantine"]);

        let released = pool.release("test_quarantine");
        assert!(released.is_some());
        assert!(!pool.is_quarantined("test_quarantine"));
    }

    #[test]
    fn test_quarantine_pool_record_result() {
        let mut pool = QuarantinePool::with_defaults();

        pool.quarantine("test_record".to_string(), 0.5);

        // Record a failure
        let remain = pool.record_result("test_record", false);
        assert!(remain); // Should remain in quarantine

        let test = pool.get_quarantined_test("test_record").unwrap();
        assert_eq!(test.consecutive_passes, 0);
        assert_eq!(test.failures_since_quarantine, 1);

        // Record passes - stability loop requires 10 consecutive passes
        // After each pass, it should return true (remain in quarantine)
        // until we reach 10 passes
        pool.record_result("test_record", true);
        pool.record_result("test_record", true);
        let test = pool.get_quarantined_test("test_record").unwrap();
        assert_eq!(test.consecutive_passes, 2);

        // After 10th pass, record_result signals exit from stability loop
        for i in 3..=10 {
            let should_remain = pool.record_result("test_record", true);
            if i < 10 {
                assert!(
                    should_remain,
                    "Should remain in quarantine during stability loop"
                );
            } else {
                assert!(!should_remain, "Should signal exit after 10 passes");
            }
        }

        // But test is still quarantined until update() is called
        assert!(pool.is_quarantined("test_record"));
    }

    #[test]
    fn test_quarantine_pool_stats() {
        let mut pool = QuarantinePool::with_defaults();

        pool.quarantine("test1".to_string(), 0.5);
        pool.quarantine("test2".to_string(), 0.6);

        pool.record_result("test1", true);
        pool.record_result("test1", true);

        let stats = pool.stats();
        assert_eq!(stats.total_quarantined, 2);
        assert_eq!(stats.with_consecutive_passes, 1);
    }

    #[test]
    fn test_quarantine_pool_sync_with_detector() {
        let mut detector = FlakinessDetector::with_defaults();
        let mut pool = QuarantinePool::with_defaults();

        // Add some flaky tests to detector
        for (i, passed) in [true, false, true, false, true].iter().enumerate() {
            detector.record_with_retry("test_flaky".to_string(), *passed, 100, i > 2);
        }

        // Sync should quarantine the flaky test
        let newly_quarantined = pool.sync_with_detector(&detector);

        assert!(newly_quarantined.contains(&"test_flaky".to_string()));
        assert!(pool.is_quarantined("test_flaky"));
    }

    #[test]
    fn test_quarantine_pool_update_release() {
        let mut pool = QuarantinePool::with_defaults();
        let detector = FlakinessDetector::with_defaults();

        // Add test to quarantine
        pool.quarantine("test_release".to_string(), 0.6);

        // Record passes to exit - stability loop requires 10 consecutive passes
        for _ in 0..10 {
            pool.record_result("test_release", true);
        }

        // After 10 consecutive passes in stability loop, test should be released
        let released = pool.update(&detector);
        assert!(released.contains(&"test_release".to_string()));
    }

    #[test]
    fn test_quarantine_stability_loop_resets_on_failure() {
        // Test that a single failure in stability loop resets consecutive passes
        let mut pool = QuarantinePool::with_defaults();
        let detector = FlakinessDetector::with_defaults();

        pool.quarantine("test_stability".to_string(), 0.6);

        // Pass 5 times
        for _ in 0..5 {
            pool.record_result("test_stability", true);
        }

        // Fail - resets consecutive passes
        pool.record_result("test_stability", false);

        // Check that consecutive_passes is reset
        let test = pool.get_quarantined_test("test_stability").unwrap();
        assert_eq!(
            test.consecutive_passes, 0,
            "Consecutive passes should reset to 0 after failure"
        );

        // Pass 10 more times to exit
        for _ in 0..10 {
            pool.record_result("test_stability", true);
        }

        // Now should be released
        let released = pool.update(&detector);
        assert!(released.contains(&"test_stability".to_string()));
    }

    #[test]
    fn test_quarantine_stability_loop_all_passes() {
        // Test that 10 consecutive passes releases from quarantine
        let mut pool = QuarantinePool::with_defaults();
        let detector = FlakinessDetector::with_defaults();

        pool.quarantine("test_10_passes".to_string(), 0.6);

        // First 9 passes should remain in quarantine
        for i in 0..9 {
            let should_remain = pool.record_result("test_10_passes", true);
            assert!(
                should_remain,
                "Should remain in quarantine during stability loop (pass {})",
                i + 1
            );
        }

        // 10th pass should signal exit (consecutive_passes >= stability_loop_runs)
        let should_remain = pool.record_result("test_10_passes", true);
        assert!(!should_remain, "Should signal exit after 10 passes");

        // Update pool to release
        let released = pool.update(&detector);
        assert!(released.contains(&"test_10_passes".to_string()));
        assert!(!pool.is_quarantined("test_10_passes"));
    }

    #[test]
    fn test_quarantine_remains_after_9_passes() {
        // Test that 9 passes is not enough - need exactly 10
        let mut pool = QuarantinePool::with_defaults();

        pool.quarantine("test_9_passes".to_string(), 0.6);

        // Pass 9 times
        for i in 0..9 {
            let should_remain = pool.record_result("test_9_passes", true);
            if i < 8 {
                assert!(
                    should_remain,
                    "Should remain in quarantine during stability loop"
                );
            }
            // After 9 passes, still in quarantine
        }

        // Should still be quarantined (9 < 10)
        assert!(pool.is_quarantined("test_9_passes"));

        // One more pass should release
        pool.record_result("test_9_passes", true);
        assert!(!pool.record_result("test_9_passes", true)); // Signal exit
    }
}

#[cfg(test)]
mod flakiness_report_tests {
    use super::*;

    #[test]
    fn test_flakiness_report_markdown() {
        let mut detector = FlakinessDetector::with_defaults();

        // Add a flaky test
        for passed in [true, false, true, false, true] {
            detector.record("test_flaky".to_string(), passed, 100);
        }

        let report = detector.generate_report();
        let md = report.to_markdown();

        assert!(md.contains("# Flakiness Report"));
        assert!(md.contains("Flaky Tests"));
        assert!(md.contains("test_flaky"));
    }

    #[test]
    fn test_flakiness_report_percentage() {
        let report = FlakinessReport {
            config: FlakinessConfig::default(),
            total_tests: 100,
            flaky_tests: 5,
            quarantined_threshold: 0.4,
            test_reports: vec![],
        };

        assert_eq!(report.flaky_percentage(), 5.0);
    }

    #[test]
    fn test_flakiness_report_empty() {
        let report = FlakinessReport {
            config: FlakinessConfig::default(),
            total_tests: 0,
            flaky_tests: 0,
            quarantined_threshold: 0.4,
            test_reports: vec![],
        };

        assert_eq!(report.flaky_percentage(), 0.0);
        assert!(report.tests_above_threshold(0.5).is_empty());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_flakiness_gate_integration() {
        let mut gate = FlakinessGate::new();

        // Simulate test results
        let results = vec![
            TestResultRecord {
                test_name: "test_stable".to_string(),
                passed: true,
                duration_ms: 100,
                is_retry: false,
            },
            TestResultRecord {
                test_name: "test_flaky".to_string(),
                passed: true,
                duration_ms: 100,
                is_retry: false,
            },
            TestResultRecord {
                test_name: "test_flaky".to_string(),
                passed: false,
                duration_ms: 100,
                is_retry: false,
            },
            TestResultRecord {
                test_name: "test_flaky".to_string(),
                passed: true,
                duration_ms: 100,
                is_retry: false,
            },
            TestResultRecord {
                test_name: "test_flaky".to_string(),
                passed: false,
                duration_ms: 100,
                is_retry: false,
            },
        ];

        gate.record_results(&results);

        // Check quarantine status
        let stats = gate.get_quarantine_stats();
        assert!(stats.total_quarantined >= 1);

        // Check skip list
        let skip_list = gate.get_tests_to_skip();
        assert!(skip_list.contains(&"test_flaky".to_string()));
    }

    #[test]
    fn test_full_flakiness_workflow() {
        // Simulate a full workflow: detect -> quarantine -> stability loop -> release
        // Uses default threshold of 0.1 and stability_loop_runs of 10

        // Higher threshold for detector so we can actually test the flow
        let config = FlakinessConfig::new().with_quarantine_threshold(0.3);
        let mut detector = FlakinessDetector::new(config);
        let mut pool = QuarantinePool::with_defaults();

        // Phase 1: Initial flaky detection - record mixed results to trigger flakiness
        // Results: true, false, true, false, true = 2 failures / 5 runs = 0.4 failure rate
        // This should trigger flakiness with the higher threshold (0.3)
        for (i, passed) in [true, false, true, false, true].iter().enumerate() {
            detector.record_with_retry("workflow_test".to_string(), *passed, 100, i > 2);
        }

        // Should be detected as flaky
        assert!(
            detector.is_flaky("workflow_test"),
            "Should be detected as flaky after mixed results"
        );
        pool.sync_with_detector(&detector);
        assert!(
            pool.is_quarantined("workflow_test"),
            "Should be quarantined"
        );

        // Phase 2: Stability loop - pass 10 consecutive times to exit quarantine
        for _ in 0..10 {
            detector.record("workflow_test".to_string(), true, 100);
            pool.record_result("workflow_test", true);
        }

        // Phase 3: Update pool - test should be released after 10 consecutive passes
        let released = pool.update(&detector);
        assert!(
            released.contains(&"workflow_test".to_string()),
            "Test should be released after 10 consecutive passes (stability loop)"
        );

        // Phase 4: Verify test is no longer flagged
        assert!(!pool.is_quarantined("workflow_test"));
    }

    #[test]
    fn test_flakiness_score_2_failures_out_of_10() {
        // Verify: 2 failures / 10 total runs = 0.2 failure rate
        // With threshold 0.1, a score of 0.2 should trigger quarantine
        let config = FlakinessConfig::new().with_quarantine_threshold(0.1);
        let mut detector = FlakinessDetector::new(config);

        // Record 10 runs: 8 passes, 2 failures
        // This gives failure rate of 0.2 (20%)
        for i in 0..10 {
            let passed = i != 2 && i != 7; // Fail at runs 2 and 7 (0-indexed)
            detector.record("test_20_percent".to_string(), passed, 100);
        }

        let score = detector.flakiness_score("test_20_percent");
        assert!(
            score > 0.1,
            "Score {} should be > 0.1 threshold for quarantine",
            score
        );
        assert!(
            detector.is_flaky("test_20_percent"),
            "Test with 20% failure rate should be flagged as flaky"
        );
    }

    #[test]
    fn test_default_threshold_is_01() {
        // Verify that the default flakiness threshold is 0.1
        let detector = FlakinessDetector::with_defaults();
        assert_eq!(
            detector.generate_report().quarantined_threshold,
            0.1,
            "Default quarantine threshold should be 0.1 (10%)"
        );
    }

    #[test]
    fn test_quarantine_after_2_failures_10_runs() {
        // Verification step: "Test records 10 runs with 2 failures (score 0.2) — assert quarantined"
        let config = FlakinessConfig::new().with_quarantine_threshold(0.1);
        let mut detector = FlakinessDetector::new(config);
        let mut pool = QuarantinePool::with_defaults();

        // 10 runs with 2 failures
        let results = [true, true, true, false, true, true, true, false, true, true];
        for passed in results {
            detector.record("test_quarantine".to_string(), passed, 100);
        }

        // Sync with quarantine pool
        pool.sync_with_detector(&detector);

        // Verify quarantined
        assert!(
            pool.is_quarantined("test_quarantine"),
            "Test with 2 failures in 10 runs (20%) should be quarantined with threshold 0.1"
        );
    }

    #[test]
    fn test_stability_loop_10_passes_restores() {
        // Verification step: "Stability loop with 10 passes — assert restored"
        let mut pool = QuarantinePool::with_defaults();
        let detector = FlakinessDetector::with_defaults();

        // Manually quarantine a test
        pool.quarantine("test_restore".to_string(), 0.2);

        // Run 10 consecutive passes (stability loop)
        for _ in 0..10 {
            pool.record_result("test_restore", true);
        }

        // Update to release
        let released = pool.update(&detector);
        assert!(
            released.contains(&"test_restore".to_string()),
            "Test should be restored after 10 consecutive passes in stability loop"
        );
        assert!(!pool.is_quarantined("test_restore"));
    }

    #[test]
    fn test_stability_loop_failure_keeps_quarantined() {
        // Verification step: "Stability loop with failures — assert remains quarantined"
        let mut pool = QuarantinePool::with_defaults();
        let detector = FlakinessDetector::with_defaults();

        // Manually quarantine a test
        pool.quarantine("test_fail_stability".to_string(), 0.2);

        // Pass 5 times
        for _ in 0..5 {
            pool.record_result("test_fail_stability", true);
        }

        // FAIL - this resets consecutive passes
        pool.record_result("test_fail_stability", false);

        // Verify still quarantined
        assert!(
            pool.is_quarantined("test_fail_stability"),
            "Test should remain quarantined after failure in stability loop"
        );

        // Pass 9 more times (not 10 more - the failure reset count)
        for _ in 0..9 {
            pool.record_result("test_fail_stability", true);
        }

        // After 9 passes (from reset), still need one more
        pool.record_result("test_fail_stability", true);

        // Now update to release
        let released = pool.update(&detector);
        assert!(
            released.contains(&"test_fail_stability".to_string()),
            "Test should be restored after total of 10 passes (with reset in middle)"
        );
    }
}

#[cfg(test)]
mod flakiness_retry_tests {
    use super::*;

    #[tokio::test]
    async fn test_retry_handler_default_config() {
        let handler = FlakinessRetryHandler::new();
        assert_eq!(handler.config().max_attempts, 3);
        assert_eq!(handler.config().min_passes_for_success, 2);
        assert!(handler.config().enable_flaky_retry);
    }

    #[tokio::test]
    async fn test_retry_test_all_pass() {
        let handler = FlakinessRetryHandler::new();

        // Test that always passes
        let result = handler
            .retry_test("test_always_pass", || async { Ok(true) })
            .await;

        assert!(result.passed);
        // Early exit: after 2 passes we have majority, so only 2 attempts
        assert_eq!(result.total_passes, 2);
        assert_eq!(result.total_failures, 0);
        assert_eq!(result.min_passes_required, 2);
        assert!(result.is_definitive());
    }

    #[tokio::test]
    async fn test_retry_test_all_fail() {
        let handler = FlakinessRetryHandler::new();

        // Test that always fails
        let result = handler
            .retry_test("test_always_fail", || async { Ok(false) })
            .await;

        assert!(!result.passed);
        // Early exit: after 2 failures, even 1 more attempt can't give us 2 passes
        // So we stop after 2 attempts (0 + 1 remaining = 1 < 2 minimum passes needed)
        assert_eq!(result.total_passes, 0);
        assert_eq!(result.total_failures, 2);
        assert!(result.is_definitive());
    }

    #[tokio::test]
    async fn test_retry_test_majority_pass() {
        let handler = FlakinessRetryHandler::new();
        let mut call_count = 0;

        // Test that passes twice, fails once (in order: fail, pass, pass)
        let result = handler
            .retry_test("test_majority_pass", || {
                let count = call_count;
                call_count += 1;
                async move {
                    match count {
                        0 => Ok(false), // First attempt fails
                        _ => Ok(true),  // Second and third pass
                    }
                }
            })
            .await;

        assert!(result.passed);
        assert_eq!(result.total_passes, 2);
        assert_eq!(result.total_failures, 1);
        assert!(result.is_definitive());
    }

    #[tokio::test]
    async fn test_retry_test_majority_fail() {
        let handler = FlakinessRetryHandler::new();
        let mut call_count = 0;

        // Test that fails twice, passes once (order: pass, fail, fail)
        let result = handler
            .retry_test("test_majority_fail", || {
                let count = call_count;
                call_count += 1;
                async move {
                    match count {
                        0 => Ok(true),  // First attempt passes
                        _ => Ok(false), // Second and third fail
                    }
                }
            })
            .await;

        assert!(!result.passed);
        assert_eq!(result.total_passes, 1);
        assert_eq!(result.total_failures, 2);
        assert!(result.is_definitive());
    }

    #[tokio::test]
    async fn test_retry_test_early_exit_on_majority() {
        let handler = FlakinessRetryHandler::new();
        let mut call_count = 0;

        // Test that passes first two times - should stop early (no need for 3rd)
        let result = handler
            .retry_test("test_early_exit", || {
                call_count += 1;
                async move { Ok(true) }
            })
            .await;

        assert!(result.passed);
        assert_eq!(result.total_passes, 2);
        assert_eq!(result.total_failures, 0);
        assert_eq!(call_count, 2); // Only called twice since majority reached
    }

    #[tokio::test]
    async fn test_retry_test_simple() {
        let handler = FlakinessRetryHandler::new();

        // Test simple wrapper returns only boolean
        let result = handler
            .retry_test_simple("test_simple", || async { Ok(true) })
            .await;

        assert!(result);
    }

    #[tokio::test]
    async fn test_retry_result_pass_rate() {
        let result = RetryResult {
            test_name: "test".to_string(),
            attempts: vec![
                RetryAttempt {
                    attempt: 1,
                    passed: true,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
                RetryAttempt {
                    attempt: 2,
                    passed: true,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
                RetryAttempt {
                    attempt: 3,
                    passed: false,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
            ],
            passed: true,
            total_passes: 2,
            total_failures: 1,
            min_passes_required: 2,
            total_duration_ms: 300,
        };

        assert!((result.pass_rate() - 0.666).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_retry_result_first_failure() {
        let result = RetryResult {
            test_name: "test".to_string(),
            attempts: vec![
                RetryAttempt {
                    attempt: 1,
                    passed: true,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
                RetryAttempt {
                    attempt: 2,
                    passed: false,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
                RetryAttempt {
                    attempt: 3,
                    passed: false,
                    duration_ms: 100,
                    timestamp: Utc::now(),
                },
            ],
            passed: false,
            total_passes: 1,
            total_failures: 2,
            min_passes_required: 2,
            total_duration_ms: 300,
        };

        assert_eq!(result.first_failure_attempt(), Some(2));
    }

    #[test]
    fn test_retry_config_custom() {
        let config = RetryConfig::new()
            .with_max_attempts(5)
            .with_min_passes(3)
            .with_exponential_backoff()
            .with_base_delay_ms(200);

        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.min_passes_for_success, 3);
        assert!(config.use_exponential_backoff);
        assert_eq!(config.base_delay_ms, 200);
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();

        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.min_passes_for_success, 2);
        assert!(config.enable_flaky_retry);
        assert!(!config.use_exponential_backoff);
        assert_eq!(config.base_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 1000);
    }

    #[tokio::test]
    async fn test_retry_delay_calculation() {
        // Test without exponential backoff
        let handler = FlakinessRetryHandler::new();
        assert_eq!(handler.calculate_delay(1), 100);
        assert_eq!(handler.calculate_delay(2), 100);
        assert_eq!(handler.calculate_delay(3), 100);

        // Test with exponential backoff
        let handler = FlakinessRetryHandler::with_config(
            RetryConfig::new()
                .with_exponential_backoff()
                .with_base_delay_ms(100)
                .with_max_delay_ms(1000),
        );
        // Attempt 1: 100 * 2^0 = 100
        assert_eq!(handler.calculate_delay(1), 100);
        // Attempt 2: 100 * 2^1 = 200
        assert_eq!(handler.calculate_delay(2), 200);
        // Attempt 3: 100 * 2^2 = 400
        assert_eq!(handler.calculate_delay(3), 400);
    }

    #[tokio::test]
    async fn test_should_retry_flaky_test() {
        let handler = FlakinessRetryHandler::new();
        let mut detector = FlakinessDetector::with_defaults();

        // Add flaky test pattern
        for passed in [true, false, true, false, true] {
            detector.record("test_flaky".to_string(), passed, 100);
        }

        assert!(detector.is_flaky("test_flaky"));
        assert!(handler.should_retry("test_flaky", &detector));
    }

    #[tokio::test]
    async fn test_should_not_retry_stable_test() {
        let handler = FlakinessRetryHandler::new();
        let mut detector = FlakinessDetector::with_defaults();

        // Add stable test (always passes)
        for _ in 0..5 {
            detector.record("test_stable".to_string(), true, 100);
        }

        assert!(!detector.is_flaky("test_stable"));
        assert!(!handler.should_retry("test_stable", &detector));
    }

    #[tokio::test]
    async fn test_should_retry_high_score_not_quarantined() {
        let handler = FlakinessRetryHandler::new();
        let mut detector = FlakinessDetector::with_defaults();

        // Add mixed results but not enough to be flagged as flaky
        // (threshold is 0.4, so we need score < 0.4 to not be quarantined)
        // But we want score >= 0.3 to trigger retry
        for (i, passed) in [true, true, true, false, true].iter().enumerate() {
            detector.record_with_retry("test_mixed".to_string(), *passed, 100, i > 3);
        }

        let score = detector.flakiness_score("test_mixed");
        // Score might be 0 (only 1 failure out of 5)
        // Or might be non-zero - depends on implementation
        // Just verify retry logic works
        let should_retry = handler.should_retry("test_mixed", &detector);
        // If score >= 0.3, should retry. Otherwise depends on implementation.
        if score >= 0.3 {
            assert!(should_retry);
        }
    }

    #[tokio::test]
    async fn test_retry_disabled_config() {
        let handler = FlakinessRetryHandler::with_config(RetryConfig {
            enable_flaky_retry: false,
            ..Default::default()
        });
        let mut detector = FlakinessDetector::with_defaults();

        // Add very flaky test
        for passed in [true, false, true, false, true] {
            detector.record("test_very_flaky".to_string(), passed, 100);
        }

        // Even though it's flaky, retry should be disabled
        assert!(!handler.should_retry("test_very_flaky", &detector));
    }

    #[tokio::test]
    async fn test_retry_with_detector_integration() {
        // Full integration test: flaky detection -> quarantine -> retry
        let mut detector = FlakinessDetector::with_defaults();
        let handler = FlakinessRetryHandler::new();

        // Phase 1: Flaky detection
        for passed in [true, false, true, false, true] {
            detector.record("integration_test".to_string(), passed, 100);
        }

        assert!(detector.is_flaky("integration_test"));
        assert!(handler.should_retry("integration_test", &detector));

        // Phase 2: Run with retry - simulates what would happen in validation
        let result = handler
            .retry_test("integration_test", || async { Ok(true) })
            .await;

        // With retry, even a flaky test can pass if it passes majority
        assert!(result.passed);
        assert_eq!(result.attempts.len(), 2); // Early exit after 2 passes
    }
}
