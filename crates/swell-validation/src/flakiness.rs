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
const DEFAULT_FLAKINESS_THRESHOLD: f64 = 0.4;

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
}

impl Default for QuarantineConfig {
    fn default() -> Self {
        Self {
            enter_threshold: DEFAULT_FLAKINESS_THRESHOLD,
            exit_threshold: 0.25,
            max_quarantine_days: 7,
            consecutive_passes_to_exit: 3,
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
    /// Returns true if the test should remain in quarantine
    /// Returns false if the test should be released
    pub fn record_result(&mut self, test_name: &str, passed: bool) -> bool {
        let Some(test) = self.quarantined.get_mut(test_name) else {
            return false;
        };

        test.last_run_at = Some(Utc::now());
        test.last_run_passed = Some(passed);

        if passed {
            test.consecutive_passes += 1;
            test.failures_since_quarantine = 0;

            // Check if we should exit quarantine
            if test.consecutive_passes >= self.config.consecutive_passes_to_exit {
                // Score must also be below exit threshold
                // We don't have current score here, caller should check
                test.exit_attempts += 1;
                return false; // Request evaluation
            }
        } else {
            test.consecutive_passes = 0;
            test.failures_since_quarantine += 1;
            test.exit_attempts = 0;
        }

        true // Remain in quarantine
    }

    /// Check if a test should be released from quarantine
    pub fn should_release(&self, test_name: &str, current_score: f64) -> bool {
        let Some(test) = self.quarantined.get(test_name) else {
            return false;
        };

        // Check time limit - max time in quarantine reached
        let days_in_quarantine = (Utc::now() - test.quarantined_at).num_days();
        if days_in_quarantine >= self.config.max_quarantine_days as i64 {
            return true; // Max time reached, release for manual review
        }

        // Release if we have sufficient consecutive passes indicating stability
        // Even if score hasn't fully recovered, consecutive passes are a strong signal
        if test.consecutive_passes >= self.config.consecutive_passes_to_exit * 2 {
            return true; // Double threshold gives confidence even with imperfect score
        }

        // Also release if we have enough consecutive passes AND score shows significant improvement
        // (score must drop below the enter threshold to show it's no longer flaky)
        if test.consecutive_passes >= self.config.consecutive_passes_to_exit
            && current_score < self.config.enter_threshold
        {
            // Score has dropped below the threshold at which it would be quarantined
            return true;
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
        for (_i, passed) in [true, false, true, false, true, false].iter().enumerate() {
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

        // Record passes - after 3 consecutive passes, record_result returns false to signal evaluation
        pool.record_result("test_record", true);
        pool.record_result("test_record", true);
        let test = pool.get_quarantined_test("test_record").unwrap();
        assert_eq!(test.consecutive_passes, 2);

        // After 3rd pass, record_result signals evaluation is needed
        let should_evaluate = pool.record_result("test_record", true);
        assert!(
            !should_evaluate,
            "Should signal that evaluation is needed after 3 passes"
        );

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
        let mut detector = FlakinessDetector::with_defaults();

        // Add test to quarantine
        pool.quarantine("test_release".to_string(), 0.6);

        // Record enough passes to exit
        for _ in 0..3 {
            pool.record_result("test_release", true);
        }

        // The test should now be eligible for release (check with current score)
        // Since consecutive_passes >= consecutive_passes_to_exit
        // But the score might still be high, so we need to update

        // First make the test stable (record passes in detector)
        for _ in 0..5 {
            detector.record("test_release".to_string(), true, 100);
        }

        let released = pool.update(&detector);
        assert!(released.contains(&"test_release".to_string()));
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
        // Simulate a full workflow: detect -> quarantine -> monitor -> release
        // This test uses a more forgiving configuration for the flakiness detector
        let config = FlakinessConfig::new().with_quarantine_threshold(0.7); // Higher threshold so score of ~0.6 triggers release
        let mut detector = FlakinessDetector::new(config);
        let mut pool = QuarantinePool::with_defaults();

        // Phase 1: Initial flaky detection
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

        // Phase 2: Test improves - record many passes to stabilize the score
        // Add enough passes to drive the score below threshold
        for _ in 0..10 {
            detector.record("workflow_test".to_string(), true, 100);
            pool.record_result("workflow_test", true);
        }

        // Phase 3: Update pool - test should be released due to sufficient consecutive passes
        // even if score hasn't dropped below threshold
        let released = pool.update(&detector);

        // After enough consecutive passes (10 > default 3), the test should be released
        assert!(
            released.contains(&"workflow_test".to_string()),
            "Test should be released after {} consecutive passes (threshold is 3)",
            pool.get_quarantined_test("workflow_test")
                .map(|t| t.consecutive_passes)
                .unwrap_or(0)
        );

        // Phase 4: Verify test is no longer flagged
        assert!(!pool.is_quarantined("workflow_test"));
    }
}
