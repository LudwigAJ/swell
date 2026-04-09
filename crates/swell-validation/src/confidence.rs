//! Confidence scoring module for validation outcomes.
//!
//! Computes confidence scores from multiple signals including:
//! - Test coverage
//! - Lint pass/fail
//! - Security scan results
//! - Historical patterns

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A signal contributing to the confidence score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceSignal {
    /// Name of the signal source
    pub source: String,
    /// Weight of this signal (0.0 to 1.0)
    pub weight: f64,
    /// Raw value from the signal
    pub value: f64,
    /// Whether this signal indicates pass or fail
    pub passed: bool,
}

/// Confidence score result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceScore {
    /// Overall score between 0.0 and 1.0
    pub score: f64,
    /// Confidence level categorization
    pub level: ConfidenceLevel,
    /// Individual signal scores
    pub signals: Vec<ConfidenceSignal>,
    /// Threshold values used
    pub thresholds: ConfidenceThresholds,
}

/// Confidence level categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    /// Score < 0.3: Low confidence, substantial issues detected
    Low,
    /// Score 0.3 - 0.6: Medium confidence, some concerns
    Medium,
    /// Score 0.6 - 0.8: High confidence, minor concerns
    High,
    /// Score > 0.8: Very high confidence, ready for merge
    VeryHigh,
}

/// Thresholds for confidence level determination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceThresholds {
    /// Below this score is considered low confidence
    pub low_threshold: f64,
    /// Below this score is considered medium confidence
    pub medium_threshold: f64,
    /// Below this score is considered high confidence
    pub high_threshold: f64,
    /// Auto-merge threshold
    pub auto_merge_threshold: f64,
}

impl Default for ConfidenceThresholds {
    fn default() -> Self {
        Self {
            low_threshold: 0.3,
            medium_threshold: 0.6,
            high_threshold: 0.8,
            auto_merge_threshold: 0.85,
        }
    }
}

impl ConfidenceLevel {
    /// Determine level from score and thresholds
    pub fn from_score(score: f64, thresholds: &ConfidenceThresholds) -> Self {
        if score < thresholds.low_threshold {
            ConfidenceLevel::Low
        } else if score < thresholds.medium_threshold {
            ConfidenceLevel::Medium
        } else if score < thresholds.high_threshold {
            ConfidenceLevel::High
        } else {
            ConfidenceLevel::VeryHigh
        }
    }
}

impl ConfidenceScore {
    /// Create a new confidence score from signals
    pub fn from_signals(signals: Vec<ConfidenceSignal>, thresholds: ConfidenceThresholds) -> Self {
        let score = Self::compute_score(&signals);
        let level = ConfidenceLevel::from_score(score, &thresholds);

        Self {
            score,
            level,
            signals,
            thresholds,
        }
    }

    /// Compute weighted score from signals
    fn compute_score(signals: &[ConfidenceSignal]) -> f64 {
        if signals.is_empty() {
            return 1.0; // Default to full confidence if no signals
        }

        let total_weight: f64 = signals.iter().map(|s| s.weight).sum();
        if total_weight == 0.0 {
            return 1.0;
        }

        let weighted_score: f64 = signals
            .iter()
            .map(|s| {
                let normalized_value = if s.passed { s.value } else { 1.0 - s.value };
                s.weight * normalized_value
            })
            .sum();

        (weighted_score / total_weight).clamp(0.0, 1.0)
    }

    /// Check if this score qualifies for auto-merge
    pub fn can_auto_merge(&self) -> bool {
        self.score >= self.thresholds.auto_merge_threshold && self.level == ConfidenceLevel::VeryHigh
    }

    /// Get a human-readable summary
    pub fn summary(&self) -> String {
        let level_str = match self.level {
            ConfidenceLevel::Low => "LOW",
            ConfidenceLevel::Medium => "MEDIUM",
            ConfidenceLevel::High => "HIGH",
            ConfidenceLevel::VeryHigh => "VERY HIGH",
        };

        format!(
            "Confidence: {} ({:.1}%) - {}",
            level_str,
            self.score * 100.0,
            if self.can_auto_merge() { "Eligible for auto-merge" } else { "Requires human review" }
        )
    }
}

/// Builder for constructing confidence scores
#[derive(Debug, Default)]
pub struct ConfidenceScorer {
    signals: Vec<ConfidenceSignal>,
    thresholds: ConfidenceThresholds,
}

impl ConfidenceScorer {
    /// Create a new scorer
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a lint signal
    pub fn with_lint(mut self, passed: bool, warning_ratio: f64) -> Self {
        self.signals.push(ConfidenceSignal {
            source: "lint".to_string(),
            weight: 0.25,
            value: 1.0 - warning_ratio,
            passed,
        });
        self
    }

    /// Add a test signal
    pub fn with_tests(mut self, passed: bool, coverage: f64) -> Self {
        self.signals.push(ConfidenceSignal {
            source: "test".to_string(),
            weight: 0.35,
            value: coverage,
            passed,
        });
        self
    }

    /// Add a security signal
    pub fn with_security(mut self, passed: bool, critical_findings: i32) -> Self {
        let value = if critical_findings > 0 {
            0.0
        } else {
            1.0
        };
        self.signals.push(ConfidenceSignal {
            source: "security".to_string(),
            weight: 0.25,
            value,
            passed,
        });
        self
    }

    /// Add an AI review signal
    pub fn with_ai_review(mut self, passed: bool, confidence: f64) -> Self {
        self.signals.push(ConfidenceSignal {
            source: "ai_review".to_string(),
            weight: 0.15,
            value: confidence,
            passed,
        });
        self
    }

    /// Add custom signal
    pub fn with_signal(mut self, signal: ConfidenceSignal) -> Self {
        self.signals.push(signal);
        self
    }

    /// Set custom thresholds
    pub fn with_thresholds(mut self, thresholds: ConfidenceThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Compute the final confidence score
    pub fn score(self) -> ConfidenceScore {
        ConfidenceScore::from_signals(self.signals, self.thresholds)
    }
}

/// History tracker for flakiness detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessHistory {
    /// Test name to run history
    runs: HashMap<String, Vec<TestRun>>,
}

/// A single test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    /// Timestamp of the run
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Whether the test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Whether this was a retry
    pub is_retry: bool,
}

impl FlakinessHistory {
    /// Create a new empty history
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
        }
    }

    /// Record a test run
    pub fn record(&mut self, test_name: String, run: TestRun) {
        self.runs.entry(test_name).or_default().push(run);
    }

    /// Check if a test shows flakiness patterns
    pub fn is_flaky(&self, test_name: &str, min_runs: usize) -> bool {
        let runs = self.runs.get(test_name).map(|r| r.as_slice()).unwrap_or(&[]);
        if runs.len() < min_runs {
            return false;
        }

        // A test is considered flaky if:
        // 1. It has both passes and failures
        // 2. The failure rate is between 10% and 90% (not always failing or always passing)
        let total = runs.len();
        let failures: usize = runs.iter().filter(|r| !r.passed).count();
        let passes = total - failures;

        // Must have mixed results to be flaky
        if failures == 0 || passes == 0 {
            return false;
        }

        // Failure rate should be between 10% and 90%
        let failure_rate = failures as f64 / total as f64;
        failure_rate > 0.1 && failure_rate < 0.9
    }

    /// Get flakiness score for a test (0.0 = always passes, 1.0 = always fails or chaotic)
    pub fn flakiness_score(&self, test_name: &str) -> f64 {
        let runs = self.runs.get(test_name).map(|r| r.as_slice()).unwrap_or(&[]);
        if runs.len() < 2 {
            return 0.0;
        }

        let total = runs.len();
        let failures: usize = runs.iter().filter(|r| !r.passed).count();
        let passes = total - failures;

        // Score based on inconsistency
        // More balanced pass/fail ratio = higher flakiness
        let balance = (failures as f64 / total as f64).abs() - 0.5;
        let instability = 1.0 - (balance * 2.0).abs();

        instability
    }

    /// Get tests that should be quarantined
    pub fn quarantine_list(&self, max_flakiness: f64) -> Vec<String> {
        self.runs
            .keys()
            .filter(|name| self.flakiness_score(name) > max_flakiness)
            .cloned()
            .collect()
    }
}

impl Default for FlakinessHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_confidence_score_empty() {
        let score = ConfidenceScore::from_signals(vec![], ConfidenceThresholds::default());
        assert_eq!(score.score, 1.0);
        assert_eq!(score.level, ConfidenceLevel::VeryHigh);
    }

    #[test]
    fn test_confidence_score_all_pass() {
        let signals = vec![
            ConfidenceSignal {
                source: "lint".to_string(),
                weight: 1.0,
                value: 1.0,
                passed: true,
            },
        ];
        let score = ConfidenceScore::from_signals(signals, ConfidenceThresholds::default());
        assert_eq!(score.score, 1.0);
    }

    #[test]
    fn test_confidence_score_weighted() {
        let signals = vec![
            ConfidenceSignal {
                source: "lint".to_string(),
                weight: 0.5,
                value: 1.0,
                passed: true,
            },
            ConfidenceSignal {
                source: "test".to_string(),
                weight: 0.5,
                value: 0.5,
                passed: true,
            },
        ];
        let score = ConfidenceScore::from_signals(signals, ConfidenceThresholds::default());
        // (0.5 * 1.0 + 0.5 * 0.5) / 1.0 = 0.75
        assert_eq!(score.score, 0.75);
        assert_eq!(score.level, ConfidenceLevel::High);
    }

    #[test]
    fn test_confidence_levels() {
        let thresholds = ConfidenceThresholds::default();

        assert_eq!(
            ConfidenceLevel::from_score(0.2, &thresholds),
            ConfidenceLevel::Low
        );
        assert_eq!(
            ConfidenceLevel::from_score(0.5, &thresholds),
            ConfidenceLevel::Medium
        );
        assert_eq!(
            ConfidenceLevel::from_score(0.7, &thresholds),
            ConfidenceLevel::High
        );
        assert_eq!(
            ConfidenceLevel::from_score(0.9, &thresholds),
            ConfidenceLevel::VeryHigh
        );
    }

    #[test]
    fn test_scorer_builder() {
        let score = ConfidenceScorer::new()
            .with_lint(true, 0.1)
            .with_tests(true, 0.8)
            .with_security(true, 0)
            .with_ai_review(true, 0.9)
            .score();

        assert!(score.score > 0.7);
    }

    #[test]
    fn test_flakiness_detection() {
        let mut history = FlakinessHistory::new();
        let test_name = "test_example".to_string();

        // Record 5 runs with mixed results
        for (i, passed) in [true, false, true, false, true].iter().enumerate() {
            history.record(
                test_name.clone(),
                TestRun {
                    timestamp: Utc::now(),
                    passed: *passed,
                    duration_ms: 100,
                    is_retry: i > 2,
                },
            );
        }

        assert!(history.is_flaky(&test_name, 3));
        assert!(history.flakiness_score(&test_name) > 0.0);
    }

    #[test]
    fn test_flakiness_always_passing() {
        let mut history = FlakinessHistory::new();
        let test_name = "test_always_passes".to_string();

        for _ in 0..5 {
            history.record(
                test_name.clone(),
                TestRun {
                    timestamp: Utc::now(),
                    passed: true,
                    duration_ms: 100,
                    is_retry: false,
                },
            );
        }

        assert!(!history.is_flaky(&test_name, 3));
        assert_eq!(history.flakiness_score(&test_name), 0.0);
    }

    #[test]
    fn test_quarantine_list() {
        let mut history = FlakinessHistory::new();

        // Add a flaky test
        for (i, passed) in [true, false, true, false, true].iter().enumerate() {
            history.record(
                "test_flaky".to_string(),
                TestRun {
                    timestamp: Utc::now(),
                    passed: *passed,
                    duration_ms: 100,
                    is_retry: i > 2,
                },
            );
        }

        // Add a stable test
        for _ in 0..5 {
            history.record(
                "test_stable".to_string(),
                TestRun {
                    timestamp: Utc::now(),
                    passed: true,
                    duration_ms: 100,
                    is_retry: false,
                },
            );
        }

        let quarantined = history.quarantine_list(0.3);
        assert!(quarantined.contains(&"test_flaky".to_string()));
        assert!(!quarantined.contains(&"test_stable".to_string()));
    }

    #[test]
    fn test_can_auto_merge() {
        let score = ConfidenceScorer::new()
            .with_lint(true, 0.0)
            .with_tests(true, 0.9)
            .with_security(true, 0)
            .with_ai_review(true, 0.9)
            .score();

        assert!(score.can_auto_merge());
    }
}
