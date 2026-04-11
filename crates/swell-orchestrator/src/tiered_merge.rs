//! Risk-tiered merge module.
//!
//! Implements three-tier merge strategy based on task risk level:
//! - Low-risk: auto-merge
//! - Medium-risk: auto-merge with AI review
//! - High-risk: human review required

use serde::{Deserialize, Serialize};
use std::fmt;
use swell_core::{RiskLevel, TaskState};

/// Merge strategy based on risk tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStrategy {
    /// Auto-merge without any review
    AutoMerge,
    /// Auto-merge after AI review passes
    AutoMergeWithAiReview,
    /// Human review required before merge
    HumanReview,
}

impl fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeStrategy::AutoMerge => write!(f, "AutoMerge"),
            MergeStrategy::AutoMergeWithAiReview => write!(f, "AutoMergeWithAiReview"),
            MergeStrategy::HumanReview => write!(f, "HumanReview"),
        }
    }
}

/// Result of merge eligibility check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeEligibility {
    /// Recommended merge strategy
    pub strategy: MergeStrategy,
    /// Whether merge is allowed
    pub can_merge: bool,
    /// Reason for the decision
    pub reason: String,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
}

impl MergeEligibility {
    /// Create an eligible merge result
    pub fn eligible(strategy: MergeStrategy, reason: impl Into<String>, confidence: f64) -> Self {
        Self {
            strategy,
            can_merge: true,
            reason: reason.into(),
            confidence,
        }
    }

    /// Create an ineligible merge result
    pub fn ineligible(reason: impl Into<String>) -> Self {
        Self {
            strategy: MergeStrategy::HumanReview,
            can_merge: false,
            reason: reason.into(),
            confidence: 0.0,
        }
    }
}

/// Determines merge strategy based on task risk level and evaluation result
pub struct TieredMerge;

impl TieredMerge {
    /// Determine merge eligibility based on plan risk level and confidence score
    ///
    /// # Arguments
    /// * `plan_risk` - Risk level of the task plan
    /// * `confidence_score` - Evaluation confidence score (0.0 to 1.0)
    ///
    /// # Returns
    /// `MergeEligibility` with recommended strategy and whether merge is allowed
    pub fn evaluate(plan_risk: RiskLevel, confidence_score: f64) -> MergeEligibility {
        match plan_risk {
            RiskLevel::Low => Self::evaluate_low_risk(confidence_score),
            RiskLevel::Medium => Self::evaluate_medium_risk(confidence_score),
            RiskLevel::High => Self::evaluate_high_risk(confidence_score),
        }
    }

    /// Low-risk: auto-merge if confidence >= 0.7, otherwise human review
    fn evaluate_low_risk(confidence: f64) -> MergeEligibility {
        if confidence >= 0.7 {
            MergeEligibility::eligible(
                MergeStrategy::AutoMerge,
                "Low-risk task with high confidence - auto-merge enabled",
                confidence,
            )
        } else {
            MergeEligibility::eligible(
                MergeStrategy::HumanReview,
                "Low-risk task but confidence below threshold (0.7) - human review recommended",
                confidence,
            )
        }
    }

    /// Medium-risk: auto-merge with AI review if confidence >= 0.85, otherwise human review
    fn evaluate_medium_risk(confidence: f64) -> MergeEligibility {
        if confidence >= 0.85 {
            MergeEligibility::eligible(
                MergeStrategy::AutoMergeWithAiReview,
                "Medium-risk task with high confidence - auto-merge with AI review",
                confidence,
            )
        } else {
            MergeEligibility::eligible(
                MergeStrategy::HumanReview,
                "Medium-risk task with confidence below threshold (0.85) - human review required",
                confidence,
            )
        }
    }

    /// High-risk: always human review required
    fn evaluate_high_risk(_confidence: f64) -> MergeEligibility {
        MergeEligibility::eligible(
            MergeStrategy::HumanReview,
            "High-risk task - human review always required",
            0.0,
        )
    }

    /// Check if a task state indicates merge readiness
    pub fn is_mergeable_state(state: TaskState) -> bool {
        matches!(state, TaskState::Accepted)
    }

    /// Get the required review type for a given merge strategy
    pub fn required_review_type(strategy: MergeStrategy) -> Option<&'static str> {
        match strategy {
            MergeStrategy::AutoMerge => None,
            MergeStrategy::AutoMergeWithAiReview => Some("ai_review"),
            MergeStrategy::HumanReview => Some("human_review"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- MergeStrategy Tests ---

    #[test]
    fn test_merge_strategy_display() {
        assert_eq!(format!("{}", MergeStrategy::AutoMerge), "AutoMerge");
        assert_eq!(format!("{}", MergeStrategy::AutoMergeWithAiReview), "AutoMergeWithAiReview");
        assert_eq!(format!("{}", MergeStrategy::HumanReview), "HumanReview");
    }

    // --- TieredMerge::evaluate Tests ---

    #[test]
    fn test_low_risk_high_confidence() {
        // Confidence >= 0.7 should allow auto-merge
        let result = TieredMerge::evaluate(RiskLevel::Low, 0.85);
        assert_eq!(result.strategy, MergeStrategy::AutoMerge);
        assert!(result.can_merge);
        assert!(result.confidence >= 0.7);
    }

    #[test]
    fn test_low_risk_low_confidence() {
        // Confidence < 0.7 should recommend human review
        let result = TieredMerge::evaluate(RiskLevel::Low, 0.5);
        assert_eq!(result.strategy, MergeStrategy::HumanReview);
        assert!(result.can_merge); // Still mergeable but needs human review
        assert!(result.confidence < 0.7);
    }

    #[test]
    fn test_low_risk_boundary_70() {
        // Exactly at 0.7 threshold should auto-merge
        let result = TieredMerge::evaluate(RiskLevel::Low, 0.7);
        assert_eq!(result.strategy, MergeStrategy::AutoMerge);
        assert!(result.can_merge);
    }

    #[test]
    fn test_low_risk_just_below_70() {
        // Just below 0.7 threshold should need human review
        let result = TieredMerge::evaluate(RiskLevel::Low, 0.69);
        assert_eq!(result.strategy, MergeStrategy::HumanReview);
    }

    #[test]
    fn test_medium_risk_high_confidence() {
        // Confidence >= 0.85 should allow auto-merge with AI review
        let result = TieredMerge::evaluate(RiskLevel::Medium, 0.9);
        assert_eq!(result.strategy, MergeStrategy::AutoMergeWithAiReview);
        assert!(result.can_merge);
    }

    #[test]
    fn test_medium_risk_low_confidence() {
        // Confidence < 0.85 should require human review
        let result = TieredMerge::evaluate(RiskLevel::Medium, 0.7);
        assert_eq!(result.strategy, MergeStrategy::HumanReview);
        assert!(result.can_merge);
    }

    #[test]
    fn test_medium_risk_boundary_85() {
        // Exactly at 0.85 threshold
        let result = TieredMerge::evaluate(RiskLevel::Medium, 0.85);
        assert_eq!(result.strategy, MergeStrategy::AutoMergeWithAiReview);
    }

    #[test]
    fn test_medium_risk_just_below_85() {
        // Just below 0.85 threshold
        let result = TieredMerge::evaluate(RiskLevel::Medium, 0.84);
        assert_eq!(result.strategy, MergeStrategy::HumanReview);
    }

    #[test]
    fn test_high_risk_always_human_review() {
        // High risk always requires human review regardless of confidence
        let result = TieredMerge::evaluate(RiskLevel::High, 0.95);
        assert_eq!(result.strategy, MergeStrategy::HumanReview);
        assert!(result.can_merge);
        assert_eq!(result.confidence, 0.0); // Confidence is zeroed for high-risk
    }

    #[test]
    fn test_high_risk_any_confidence() {
        let result1 = TieredMerge::evaluate(RiskLevel::High, 1.0);
        let result2 = TieredMerge::evaluate(RiskLevel::High, 0.5);
        let result3 = TieredMerge::evaluate(RiskLevel::High, 0.0);

        for result in [result1, result2, result3] {
            assert_eq!(result.strategy, MergeStrategy::HumanReview);
            assert!(result.can_merge);
            assert_eq!(result.confidence, 0.0);
        }
    }

    // --- MergeEligibility Tests ---

    #[test]
    fn test_merge_eligibility_eligible() {
        let eligibility = MergeEligibility::eligible(
            MergeStrategy::AutoMerge,
            "All checks passed",
            0.95,
        );

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::AutoMerge);
        assert_eq!(eligibility.reason, "All checks passed");
        assert_eq!(eligibility.confidence, 0.95);
    }

    #[test]
    fn test_merge_eligibility_ineligible() {
        let eligibility = MergeEligibility::ineligible("Validation failed");

        assert!(!eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::HumanReview);
        assert_eq!(eligibility.reason, "Validation failed");
        assert_eq!(eligibility.confidence, 0.0);
    }

    // --- is_mergeable_state Tests ---

    #[test]
    fn test_is_mergeable_state_accepted() {
        assert!(TieredMerge::is_mergeable_state(TaskState::Accepted));
    }

    #[test]
    fn test_is_mergeable_state_rejected() {
        assert!(!TieredMerge::is_mergeable_state(TaskState::Rejected));
    }

    #[test]
    fn test_is_mergeable_state_executing() {
        assert!(!TieredMerge::is_mergeable_state(TaskState::Executing));
    }

    #[test]
    fn test_is_mergeable_state_validating() {
        assert!(!TieredMerge::is_mergeable_state(TaskState::Validating));
    }

    #[test]
    fn test_is_mergeable_state_created() {
        assert!(!TieredMerge::is_mergeable_state(TaskState::Created));
    }

    #[test]
    fn test_is_mergeable_state_escalated() {
        assert!(!TieredMerge::is_mergeable_state(TaskState::Escalated));
    }

    // --- required_review_type Tests ---

    #[test]
    fn test_required_review_type_auto_merge() {
        assert_eq!(TieredMerge::required_review_type(MergeStrategy::AutoMerge), None);
    }

    #[test]
    fn test_required_review_type_ai_review() {
        assert_eq!(TieredMerge::required_review_type(MergeStrategy::AutoMergeWithAiReview), Some("ai_review"));
    }

    #[test]
    fn test_required_review_type_human() {
        assert_eq!(TieredMerge::required_review_type(MergeStrategy::HumanReview), Some("human_review"));
    }

    // --- Integration Tests ---

    #[test]
    fn test_full_workflow_low_risk_auto_merge() {
        // Simulate low-risk task with high confidence
        let plan_risk = RiskLevel::Low;
        let confidence = 0.9;

        let eligibility = TieredMerge::evaluate(plan_risk, confidence);

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::AutoMerge);
        assert!(TieredMerge::required_review_type(eligibility.strategy).is_none());
    }

    #[test]
    fn test_full_workflow_medium_risk_with_ai_review() {
        // Simulate medium-risk task with high confidence
        let plan_risk = RiskLevel::Medium;
        let confidence = 0.92;

        let eligibility = TieredMerge::evaluate(plan_risk, confidence);

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::AutoMergeWithAiReview);
        assert_eq!(TieredMerge::required_review_type(eligibility.strategy), Some("ai_review"));
    }

    #[test]
    fn test_full_workflow_high_risk_human_review() {
        // Simulate high-risk task
        let plan_risk = RiskLevel::High;
        let confidence = 0.95;

        let eligibility = TieredMerge::evaluate(plan_risk, confidence);

        assert!(eligibility.can_merge);
        assert_eq!(eligibility.strategy, MergeStrategy::HumanReview);
        assert_eq!(TieredMerge::required_review_type(eligibility.strategy), Some("human_review"));
    }
}
