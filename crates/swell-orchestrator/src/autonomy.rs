//! Autonomy level controller for managing approval workflows.
//!
//! Implements per-task autonomy levels:
//! - L1 (Supervised): Every action requires approval
//! - L2 (Guided): Plan approval required, auto-execute (default)
//! - L3 (Autonomous): Minimal guidance, only high-risk actions need approval
//! - L4 (Full Auto): Fully autonomous, no approvals needed

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Represents an approval request for a pending action
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub task_id: Uuid,
    pub request_id: Uuid,
    pub step_id: Option<Uuid>,
    pub action_description: String,
    pub risk_level: swell_core::RiskLevel,
    pub autonomy_level: swell_core::AutonomyLevel,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ApprovalRequest {
    pub fn new(
        task_id: Uuid,
        action_description: String,
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
    ) -> Self {
        Self {
            task_id,
            request_id: Uuid::new_v4(),
            step_id: None,
            action_description,
            risk_level,
            autonomy_level,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn for_step(
        task_id: Uuid,
        step_id: Uuid,
        action_description: String,
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
    ) -> Self {
        Self {
            task_id,
            request_id: Uuid::new_v4(),
            step_id: Some(step_id),
            action_description,
            risk_level,
            autonomy_level,
            created_at: chrono::Utc::now(),
        }
    }
}

/// Result of an approval decision
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
    Pending,
}

/// Manages approval requests and decisions for autonomy levels
pub struct AutonomyController {
    pending_requests: Arc<RwLock<HashMap<Uuid, ApprovalRequest>>>,
    decisions: Arc<RwLock<HashMap<Uuid, ApprovalDecision>>>,
}

impl AutonomyController {
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if an approval is needed based on task autonomy level and risk
    pub async fn needs_approval(
        &self,
        _task_id: Uuid,
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
    ) -> bool {
        // L4 FullAuto never needs approval
        if autonomy_level == swell_core::AutonomyLevel::FullAuto {
            return false;
        }

        // L1 Supervised always needs approval
        if autonomy_level == swell_core::AutonomyLevel::Supervised {
            return true;
        }

        // L2 Guided only needs approval for plan (handled separately)
        if autonomy_level == swell_core::AutonomyLevel::Guided {
            return false; // Steps auto-execute after plan approval
        }

        // L3 Autonomous only needs approval for high-risk actions
        if autonomy_level == swell_core::AutonomyLevel::Autonomous {
            return risk_level == swell_core::RiskLevel::High;
        }

        false
    }

    /// Check if plan approval is needed based on autonomy level
    pub async fn needs_plan_approval(&self, autonomy_level: swell_core::AutonomyLevel) -> bool {
        autonomy_level.needs_plan_approval()
    }

    /// Request approval for an action
    pub async fn request_approval(&self, request: ApprovalRequest) -> Uuid {
        let request_id = request.request_id;
        let mut pending = self.pending_requests.write().await;
        pending.insert(request_id, request.clone());
        request_id
    }

    /// Approve a pending request
    pub async fn approve(&self, request_id: Uuid) -> bool {
        let mut pending = self.pending_requests.write().await;
        let mut decisions = self.decisions.write().await;

        if pending.remove(&request_id).is_some() {
            decisions.insert(request_id, ApprovalDecision::Approved);
            return true;
        }
        false
    }

    /// Reject a pending request
    pub async fn reject(&self, request_id: Uuid) -> bool {
        let mut pending = self.pending_requests.write().await;
        let mut decisions = self.decisions.write().await;

        if pending.remove(&request_id).is_some() {
            decisions.insert(request_id, ApprovalDecision::Rejected);
            return true;
        }
        false
    }

    /// Get the decision for a request
    pub async fn get_decision(&self, request_id: Uuid) -> Option<ApprovalDecision> {
        let decisions = self.decisions.read().await;
        decisions.get(&request_id).copied()
    }

    /// Check if a request is pending
    pub async fn is_pending(&self, request_id: Uuid) -> bool {
        let pending = self.pending_requests.read().await;
        pending.contains_key(&request_id)
    }

    /// Get all pending requests for a task
    pub async fn get_pending_for_task(&self, task_id: Uuid) -> Vec<ApprovalRequest> {
        let pending = self.pending_requests.read().await;
        pending
            .values()
            .filter(|r| r.task_id == task_id)
            .cloned()
            .collect()
    }

    /// Clear all pending requests (used when task is cancelled or completed)
    pub async fn clear_task_requests(&self, task_id: Uuid) {
        let mut pending = self.pending_requests.write().await;
        pending.retain(|_, r| r.task_id != task_id);
    }
}

impl Default for AutonomyController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fullauto_never_needs_approval() {
        let controller = AutonomyController::new();

        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::FullAuto
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::FullAuto
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_supervised_always_needs_approval() {
        let controller = AutonomyController::new();

        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Supervised
                )
                .await
        );
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Supervised
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_guided_steps_dont_need_approval() {
        let controller = AutonomyController::new();

        // L2 Guided: steps auto-execute after plan approval
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Guided
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Guided
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_autonomous_high_risk_needs_approval() {
        let controller = AutonomyController::new();

        // L3 Autonomous: only high-risk needs approval
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Autonomous
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Medium,
                    swell_core::AutonomyLevel::Autonomous
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Autonomous
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_needs_plan_approval() {
        let controller = AutonomyController::new();

        // L1 and L2 need plan approval
        assert!(
            controller
                .needs_plan_approval(swell_core::AutonomyLevel::Supervised)
                .await
        );
        assert!(
            controller
                .needs_plan_approval(swell_core::AutonomyLevel::Guided)
                .await
        );

        // L3 and L4 don't need plan approval
        assert!(
            !controller
                .needs_plan_approval(swell_core::AutonomyLevel::Autonomous)
                .await
        );
        assert!(
            !controller
                .needs_plan_approval(swell_core::AutonomyLevel::FullAuto)
                .await
        );
    }

    #[tokio::test]
    async fn test_approval_request_flow() {
        let controller = AutonomyController::new();
        let task_id = Uuid::new_v4();

        // Request approval
        let request = ApprovalRequest::new(
            task_id,
            "Delete file".to_string(),
            swell_core::RiskLevel::High,
            swell_core::AutonomyLevel::Autonomous,
        );
        let request_id = controller.request_approval(request).await;

        assert!(controller.is_pending(request_id).await);

        // Approve
        assert!(controller.approve(request_id).await);
        assert!(!controller.is_pending(request_id).await);
        assert_eq!(
            controller.get_decision(request_id).await,
            Some(ApprovalDecision::Approved)
        );
    }

    #[tokio::test]
    async fn test_reject_approval() {
        let controller = AutonomyController::new();
        let task_id = Uuid::new_v4();

        let request = ApprovalRequest::new(
            task_id,
            "Delete file".to_string(),
            swell_core::RiskLevel::High,
            swell_core::AutonomyLevel::Autonomous,
        );
        let request_id = controller.request_approval(request).await;

        assert!(controller.reject(request_id).await);
        assert_eq!(
            controller.get_decision(request_id).await,
            Some(ApprovalDecision::Rejected)
        );
    }

    #[tokio::test]
    async fn test_clear_task_requests() {
        let controller = AutonomyController::new();
        let task_id = Uuid::new_v4();

        // Create multiple requests for the same task
        for i in 0..3 {
            let request = ApprovalRequest::new(
                task_id,
                format!("Action {}", i),
                swell_core::RiskLevel::Low,
                swell_core::AutonomyLevel::Autonomous,
            );
            controller.request_approval(request).await;
        }

        // Create request for different task
        let other_task = Uuid::new_v4();
        let request = ApprovalRequest::new(
            other_task,
            "Other task action".to_string(),
            swell_core::RiskLevel::Low,
            swell_core::AutonomyLevel::Autonomous,
        );
        let _other_request_id = controller.request_approval(request).await;

        // Clear task requests
        controller.clear_task_requests(task_id).await;

        // Task requests should be cleared
        assert!(controller.get_pending_for_task(task_id).await.is_empty());

        // Other task requests should remain
        assert!(!controller.get_pending_for_task(other_task).await.is_empty());
    }

    #[tokio::test]
    async fn test_autonomy_level_default() {
        assert_eq!(
            swell_core::AutonomyLevel::default(),
            swell_core::AutonomyLevel::Guided
        );
    }

    #[tokio::test]
    async fn test_autonomy_level_methods() {
        // Test needs_plan_approval
        assert!(swell_core::AutonomyLevel::Supervised.needs_plan_approval());
        assert!(swell_core::AutonomyLevel::Guided.needs_plan_approval());
        assert!(!swell_core::AutonomyLevel::Autonomous.needs_plan_approval());
        assert!(!swell_core::AutonomyLevel::FullAuto.needs_plan_approval());

        // Test needs_step_approval
        assert!(
            swell_core::AutonomyLevel::Supervised.needs_step_approval(swell_core::RiskLevel::Low)
        );
        assert!(!swell_core::AutonomyLevel::Guided.needs_step_approval(swell_core::RiskLevel::Low));
        assert!(
            !swell_core::AutonomyLevel::Autonomous.needs_step_approval(swell_core::RiskLevel::Low)
        );
        assert!(
            swell_core::AutonomyLevel::Autonomous.needs_step_approval(swell_core::RiskLevel::High)
        );
        assert!(
            !swell_core::AutonomyLevel::FullAuto.needs_step_approval(swell_core::RiskLevel::High)
        );

        // Test needs_validation_approval
        assert!(swell_core::AutonomyLevel::Supervised.needs_validation_approval());
        assert!(!swell_core::AutonomyLevel::Guided.needs_validation_approval());
        assert!(!swell_core::AutonomyLevel::Autonomous.needs_validation_approval());
        assert!(!swell_core::AutonomyLevel::FullAuto.needs_validation_approval());
    }
}
