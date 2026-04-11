//! Self-Improving Tests Module
//!
//! Tracks test value metrics over time and retires low-value tests that cost more than they provide.
//!
//! # Features
//!
//! - **Test Value Tracking**: Collects and maintains historical metrics for each test including:
//!   - Pass rate and trend
//!   - Flakiness index
//!   - Execution time (average, p95, p99)
//!   - Coverage contribution
//!   - Bug detection rate
//!
//! - **Cost/Benefit Analysis**: Calculates the value of each test based on:
//!   - **Cost**: Execution time, maintenance overhead, resource usage
//!   - **Benefit**: Coverage provided, bug detection history, regression prevention
//!
//! - **Automatic Retirement**: Identifies and can automatically retire low-value tests:
//!   - Tests that consistently fail without detecting real bugs
//!   - Tests that take too long relative to their coverage benefit
//!   - Duplicate or overlapping tests
//!   - Tests that haven't detected a bug in a long time
//!
//! # Usage
//!
//! ```rust
//! use swell_validation::self_improving_tests::{
//!     TestValueTracker, TestValueConfig, RetirementPolicy
//! };
//! use swell_validation::test_planning::TestCase;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let tracker = TestValueTracker::new(TestValueConfig::default());
//!
//!     // Record a test run
//!     tracker.record_test_run("tests::auth::test_login", true, 150).await?;
//!
//!     // Calculate value for a test
//!     let value = tracker.calculate_test_value("tests::auth::test_login").await?;
//!     println!("Test value score: {:.2}", value.score);
//!
//!     // Get cost/benefit analysis
//!     let analysis = tracker.analyze_cost_benefit("tests::auth::test_login").await?;
//!     println!("Cost: {:.2}ms, Benefit: {:.2}", analysis.cost.execution_time_ms, analysis.benefit.score);
//!
//!     // Find retirement candidates
//!     let candidates = tracker.find_retirement_candidates().await?;
//!     for candidate in candidates {
//!         println!("Consider retiring: {} (value: {:.2})", candidate.test_name, candidate.value_score);
//!     }
//!
//!     Ok(())
//! }
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use swell_core::{SwellError, ValidationOutcome};
use tokio::sync::RwLock;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for test value tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestValueConfig {
    /// Minimum number of runs before computing value (to ensure statistical significance)
    pub min_runs_for_value: usize,
    /// Weight for pass rate in value calculation (0.0 to 1.0)
    pub pass_rate_weight: f64,
    /// Weight for execution speed in value calculation
    pub speed_weight: f64,
    /// Weight for coverage contribution
    pub coverage_weight: f64,
    /// Weight for bug detection history
    pub bug_detection_weight: f64,
    /// Maximum acceptable execution time in ms before considered "slow"
    pub slow_test_threshold_ms: u64,
    /// Minimum value score to avoid retirement
    pub min_value_score: f64,
    /// Time window for analysis (days)
    pub analysis_window_days: u32,
    /// Flakiness threshold above which test is considered flaky (0.0 to 1.0)
    pub flakiness_threshold: f64,
}

impl Default for TestValueConfig {
    fn default() -> Self {
        Self {
            min_runs_for_value: 10,
            pass_rate_weight: 0.30,
            speed_weight: 0.20,
            coverage_weight: 0.25,
            bug_detection_weight: 0.25,
            slow_test_threshold_ms: 1000, // 1 second
            min_value_score: 0.3,
            analysis_window_days: 30,
            flakiness_threshold: 0.2, // 20% failure rate on "flaky" runs
        }
    }
}

/// Retirement policy settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetirementPolicy {
    /// Enable automatic retirement
    pub enabled: bool,
    /// Maximum tests that can be retired per analysis
    pub max_retirements_per_run: usize,
    /// Minimum value score to trigger retirement
    pub retirement_threshold: f64,
    /// Require manual approval for retirement
    pub require_approval: bool,
    /// Tests that are protected from retirement (by name pattern)
    pub protected_patterns: Vec<String>,
}

impl Default for RetirementPolicy {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for safety
            max_retirements_per_run: 5,
            retirement_threshold: 0.2,
            require_approval: true,
            protected_patterns: vec![
                "test_critical".to_string(),
                "test_security".to_string(),
                "integration_".to_string(),
            ],
        }
    }
}

// ============================================================================
// Test Metrics
// ============================================================================

/// Historical metrics for a single test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMetrics {
    /// Test name (fully qualified)
    pub test_name: String,
    /// Total number of runs
    pub total_runs: usize,
    /// Number of passes
    pub pass_count: usize,
    /// Number of failures
    pub fail_count: usize,
    /// Number of flaky runs (passed then failed on retry or vice versa)
    pub flaky_count: usize,
    /// Execution times in milliseconds (for statistics)
    pub execution_times_ms: Vec<u64>,
    /// Last run timestamp
    pub last_run: Option<DateTime<Utc>>,
    /// First run timestamp
    pub first_run: Option<DateTime<Utc>>,
    /// Number of bugs detected by this test
    pub bugs_detected: usize,
    /// Coverage lines (if known)
    pub coverage_lines: Option<usize>,
    /// Date when test was added to tracking
    pub tracked_since: DateTime<Utc>,
}

impl TestMetrics {
    /// Create new metrics for a test
    pub fn new(test_name: String) -> Self {
        Self {
            test_name,
            total_runs: 0,
            pass_count: 0,
            fail_count: 0,
            flaky_count: 0,
            execution_times_ms: Vec::new(),
            last_run: None,
            first_run: None,
            bugs_detected: 0,
            coverage_lines: None,
            tracked_since: Utc::now(),
        }
    }

    /// Record a test run
    pub fn record_run(&mut self, passed: bool, execution_time_ms: u64) {
        self.total_runs += 1;

        if passed {
            self.pass_count += 1;
        } else {
            self.fail_count += 1;
        }

        self.last_run = Some(Utc::now());
        self.first_run.get_or_insert(self.last_run.unwrap());
        self.execution_times_ms.push(execution_time_ms);

        // Keep only recent execution times (last 1000)
        if self.execution_times_ms.len() > 1000 {
            self.execution_times_ms.remove(0);
        }
    }

    /// Record this test detected a bug
    pub fn record_bug_detection(&mut self) {
        self.bugs_detected += 1;
    }

    /// Set coverage lines
    pub fn set_coverage(&mut self, lines: usize) {
        self.coverage_lines = Some(lines);
    }

    /// Calculate pass rate (0.0 to 1.0)
    pub fn pass_rate(&self) -> f64 {
        if self.total_runs == 0 {
            return 1.0; // New tests are assumed good
        }
        self.pass_count as f64 / self.total_runs as f64
    }

    /// Calculate flakiness index (0.0 to 1.0)
    /// Flakiness is the ratio of flaky runs to total runs
    pub fn flakiness_index(&self) -> f64 {
        if self.total_runs == 0 {
            return 0.0;
        }
        self.flaky_count as f64 / self.total_runs as f64
    }

    /// Calculate average execution time in ms
    pub fn avg_execution_time(&self) -> f64 {
        if self.execution_times_ms.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.execution_times_ms.iter().sum();
        sum as f64 / self.execution_times_ms.len() as f64
    }

    /// Calculate p95 execution time in ms
    pub fn p95_execution_time(&self) -> f64 {
        if self.execution_times_ms.is_empty() {
            return 0.0;
        }
        let mut times = self.execution_times_ms.clone();
        times.sort();
        let idx = (times.len() as f64 * 0.95) as usize;
        times[idx.min(times.len() - 1)] as f64
    }

    /// Calculate p99 execution time in ms
    pub fn p99_execution_time(&self) -> f64 {
        if self.execution_times_ms.is_empty() {
            return 0.0;
        }
        let mut times = self.execution_times_ms.clone();
        times.sort();
        let idx = (times.len() as f64 * 0.99) as usize;
        times[idx.min(times.len() - 1)] as f64
    }

    /// Check if test is considered "slow"
    pub fn is_slow(&self, threshold_ms: u64) -> bool {
        self.p95_execution_time() > threshold_ms as f64
    }

    /// Get age of the test in days
    pub fn age_days(&self) -> f64 {
        let now = Utc::now();
        let elapsed = now.signed_duration_since(self.tracked_since);
        elapsed.num_days() as f64
    }

    /// Get days since last bug detection
    pub fn days_since_last_bug(&self) -> Option<f64> {
        // This would need more tracking - for now return None
        // In a full implementation, we'd track last bug detection date
        None
    }
}

// ============================================================================
// Value Calculation
// ============================================================================

/// Calculated value for a test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestValue {
    /// Test name
    pub test_name: String,
    /// Overall value score (0.0 to 1.0)
    pub score: f64,
    /// Whether test is recommended for retirement
    pub recommended_for_retirement: bool,
    /// Retirement reason (if recommended)
    pub retirement_reason: Option<String>,
    /// Component scores
    pub components: ValueComponents,
    /// Confidence in the score (based on sample size)
    pub confidence: f64,
}

/// Component scores that make up the overall value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueComponents {
    /// Pass rate component (higher is better)
    pub pass_rate_score: f64,
    /// Speed component (faster is better)
    pub speed_score: f64,
    /// Coverage component (more coverage is better)
    pub coverage_score: f64,
    /// Bug detection component (detected bugs is better)
    pub bug_detection_score: f64,
    /// Stability component (lower flakiness is better)
    pub stability_score: f64,
}

/// Result of cost/benefit analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBenefitAnalysis {
    /// Test name
    pub test_name: String,
    /// Cost metrics
    pub cost: TestCost,
    /// Benefit metrics
    pub benefit: TestBenefit,
    /// Ratio of benefit to cost (higher is better)
    pub ratio: f64,
    /// Recommendation
    pub recommendation: CostBenefitRecommendation,
}

/// Cost metrics for a test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCost {
    /// Average execution time in ms
    pub execution_time_ms: f64,
    /// P95 execution time in ms
    pub p95_time_ms: f64,
    /// Is the test slow
    pub is_slow: bool,
    /// Maintenance burden (0.0 to 1.0, based on flakiness and failure rate)
    pub maintenance_burden: f64,
}

/// Benefit metrics for a test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestBenefit {
    /// Coverage contribution (0.0 to 1.0)
    pub coverage_contribution: f64,
    /// Bug detection rate (bugs detected per run)
    pub bug_detection_rate: f64,
    /// Regression prevention value (0.0 to 1.0)
    pub regression_prevention: f64,
    /// Overall benefit score (0.0 to 1.0)
    pub score: f64,
}

/// Recommendation based on cost/benefit analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostBenefitRecommendation {
    /// Keep the test - high value
    Keep,
    /// Monitor the test - moderate value
    Monitor,
    /// Consider retirement - low value
    ConsiderRetirement,
    /// Immediate retirement recommended
    Retire,
}

impl CostBenefitRecommendation {
    /// Get the threshold for this recommendation
    pub fn threshold(&self) -> f64 {
        match self {
            CostBenefitRecommendation::Keep => 0.7,
            CostBenefitRecommendation::Monitor => 0.4,
            CostBenefitRecommendation::ConsiderRetirement => 0.2,
            CostBenefitRecommendation::Retire => 0.0,
        }
    }
}

/// A test that is a candidate for retirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetirementCandidate {
    /// Test name
    pub test_name: String,
    /// Current value score
    pub value_score: f64,
    /// Reason for retirement recommendation
    pub reason: String,
    /// Confidence in the recommendation
    pub confidence: f64,
    /// Potential savings (ms per run)
    pub potential_savings_ms: f64,
    /// Alternative tests that provide overlapping coverage
    pub alternative_tests: Vec<String>,
}

/// Summary of test value analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestValueSummary {
    /// Total tests tracked
    pub total_tests: usize,
    /// Tests recommended for retirement
    pub retirement_candidates: Vec<RetirementCandidate>,
    /// High value tests (top performers)
    pub high_value_tests: Vec<String>,
    /// Slow tests identified
    pub slow_tests: Vec<String>,
    /// Flaky tests identified
    pub flaky_tests: Vec<String>,
    /// Overall test suite health score (0.0 to 1.0)
    pub health_score: f64,
    /// Potential time savings from retirement (ms per full run)
    pub potential_time_savings_ms: f64,
}

// ============================================================================
// Test Value Tracker
// ============================================================================

/// Main tracker for test value metrics
#[derive(Debug)]
pub struct TestValueTracker {
    config: TestValueConfig,
    policy: RetirementPolicy,
    metrics: Arc<RwLock<HashMap<String, TestMetrics>>>,
}

impl TestValueTracker {
    /// Create a new tracker with default configuration
    pub fn new(config: TestValueConfig) -> Self {
        Self {
            config,
            policy: RetirementPolicy::default(),
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create with custom retirement policy
    pub fn with_policy(mut self, policy: RetirementPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Record a test run
    pub async fn record_test_run(
        &self,
        test_name: &str,
        passed: bool,
        execution_time_ms: u64,
    ) -> Result<(), SwellError> {
        let mut metrics_map = self.metrics.write().await;
        let metrics = metrics_map
            .entry(test_name.to_string())
            .or_insert_with(|| TestMetrics::new(test_name.to_string()));
        metrics.record_run(passed, execution_time_ms);
        Ok(())
    }

    /// Record that a test detected a bug
    pub async fn record_bug_detection(&self, test_name: &str) -> Result<(), SwellError> {
        let mut metrics_map = self.metrics.write().await;
        if let Some(metrics) = metrics_map.get_mut(test_name) {
            metrics.record_bug_detection();
        }
        Ok(())
    }

    /// Set coverage for a test
    pub async fn set_coverage(&self, test_name: &str, lines: usize) -> Result<(), SwellError> {
        let mut metrics_map = self.metrics.write().await;
        if let Some(metrics) = metrics_map.get_mut(test_name) {
            metrics.set_coverage(lines);
        }
        Ok(())
    }

    /// Get metrics for a test
    pub async fn get_metrics(&self, test_name: &str) -> Option<TestMetrics> {
        let metrics_map = self.metrics.read().await;
        metrics_map.get(test_name).cloned()
    }

    /// Get all tracked test names
    pub async fn get_all_tests(&self) -> Vec<String> {
        let metrics_map = self.metrics.read().await;
        metrics_map.keys().cloned().collect()
    }

    /// Calculate value score for a test
    pub async fn calculate_test_value(&self, test_name: &str) -> Result<TestValue, SwellError> {
        let metrics_map = self.metrics.read().await;
        let metrics = metrics_map
            .get(test_name)
            .cloned()
            .ok_or_else(|| SwellError::InvalidOperation(format!("Test not found: {}", test_name)))?;

        drop(metrics_map);

        self.compute_value(&metrics).await
    }

    /// Internal: Compute value from metrics
    async fn compute_value(&self, metrics: &TestMetrics) -> Result<TestValue, SwellError> {
        let config = &self.config;

        // Calculate component scores
        let pass_rate_score = metrics.pass_rate();

        // Speed score: normalize against threshold
        // 0.0 = at or above threshold, 1.0 = instant (0ms)
        let avg_time = metrics.avg_execution_time();
        let speed_score = if avg_time == 0.0 {
            1.0 // No execution data = can't judge speed
        } else if avg_time >= config.slow_test_threshold_ms as f64 {
            // Linear scale from 0.5 at threshold to 0.0 at 10x threshold
            let ratio = avg_time / config.slow_test_threshold_ms as f64;
            (1.0 / ratio).max(0.0).min(1.0) * 0.5
        } else {
            // Exponential decay: fast tests get high scores
            let ratio = avg_time / config.slow_test_threshold_ms as f64;
            ((1.0 - ratio) * 0.5 + 0.5).max(0.0).min(1.0)
        };

        // Coverage score (0.0 to 1.0)
        let coverage_score = if let Some(lines) = metrics.coverage_lines {
            // Normalize: assume 10000 lines is "full coverage"
            (lines as f64 / 10000.0).min(1.0)
        } else {
            0.5 // Unknown coverage = medium score
        };

        // Bug detection score (0.0 to 1.0)
        // More bugs detected over time = higher score
        let days_active = metrics.age_days().max(1.0);
        let bug_detection_rate = metrics.bugs_detected as f64 / days_active;
        // Normalize: 1 bug per 30 days = 0.5, scale accordingly
        let bug_detection_score = (bug_detection_rate * 30.0).min(1.0);

        // Stability score (inverse of flakiness)
        let stability_score = 1.0 - metrics.flakiness_index();

        // Weighted overall score
        let raw_score = (pass_rate_score * config.pass_rate_weight)
            + (speed_score * config.speed_weight)
            + (coverage_score * config.coverage_weight)
            + (bug_detection_score * config.bug_detection_weight);

        // Apply stability as a multiplier (flaky tests are worth less)
        let score = raw_score * stability_score;

        // Check if recommended for retirement
        let recommended_for_retirement = score < config.min_value_score;
        let retirement_reason = if recommended_for_retirement {
            Some(format!(
                "Value score {:.2} below threshold {:.2}",
                score, config.min_value_score
            ))
        } else {
            None
        };

        // Confidence based on number of runs
        let confidence = if metrics.total_runs < config.min_runs_for_value {
            metrics.total_runs as f64 / config.min_runs_for_value as f64
        } else {
            1.0
        };

        Ok(TestValue {
            test_name: metrics.test_name.clone(),
            score,
            recommended_for_retirement,
            retirement_reason,
            components: ValueComponents {
                pass_rate_score,
                speed_score,
                coverage_score,
                bug_detection_score,
                stability_score,
            },
            confidence,
        })
    }

    /// Analyze cost/benefit for a test
    pub async fn analyze_cost_benefit(&self, test_name: &str) -> Result<CostBenefitAnalysis, SwellError> {
        let metrics_map = self.metrics.read().await;
        let metrics = metrics_map
            .get(test_name)
            .cloned()
            .ok_or_else(|| SwellError::InvalidOperation(format!("Test not found: {}", test_name)))?;

        let config = &self.config;

        // Calculate cost
        let is_slow = metrics.is_slow(config.slow_test_threshold_ms);
        let maintenance_burden = (1.0 - metrics.pass_rate()) * 0.5 + metrics.flakiness_index() * 0.5;

        let cost = TestCost {
            execution_time_ms: metrics.avg_execution_time(),
            p95_time_ms: metrics.p95_execution_time(),
            is_slow,
            maintenance_burden,
        };

        // Calculate benefit
        let coverage_contribution = if let Some(lines) = metrics.coverage_lines {
            (lines as f64 / 10000.0).min(1.0)
        } else {
            0.3 // Assume some coverage if unknown
        };

        let days_active = metrics.age_days().max(1.0);
        let bug_detection_rate = metrics.bugs_detected as f64 / (metrics.total_runs.max(1) as f64);
        let regression_prevention = metrics.pass_rate() * coverage_contribution;

        let benefit_score = (coverage_contribution + bug_detection_rate + regression_prevention) / 3.0;

        let benefit = TestBenefit {
            coverage_contribution,
            bug_detection_rate,
            regression_prevention,
            score: benefit_score,
        };

        // Calculate ratio (benefit / cost where cost is normalized 0-1)
        let cost_normalized = (cost.execution_time_ms as f64 / 10000.0).min(1.0);
        let ratio = if cost_normalized > 0.0 {
            benefit_score / cost_normalized
        } else {
            benefit_score * 2.0 // No cost = high ratio
        };

        // Determine recommendation based on benefit score
        let recommendation = if benefit_score >= CostBenefitRecommendation::Keep.threshold() {
            CostBenefitRecommendation::Keep
        } else if benefit_score >= CostBenefitRecommendation::Monitor.threshold() {
            CostBenefitRecommendation::Monitor
        } else if benefit_score >= CostBenefitRecommendation::ConsiderRetirement.threshold() {
            CostBenefitRecommendation::ConsiderRetirement
        } else {
            CostBenefitRecommendation::Retire
        };

        Ok(CostBenefitAnalysis {
            test_name: test_name.to_string(),
            cost,
            benefit,
            ratio,
            recommendation,
        })
    }

    /// Find tests that are candidates for retirement
    pub async fn find_retirement_candidates(&self) -> Result<Vec<RetirementCandidate>, SwellError> {
        let metrics_map = self.metrics.read().await;
        let mut candidates = Vec::new();

        for (name, metrics) in metrics_map.iter() {
            // Check if test is protected
            if self.is_protected(name) {
                continue;
            }

            let value = self.compute_value(metrics).await?;

            if value.score < self.policy.retirement_threshold {
                candidates.push(RetirementCandidate {
                    test_name: name.clone(),
                    value_score: value.score,
                    reason: value.retirement_reason.unwrap_or_else(|| "Low value score".to_string()),
                    confidence: value.confidence,
                    potential_savings_ms: metrics.avg_execution_time(),
                    alternative_tests: self.find_alternative_tests(name, &metrics_map).await,
                });
            }
        }

        // Sort by value score (lowest first)
        candidates.sort_by(|a, b| a.value_score.partial_cmp(&b.value_score).unwrap());

        // Limit number of candidates
        candidates.truncate(self.policy.max_retirements_per_run);

        Ok(candidates)
    }

    /// Internal: Check if a test is protected from retirement
    fn is_protected(&self, test_name: &str) -> bool {
        for pattern in &self.policy.protected_patterns {
            if test_name.contains(pattern) {
                return true;
            }
        }
        false
    }

    /// Internal: Compute cost/benefit from metrics (without test name lookup)
    async fn analyze_cost_benefit_internal(
        &self,
        metrics: &TestMetrics,
    ) -> CostBenefitAnalysis {
        let config = &self.config;

        let is_slow = metrics.is_slow(config.slow_test_threshold_ms);
        let maintenance_burden = (1.0 - metrics.pass_rate()) * 0.5 + metrics.flakiness_index() * 0.5;

        let cost = TestCost {
            execution_time_ms: metrics.avg_execution_time(),
            p95_time_ms: metrics.p95_execution_time(),
            is_slow,
            maintenance_burden,
        };

        let coverage_contribution = if let Some(lines) = metrics.coverage_lines {
            (lines as f64 / 10000.0).min(1.0)
        } else {
            0.3
        };

        let bug_detection_rate =
            metrics.bugs_detected as f64 / (metrics.total_runs.max(1) as f64);
        let regression_prevention = metrics.pass_rate() * coverage_contribution;

        let benefit_score = (coverage_contribution + bug_detection_rate + regression_prevention) / 3.0;

        let benefit = TestBenefit {
            coverage_contribution,
            bug_detection_rate,
            regression_prevention,
            score: benefit_score,
        };

        let cost_normalized = (cost.execution_time_ms as f64 / 10000.0).min(1.0);
        let ratio = if cost_normalized > 0.0 {
            benefit_score / cost_normalized
        } else {
            benefit_score * 2.0
        };

        let recommendation = if benefit_score >= CostBenefitRecommendation::Keep.threshold() {
            CostBenefitRecommendation::Keep
        } else if benefit_score >= CostBenefitRecommendation::Monitor.threshold() {
            CostBenefitRecommendation::Monitor
        } else if benefit_score >= CostBenefitRecommendation::ConsiderRetirement.threshold() {
            CostBenefitRecommendation::ConsiderRetirement
        } else {
            CostBenefitRecommendation::Retire
        };

        CostBenefitAnalysis {
            test_name: metrics.test_name.clone(),
            cost,
            benefit,
            ratio,
            recommendation,
        }
    }

    /// Internal: Find alternative tests that provide overlapping coverage
    async fn find_alternative_tests(
        &self,
        test_name: &str,
        metrics_map: &HashMap<String, TestMetrics>,
    ) -> Vec<String> {
        // Simple heuristic: find tests in the same module
        let module = test_name.split("::").take(2).collect::<Vec<_>>().join("::");

        metrics_map
            .keys()
            .filter(|name| {
                name.starts_with(&module) && *name != test_name
            })
            .cloned()
            .take(3)
            .collect()
    }

    /// Get a summary of all test values
    pub async fn get_summary(&self) -> Result<TestValueSummary, SwellError> {
        let metrics_map = self.metrics.read().await;

        let mut retirement_candidates = Vec::new();
        let mut high_value_tests = Vec::new();
        let mut slow_tests = Vec::new();
        let mut flaky_tests = Vec::new();
        let mut total_potential_savings: f64 = 0.0;
        let mut health_sum = 0.0;
        let mut health_count = 0;

        for (name, metrics) in metrics_map.iter() {
            if self.is_protected(name) {
                continue;
            }

            let value = self.compute_value(metrics).await.unwrap_or(TestValue {
                test_name: name.clone(),
                score: 0.5,
                recommended_for_retirement: false,
                retirement_reason: None,
                components: ValueComponents {
                    pass_rate_score: 0.5,
                    speed_score: 0.5,
                    coverage_score: 0.5,
                    bug_detection_score: 0.5,
                    stability_score: 0.5,
                },
                confidence: 0.5,
            });

            health_sum += value.score;
            health_count += 1;

            if value.recommended_for_retirement {
                retirement_candidates.push(RetirementCandidate {
                    test_name: name.clone(),
                    value_score: value.score,
                    reason: value.retirement_reason.unwrap_or_default(),
                    confidence: value.confidence,
                    potential_savings_ms: metrics.avg_execution_time(),
                    alternative_tests: self
                        .find_alternative_tests(name, &metrics_map)
                        .await,
                });
            }

            if value.score >= 0.7 {
                high_value_tests.push(name.clone());
            }

            if metrics.is_slow(self.config.slow_test_threshold_ms) {
                slow_tests.push(name.clone());
            }

            if metrics.flakiness_index() > self.config.flakiness_threshold {
                flaky_tests.push(name.clone());
            }

            total_potential_savings += metrics.avg_execution_time();
        }

        retirement_candidates.sort_by(|a, b| a.value_score.partial_cmp(&b.value_score).unwrap());
        retirement_candidates.truncate(self.policy.max_retirements_per_run);

        let health_score = if health_count > 0 {
            health_sum / health_count as f64
        } else {
            1.0 // No tests = perfect health
        };

        Ok(TestValueSummary {
            total_tests: metrics_map.len(),
            retirement_candidates,
            high_value_tests,
            slow_tests,
            flaky_tests,
            health_score,
            potential_time_savings_ms: total_potential_savings,
        })
    }

    /// Merge metrics from another tracker (for distributed tracking)
    pub async fn merge(&self, other: &TestValueTracker) -> Result<(), SwellError> {
        let mut self_metrics = self.metrics.write().await;
        let other_metrics = other.metrics.read().await;

        for (name, other_data) in other_metrics.iter() {
            if let Some(self_data) = self_metrics.get_mut(name) {
                // Merge execution times (keep most recent N)
                let mut combined = std::mem::take(&mut self_data.execution_times_ms);
                combined.extend(other_data.execution_times_ms.iter());
                combined.sort();
                combined.dedup();
                // Keep only most recent 1000
                combined.drain(0..combined.len().saturating_sub(1000));

                self_data.pass_count += other_data.pass_count;
                self_data.fail_count += other_data.fail_count;
                self_data.flaky_count += other_data.flaky_count;
                self_data.bugs_detected += other_data.bugs_detected;
                self_data.total_runs = self_data.pass_count + self_data.fail_count;
                self_data.execution_times_ms = combined;
            } else {
                // Copy other data
                self_metrics.insert(name.clone(), other_data.clone());
            }
        }

        Ok(())
    }
}

// ============================================================================
// Validation Gate Integration
// ============================================================================

/// A validation gate that checks for low-value tests
pub struct TestValueGate {
    tracker: TestValueTracker,
}

impl TestValueGate {
    /// Create a new TestValueGate
    pub fn new(tracker: TestValueTracker) -> Self {
        Self { tracker }
    }

    /// Get the tracker for external use
    pub fn tracker(&self) -> &TestValueTracker {
        &self.tracker
    }
}

impl Default for TestValueGate {
    fn default() -> Self {
        Self::new(TestValueTracker::new(TestValueConfig::default()))
    }
}

#[async_trait::async_trait]
impl swell_core::ValidationGate for TestValueGate {
    fn name(&self) -> &'static str {
        "test_value"
    }

    fn order(&self) -> u32 {
        25
    }

    async fn validate(
        &self,
        _context: swell_core::ValidationContext,
    ) -> Result<swell_core::ValidationOutcome, SwellError> {
        let summary = self.tracker.get_summary().await?;

        let mut messages = Vec::new();
        let mut passed = true;

        // Report on test suite health
        messages.push(swell_core::ValidationMessage {
            level: swell_core::ValidationLevel::Info,
            code: Some("TEST_HEALTH".to_string()),
            message: format!(
                "Test suite health score: {:.0}% ({} tests tracked)",
                summary.health_score * 100.0,
                summary.total_tests
            ),
            file: None,
            line: None,
        });

        // Report on slow tests
        if !summary.slow_tests.is_empty() {
            passed = false;
            messages.push(swell_core::ValidationMessage {
                level: swell_core::ValidationLevel::Warning,
                code: Some("SLOW_TESTS".to_string()),
                message: format!(
                    "Slow tests detected ({}): {}",
                    summary.slow_tests.len(),
                    summary.slow_tests.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                ),
                file: None,
                line: None,
            });
        }

        // Report on flaky tests
        if !summary.flaky_tests.is_empty() {
            passed = false;
            messages.push(swell_core::ValidationMessage {
                level: swell_core::ValidationLevel::Warning,
                code: Some("FLAKY_TESTS".to_string()),
                message: format!(
                    "Flaky tests detected ({}): {}",
                    summary.flaky_tests.len(),
                    summary.flaky_tests.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                ),
                file: None,
                line: None,
            });
        }

        // Report on retirement candidates
        if !summary.retirement_candidates.is_empty() {
            messages.push(swell_core::ValidationMessage {
                level: swell_core::ValidationLevel::Warning,
                code: Some("RETIREMENT_CANDIDATES".to_string()),
                message: format!(
                    "Test retirement candidates ({}): {}",
                    summary.retirement_candidates.len(),
                    summary
                        .retirement_candidates
                        .iter()
                        .take(5)
                        .map(|c| format!("{} ({:.2})", c.test_name, c.value_score))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                file: None,
                line: None,
            });
        }

        // Report on potential time savings
        if summary.potential_time_savings_ms > 0.0 {
            messages.push(swell_core::ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("TIME_SAVINGS".to_string()),
                message: format!(
                    "Potential time savings from retirement: {:.0}ms",
                    summary.potential_time_savings_ms
                ),
                file: None,
                line: None,
            });
        }

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod self_improving_tests {
    use super::*;

    #[tokio::test]
    async fn test_record_test_run() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        tracker
            .record_test_run("test_module::test_one", true, 100)
            .await
            .unwrap();

        let metrics = tracker.get_metrics("test_module::test_one").await;
        assert!(metrics.is_some());
        let metrics = metrics.unwrap();
        assert_eq!(metrics.total_runs, 1);
        assert_eq!(metrics.pass_count, 1);
        assert_eq!(metrics.fail_count, 0);
    }

    #[tokio::test]
    async fn test_record_test_run_fail() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        tracker
            .record_test_run("test_module::test_fail", false, 50)
            .await
            .unwrap();

        let metrics = tracker.get_metrics("test_module::test_fail").await;
        assert!(metrics.is_some());
        let metrics = metrics.unwrap();
        assert_eq!(metrics.total_runs, 1);
        assert_eq!(metrics.pass_count, 0);
        assert_eq!(metrics.fail_count, 1);
    }

    #[tokio::test]
    async fn test_pass_rate_calculation() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // 7 passes, 3 fails = 70% pass rate
        for _ in 0..7 {
            tracker
                .record_test_run("test_module::test_rate", true, 100)
                .await
                .unwrap();
        }
        for _ in 0..3 {
            tracker
                .record_test_run("test_module::test_rate", false, 100)
                .await
                .unwrap();
        }

        let metrics = tracker.get_metrics("test_module::test_rate").await.unwrap();
        let pass_rate = metrics.pass_rate();
        assert!(pass_rate > 0.69 && pass_rate < 0.71, "Pass rate should be ~0.7, got {}", pass_rate);
    }

    #[tokio::test]
    async fn test_execution_time_stats() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Record different execution times
        let times = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        for time in times {
            tracker
                .record_test_run("test_module::test_timing", true, time)
                .await
                .unwrap();
        }

        let metrics = tracker.get_metrics("test_module::test_timing").await.unwrap();
        let avg_time = metrics.avg_execution_time();
        assert!(avg_time > 54.9 && avg_time < 55.1, "Average time should be ~55, got {}", avg_time);
    }

    #[tokio::test]
    async fn test_slow_test_detection() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Record slow tests
        for _ in 0..10 {
            tracker
                .record_test_run("test_module::test_slow", true, 2000)
                .await
                .unwrap();
        }

        let metrics = tracker.get_metrics("test_module::test_slow").await.unwrap();
        assert!(metrics.is_slow(1000)); // 1000ms threshold
    }

    #[tokio::test]
    async fn test_calculate_test_value() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Create a "good" test with many passes and reasonable speed
        for _ in 0..20 {
            tracker
                .record_test_run("test_module::test_good", true, 50)
                .await
                .unwrap();
        }
        tracker.record_bug_detection("test_module::test_good").await.unwrap();

        let value = tracker.calculate_test_value("test_module::test_good").await.unwrap();

        // Should have high value
        assert!(value.score > 0.5, "Good test should have score > 0.5, got {}", value.score);
        assert!(!value.recommended_for_retirement);
    }

    #[tokio::test]
    async fn test_low_value_retirement() {
        let mut config = TestValueConfig::default();
        config.min_value_score = 0.5; // Set higher threshold so failing tests get retired

        let tracker = TestValueTracker::new(config);

        // Create a "bad" test with many failures and slow execution
        for _ in 0..20 {
            tracker
                .record_test_run("test_module::test_bad", false, 2000) // slow and failing
                .await
                .unwrap();
        }

        let value = tracker
            .calculate_test_value("test_module::test_bad")
            .await
            .unwrap();

        // Should have low value and recommended for retirement
        assert!(value.score < 0.5);
        assert!(value.recommended_for_retirement);
    }

    #[tokio::test]
    async fn test_protected_test_not_retired() {
        let mut config = TestValueConfig::default();
        config.min_value_score = 0.5;

        let policy = RetirementPolicy {
            protected_patterns: vec!["integration_".to_string()],
            ..Default::default()
        };

        let tracker = TestValueTracker::new(config).with_policy(policy);

        // Create a failing "integration" test (protected)
        for _ in 0..20 {
            tracker
                .record_test_run("integration_api::test_failing", false, 100)
                .await
                .unwrap();
        }

        let candidates = tracker.find_retirement_candidates().await.unwrap();

        // Protected test should not be in candidates
        assert!(
            !candidates.iter().any(|c| c.test_name.contains("integration_"))
        );
    }

    #[tokio::test]
    async fn test_cost_benefit_analysis() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Record a fast, passing test
        for _ in 0..10 {
            tracker
                .record_test_run("test_module::test_efficient", true, 10)
                .await
                .unwrap();
        }
        tracker.set_coverage("test_module::test_efficient", 5000).await.unwrap();

        let analysis = tracker
            .analyze_cost_benefit("test_module::test_efficient")
            .await
            .unwrap();

        // Should have low cost, moderate benefit
        assert!(analysis.cost.execution_time_ms < 20.0);
        assert!(analysis.benefit.score > 0.0);
        assert!(analysis.ratio > 0.0);
    }

    #[tokio::test]
    async fn test_summary_generation() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Add multiple tests
        tracker
            .record_test_run("test_module::test_one", true, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_module::test_two", true, 200)
            .await
            .unwrap();
        tracker
            .record_test_run("test_module::test_three", false, 50)
            .await
            .unwrap();

        let summary = tracker.get_summary().await.unwrap();

        assert_eq!(summary.total_tests, 3);
        assert!(summary.retirement_candidates.len() <= 5); // Limited by policy
    }

    #[tokio::test]
    async fn test_bug_detection_recording() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        tracker
            .record_test_run("test_module::test_bugs", true, 100)
            .await
            .unwrap();
        tracker.record_bug_detection("test_module::test_bugs").await.unwrap();
        tracker.record_bug_detection("test_module::test_bugs").await.unwrap();

        let metrics = tracker.get_metrics("test_module::test_bugs").await.unwrap();
        assert_eq!(metrics.bugs_detected, 2);
    }

    #[tokio::test]
    async fn test_flakiness_tracking() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Simulate flaky runs
        tracker
            .record_test_run("test_module::test_flaky", true, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_module::test_flaky", true, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_module::test_flaky", false, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_module::test_flaky", true, 100)
            .await
            .unwrap();

        let metrics = tracker.get_metrics("test_module::test_flaky").await.unwrap();
        // Flakiness is manually tracked, not automatically detected from passes/fails
        assert_eq!(metrics.total_runs, 4);
    }

    #[tokio::test]
    async fn test_confidence_based_on_runs() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        // Record 5 runs (less than min_runs_for_value = 10)
        for _ in 0..5 {
            tracker
                .record_test_run("test_module::test_confidence", true, 100)
                .await
                .unwrap();
        }

        let value = tracker
            .calculate_test_value("test_module::test_confidence")
            .await
            .unwrap();

        // Confidence should be 50% (5/10)
        let confidence = value.confidence;
        assert!(confidence > 0.49 && confidence < 0.51, "Confidence should be ~0.5, got {}", confidence);
    }

    #[tokio::test]
    async fn test_all_tests_retrieval() {
        let tracker = TestValueTracker::new(TestValueConfig::default());

        tracker
            .record_test_run("test_a", true, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_b", true, 100)
            .await
            .unwrap();
        tracker
            .record_test_run("test_c", true, 100)
            .await
            .unwrap();

        let all_tests = tracker.get_all_tests().await;
        assert_eq!(all_tests.len(), 3);
        assert!(all_tests.contains(&"test_a".to_string()));
        assert!(all_tests.contains(&"test_b".to_string()));
        assert!(all_tests.contains(&"test_c".to_string()));
    }

    #[tokio::test]
    async fn test_test_value_gate() {
        use swell_core::ValidationGate;
        use uuid::Uuid;

        let tracker = TestValueTracker::new(TestValueConfig::default());
        let gate = TestValueGate::new(tracker);

        // Add some tests
        gate.tracker()
            .record_test_run("test_one", true, 100)
            .await
            .unwrap();
        gate.tracker()
            .record_test_run("test_two", true, 2000) // slow
            .await
            .unwrap();

        let context = swell_core::ValidationContext {
            task_id: Uuid::new_v4(),
            workspace_path: "/tmp".to_string(),
            changed_files: vec![],
            plan: None,
        };

        let result = gate.validate(context).await.unwrap();

        // Should have passed but with warnings
        assert!(!result.passed); // Because of slow test
        assert!(result.messages.iter().any(|m| m.code == Some("SLOW_TESTS".to_string())));
    }
}
