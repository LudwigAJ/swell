//! Multi-signal validation module for aggregating validation results.
//!
//! This module provides comprehensive validation signal aggregation from multiple sources:
//! - Tests (unit, integration, property-based)
//! - Lint (clippy, rustfmt)
//! - Type checking (cargo check)
//! - Static analysis (security scanners)
//! - LLM review (AI-powered code review)
//! - Spec conformance (acceptance criteria validation)
//!
//! # Architecture
//!
//! The multi-signal validation system combines individual validation signals
//! into a unified confidence score using weighted aggregation.
//!
//! ## Signal Types
//!
//! Each signal type has specific weights based on reliability and coverage:
//!
//! | Signal | Default Weight | Description |
//! |--------|----------------|-------------|
//! | test | 0.30 | Unit and integration test results |
//! | lint | 0.20 | Clippy and format checks |
//! | types | 0.15 | Type checking via cargo check |
//! | static_analysis | 0.15 | Security and static analysis |
//! | llm_review | 0.10 | AI-powered review |
//! | spec_conformance | 0.10 | Acceptance criteria match |
//!
//! # Usage
//!
//! ```rust
//! use swell_validation::multi_signal::{MultiSignalValidator, SignalConfig, ValidationSignal};
//!
//! let mut validator = MultiSignalValidator::with_defaults();
//! validator.add_signal(ValidationSignal::test(true, 0.85));
//! validator.add_signal(ValidationSignal::lint(true, 0.1));
//!
//! let outcome = validator.compute_outcome();
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use swell_core::ValidationLevel;

/// A validation signal from a specific source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSignal {
    /// Source identifier (e.g., "test", "lint", "security")
    pub source: String,
    /// Whether this signal passed validation
    pub passed: bool,
    /// Raw score value (0.0 to 1.0, where 1.0 is perfect)
    pub value: f64,
    /// Optional weight override (uses default if None)
    pub weight_override: Option<f64>,
    /// Optional metadata about the signal
    pub metadata: HashMap<String, String>,
    /// Severity level if this signal indicates a failure
    pub severity: Option<SignalSeverity>,
}

/// Severity levels for validation signals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSeverity {
    /// Critical issue - blocks merge
    Critical,
    /// Warning - does not block but should be addressed
    Warning,
    /// Info - informational only
    Info,
}

impl SignalSeverity {
    /// Convert to validation level
    pub fn to_validation_level(self) -> ValidationLevel {
        match self {
            SignalSeverity::Critical => ValidationLevel::Error,
            SignalSeverity::Warning => ValidationLevel::Warning,
            SignalSeverity::Info => ValidationLevel::Info,
        }
    }
}

impl ValidationSignal {
    /// Create a new test signal
    pub fn test(passed: bool, coverage: f64) -> Self {
        Self {
            source: "test".to_string(),
            passed,
            value: coverage,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Critical)
            },
        }
    }

    /// Create a new lint signal
    pub fn lint(passed: bool, warning_ratio: f64) -> Self {
        Self {
            source: "lint".to_string(),
            passed,
            value: 1.0 - warning_ratio,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Warning)
            },
        }
    }

    /// Create a new type checking signal
    pub fn types(passed: bool, type_safety_score: f64) -> Self {
        Self {
            source: "types".to_string(),
            passed,
            value: type_safety_score,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Critical)
            },
        }
    }

    /// Create a new static analysis signal
    pub fn static_analysis(passed: bool, finding_score: f64) -> Self {
        Self {
            source: "static_analysis".to_string(),
            passed,
            value: finding_score,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Critical)
            },
        }
    }

    /// Create a new LLM review signal
    pub fn llm_review(passed: bool, confidence: f64) -> Self {
        Self {
            source: "llm_review".to_string(),
            passed,
            value: confidence,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Warning)
            },
        }
    }

    /// Create a new spec conformance signal
    pub fn spec_conformance(passed: bool, match_score: f64) -> Self {
        Self {
            source: "spec_conformance".to_string(),
            passed,
            value: match_score,
            weight_override: None,
            metadata: HashMap::new(),
            severity: if passed {
                None
            } else {
                Some(SignalSeverity::Critical)
            },
        }
    }

    /// Create a custom signal with full control
    pub fn custom(
        source: impl Into<String>,
        passed: bool,
        value: f64,
        weight: Option<f64>,
        severity: Option<SignalSeverity>,
    ) -> Self {
        Self {
            source: source.into(),
            passed,
            value: value.clamp(0.0, 1.0),
            weight_override: weight,
            metadata: HashMap::new(),
            severity,
        }
    }

    /// Add metadata to the signal
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Set severity level
    pub fn with_severity(mut self, severity: SignalSeverity) -> Self {
        self.severity = Some(severity);
        self
    }

    /// Set weight override
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight_override = Some(weight);
        self
    }
}

/// Default weights for each signal type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalWeights {
    /// Test signal weight (default: 0.30)
    pub test: f64,
    /// Lint signal weight (default: 0.20)
    pub lint: f64,
    /// Type checking weight (default: 0.15)
    pub types: f64,
    /// Static analysis weight (default: 0.15)
    pub static_analysis: f64,
    /// LLM review weight (default: 0.10)
    pub llm_review: f64,
    /// Spec conformance weight (default: 0.10)
    pub spec_conformance: f64,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self {
            test: 0.30,
            lint: 0.20,
            types: 0.15,
            static_analysis: 0.15,
            llm_review: 0.10,
            spec_conformance: 0.10,
        }
    }
}

impl SignalWeights {
    /// Get weight for a specific signal source
    pub fn get_weight(&self, source: &str) -> f64 {
        match source {
            "test" => self.test,
            "lint" => self.lint,
            "types" => self.types,
            "static_analysis" => self.static_analysis,
            "llm_review" => self.llm_review,
            "spec_conformance" => self.spec_conformance,
            _ => 0.1, // Default weight for unknown sources
        }
    }

    /// Normalize weights to sum to 1.0
    pub fn normalize(&mut self) {
        let total = self.test
            + self.lint
            + self.types
            + self.static_analysis
            + self.llm_review
            + self.spec_conformance;
        if total > 0.0 {
            let factor = 1.0 / total;
            self.test *= factor;
            self.lint *= factor;
            self.types *= factor;
            self.static_analysis *= factor;
            self.llm_review *= factor;
            self.spec_conformance *= factor;
        }
    }

    /// Create with custom weights
    pub fn new(
        test: f64,
        lint: f64,
        types: f64,
        static_analysis: f64,
        llm_review: f64,
        spec_conformance: f64,
    ) -> Self {
        Self {
            test,
            lint,
            types,
            static_analysis,
            llm_review,
            spec_conformance,
        }
    }
}

/// Configuration for multi-signal validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Weights for each signal type
    pub weights: SignalWeights,
    /// Threshold for overall pass (0.0 to 1.0)
    pub pass_threshold: f64,
    /// Whether to require all signals to pass
    pub require_all_pass: bool,
    /// Auto-normalize weights
    pub auto_normalize: bool,
    /// Minimum signals required for computation
    pub min_signals: usize,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            weights: SignalWeights::default(),
            pass_threshold: 0.7,
            require_all_pass: false,
            auto_normalize: true,
            min_signals: 2,
        }
    }
}

/// Result of multi-signal validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiSignalOutcome {
    /// Overall passed status
    pub passed: bool,
    /// Weighted confidence score (0.0 to 1.0)
    pub score: f64,
    /// Individual signal results
    pub signals: Vec<ValidationSignal>,
    /// Weighted contributions from each signal
    pub contributions: HashMap<String, SignalContribution>,
    /// Overall outcome message
    pub message: String,
    /// Level of detail for reporting
    pub level: OutcomeLevel,
    /// Any blocking issues found
    pub blocking_issues: Vec<String>,
}

/// Level of detail in outcome reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutcomeLevel {
    /// Minimal output (pass/fail only)
    Minimal,
    /// Standard output (score and summary)
    #[default]
    Standard,
    /// Verbose output (all signals and details)
    Verbose,
}

/// Contribution of a signal to the overall score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalContribution {
    /// Raw value
    pub raw_value: f64,
    /// Applied weight
    pub weight: f64,
    /// Weighted contribution to final score
    pub weighted_contribution: f64,
    /// Whether this signal passed
    pub passed: bool,
}

/// Multi-signal validator that aggregates validation signals
#[derive(Debug, Clone)]
pub struct MultiSignalValidator {
    /// Configuration
    config: SignalConfig,
    /// Collected signals
    signals: Vec<ValidationSignal>,
    /// Signal history for trend analysis
    history: Vec<Vec<ValidationSignal>>,
}

impl MultiSignalValidator {
    /// Create a new validator with default configuration
    pub fn new() -> Self {
        Self::with_config(SignalConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: SignalConfig) -> Self {
        Self {
            config,
            signals: Vec::new(),
            history: Vec::new(),
        }
    }

    /// Create with default weights but custom threshold
    pub fn with_threshold(threshold: f64) -> Self {
        Self::with_config(SignalConfig {
            pass_threshold: threshold,
            ..Default::default()
        })
    }

    /// Create with weights that sum to 1.0
    pub fn with_defaults() -> Self {
        Self::new()
    }

    /// Add a signal to the validator
    pub fn add_signal(&mut self, signal: ValidationSignal) -> &mut Self {
        self.signals.push(signal);
        self
    }

    /// Add multiple signals at once
    pub fn add_signals(
        &mut self,
        signals: impl IntoIterator<Item = ValidationSignal>,
    ) -> &mut Self {
        for signal in signals {
            self.signals.push(signal);
        }
        self
    }

    /// Add a test signal
    pub fn add_test(&mut self, passed: bool, coverage: f64) -> &mut Self {
        self.add_signal(ValidationSignal::test(passed, coverage))
    }

    /// Add a lint signal
    pub fn add_lint(&mut self, passed: bool, warning_ratio: f64) -> &mut Self {
        self.add_signal(ValidationSignal::lint(passed, warning_ratio))
    }

    /// Add a type checking signal
    pub fn add_types(&mut self, passed: bool, type_safety_score: f64) -> &mut Self {
        self.add_signal(ValidationSignal::types(passed, type_safety_score))
    }

    /// Add a static analysis signal
    pub fn add_static_analysis(&mut self, passed: bool, finding_score: f64) -> &mut Self {
        self.add_signal(ValidationSignal::static_analysis(passed, finding_score))
    }

    /// Add an LLM review signal
    pub fn add_llm_review(&mut self, passed: bool, confidence: f64) -> &mut Self {
        self.add_signal(ValidationSignal::llm_review(passed, confidence))
    }

    /// Add a spec conformance signal
    pub fn add_spec_conformance(&mut self, passed: bool, match_score: f64) -> &mut Self {
        self.add_signal(ValidationSignal::spec_conformance(passed, match_score))
    }

    /// Clear all signals
    pub fn clear(&mut self) -> &mut Self {
        self.signals.clear();
        self
    }

    /// Get current signals
    pub fn get_signals(&self) -> &[ValidationSignal] {
        &self.signals
    }

    /// Check if enough signals are present
    fn has_min_signals(&self) -> bool {
        self.signals.len() >= self.config.min_signals
    }

    /// Compute weighted score from signals
    fn compute_weighted_score(&self) -> (f64, HashMap<String, SignalContribution>) {
        let mut contributions: HashMap<String, SignalContribution> = HashMap::new();
        let mut total_weight = 0.0;

        // Group signals by source and compute contributions
        for signal in &self.signals {
            let base_weight = self.config.weights.get_weight(&signal.source);
            let weight = signal.weight_override.unwrap_or(base_weight);

            // For failed signals, contribution is 0
            // For passed signals, contribution is weight * value
            let contribution = if signal.passed {
                weight * signal.value
            } else {
                0.0 // Failed signals contribute 0 regardless of severity
            };

            // Update or accumulate
            if let Some(existing) = contributions.get_mut(&signal.source) {
                // Average if multiple signals from same source
                let n = self
                    .signals
                    .iter()
                    .filter(|s| s.source == signal.source)
                    .count() as f64;
                existing.raw_value = (existing.raw_value + signal.value) / n;
                existing.weighted_contribution =
                    (existing.weighted_contribution + contribution) / n;
                existing.passed = existing.passed && signal.passed;
            } else {
                total_weight += weight;
                contributions.insert(
                    signal.source.clone(),
                    SignalContribution {
                        raw_value: signal.value,
                        weight,
                        weighted_contribution: contribution,
                        passed: signal.passed,
                    },
                );
            }
        }

        // Normalize if auto_normalize is enabled
        let final_score = if self.config.auto_normalize && total_weight > 0.0 {
            let factor = 1.0 / total_weight;
            for contrib in contributions.values_mut() {
                contrib.weight *= factor;
                contrib.weighted_contribution *= factor;
            }
            contributions
                .values()
                .map(|c| c.weighted_contribution)
                .sum::<f64>()
        } else {
            contributions
                .values()
                .map(|c| c.weighted_contribution)
                .sum::<f64>()
        };

        (final_score.clamp(0.0, 1.0), contributions)
    }

    /// Check if any signal has blocking issues
    fn find_blocking_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for signal in &self.signals {
            if !signal.passed {
                let severity = signal.severity.unwrap_or(
                    if signal.source == "test" || signal.source == "types" {
                        SignalSeverity::Critical
                    } else {
                        SignalSeverity::Warning
                    },
                );

                if severity == SignalSeverity::Critical {
                    issues.push(format!(
                        "{}: {} (score: {:.0}%)",
                        signal.source,
                        if signal.value < 0.5 {
                            "critical failure"
                        } else {
                            "failure with warnings"
                        },
                        signal.value * 100.0
                    ));
                }
            }
        }
        issues
    }

    /// Generate outcome message
    fn generate_message(&self, score: f64, passed: bool) -> String {
        if passed {
            format!(
                "Validation passed with confidence score: {:.0}%",
                score * 100.0
            )
        } else {
            let blockers = self.find_blocking_issues();
            if blockers.is_empty() {
                format!(
                    "Validation failed - score {:.0}% below threshold {:.0}%",
                    score * 100.0,
                    self.config.pass_threshold * 100.0
                )
            } else {
                format!(
                    "Validation failed with {} blocking issue(s): {}",
                    blockers.len(),
                    blockers.join("; ")
                )
            }
        }
    }

    /// Compute the multi-signal outcome
    pub fn compute_outcome(&self) -> MultiSignalOutcome {
        // Check if we have enough signals
        if !self.has_min_signals() {
            return MultiSignalOutcome {
                passed: false,
                score: 0.0,
                signals: self.signals.clone(),
                contributions: HashMap::new(),
                message: format!(
                    "Insufficient signals: {} provided, {} required",
                    self.signals.len(),
                    self.config.min_signals
                ),
                level: OutcomeLevel::Minimal,
                blocking_issues: vec!["insufficient_signals".to_string()],
            };
        }

        // Compute weighted score
        let (score, contributions) = self.compute_weighted_score();

        // Determine pass/fail
        let all_passed = self.signals.iter().all(|s| s.passed);
        let passed = if self.config.require_all_pass {
            all_passed && score >= self.config.pass_threshold
        } else {
            score >= self.config.pass_threshold
        };

        let blocking_issues = if passed {
            Vec::new()
        } else {
            self.find_blocking_issues()
        };

        let message = self.generate_message(score, passed);

        MultiSignalOutcome {
            passed,
            score,
            signals: self.signals.clone(),
            contributions,
            message,
            level: OutcomeLevel::Standard,
            blocking_issues,
        }
    }

    /// Compute outcome with verbose level
    pub fn compute_outcome_verbose(&self) -> MultiSignalOutcome {
        let mut outcome = self.compute_outcome();
        outcome.level = OutcomeLevel::Verbose;
        outcome
    }

    /// Reset validator state
    pub fn reset(&mut self) -> &mut Self {
        self.signals.clear();
        self.history.clear();
        self
    }

    /// Save current signals to history
    pub fn save_to_history(&mut self) -> &mut Self {
        self.history.push(self.signals.clone());
        self.signals.clear();
        self
    }

    /// Get the most recent history entry
    pub fn get_last_history(&self) -> Option<&[ValidationSignal]> {
        self.history.last().map(|h| h.as_slice())
    }
}

impl Default for MultiSignalValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing multi-signal outcomes
#[derive(Debug, Default)]
pub struct MultiSignalOutcomeBuilder {
    signals: Vec<ValidationSignal>,
    config: SignalConfig,
    level: OutcomeLevel,
}

impl MultiSignalOutcomeBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a signal
    pub fn add_signal(mut self, signal: ValidationSignal) -> Self {
        self.signals.push(signal);
        self
    }

    /// Add test signal
    pub fn test(mut self, passed: bool, coverage: f64) -> Self {
        self.signals.push(ValidationSignal::test(passed, coverage));
        self
    }

    /// Add lint signal
    pub fn lint(mut self, passed: bool, warning_ratio: f64) -> Self {
        self.signals
            .push(ValidationSignal::lint(passed, warning_ratio));
        self
    }

    /// Add types signal
    pub fn types(mut self, passed: bool, score: f64) -> Self {
        self.signals.push(ValidationSignal::types(passed, score));
        self
    }

    /// Add static analysis signal
    pub fn static_analysis(mut self, passed: bool, score: f64) -> Self {
        self.signals
            .push(ValidationSignal::static_analysis(passed, score));
        self
    }

    /// Add LLM review signal
    pub fn llm_review(mut self, passed: bool, confidence: f64) -> Self {
        self.signals
            .push(ValidationSignal::llm_review(passed, confidence));
        self
    }

    /// Add spec conformance signal
    pub fn spec_conformance(mut self, passed: bool, score: f64) -> Self {
        self.signals
            .push(ValidationSignal::spec_conformance(passed, score));
        self
    }

    /// Set configuration
    pub fn config(mut self, config: SignalConfig) -> Self {
        self.config = config;
        self
    }

    /// Set verbose output
    pub fn verbose(mut self) -> Self {
        self.level = OutcomeLevel::Verbose;
        self
    }

    /// Build the outcome
    pub fn build(self) -> MultiSignalOutcome {
        let mut validator = MultiSignalValidator::with_config(self.config);
        validator.add_signals(self.signals);

        let mut outcome = validator.compute_outcome();
        outcome.level = self.level;
        outcome
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod signal_tests {
    use super::*;

    #[test]
    fn test_signal_test_pass() {
        let signal = ValidationSignal::test(true, 0.9);
        assert_eq!(signal.source, "test");
        assert!(signal.passed);
        assert_eq!(signal.value, 0.9);
        assert!(signal.severity.is_none());
    }

    #[test]
    fn test_signal_test_fail() {
        let signal = ValidationSignal::test(false, 0.7);
        assert!(!signal.passed);
        assert_eq!(signal.severity, Some(SignalSeverity::Critical));
    }

    #[test]
    fn test_signal_lint() {
        let signal = ValidationSignal::lint(true, 0.1);
        assert_eq!(signal.source, "lint");
        assert!(signal.passed);
        assert_eq!(signal.value, 0.9); // 1.0 - 0.1 warning ratio
    }

    #[test]
    fn test_signal_custom() {
        let signal = ValidationSignal::custom("custom_source", false, 0.5, Some(0.2), None);
        assert_eq!(signal.source, "custom_source");
        assert!(!signal.passed);
        assert_eq!(signal.value, 0.5);
        assert_eq!(signal.weight_override, Some(0.2));
    }

    #[test]
    fn test_signal_with_metadata() {
        let signal = ValidationSignal::test(true, 0.8).with_metadata("file", "test.rs");
        assert_eq!(signal.metadata.get("file"), Some(&"test.rs".to_string()));
    }
}

#[cfg(test)]
mod weights_tests {
    use super::*;

    #[test]
    fn test_default_weights() {
        let weights = SignalWeights::default();
        assert_eq!(weights.test, 0.30);
        assert_eq!(weights.lint, 0.20);
        assert_eq!(weights.types, 0.15);
        assert_eq!(weights.static_analysis, 0.15);
        assert_eq!(weights.llm_review, 0.10);
        assert_eq!(weights.spec_conformance, 0.10);
    }

    #[test]
    fn test_get_weight() {
        let weights = SignalWeights::default();
        assert_eq!(weights.get_weight("test"), 0.30);
        assert_eq!(weights.get_weight("lint"), 0.20);
        assert_eq!(weights.get_weight("unknown"), 0.1);
    }

    #[test]
    fn test_normalize_weights() {
        let mut weights = SignalWeights::new(0.5, 0.5, 0.5, 0.5, 0.5, 0.5);
        weights.normalize();
        let total = weights.test
            + weights.lint
            + weights.types
            + weights.static_analysis
            + weights.llm_review
            + weights.spec_conformance;
        assert!((total - 1.0).abs() < 0.001);
    }
}

#[cfg(test)]
mod validator_tests {
    use super::*;

    #[test]
    fn test_validator_empty() {
        let validator = MultiSignalValidator::new();
        let outcome = validator.compute_outcome();
        assert!(!outcome.passed);
        assert_eq!(outcome.score, 0.0);
    }

    #[test]
    fn test_validator_single_signal() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(true, 0.9);
        let outcome = validator.compute_outcome();
        assert!(!outcome.passed); // Need min 2 signals
    }

    #[test]
    fn test_validator_two_signals_pass() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(true, 0.9).add_lint(true, 0.1);
        let outcome = validator.compute_outcome();
        assert!(outcome.passed);
        assert!(outcome.score > 0.8);
    }

    #[test]
    fn test_validator_two_signals_fail() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(false, 0.5).add_lint(false, 0.5);
        let outcome = validator.compute_outcome();
        assert!(!outcome.passed);
    }

    #[test]
    fn test_validator_contributions() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(true, 1.0).add_lint(true, 0.0); // 100% test, 100% lint

        let outcome = validator.compute_outcome();
        assert!(outcome.contributions.contains_key("test"));
        assert!(outcome.contributions.contains_key("lint"));

        let test_contrib = outcome.contributions.get("test").unwrap();
        assert!(test_contrib.passed);
    }

    #[test]
    fn test_validator_blocking_issues() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(false, 0.3).add_lint(false, 0.9); // Critical test failure

        let outcome = validator.compute_outcome();
        assert!(!outcome.passed);
        assert!(!outcome.blocking_issues.is_empty());
    }

    #[test]
    fn test_validator_weighted_score() {
        // Test with known weights
        let config = SignalConfig {
            weights: SignalWeights::new(0.5, 0.5, 0.0, 0.0, 0.0, 0.0),
            pass_threshold: 0.75,
            require_all_pass: false,
            auto_normalize: true,
            min_signals: 1,
        };
        let mut validator = MultiSignalValidator::with_config(config);
        validator.add_test(true, 1.0); // 100% coverage
        validator.add_lint(true, 0.0); // No warnings

        let outcome = validator.compute_outcome();
        // Both passed with perfect scores, should be 1.0
        assert_eq!(outcome.score, 1.0);
        assert!(outcome.passed);
    }

    #[test]
    fn test_validator_require_all_pass() {
        let config = SignalConfig {
            weights: SignalWeights::default(),
            pass_threshold: 0.5,
            require_all_pass: true,
            auto_normalize: true,
            min_signals: 1,
        };
        let mut validator = MultiSignalValidator::with_config(config);
        validator.add_test(true, 0.8).add_lint(false, 0.5); // One pass, one fail

        let outcome = validator.compute_outcome();
        // Should fail because not all passed
        assert!(!outcome.passed);
    }

    #[test]
    fn test_builder_pattern() {
        let outcome = MultiSignalOutcomeBuilder::new()
            .test(true, 0.9)
            .lint(true, 0.1)
            .types(true, 0.95)
            .static_analysis(true, 1.0)
            .llm_review(true, 0.8)
            .spec_conformance(true, 0.85)
            .build();

        assert!(outcome.passed);
        assert!(outcome.score > 0.8);
    }

    #[test]
    fn test_validator_clear() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(true, 0.9).add_lint(true, 0.1);
        validator.clear();
        assert!(validator.get_signals().is_empty());
    }

    #[test]
    fn test_validator_save_history() {
        let mut validator = MultiSignalValidator::new();
        validator.add_test(true, 0.9);
        validator.save_to_history();
        validator.add_lint(true, 0.1);
        validator.save_to_history();

        assert_eq!(validator.history.len(), 2);
        assert!(validator.get_signals().is_empty());
        assert!(validator.get_last_history().is_some());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_validation_workflow() {
        // Simulate a full validation workflow with all signals
        let outcome = MultiSignalOutcomeBuilder::new()
            .test(true, 0.85) // 85% test coverage
            .lint(true, 0.05) // Only 5% warnings
            .types(true, 0.98) // Excellent type safety
            .static_analysis(true, 0.9) // Only minor findings
            .llm_review(true, 0.75) // AI review is confident
            .spec_conformance(true, 0.8) // Good spec match
            .build();

        assert!(outcome.passed);
        assert!(outcome.score > 0.7);
        assert!(outcome.blocking_issues.is_empty());
    }

    #[tokio::test]
    async fn test_validation_with_failures() {
        // Simulate a validation with some failures
        let outcome = MultiSignalOutcomeBuilder::new()
            .test(false, 0.6) // Test failure - critical
            .lint(true, 0.1)
            .types(true, 0.95)
            .static_analysis(true, 0.85)
            .llm_review(true, 0.7)
            .spec_conformance(true, 0.75)
            .build();

        assert!(!outcome.passed);
        assert!(outcome.score < 0.7);
        assert!(!outcome.blocking_issues.is_empty());
    }

    #[tokio::test]
    async fn test_validation_edge_cases() {
        // All signals pass with high scores - verify high score
        // Note: For lint signal, warning_ratio=0.1 means 90% lint quality (only 10% warnings)
        let outcome = MultiSignalOutcomeBuilder::new()
            .test(true, 0.9)
            .lint(true, 0.1) // 90% lint quality = 10% warning ratio
            .types(true, 0.9)
            .static_analysis(true, 0.9)
            .llm_review(true, 0.9)
            .spec_conformance(true, 0.9)
            .build();

        assert!(
            outcome.passed,
            "Expected all passing signals to pass validation"
        );
        // Score = 0.9 * (0.30 + 0.20 + 0.15 + 0.15 + 0.10 + 0.10) = 0.9
        assert!(
            outcome.score > 0.85,
            "Expected score > 0.85, got {}",
            outcome.score
        );

        // All signals fail with zero scores - verify zero score
        let outcome = MultiSignalOutcomeBuilder::new()
            .test(false, 0.0)
            .lint(false, 0.0)
            .types(false, 0.0)
            .static_analysis(false, 0.0)
            .llm_review(false, 0.0)
            .spec_conformance(false, 0.0)
            .build();

        assert!(
            !outcome.passed,
            "Expected all failing signals to fail validation"
        );
        assert_eq!(
            outcome.score, 0.0,
            "Expected zero score for all failing signals"
        );
    }
}
