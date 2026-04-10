// deprecation.rs - Memory deprecation with superseded_by links
//
// This module provides functionality to mark memories as deprecated when their
// confidence drops below 0.3, linking them to replacement knowledge via the
// superseded_by field.
//
// Deprecated memories remain queryable but are marked with their deprecated status
// to prevent usage while preserving the knowledge that they were superseded.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Deprecation confidence threshold - memories with confidence below this are deprecated
pub const DEPRECATION_CONFIDENCE_THRESHOLD: f64 = 0.3;

/// Reason for deprecation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeprecationReason {
    /// Confidence dropped below threshold due to failures
    ConfidenceDropped {
        previous_confidence: f64,
        current_confidence: f64,
    },
    /// Memory was explicitly superseded by another
    ExplicitlySuperseded {
        replacement_id: Uuid,
        reason: String,
    },
    /// Memory is outdated and no longer relevant
    Outdated {
        last_updated: DateTime<Utc>,
        reason: String,
    },
    /// Memory was found to be incorrect
    Incorrect { correction_id: Uuid, reason: String },
}

impl DeprecationReason {
    /// Get a human-readable description of the deprecation reason
    pub fn description(&self) -> String {
        match self {
            DeprecationReason::ConfidenceDropped {
                previous_confidence,
                current_confidence,
            } => format!(
                "Confidence dropped from {:.2} to {:.2} (threshold: {:.2})",
                previous_confidence, current_confidence, DEPRECATION_CONFIDENCE_THRESHOLD
            ),
            DeprecationReason::ExplicitlySuperseded { reason, .. } => {
                format!("Explicitly superseded: {}", reason)
            }
            DeprecationReason::Outdated { reason, .. } => {
                format!("Outdated: {}", reason)
            }
            DeprecationReason::Incorrect { reason, .. } => {
                format!("Found to be incorrect: {}", reason)
            }
        }
    }
}

/// Information about a deprecated memory
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeprecationInfo {
    /// Whether this memory is deprecated
    pub is_deprecated: bool,
    /// UUID of the memory that supersedes this one (if any)
    pub superseded_by: Option<Uuid>,
    /// Why this memory was deprecated
    pub deprecation_reason: Option<DeprecationReason>,
    /// When this memory was deprecated
    pub deprecated_at: Option<DateTime<Utc>>,
}

impl DeprecationInfo {
    /// Create a new deprecation info as non-deprecated
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark this memory as deprecated due to confidence drop
    pub fn mark_deprecated(&mut self, previous_confidence: f64, current_confidence: f64) {
        self.is_deprecated = true;
        self.deprecated_at = Some(Utc::now());
        self.deprecation_reason = Some(DeprecationReason::ConfidenceDropped {
            previous_confidence,
            current_confidence,
        });
    }

    /// Mark this memory as superseded by another memory
    pub fn mark_superseded(&mut self, replacement_id: Uuid, reason: String) {
        self.is_deprecated = true;
        self.deprecated_at = Some(Utc::now());
        self.deprecation_reason = Some(DeprecationReason::ExplicitlySuperseded {
            replacement_id,
            reason,
        });
        self.superseded_by = Some(replacement_id);
    }

    /// Mark this memory as outdated
    pub fn mark_outdated(&mut self, last_updated: DateTime<Utc>, reason: String) {
        self.is_deprecated = true;
        self.deprecated_at = Some(Utc::now());
        self.deprecation_reason = Some(DeprecationReason::Outdated {
            last_updated,
            reason,
        });
    }

    /// Mark this memory as incorrect with a correction
    pub fn mark_incorrect(&mut self, correction_id: Uuid, reason: String) {
        self.is_deprecated = true;
        self.deprecated_at = Some(Utc::now());
        self.deprecation_reason = Some(DeprecationReason::Incorrect {
            correction_id,
            reason,
        });
        self.superseded_by = Some(correction_id);
    }

    /// Reactivate a deprecated memory (if it was deprecated incorrectly)
    pub fn reactivate(&mut self) {
        self.is_deprecated = false;
        self.deprecated_at = None;
        self.deprecation_reason = None;
        self.superseded_by = None;
    }

    /// Check if this memory has a replacement available
    pub fn has_replacement(&self) -> bool {
        self.superseded_by.is_some()
    }

    /// Get the replacement ID if available
    pub fn get_replacement_id(&self) -> Option<Uuid> {
        self.superseded_by
    }
}

/// Check if a confidence value indicates deprecation (below threshold)
pub fn should_be_deprecated(confidence: f64) -> bool {
    confidence < DEPRECATION_CONFIDENCE_THRESHOLD
}

/// Calculate the deprecation score (how strongly deprecated)
/// Returns a value from 0.0 to 1.0 where 1.0 is most deprecated
pub fn deprecation_score(confidence: f64) -> f64 {
    if confidence >= DEPRECATION_CONFIDENCE_THRESHOLD {
        return 0.0;
    }
    // Linear interpolation: at 0.0 confidence = max deprecation (1.0)
    // at 0.3 threshold = no deprecation (0.0)
    (DEPRECATION_CONFIDENCE_THRESHOLD - confidence) / DEPRECATION_CONFIDENCE_THRESHOLD
}

/// Extension trait for memory types that can be deprecated
pub trait Deprecatable {
    /// Get the current confidence level
    fn get_confidence(&self) -> f64;

    /// Check if this item should be deprecated
    fn should_deprecate(&self) -> bool {
        should_be_deprecated(self.get_confidence())
    }
}

/// Result of a deprecation check
#[derive(Debug, Clone)]
pub struct DeprecationCheckResult {
    /// Whether the memory is deprecated
    pub is_deprecated: bool,
    /// Whether the memory should be deprecated based on confidence
    pub should_be_deprecated: bool,
    /// Current confidence level
    pub current_confidence: f64,
    /// Deprecation info if deprecated
    pub deprecation_info: Option<DeprecationInfo>,
    /// Recommendation for action
    pub recommendation: DeprecationRecommendation,
}

/// Recommendation for handling deprecated memory
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeprecationRecommendation {
    /// Memory is fine to use
    Use,
    /// Memory is deprecated but has a replacement - use the replacement instead
    UseReplacement,
    /// Memory is deprecated without replacement - use with caution
    UseWithCaution,
    /// Memory is deprecated and should not be used
    DoNotUse,
}

impl std::fmt::Display for DeprecationRecommendation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeprecationRecommendation::Use => write!(f, "use"),
            DeprecationRecommendation::UseReplacement => write!(f, "use_replacement"),
            DeprecationRecommendation::UseWithCaution => write!(f, "use_with_caution"),
            DeprecationRecommendation::DoNotUse => write!(f, "do_not_use"),
        }
    }
}

impl DeprecationCheckResult {
    /// Create a result for a non-deprecated memory with high confidence
    pub fn healthy(confidence: f64) -> Self {
        Self {
            is_deprecated: false,
            should_be_deprecated: false,
            current_confidence: confidence,
            deprecation_info: None,
            recommendation: DeprecationRecommendation::Use,
        }
    }

    /// Create a result for a deprecated memory without replacement
    pub fn deprecated_no_replacement(confidence: f64, previous_confidence: f64) -> Self {
        let info = DeprecationInfo {
            is_deprecated: true,
            superseded_by: None,
            deprecation_reason: Some(DeprecationReason::ConfidenceDropped {
                previous_confidence,
                current_confidence: confidence,
            }),
            deprecated_at: Some(Utc::now()),
        };

        Self {
            is_deprecated: true,
            should_be_deprecated: true,
            current_confidence: confidence,
            deprecation_info: Some(info),
            recommendation: DeprecationRecommendation::UseWithCaution,
        }
    }

    /// Create a result for a deprecated memory with replacement
    pub fn deprecated_with_replacement(
        confidence: f64,
        replacement_id: Uuid,
        previous_confidence: f64,
    ) -> Self {
        let info = DeprecationInfo {
            is_deprecated: true,
            superseded_by: Some(replacement_id),
            deprecation_reason: Some(DeprecationReason::ExplicitlySuperseded {
                replacement_id,
                reason: format!(
                    "Confidence dropped from {:.2} to {:.2} (threshold: {:.2})",
                    previous_confidence, confidence, DEPRECATION_CONFIDENCE_THRESHOLD
                ),
            }),
            deprecated_at: Some(Utc::now()),
        };

        Self {
            is_deprecated: true,
            should_be_deprecated: true,
            current_confidence: confidence,
            deprecation_info: Some(info),
            recommendation: DeprecationRecommendation::UseReplacement,
        }
    }
}

/// Check deprecation status and return recommendation
pub fn check_deprecation(
    confidence: f64,
    deprecation_info: Option<&DeprecationInfo>,
) -> DeprecationCheckResult {
    let currently_deprecated = deprecation_info.map(|d| d.is_deprecated).unwrap_or(false);

    let should_be = should_be_deprecated(confidence);

    let recommendation = if !currently_deprecated && !should_be {
        DeprecationCheckResult::healthy(confidence).recommendation
    } else if currently_deprecated {
        if deprecation_info.and_then(|d| d.superseded_by).is_some() {
            DeprecationRecommendation::UseReplacement
        } else {
            DeprecationRecommendation::UseWithCaution
        }
    } else {
        // Should be deprecated but isn't marked yet
        DeprecationRecommendation::DoNotUse
    };

    DeprecationCheckResult {
        is_deprecated: currently_deprecated,
        should_be_deprecated: should_be,
        current_confidence: confidence,
        deprecation_info: deprecation_info.cloned(),
        recommendation,
    }
}

/// Apply deprecation to a memory based on confidence
pub fn apply_confidence_deprecation(
    current_confidence: f64,
    previous_confidence: Option<f64>,
) -> Option<DeprecationInfo> {
    if should_be_deprecated(current_confidence) {
        let prev = previous_confidence.unwrap_or(current_confidence);
        let mut info = DeprecationInfo::default();
        info.mark_deprecated(prev, current_confidence);
        Some(info)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deprecation_threshold_constant() {
        assert_eq!(DEPRECATION_CONFIDENCE_THRESHOLD, 0.3);
    }

    #[test]
    fn test_should_be_deprecated() {
        // Below threshold
        assert!(should_be_deprecated(0.0));
        assert!(should_be_deprecated(0.1));
        assert!(should_be_deprecated(0.29));

        // At threshold - not deprecated
        assert!(!should_be_deprecated(0.3));
        assert!(!should_be_deprecated(0.5));
        assert!(!should_be_deprecated(1.0));
    }

    #[test]
    fn test_deprecation_score() {
        // At 0.0 confidence - max deprecation
        assert!((deprecation_score(0.0) - 1.0).abs() < 0.001);

        // At 0.15 confidence - halfway
        assert!((deprecation_score(0.15) - 0.5).abs() < 0.001);

        // At threshold - no deprecation
        assert!((deprecation_score(0.3) - 0.0).abs() < 0.001);

        // Above threshold - no deprecation
        assert!((deprecation_score(0.5) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_deprecation_info_default() {
        let info = DeprecationInfo::default();
        assert!(!info.is_deprecated);
        assert!(info.superseded_by.is_none());
        assert!(info.deprecation_reason.is_none());
        assert!(info.deprecated_at.is_none());
    }

    #[test]
    fn test_deprecation_info_mark_deprecated() {
        let mut info = DeprecationInfo::new();
        info.mark_deprecated(0.5, 0.2);

        assert!(info.is_deprecated);
        assert!(info.deprecated_at.is_some());
        assert!(info.superseded_by.is_none());

        if let Some(DeprecationReason::ConfidenceDropped {
            previous_confidence,
            current_confidence,
        }) = info.deprecation_reason
        {
            assert!((previous_confidence - 0.5).abs() < 0.001);
            assert!((current_confidence - 0.2).abs() < 0.001);
        } else {
            panic!("Expected ConfidenceDropped reason");
        }
    }

    #[test]
    fn test_deprecation_info_mark_superseded() {
        let mut info = DeprecationInfo::new();
        let replacement_id = Uuid::new_v4();
        info.mark_superseded(
            replacement_id,
            "Better implementation available".to_string(),
        );

        assert!(info.is_deprecated);
        assert!(info.superseded_by.is_some());
        assert_eq!(info.superseded_by, Some(replacement_id));

        if let Some(DeprecationReason::ExplicitlySuperseded { reason, .. }) =
            &info.deprecation_reason
        {
            assert_eq!(reason, "Better implementation available");
        } else {
            panic!("Expected ExplicitlySuperseded reason");
        }
    }

    #[test]
    fn test_deprecation_info_reactivate() {
        let mut info = DeprecationInfo::new();
        info.mark_deprecated(0.5, 0.2);
        assert!(info.is_deprecated);

        info.reactivate();
        assert!(!info.is_deprecated);
        assert!(info.superseded_by.is_none());
        assert!(info.deprecation_reason.is_none());
    }

    #[test]
    fn test_deprecation_reason_description() {
        let reason = DeprecationReason::ConfidenceDropped {
            previous_confidence: 0.5,
            current_confidence: 0.2,
        };
        assert!(reason.description().contains("0.50"));
        assert!(reason.description().contains("0.20"));

        let reason2 = DeprecationReason::ExplicitlySuperseded {
            replacement_id: Uuid::new_v4(),
            reason: "New version".to_string(),
        };
        assert!(reason2.description().contains("New version"));
    }

    #[test]
    fn test_deprecation_check_result_healthy() {
        let result = DeprecationCheckResult::healthy(0.8);
        assert!(!result.is_deprecated);
        assert!(!result.should_be_deprecated);
        assert_eq!(result.recommendation, DeprecationRecommendation::Use);
    }

    #[test]
    fn test_deprecation_check_result_deprecated_no_replacement() {
        let result = DeprecationCheckResult::deprecated_no_replacement(0.2, 0.5);
        assert!(result.is_deprecated);
        assert!(result.should_be_deprecated);
        assert_eq!(
            result.recommendation,
            DeprecationRecommendation::UseWithCaution
        );
    }

    #[test]
    fn test_deprecation_check_result_deprecated_with_replacement() {
        let replacement_id = Uuid::new_v4();
        let result = DeprecationCheckResult::deprecated_with_replacement(0.2, replacement_id, 0.5);
        assert!(result.is_deprecated);
        assert!(result.should_be_deprecated);
        assert_eq!(
            result.recommendation,
            DeprecationRecommendation::UseReplacement
        );
        assert!(result.deprecation_info.is_some());
        assert_eq!(
            result.deprecation_info.as_ref().unwrap().superseded_by,
            Some(replacement_id)
        );
    }

    #[test]
    fn test_check_deprecation_no_info() {
        // High confidence, no deprecation info
        let result = check_deprecation(0.8, None);
        assert!(!result.is_deprecated);
        assert!(!result.should_be_deprecated);
        assert_eq!(result.recommendation, DeprecationRecommendation::Use);

        // Low confidence, no deprecation info - should be deprecated
        let result2 = check_deprecation(0.2, None);
        assert!(!result2.is_deprecated); // Not yet marked
        assert!(result2.should_be_deprecated);
        assert_eq!(result2.recommendation, DeprecationRecommendation::DoNotUse);
    }

    #[test]
    fn test_check_deprecation_with_info() {
        // Deprecated with replacement
        let mut info = DeprecationInfo::new();
        let replacement_id = Uuid::new_v4();
        info.mark_superseded(replacement_id, "Better version".to_string());

        let result = check_deprecation(0.2, Some(&info));
        assert!(result.is_deprecated);
        assert_eq!(
            result.recommendation,
            DeprecationRecommendation::UseReplacement
        );
    }

    #[test]
    fn test_apply_confidence_deprecation() {
        // Above threshold - no deprecation
        let result = apply_confidence_deprecation(0.5, Some(0.7));
        assert!(result.is_none());

        // Below threshold - should be deprecated
        let result = apply_confidence_deprecation(0.2, Some(0.5));
        assert!(result.is_some());
        let info = result.unwrap();
        assert!(info.is_deprecated);
    }

    #[test]
    fn test_apply_confidence_deprecation_no_previous() {
        let result = apply_confidence_deprecation(0.2, None);
        assert!(result.is_some());
        let info = result.unwrap();
        assert!(info.is_deprecated);

        // Previous confidence defaults to current
        if let Some(DeprecationReason::ConfidenceDropped {
            previous_confidence,
            current_confidence,
        }) = info.deprecation_reason
        {
            assert!((previous_confidence - 0.2).abs() < 0.001);
            assert!((current_confidence - 0.2).abs() < 0.001);
        }
    }

    #[test]
    fn test_deprecation_info_has_replacement() {
        let mut info = DeprecationInfo::new();
        assert!(!info.has_replacement());

        info.mark_incorrect(Uuid::new_v4(), "Found error".to_string());
        assert!(info.has_replacement());

        let mut info2 = DeprecationInfo::new();
        info2.mark_deprecated(0.5, 0.2);
        assert!(!info2.has_replacement());
    }

    #[test]
    fn test_deprecation_recommendation_display() {
        assert_eq!(format!("{}", DeprecationRecommendation::Use), "use");
        assert_eq!(
            format!("{}", DeprecationRecommendation::UseReplacement),
            "use_replacement"
        );
        assert_eq!(
            format!("{}", DeprecationRecommendation::UseWithCaution),
            "use_with_caution"
        );
        assert_eq!(
            format!("{}", DeprecationRecommendation::DoNotUse),
            "do_not_use"
        );
    }
}
