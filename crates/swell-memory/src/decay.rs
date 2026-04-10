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
}
