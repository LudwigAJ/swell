//! Calibrated confidence module for validation outcomes.
//!
//! Trains model against post-merge defect rates to produce calibrated confidence scores
//! that accurately predict the probability of validation passing.
//!
//! # Key Components
//!
//! - [`DefectRecord`] - Records observed post-merge defects
//! - [`CalibrationModel`] - Statistical model that learns from validation outcomes
//! - [`Predictor`] - Predicts validation pass probability based on signals

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A record of a post-merge defect
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefectRecord {
    /// Unique identifier for the merge
    pub merge_id: String,
    /// When the merge occurred
    pub merged_at: DateTime<Utc>,
    /// When the defect was detected
    pub detected_at: DateTime<Utc>,
    /// Number of defects detected
    pub defect_count: usize,
    /// Severity of the defect (1-5, 5 being most severe)
    pub severity: u8,
    /// Files that were modified in the merge
    pub changed_files: Vec<String>,
    /// Validation confidence score at time of merge
    pub validation_confidence: f64,
    /// Whether validation passed at time of merge
    pub validation_passed: bool,
    /// Types of gates that were run
    pub gates_run: Vec<String>,
}

/// Historical validation record for training
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRecord {
    /// Unique identifier
    pub id: String,
    /// Task or merge ID this record belongs to
    pub task_id: String,
    /// When validation was run
    pub timestamp: DateTime<Utc>,
    /// Whether validation passed
    pub passed: bool,
    /// Number of errors
    pub error_count: usize,
    /// Number of warnings
    pub warning_count: usize,
    /// Test pass rate (0.0 to 1.0)
    pub test_pass_rate: f64,
    /// Lint pass rate (0.0 to 1.0)
    pub lint_pass_rate: f64,
    /// Security findings count
    pub security_findings: usize,
    /// AI review confidence (0.0 to 1.0)
    pub ai_review_confidence: f64,
    /// Whether defect was later found in post-merge
    pub had_post_merge_defect: bool,
}

/// Signal features used for prediction
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationFeatures {
    /// Lint score (0.0 to 1.0, higher is better)
    pub lint_score: f64,
    /// Test coverage score (0.0 to 1.0)
    pub test_score: f64,
    /// Security score (0.0 to 1.0, higher means fewer issues)
    pub security_score: f64,
    /// AI review confidence (0.0 to 1.0)
    pub ai_review_confidence: f64,
    /// Number of changed files
    pub changed_file_count: usize,
    /// Number of errors in validation
    pub error_count: usize,
    /// Number of warnings in validation
    pub warning_count: usize,
    /// Historical defect rate for similar changes
    pub historical_defect_rate: f64,
}

impl CalibrationFeatures {
    /// Calculate a simple weighted score from features
    pub fn weighted_score(&self) -> f64 {
        let mut score = 0.0;
        let mut weight = 0.0;

        // Lint contributes 15%
        score += self.lint_score * 0.15;
        weight += 0.15;

        // Tests contribute 30%
        score += self.test_score * 0.30;
        weight += 0.30;

        // Security contributes 20%
        score += self.security_score * 0.20;
        weight += 0.20;

        // AI review contributes 15%
        score += self.ai_review_confidence * 0.15;
        weight += 0.15;

        // Penalize for errors and warnings (10%)
        let error_penalty = (self.error_count as f64 * 0.05).min(0.10);
        let warning_penalty = (self.warning_count as f64 * 0.01).min(0.05);
        score -= error_penalty + warning_penalty;
        weight += 0.10;

        if weight > 0.0 {
            score / weight
        } else {
            0.5
        }
    }
}

/// Calibration parameters learned from historical data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationParams {
    /// Bias term for logistic regression
    pub bias: f64,
    /// Weight for lint score
    pub lint_weight: f64,
    /// Weight for test score
    pub test_weight: f64,
    /// Weight for security score
    pub security_weight: f64,
    /// Weight for AI review confidence
    pub ai_review_weight: f64,
    /// Weight for file count (more files = higher risk)
    pub file_count_weight: f64,
    /// Weight for error count
    pub error_weight: f64,
    /// Weight for warning count
    pub warning_weight: f64,
    /// Number of training samples
    pub sample_count: usize,
    /// Model accuracy on training data
    pub accuracy: f64,
}

impl Default for CalibrationParams {
    fn default() -> Self {
        // Fairly neutral weights - will be updated during training
        Self {
            bias: 0.0,
            lint_weight: 1.0,
            test_weight: 1.0,
            security_weight: 1.0,
            ai_review_weight: 1.0,
            file_count_weight: 0.0,
            error_weight: -0.5,
            warning_weight: -0.1,
            sample_count: 0,
            accuracy: 0.0,
        }
    }
}

/// Calibrated confidence result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibratedConfidence {
    /// Predicted probability of passing validation
    pub pass_probability: f64,
    /// Calibration quality (0.0 to 1.0, higher is better calibrated)
    pub calibration_quality: f64,
    /// Risk level based on probability
    pub risk_level: RiskLevel,
    /// Model used for prediction
    pub model_version: String,
}

/// Risk level categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Very low risk, high confidence pass
    VeryLow,
    /// Low risk, likely to pass
    Low,
    /// Medium risk, uncertain outcome
    Medium,
    /// High risk, likely to fail
    High,
    /// Very high risk, almost certainly will fail
    VeryHigh,
}

impl RiskLevel {
    /// Determine risk level from pass probability
    pub fn from_probability(prob: f64) -> Self {
        if prob >= 0.95 {
            RiskLevel::VeryLow
        } else if prob >= 0.85 {
            RiskLevel::Low
        } else if prob >= 0.70 {
            RiskLevel::Medium
        } else if prob >= 0.50 {
            RiskLevel::High
        } else {
            RiskLevel::VeryHigh
        }
    }

    /// Get a human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::VeryLow => "Very Low",
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
            RiskLevel::VeryHigh => "Very High",
        }
    }
}

/// Tracks defect history and calculates defect rates
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefectTracker {
    /// Historical defect records
    records: Vec<DefectRecord>,
    /// Defect rate by file pattern (e.g., "src/*.rs")
    file_pattern_defect_rates: HashMap<String, f64>,
    /// Overall defect rate
    overall_defect_rate: f64,
    /// Defect rate by day of week
    day_of_week_defect_rates: [f64; 7],
}

impl DefectTracker {
    /// Create a new empty defect tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new defect
    pub fn record_defect(&mut self, record: DefectRecord) {
        self.records.push(record.clone());
        self.update_rates();
    }

    /// Record a merge without defect
    pub fn record_clean_merge(&mut self, merge_id: String, merged_at: DateTime<Utc>) {
        self.records.push(DefectRecord {
            merge_id,
            merged_at,
            detected_at: merged_at, // No defect detected, use merge time
            defect_count: 0,
            severity: 0,
            changed_files: vec![],
            validation_confidence: 1.0,
            validation_passed: true,
            gates_run: vec![],
        });
        self.update_rates();
    }

    /// Get overall defect rate
    pub fn defect_rate(&self) -> f64 {
        self.overall_defect_rate
    }

    /// Get defect rate for a specific file
    pub fn file_defect_rate(&self, file: &str) -> f64 {
        // Find the most specific matching pattern
        let mut best_rate = self.overall_defect_rate;

        for (pattern, rate) in &self.file_pattern_defect_rates {
            if (file.contains(pattern.trim_start_matches("*.")) || pattern == "*")
                && *rate > best_rate
            {
                best_rate = *rate;
            }
        }

        best_rate
    }

    /// Get defect rate for changed files
    pub fn changed_files_defect_rate(&self, files: &[String]) -> f64 {
        if files.is_empty() {
            return self.overall_defect_rate;
        }

        let rates: Vec<f64> = files.iter().map(|f| self.file_defect_rate(f)).collect();
        let sum: f64 = rates.iter().sum();
        sum / rates.len() as f64
    }

    /// Get number of records
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Get records with defects
    pub fn records_with_defects(&self) -> usize {
        self.records.iter().filter(|r| r.defect_count > 0).count()
    }

    /// Get average severity of defects
    pub fn average_severity(&self) -> f64 {
        let defects: Vec<&DefectRecord> =
            self.records.iter().filter(|r| r.defect_count > 0).collect();
        if defects.is_empty() {
            return 0.0;
        }
        let sum: u32 = defects.iter().map(|d| d.severity as u32).sum();
        sum as f64 / defects.len() as f64
    }

    /// Update internal rate calculations
    fn update_rates(&mut self) {
        let total = self.records.len();
        if total == 0 {
            self.overall_defect_rate = 0.0;
            return;
        }

        // Calculate overall defect rate
        let defective = self.records.iter().filter(|r| r.defect_count > 0).count();
        self.overall_defect_rate = defective as f64 / total as f64;

        // Update file pattern rates
        let mut file_counts: HashMap<String, (usize, usize)> = HashMap::new(); // pattern -> (total, defective)

        for record in &self.records {
            for file in &record.changed_files {
                let extension = file.split('.').next_back().unwrap_or("*");
                let pattern = format!("*.{}", extension);

                let entry = file_counts.entry(pattern).or_insert((0, 0));
                entry.0 += 1;
                if record.defect_count > 0 {
                    entry.1 += 1;
                }
            }
        }

        for (pattern, (total, defective)) in file_counts {
            if total >= 3 {
                // Only track patterns with enough data
                self.file_pattern_defect_rates
                    .insert(pattern, defective as f64 / total as f64);
            }
        }

        // Update day of week rates
        let mut day_counts: [usize; 7] = [0; 7];
        let mut day_defects: [usize; 7] = [0; 7];

        for record in &self.records {
            let day = record.merged_at.weekday().num_days_from_monday() as usize;
            day_counts[day] += 1;
            if record.defect_count > 0 {
                day_defects[day] += 1;
            }
        }

        for i in 0..7 {
            if day_counts[i] >= 3 {
                self.day_of_week_defect_rates[i] = day_defects[i] as f64 / day_counts[i] as f64;
            }
        }
    }

    /// Merge another tracker into this one
    pub fn merge(&mut self, other: &DefectTracker) {
        self.records.extend(other.records.clone());
        self.update_rates();
    }

    /// Clear all records
    pub fn clear(&mut self) {
        self.records.clear();
        self.file_pattern_defect_rates.clear();
        self.overall_defect_rate = 0.0;
        self.day_of_week_defect_rates = [0.0; 7];
    }
}

/// Calibration model that learns from historical validation outcomes
#[derive(Debug, Clone)]
pub struct CalibrationModel {
    /// Learned parameters
    params: CalibrationParams,
    /// Version identifier
    version: String,
    /// Minimum samples before making predictions
    min_samples: usize,
}

impl CalibrationModel {
    /// Create a new calibration model with default parameters
    pub fn new() -> Self {
        Self {
            params: CalibrationParams::default(),
            version: "1.0.0".to_string(),
            min_samples: 10,
        }
    }

    /// Create with custom minimum sample requirement
    pub fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Train the model on historical validation records
    pub fn train(&mut self, records: &[ValidationRecord]) -> TrainingResult {
        let sample_count = records.len();

        if sample_count < self.min_samples {
            return TrainingResult {
                success: false,
                samples_used: sample_count,
                accuracy: 0.0,
                message: format!(
                    "Not enough samples: need {}, got {}",
                    self.min_samples, sample_count
                ),
            };
        }

        // Simple logistic regression training using gradient descent
        let mut params = self.params.clone();
        params.sample_count = sample_count;

        // Initialize weights from feature correlations
        let learning_rate = 0.1;
        let iterations = 100;

        for _ in 0..iterations {
            let mut gradient_bias = 0.0;
            let mut gradient_lint = 0.0;
            let mut gradient_test = 0.0;
            let mut gradient_security = 0.0;
            let mut gradient_ai = 0.0;
            let mut gradient_files = 0.0;
            let mut gradient_errors = 0.0;
            let mut gradient_warnings = 0.0;

            for record in records {
                let features = self.extract_features(record);
                let prediction = self.sigmoid(&self.predict_raw(&features, &params));
                let label = if record.passed { 1.0 } else { 0.0 };
                let error = prediction - label;

                gradient_bias += error;
                gradient_lint += error * features.lint_score;
                gradient_test += error * features.test_score;
                gradient_security += error * features.security_score;
                gradient_ai += error * features.ai_review_confidence;
                gradient_files += error * features.changed_file_count as f64;
                gradient_errors += error * features.error_count as f64;
                gradient_warnings += error * features.warning_count as f64;
            }

            // Normalize gradients
            let n = sample_count as f64;
            params.bias -= learning_rate * gradient_bias / n;
            params.lint_weight -= learning_rate * gradient_lint / n;
            params.test_weight -= learning_rate * gradient_test / n;
            params.security_weight -= learning_rate * gradient_security / n;
            params.ai_review_weight -= learning_rate * gradient_ai / n;
            params.file_count_weight -= learning_rate * gradient_files / n;
            params.error_weight -= learning_rate * gradient_errors / n;
            params.warning_weight -= learning_rate * gradient_warnings / n;
        }

        // Calculate accuracy on training data
        let mut correct = 0usize;
        for record in records {
            let features = self.extract_features(record);
            let prediction = self.predict_probability_internal(&features, &params);
            let predicted_pass = prediction >= 0.5;
            if predicted_pass == record.passed {
                correct += 1;
            }
        }

        params.accuracy = correct as f64 / sample_count as f64;
        self.params = params;

        TrainingResult {
            success: true,
            samples_used: sample_count,
            accuracy: self.params.accuracy,
            message: "Training complete".to_string(),
        }
    }

    /// Predict pass probability for given features
    pub fn predict(&self, features: &CalibrationFeatures) -> CalibratedConfidence {
        let probability = self.predict_probability(features);
        let calibration_quality = self.calculate_calibration_quality();

        CalibratedConfidence {
            pass_probability: probability,
            calibration_quality,
            risk_level: RiskLevel::from_probability(probability),
            model_version: self.version.clone(),
        }
    }

    /// Get the current calibration parameters
    pub fn params(&self) -> &CalibrationParams {
        &self.params
    }

    /// Check if model has been trained with enough data
    pub fn is_trained(&self) -> bool {
        self.params.sample_count >= self.min_samples
    }

    /// Extract features from a validation record
    fn extract_features(&self, record: &ValidationRecord) -> CalibrationFeatures {
        CalibrationFeatures {
            lint_score: record.lint_pass_rate,
            test_score: record.test_pass_rate,
            security_score: 1.0 - (record.security_findings as f64 * 0.1).min(1.0),
            ai_review_confidence: record.ai_review_confidence,
            changed_file_count: 0, // Not available in ValidationRecord
            error_count: record.error_count,
            warning_count: record.warning_count,
            historical_defect_rate: 0.0, // Would need DefectTracker
        }
    }

    /// Raw prediction before sigmoid
    fn predict_raw(&self, features: &CalibrationFeatures, params: &CalibrationParams) -> f64 {
        params.bias
            + params.lint_weight * features.lint_score
            + params.test_weight * features.test_score
            + params.security_weight * features.security_score
            + params.ai_review_weight * features.ai_review_confidence
            + params.file_count_weight * features.changed_file_count as f64
            + params.error_weight * features.error_count as f64
            + params.warning_weight * features.warning_count as f64
    }

    /// Sigmoid function
    fn sigmoid(&self, x: &f64) -> f64 {
        1.0 / (1.0 + (-x).exp())
    }

    /// Internal probability prediction with custom params
    fn predict_probability_internal(
        &self,
        features: &CalibrationFeatures,
        params: &CalibrationParams,
    ) -> f64 {
        let raw = self.predict_raw(features, params);
        self.sigmoid(&raw)
    }

    /// Predict probability of passing
    fn predict_probability(&self, features: &CalibrationFeatures) -> f64 {
        self.predict_probability_internal(features, &self.params)
    }

    /// Calculate calibration quality (placeholder - real implementation would use calibration curves)
    fn calculate_calibration_quality(&self) -> f64 {
        // Simple heuristic based on sample count and accuracy
        if self.params.sample_count < 10 {
            0.3
        } else if self.params.sample_count < 30 {
            0.6
        } else if self.params.sample_count < 100 {
            0.8
        } else {
            0.95_f64.min(self.params.accuracy + 0.1)
        }
    }
}

impl Default for CalibrationModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of training operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingResult {
    /// Whether training succeeded
    pub success: bool,
    /// Number of samples used
    pub samples_used: usize,
    /// Model accuracy on training data
    pub accuracy: f64,
    /// Human-readable message
    pub message: String,
}

/// Predictor that combines defect tracking and calibration for validation prediction
#[derive(Debug, Clone)]
pub struct Predictor {
    /// Defect tracker for historical defect data
    defect_tracker: DefectTracker,
    /// Calibration model for statistical learning
    calibration_model: CalibrationModel,
}

impl Predictor {
    /// Create a new predictor with default settings
    pub fn new() -> Self {
        Self {
            defect_tracker: DefectTracker::new(),
            calibration_model: CalibrationModel::new(),
        }
    }

    /// Create with custom minimum training samples
    pub fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.calibration_model = self.calibration_model.with_min_samples(min_samples);
        self
    }

    /// Record a validation outcome for future training
    pub fn record_outcome(&mut self, record: ValidationRecord) {
        // The calibration model will be retrained when needed
        // For now, we just track it in the defect tracker if there was a defect
        if record.had_post_merge_defect {
            self.defect_tracker.record_defect(DefectRecord {
                merge_id: record.id.clone(),
                merged_at: record.timestamp,
                detected_at: record.timestamp,
                defect_count: 1,
                severity: 3,
                changed_files: vec![],
                validation_confidence: 0.5,
                validation_passed: record.passed,
                gates_run: vec![],
            });
        } else if record.passed {
            self.defect_tracker
                .record_clean_merge(record.id, record.timestamp);
        }
    }

    /// Train the calibration model on recorded outcomes
    /// This should be called periodically with accumulated validation records
    pub fn train(&mut self, records: &[ValidationRecord]) -> TrainingResult {
        self.calibration_model.train(records)
    }

    /// Predict validation pass probability for given features
    pub fn predict(&self, features: CalibrationFeatures) -> CalibratedConfidence {
        let mut features = features;

        // Enrich with historical defect rate
        features.historical_defect_rate = self.defect_tracker.changed_files_defect_rate(&[]);

        self.calibration_model.predict(&features)
    }

    /// Predict with file-specific risk
    pub fn predict_for_files(
        &self,
        features: CalibrationFeatures,
        files: &[String],
    ) -> CalibratedConfidence {
        let mut features = features;
        features.historical_defect_rate = self.defect_tracker.changed_files_defect_rate(files);
        self.calibration_model.predict(&features)
    }

    /// Get defect tracker reference
    pub fn defect_tracker(&self) -> &DefectTracker {
        &self.defect_tracker
    }

    /// Get defect tracker mutable reference
    pub fn defect_tracker_mut(&mut self) -> &mut DefectTracker {
        &mut self.defect_tracker
    }

    /// Get calibration model reference
    pub fn model(&self) -> &CalibrationModel {
        &self.calibration_model
    }

    /// Check if model is ready for predictions
    pub fn is_ready(&self) -> bool {
        self.calibration_model.is_trained()
    }
}

impl Default for Predictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_validation_records() -> Vec<ValidationRecord> {
        vec![
            ValidationRecord {
                id: "1".to_string(),
                task_id: "task1".to_string(),
                timestamp: Utc::now(),
                passed: true,
                error_count: 0,
                warning_count: 1,
                test_pass_rate: 1.0,
                lint_pass_rate: 1.0,
                security_findings: 0,
                ai_review_confidence: 0.9,
                had_post_merge_defect: false,
            },
            ValidationRecord {
                id: "2".to_string(),
                task_id: "task2".to_string(),
                timestamp: Utc::now(),
                passed: true,
                error_count: 1,
                warning_count: 2,
                test_pass_rate: 0.95,
                lint_pass_rate: 0.9,
                security_findings: 0,
                ai_review_confidence: 0.8,
                had_post_merge_defect: false,
            },
            ValidationRecord {
                id: "3".to_string(),
                task_id: "task3".to_string(),
                timestamp: Utc::now(),
                passed: false,
                error_count: 5,
                warning_count: 10,
                test_pass_rate: 0.7,
                lint_pass_rate: 0.6,
                security_findings: 2,
                ai_review_confidence: 0.5,
                had_post_merge_defect: true,
            },
            ValidationRecord {
                id: "4".to_string(),
                task_id: "task4".to_string(),
                timestamp: Utc::now(),
                passed: true,
                error_count: 0,
                warning_count: 0,
                test_pass_rate: 1.0,
                lint_pass_rate: 1.0,
                security_findings: 0,
                ai_review_confidence: 0.95,
                had_post_merge_defect: false,
            },
            ValidationRecord {
                id: "5".to_string(),
                task_id: "task5".to_string(),
                timestamp: Utc::now(),
                passed: false,
                error_count: 3,
                warning_count: 5,
                test_pass_rate: 0.8,
                lint_pass_rate: 0.7,
                security_findings: 1,
                ai_review_confidence: 0.6,
                had_post_merge_defect: true,
            },
        ]
    }

    #[test]
    fn test_prediction_features_weighted_score() {
        let features = CalibrationFeatures {
            lint_score: 1.0,
            test_score: 1.0,
            security_score: 1.0,
            ai_review_confidence: 1.0,
            changed_file_count: 5,
            error_count: 0,
            warning_count: 0,
            historical_defect_rate: 0.1,
        };

        let score = features.weighted_score();
        // High-quality features should produce a high score (around 0.88-0.89 due to normalization)
        assert!(
            score > 0.85,
            "High-quality features should produce high score"
        );
    }

    #[test]
    fn test_risk_level_from_probability() {
        assert_eq!(RiskLevel::from_probability(0.97), RiskLevel::VeryLow);
        assert_eq!(RiskLevel::from_probability(0.90), RiskLevel::Low);
        assert_eq!(RiskLevel::from_probability(0.75), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_probability(0.60), RiskLevel::High);
        assert_eq!(RiskLevel::from_probability(0.40), RiskLevel::VeryHigh);
    }

    #[test]
    fn test_risk_level_label() {
        assert_eq!(RiskLevel::VeryLow.label(), "Very Low");
        assert_eq!(RiskLevel::Low.label(), "Low");
        assert_eq!(RiskLevel::Medium.label(), "Medium");
        assert_eq!(RiskLevel::High.label(), "High");
        assert_eq!(RiskLevel::VeryHigh.label(), "Very High");
    }

    #[test]
    fn test_defect_tracker_basic() {
        let mut tracker = DefectTracker::new();

        assert_eq!(tracker.defect_rate(), 0.0);
        assert_eq!(tracker.record_count(), 0);

        tracker.record_clean_merge("merge1".to_string(), Utc::now());
        tracker.record_clean_merge("merge2".to_string(), Utc::now());

        assert_eq!(tracker.defect_rate(), 0.0);
        assert_eq!(tracker.record_count(), 2);
    }

    #[test]
    fn test_defect_tracker_with_defects() {
        let mut tracker = DefectTracker::new();

        tracker.record_clean_merge("merge1".to_string(), Utc::now());
        tracker.record_defect(DefectRecord {
            merge_id: "merge2".to_string(),
            merged_at: Utc::now(),
            detected_at: Utc::now(),
            defect_count: 1,
            severity: 3,
            changed_files: vec!["test.rs".to_string()],
            validation_confidence: 0.5,
            validation_passed: true,
            gates_run: vec![],
        });

        assert_eq!(tracker.defect_rate(), 0.5);
        assert_eq!(tracker.records_with_defects(), 1);
        assert_eq!(tracker.average_severity(), 3.0);
    }

    #[test]
    fn test_calibration_model_not_trained_initially() {
        let model = CalibrationModel::new();
        assert!(!model.is_trained());
    }

    #[test]
    fn test_calibration_model_train_insufficient_data() {
        let mut model = CalibrationModel::new().with_min_samples(10);

        let records = vec![ValidationRecord {
            id: "1".to_string(),
            task_id: "task1".to_string(),
            timestamp: Utc::now(),
            passed: true,
            error_count: 0,
            warning_count: 0,
            test_pass_rate: 1.0,
            lint_pass_rate: 1.0,
            security_findings: 0,
            ai_review_confidence: 0.9,
            had_post_merge_defect: false,
        }];

        let result = model.train(&records);
        assert!(!result.success);
        assert_eq!(result.samples_used, 1);
    }

    #[test]
    fn test_calibration_model_train_sufficient_data() {
        let mut model = CalibrationModel::new().with_min_samples(3);
        let records = create_test_validation_records();

        let result = model.train(&records);
        assert!(result.success);
        assert_eq!(result.samples_used, 5);
        assert!(result.accuracy > 0.0);
    }

    #[test]
    fn test_calibration_model_predict() {
        let mut model = CalibrationModel::new().with_min_samples(3);
        let records = create_test_validation_records();
        model.train(&records);

        let features = CalibrationFeatures {
            lint_score: 1.0,
            test_score: 1.0,
            security_score: 1.0,
            ai_review_confidence: 0.9,
            changed_file_count: 2,
            error_count: 0,
            warning_count: 1,
            historical_defect_rate: 0.1,
        };

        let confidence = model.predict(&features);
        assert!(confidence.pass_probability >= 0.0);
        assert!(confidence.pass_probability <= 1.0);
        assert_eq!(confidence.model_version, "1.0.0");
    }

    #[test]
    fn test_predictor_default() {
        let predictor = Predictor::new();
        assert!(!predictor.is_ready());
    }

    #[test]
    fn test_predictor_record_outcome() {
        let mut predictor = Predictor::new();

        predictor.record_outcome(ValidationRecord {
            id: "1".to_string(),
            task_id: "task1".to_string(),
            timestamp: Utc::now(),
            passed: true,
            error_count: 0,
            warning_count: 0,
            test_pass_rate: 1.0,
            lint_pass_rate: 1.0,
            security_findings: 0,
            ai_review_confidence: 0.9,
            had_post_merge_defect: false,
        });

        // Should have recorded a clean merge
        assert_eq!(predictor.defect_tracker().record_count(), 1);
    }

    #[test]
    fn test_predictor_train_and_predict() {
        let mut predictor = Predictor::new().with_min_samples(3);
        let records = create_test_validation_records();

        predictor.train(&records);
        assert!(predictor.is_ready());

        let features = CalibrationFeatures {
            lint_score: 1.0,
            test_score: 1.0,
            security_score: 1.0,
            ai_review_confidence: 0.9,
            changed_file_count: 2,
            error_count: 0,
            warning_count: 0,
            historical_defect_rate: 0.0,
        };

        let confidence = predictor.predict(features);
        assert!(confidence.pass_probability >= 0.0);
        assert!(confidence.pass_probability <= 1.0);
    }

    #[test]
    fn test_calibrated_confidence_structure() {
        let confidence = CalibratedConfidence {
            pass_probability: 0.85,
            calibration_quality: 0.9,
            risk_level: RiskLevel::Low,
            model_version: "1.0.0".to_string(),
        };

        assert_eq!(confidence.risk_level, RiskLevel::Low);
        assert_eq!(confidence.model_version, "1.0.0");
    }

    #[test]
    fn test_defect_tracker_merge() {
        let mut tracker1 = DefectTracker::new();
        let mut tracker2 = DefectTracker::new();

        tracker1.record_clean_merge("merge1".to_string(), Utc::now());
        tracker2.record_defect(DefectRecord {
            merge_id: "merge2".to_string(),
            merged_at: Utc::now(),
            detected_at: Utc::now(),
            defect_count: 1,
            severity: 3,
            changed_files: vec![],
            validation_confidence: 0.5,
            validation_passed: true,
            gates_run: vec![],
        });

        tracker1.merge(&tracker2);

        assert_eq!(tracker1.record_count(), 2);
        assert_eq!(tracker1.defect_rate(), 0.5);
    }

    #[test]
    fn test_defect_tracker_clear() {
        let mut tracker = DefectTracker::new();

        tracker.record_defect(DefectRecord {
            merge_id: "merge1".to_string(),
            merged_at: Utc::now(),
            detected_at: Utc::now(),
            defect_count: 1,
            severity: 3,
            changed_files: vec![],
            validation_confidence: 0.5,
            validation_passed: true,
            gates_run: vec![],
        });

        assert_eq!(tracker.record_count(), 1);

        tracker.clear();
        assert_eq!(tracker.record_count(), 0);
        assert_eq!(tracker.defect_rate(), 0.0);
    }
}
