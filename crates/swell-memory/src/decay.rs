// decay.rs - Time-based memory decay with different rates per memory type
//
// This module implements decay functions for different memory types:
// - Procedural: Slow decay (0.99^(days since last reinforcement)
// - Environmental: Medium decay (0.95^(days))
// - Buffer: Fast decay (0.90^(days))
//
// Decay affects retrieval probability - memories that haven't been
// reinforced recently become less likely to be retrieved.

use chrono::{DateTime, Utc};

#[cfg(test)]
use chrono::Duration;

/// Memory decay rates per category
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DecayRate {
    /// Procedural memory - slow decay (0.99 per day)
    /// Used for procedures, patterns, skills
    Procedural,
    /// Environmental memory - medium decay (0.95 per day)
    /// Used for semantic facts, entities, relationships
    Environmental,
    /// Buffer memory - fast decay (0.90 per day)
    /// Used for conversation logs, short-term recall
    Buffer,
}

impl DecayRate {
    /// Get the decay factor (base) for this rate
    /// Returns the base for the exponential decay: base^(days)
    pub fn base(&self) -> f64 {
        match self {
            DecayRate::Procedural => 0.99,
            DecayRate::Environmental => 0.95,
            DecayRate::Buffer => 0.90,
        }
    }

    /// Get the lambda (λ) parameter for Bayesian exponential decay
    /// Returns the decay rate constant for the formula: c(t) = c₀ × e^(-λt)
    pub fn lambda(&self) -> f64 {
        match self {
            DecayRate::Procedural => 0.01,
            DecayRate::Environmental => 1.0,
            DecayRate::Buffer => 5.0,
        }
    }

    /// Get the category name for logging/debugging
    pub fn name(&self) -> &'static str {
        match self {
            DecayRate::Procedural => "procedural",
            DecayRate::Environmental => "environmental",
            DecayRate::Buffer => "buffer",
        }
    }
}

/// Calculate the decay factor for a memory based on time since last reinforcement.
///
/// # Formula
/// - Procedural: 0.99^(days since last reinforcement)
/// - Environmental: 0.95^(days since last update)
/// - Buffer: 0.90^(days since last update)
///
/// # Arguments
/// * `rate` - The decay rate category
/// * `last_reinforcement` - The timestamp of last reinforcement/update
/// * `now` - Current timestamp (defaults to Utc::now())
///
/// # Returns
/// A decay factor between 0.0 and 1.0, where:
/// - 1.0 = fresh memory (no decay)
/// - 0.0 = fully decayed (retrieval probability is 0)
pub fn calculate_decay(
    rate: DecayRate,
    last_reinforcement: DateTime<Utc>,
    now: DateTime<Utc>,
) -> f64 {
    let days_elapsed = (now - last_reinforcement).num_days() as f64;
    let base = rate.base();

    // Apply exponential decay: base^days
    // For large time differences, clamp to prevent underflow
    if days_elapsed > 365.0 {
        // After a year, clamp to a minimal value
        return base.powf(365.0).max(0.001);
    }

    base.powf(days_elapsed)
}

/// Calculate days since last reinforcement
pub fn days_since(last_reinforcement: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    (now - last_reinforcement).num_days() as f64
}

/// Calculate Bayesian confidence with exponential decay using lambda parameter.
///
/// # Formula
/// c(t) = c₀ × e^(-λt)
///
/// Where:
/// - c₀ is the initial confidence
/// - λ (lambda) is the decay rate constant per memory type
/// - t is time elapsed in days
///
/// # Arguments
/// * `initial_confidence` - The confidence score at last reinforcement (0.0 to 1.0)
/// * `rate` - The decay rate category (determines λ)
/// * `last_reinforcement` - The timestamp of last reinforcement/update
/// * `now` - Current timestamp
///
/// # Returns
/// The decayed confidence score, clamped between 0.0 and initial_confidence
pub fn bayesian_confidence_decay(
    initial_confidence: f64,
    rate: DecayRate,
    last_reinforcement: DateTime<Utc>,
    now: DateTime<Utc>,
) -> f64 {
    let lambda = rate.lambda();
    let t = days_since(last_reinforcement, now);

    // Bayesian exponential decay: c(t) = c₀ × e^(-λt)
    let decayed = initial_confidence * (-lambda * t).exp();

    // Clamp to valid range [0.0, initial_confidence]
    decayed.clamp(0.0, initial_confidence)
}

/// Calculate confidence using exponential decay with time unit instead of days.
/// This allows testing with smaller time scales.
///
/// # Arguments
/// * `initial_confidence` - The confidence score at last reinforcement (0.0 to 1.0)
/// * `rate` - The decay rate category (determines λ)
/// * `last_reinforcement` - The timestamp of last reinforcement/update
/// * `now` - Current timestamp
/// * `time_unit_hours` - The time unit in hours (e.g., 1.0 for 1 hour = 1 time unit)
pub fn bayesian_confidence_decay_with_time_unit(
    initial_confidence: f64,
    rate: DecayRate,
    last_reinforcement: DateTime<Utc>,
    now: DateTime<Utc>,
    time_unit_hours: f64,
) -> f64 {
    let lambda = rate.lambda();
    // Use num_minutes to get fractional days for accurate time unit calculation
    let t_minutes = (now - last_reinforcement).num_minutes() as f64;
    let t_hours = t_minutes / 60.0;
    let time_units = t_hours / time_unit_hours;

    // Bayesian exponential decay: c(t) = c₀ × e^(-λt)
    let decayed = initial_confidence * (-lambda * time_units).exp();

    // Clamp to valid range [0.0, initial_confidence]
    decayed.clamp(0.0, initial_confidence)
}

/// Apply decay to a retrieval score
///
/// When retrieving memories, the final score is modified by the decay factor:
/// `final_score = base_score * decay_factor`
///
/// # Arguments
/// * `base_score` - The original relevance/similarity score (0.0 to 1.0)
/// * `decay_factor` - The decay factor from calculate_decay (0.0 to 1.0)
///
/// # Returns
/// The decay-adjusted score
pub fn apply_decay(base_score: f64, decay_factor: f64) -> f64 {
    base_score * decay_factor
}

/// Determine the appropriate decay rate for a memory block type
pub fn decay_rate_for_block_type(block_type: swell_core::MemoryBlockType) -> DecayRate {
    match block_type {
        // Project and Convention memories decay slowly (procedural knowledge)
        swell_core::MemoryBlockType::Project => DecayRate::Procedural,
        swell_core::MemoryBlockType::Convention => DecayRate::Procedural,
        swell_core::MemoryBlockType::Skill => DecayRate::Procedural,
        // User and Task memories decay at medium rate (environmental context)
        swell_core::MemoryBlockType::User => DecayRate::Environmental,
        swell_core::MemoryBlockType::Task => DecayRate::Environmental,
    }
}

/// Determine the appropriate decay rate for a procedural memory
pub fn procedural_decay_rate() -> DecayRate {
    DecayRate::Procedural
}

/// Determine the appropriate decay rate for semantic/environmental memory
pub fn environmental_decay_rate() -> DecayRate {
    DecayRate::Environmental
}

/// Determine the appropriate decay rate for buffer/recall memory
pub fn buffer_decay_rate() -> DecayRate {
    DecayRate::Buffer
}

/// Struct representing a decayed retrieval result
#[derive(Debug, Clone)]
pub struct DecayedScore {
    /// The raw retrieval score before decay
    pub raw_score: f64,
    /// The decay factor applied
    pub decay_factor: f64,
    /// Days elapsed since last reinforcement
    pub days_elapsed: f64,
    /// The final score after decay application
    pub final_score: f64,
}

impl DecayedScore {
    /// Create a new decayed score from raw score and decay calculation
    pub fn new(raw_score: f64, decay_rate: DecayRate, last_reinforcement: DateTime<Utc>) -> Self {
        let now = Utc::now();
        let days_elapsed = days_since(last_reinforcement, now);
        let decay_factor = decay_rate.base().powf(days_elapsed);
        let final_score = apply_decay(raw_score, decay_factor);

        Self {
            raw_score,
            decay_factor,
            days_elapsed,
            final_score,
        }
    }

    /// Create from raw score with explicit decay factor
    pub fn with_decay_factor(raw_score: f64, decay_factor: f64, days_elapsed: f64) -> Self {
        Self {
            raw_score,
            decay_factor,
            days_elapsed,
            final_score: apply_decay(raw_score, decay_factor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decay_rate_bases() {
        assert!((DecayRate::Procedural.base() - 0.99).abs() < 0.001);
        assert!((DecayRate::Environmental.base() - 0.95).abs() < 0.001);
        assert!((DecayRate::Buffer.base() - 0.90).abs() < 0.001);
    }

    #[test]
    fn test_calculate_decay_no_elapsed_time() {
        let now = Utc::now();
        let decay = calculate_decay(DecayRate::Procedural, now, now);
        assert!((decay - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_calculate_decay_one_day_procedural() {
        let now = Utc::now();
        let yesterday = now - Duration::days(1);
        let decay = calculate_decay(DecayRate::Procedural, yesterday, now);
        // Should be 0.99^1 = 0.99
        assert!((decay - 0.99).abs() < 0.001);
    }

    #[test]
    fn test_calculate_decay_one_day_environmental() {
        let now = Utc::now();
        let yesterday = now - Duration::days(1);
        let decay = calculate_decay(DecayRate::Environmental, yesterday, now);
        // Should be 0.95^1 = 0.95
        assert!((decay - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_calculate_decay_one_day_buffer() {
        let now = Utc::now();
        let yesterday = now - Duration::days(1);
        let decay = calculate_decay(DecayRate::Buffer, yesterday, now);
        // Should be 0.90^1 = 0.90
        assert!((decay - 0.90).abs() < 0.001);
    }

    #[test]
    fn test_calculate_decay_seven_days_procedural() {
        let now = Utc::now();
        let week_ago = now - Duration::days(7);
        let decay = calculate_decay(DecayRate::Procedural, week_ago, now);
        // Should be 0.99^7 ≈ 0.932
        assert!((decay - 0.932).abs() < 0.01);
    }

    #[test]
    fn test_calculate_decay_seven_days_buffer() {
        let now = Utc::now();
        let week_ago = now - Duration::days(7);
        let decay = calculate_decay(DecayRate::Buffer, week_ago, now);
        // Should be 0.90^7 ≈ 0.478
        assert!((decay - 0.478).abs() < 0.01);
    }

    #[test]
    fn test_calculate_decay_thirty_days() {
        let now = Utc::now();
        let month_ago = now - Duration::days(30);

        let procedural = calculate_decay(DecayRate::Procedural, month_ago, now);
        let environmental = calculate_decay(DecayRate::Environmental, month_ago, now);
        let buffer = calculate_decay(DecayRate::Buffer, month_ago, now);

        // 0.99^30 ≈ 0.740
        assert!((procedural - 0.740).abs() < 0.01);
        // 0.95^30 ≈ 0.215
        assert!((environmental - 0.215).abs() < 0.01);
        // 0.90^30 ≈ 0.042
        assert!((buffer - 0.042).abs() < 0.01);
    }

    #[test]
    fn test_calculate_decay_preserves_order() {
        let now = Utc::now();
        let day_ago = now - Duration::days(1);

        let procedural = calculate_decay(DecayRate::Procedural, day_ago, now);
        let environmental = calculate_decay(DecayRate::Environmental, day_ago, now);
        let buffer = calculate_decay(DecayRate::Buffer, day_ago, now);

        // Procedural decays slowest, buffer fastest
        assert!(procedural > environmental);
        assert!(environmental > buffer);
    }

    #[test]
    fn test_apply_decay() {
        let base_score = 0.8;
        let decay_factor = 0.5;
        let final_score = apply_decay(base_score, decay_factor);
        assert!((final_score - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_apply_decay_zero() {
        let base_score = 0.8;
        let decay_factor = 0.0;
        let final_score = apply_decay(base_score, decay_factor);
        assert!((final_score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_apply_decay_one() {
        let base_score = 0.8;
        let decay_factor = 1.0;
        let final_score = apply_decay(base_score, decay_factor);
        assert!((final_score - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_days_since() {
        let now = Utc::now();
        let day_ago = now - Duration::days(5);
        let days = days_since(day_ago, now);
        assert!((days - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_decay_rate_for_block_type() {
        use swell_core::MemoryBlockType;

        assert_eq!(
            decay_rate_for_block_type(MemoryBlockType::Project),
            DecayRate::Procedural
        );
        assert_eq!(
            decay_rate_for_block_type(MemoryBlockType::Convention),
            DecayRate::Procedural
        );
        assert_eq!(
            decay_rate_for_block_type(MemoryBlockType::Skill),
            DecayRate::Procedural
        );
        assert_eq!(
            decay_rate_for_block_type(MemoryBlockType::User),
            DecayRate::Environmental
        );
        assert_eq!(
            decay_rate_for_block_type(MemoryBlockType::Task),
            DecayRate::Environmental
        );
    }

    #[test]
    fn test_decayed_score_new() {
        let raw_score = 0.9;
        let now = Utc::now();
        let day_ago = now - Duration::days(1);

        let decayed = DecayedScore::new(raw_score, DecayRate::Procedural, day_ago);

        assert!((decayed.raw_score - 0.9).abs() < 0.001);
        assert!((decayed.days_elapsed - 1.0).abs() < 0.1);
        assert!((decayed.decay_factor - 0.99).abs() < 0.01);
        assert!((decayed.final_score - 0.891).abs() < 0.01);
    }

    #[test]
    fn test_decayed_score_with_decay_factor() {
        let decayed = DecayedScore::with_decay_factor(0.8, 0.5, 3.0);
        assert!((decayed.raw_score - 0.8).abs() < 0.001);
        assert!((decayed.decay_factor - 0.5).abs() < 0.001);
        assert!((decayed.days_elapsed - 3.0).abs() < 0.001);
        assert!((decayed.final_score - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_large_time_difference_clamped() {
        let now = Utc::now();
        let years_ago = now - Duration::days(500);
        let decay = calculate_decay(DecayRate::Buffer, years_ago, now);
        // Should be clamped to a minimal value
        assert!(decay < 0.01);
    }

    #[test]
    fn test_decay_ordering_by_rate_type() {
        let now = Utc::now();
        let week_ago = now - Duration::days(7);

        let procedural = calculate_decay(DecayRate::Procedural, week_ago, now);
        let environmental = calculate_decay(DecayRate::Environmental, week_ago, now);
        let buffer = calculate_decay(DecayRate::Buffer, week_ago, now);

        // Procedural slowest → highest score after time
        assert!(procedural > environmental);
        assert!(environmental > buffer);
    }

    #[test]
    fn test_decayed_score_preserves_ordering() {
        let now = Utc::now();
        let day_ago = now - Duration::days(1);

        let procedural = DecayedScore::new(0.8, DecayRate::Procedural, day_ago);
        let buffer = DecayedScore::new(0.8, DecayRate::Buffer, day_ago);

        assert!(procedural.final_score > buffer.final_score);
    }

    // =============================================================================
    // Bayesian Confidence Decay Tests (mem-confidence-decay feature)
    // =============================================================================

    #[test]
    fn test_decay_rate_lambda_values() {
        // Verify lambda values match the specification
        // Procedural: λ=0.01, Environmental: λ=1.0, Buffer: λ=5.0
        assert!((DecayRate::Procedural.lambda() - 0.01).abs() < 0.0001);
        assert!((DecayRate::Environmental.lambda() - 1.0).abs() < 0.0001);
        assert!((DecayRate::Buffer.lambda() - 5.0).abs() < 0.0001);
    }

    #[test]
    fn test_bayesian_confidence_no_elapsed_time() {
        let now = Utc::now();
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Procedural, now, now);
        // No time elapsed → no decay
        assert!((confidence - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_bayesian_confidence_decay_formula_procedural() {
        // c(t) = c₀ × e^(-λt)
        // For procedural with λ=0.01 and 1 time unit: c(1) = 1.0 × e^(-0.01×1) ≈ 0.99005
        let now = Utc::now();
        let one_time_unit_ago = now - Duration::seconds(86400); // 1 day = 1 time unit
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Procedural, one_time_unit_ago, now);
        let expected = (-0.01_f64).exp(); // e^(-0.01) ≈ 0.99005
        assert!(
            (confidence - expected).abs() < 0.0001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_decay_formula_buffer() {
        // c(t) = c₀ × e^(-λt)
        // For buffer with λ=5.0 and 1 time unit: c(1) = 1.0 × e^(-5.0×1) ≈ 0.00674
        let now = Utc::now();
        let one_time_unit_ago = now - Duration::seconds(86400); // 1 day = 1 time unit
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Buffer, one_time_unit_ago, now);
        let expected = (-5.0_f64).exp(); // e^(-5.0) ≈ 0.00674
        assert!(
            (confidence - expected).abs() < 0.001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_retains_99_percent_procedural() {
        // Procedural memories should retain ~99% confidence after 1 time unit
        // c(1) = 1.0 × e^(-0.01×1) ≈ 0.99005
        let now = Utc::now();
        let one_time_unit_ago = now - Duration::seconds(86400);
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Procedural, one_time_unit_ago, now);
        assert!(
            (confidence - 0.99005).abs() < 0.001,
            "Procedural should retain ~99% after 1 time unit, got {:.4}",
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_retains_07_percent_buffer() {
        // Buffer memories should retain ~0.7% confidence after 1 time unit
        // c(1) = 1.0 × e^(-5.0×1) ≈ 0.00674 (0.674%)
        let now = Utc::now();
        let one_time_unit_ago = now - Duration::seconds(86400);
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Buffer, one_time_unit_ago, now);
        assert!(
            confidence < 0.01 && confidence > 0.005,
            "Buffer should retain ~0.7% after 1 time unit, got {:.4}",
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_with_time_unit_procedural() {
        // Use hour-based time unit for more granular testing
        // Procedural with 1 hour time unit: c(1 hour) = 1.0 × e^(-0.01×1) ≈ 0.99005
        let now = Utc::now();
        let one_hour_ago = now - Duration::seconds(3600);
        let confidence = bayesian_confidence_decay_with_time_unit(
            1.0,
            DecayRate::Procedural,
            one_hour_ago,
            now,
            1.0, // 1 hour = 1 time unit
        );
        let expected = (-0.01_f64).exp();
        assert!(
            (confidence - expected).abs() < 0.001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_with_time_unit_buffer() {
        // Buffer with 1 hour time unit: c(1 hour) = 1.0 × e^(-5.0×1) ≈ 0.00674
        let now = Utc::now();
        let one_hour_ago = now - Duration::seconds(3600);
        let confidence = bayesian_confidence_decay_with_time_unit(
            1.0,
            DecayRate::Buffer,
            one_hour_ago,
            now,
            1.0, // 1 hour = 1 time unit
        );
        let expected = (-5.0_f64).exp();
        assert!(
            (confidence - expected).abs() < 0.001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_never_exceeds_initial() {
        // Confidence should never exceed the initial confidence
        let now = Utc::now();
        let initial_confidence = 0.75;
        let past = now - Duration::days(365); // Large time gap
        let confidence = bayesian_confidence_decay(initial_confidence, DecayRate::Buffer, past, now);
        assert!(
            confidence <= initial_confidence,
            "Confidence {:.6} should not exceed initial {:.6}",
            confidence,
            initial_confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_decay_ordering() {
        // Higher decay rates should result in lower confidence after same time
        let now = Utc::now();
        let past = now - Duration::days(1);

        let procedural_confidence = bayesian_confidence_decay(1.0, DecayRate::Procedural, past, now);
        let environmental_confidence = bayesian_confidence_decay(1.0, DecayRate::Environmental, past, now);
        let buffer_confidence = bayesian_confidence_decay(1.0, DecayRate::Buffer, past, now);

        assert!(
            procedural_confidence > environmental_confidence,
            "Procedural {:.6} should be > Environmental {:.6}",
            procedural_confidence,
            environmental_confidence
        );
        assert!(
            environmental_confidence > buffer_confidence,
            "Environmental {:.6} should be > Buffer {:.6}",
            environmental_confidence,
            buffer_confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_with_initial_less_than_one() {
        // Test with initial confidence less than 1.0
        let now = Utc::now();
        let past = now - Duration::days(1);
        let initial = 0.5;

        let confidence = bayesian_confidence_decay(initial, DecayRate::Procedural, past, now);
        let expected = 0.5 * (-0.01_f64).exp(); // 0.5 * 0.99005 ≈ 0.495
        assert!(
            (confidence - expected).abs() < 0.001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }

    #[test]
    fn test_bayesian_confidence_environmental_decay() {
        // Environmental with λ=1.0: c(1) = 1.0 × e^(-1.0×1) ≈ 0.3679
        let now = Utc::now();
        let one_time_unit_ago = now - Duration::seconds(86400);
        let confidence = bayesian_confidence_decay(1.0, DecayRate::Environmental, one_time_unit_ago, now);
        let expected = (-1.0_f64).exp(); // e^(-1.0) ≈ 0.3679
        assert!(
            (confidence - expected).abs() < 0.001,
            "Expected {:.6}, got {:.6}",
            expected,
            confidence
        );
    }
}
