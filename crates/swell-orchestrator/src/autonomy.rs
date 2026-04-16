//! Autonomy level controller for managing approval workflows.
//!
//! Implements per-task autonomy levels:
//! - L1 (Supervised): Every action requires approval
//! - L2 (Guided): Plan approval required, auto-execute (default)
//! - L3 (Autonomous): Minimal guidance, only high-risk actions need approval
//! - L4 (Full Auto): Fully autonomous, no approvals needed
//!
//! Also supports per-agent-type and per-task-type override matrix for fine-grained
//! control over approval requirements (VAL-ORCH-011).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use swell_core::AutonomyLevel;

/// Task type classification for autonomy overrides.
/// Used to determine approval requirements based on what kind of work
/// is being performed, independent of which agent is performing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Code generation tasks - implementing new features, writing functions
    CodeGeneration,
    /// Code review tasks - evaluating code quality, finding issues
    CodeReview,
    /// Refactoring tasks - restructuring existing code
    Refactoring,
    /// Documentation tasks - writing or updating docs
    Documentation,
    /// Testing tasks - writing or running tests
    Testing,
    /// Research tasks - investigating, exploring, learning
    Research,
    /// Planning tasks - creating plans, decomposing work
    Planning,
    /// General/uncategorized tasks
    #[default]
    General,
}

impl TaskType {
    /// Returns true if this task type is considered "strategic" (high-impact, complex).
    /// Strategic tasks typically require more oversight even at higher autonomy levels.
    pub fn is_strategic(&self) -> bool {
        matches!(
            self,
            TaskType::Planning
                | TaskType::Refactoring
                | TaskType::CodeGeneration // Can be strategic if complex
        )
    }

    /// Returns true if this task type is considered "routine" (low-impact, repetitive).
    /// Routine tasks typically proceed without approval at Guided and above.
    pub fn is_routine(&self) -> bool {
        matches!(
            self,
            TaskType::Documentation | TaskType::Testing | TaskType::CodeReview
        )
    }
}

/// Autonomy override configuration for specific agent/task combinations.
/// Allows fine-grained control over approval requirements beyond the
/// base autonomy level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyOverride {
    /// The effective autonomy level to use for this override
    pub level: AutonomyLevel,
    /// If true, strategic actions still require approval at this level
    /// even if the base level would allow them to proceed automatically.
    pub strategic_requires_approval: bool,
}

impl AutonomyOverride {
    /// Create a new override with a specific level
    pub fn with_level(level: AutonomyLevel) -> Self {
        Self {
            level,
            strategic_requires_approval: false,
        }
    }

    /// Create an override that requires approval for strategic actions
    pub fn with_strategic_approval(level: AutonomyLevel) -> Self {
        Self {
            level,
            strategic_requires_approval: true,
        }
    }
}

/// Override matrix for per-agent-type and per-task-type autonomy control.
/// The matrix is keyed by (AgentRole, TaskType) to allow fine-grained control
/// over what requires approval based on who is doing what.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutonomyOverrideMatrix {
    /// Overrides keyed by (agent_role_string, task_type)
    overrides: HashMap<(String, TaskType), AutonomyOverride>,
    /// Per-agent-type default overrides (when no task type specific override exists)
    agent_defaults: HashMap<String, AutonomyOverride>,
    /// Per-task-type default overrides (when no agent type specific override exists)
    task_defaults: HashMap<TaskType, AutonomyOverride>,
    /// Global fallback override (used when no specific override matches)
    global_fallback: Option<AutonomyOverride>,
}

impl AutonomyOverrideMatrix {
    /// Create a new empty override matrix
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an override for a specific (AgentRole, TaskType) combination.
    /// This takes highest priority in override resolution.
    pub fn add_override(
        mut self,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        override_config: AutonomyOverride,
    ) -> Self {
        let agent_key = format!("{:?}", agent_role);
        self.overrides.insert((agent_key, task_type), override_config);
        self
    }

    /// Add an override for a specific agent role (applies to all task types).
    /// This is used when you want a specific agent type to always use a certain
    /// autonomy level regardless of task.
    pub fn add_agent_override(
        mut self,
        agent_role: swell_core::AgentRole,
        override_config: AutonomyOverride,
    ) -> Self {
        let agent_key = format!("{:?}", agent_role);
        self.agent_defaults.insert(agent_key, override_config);
        self
    }

    /// Add an override for a specific task type (applies to all agent types).
    /// This is used when you want code-gen tasks to always use Supervised mode
    /// even if the agent is configured for Autonomous.
    pub fn add_task_override(mut self, task_type: TaskType, override_config: AutonomyOverride) -> Self {
        self.task_defaults.insert(task_type, override_config);
        self
    }

    /// Set a global fallback override used when no specific override matches.
    /// This is the lowest priority override.
    pub fn set_global_fallback(mut self, override_config: AutonomyOverride) -> Self {
        self.global_fallback = Some(override_config);
        self
    }

    /// Get the effective autonomy level and override config for a given
    /// agent role and task type combination.
    pub fn get_effective(
        &self,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        base_level: swell_core::AutonomyLevel,
    ) -> (swell_core::AutonomyLevel, bool) {
        let agent_key = format!("{:?}", agent_role);

        // Priority 1: Specific (agent_role, task_type) override
        if let Some(override_config) = self.overrides.get(&(agent_key.clone(), task_type)) {
            return (override_config.level, override_config.strategic_requires_approval);
        }

        // Priority 2: Agent-specific override (ignores task type)
        if let Some(override_config) = self.agent_defaults.get(&agent_key) {
            return (override_config.level, override_config.strategic_requires_approval);
        }

        // Priority 3: Task-specific override (ignores agent type)
        if let Some(override_config) = self.task_defaults.get(&task_type) {
            return (override_config.level, override_config.strategic_requires_approval);
        }

        // Priority 4: Global fallback
        if let Some(override_config) = &self.global_fallback {
            return (override_config.level, override_config.strategic_requires_approval);
        }

        // Default: use base level
        (base_level, false)
    }

    /// Returns true if there's any override configured for the given agent role
    pub fn has_agent_override(&self, agent_role: swell_core::AgentRole) -> bool {
        let agent_key = format!("{:?}", agent_role);
        self.agent_defaults.contains_key(&agent_key)
            || self.overrides.keys().any(|(ar, _)| ar == &agent_key)
    }

    /// Returns true if there's any override configured for the given task type
    pub fn has_task_override(&self, task_type: TaskType) -> bool {
        self.task_defaults.contains_key(&task_type)
            || self.overrides.keys().any(|(_, tt)| *tt == task_type)
    }
}

/// Determines if an action is "strategic" vs "routine" based on action characteristics.
/// Strategic actions are high-impact decisions that warrant additional oversight.
/// Routine actions are low-impact, repetitive tasks that typically proceed without approval.
pub fn classify_action_strategic(
    action_description: &str,
    affected_files: &[String],
    task_type: TaskType,
) -> bool {
    // Task types that are always strategic
    if task_type.is_strategic() {
        // But only for substantial actions
        if action_description.len() < 20 && affected_files.is_empty() {
            return false;
        }
        return true;
    }

    // Task types that are always routine
    if task_type.is_routine() {
        return false;
    }

    // Heuristics for determining strategic vs routine:
    // Strategic indicators:
    // - Architectural decisions (mentions "architecture", "design pattern", "decision")
    // - High-risk operations (mentions "delete", "migration", "refactor", "restructure")
    // - Many files affected (more than 3)
    // - Root-level or config files affected
    // - Multiple files with same extension suggesting systemic change

    let desc_lower = action_description.to_lowercase();

    // Check for strategic keywords
    let strategic_keywords = [
        "architecture",
        "design pattern",
        "decision",
        "migration",
        "restructure",
        "architecture",
        "architectural",
        "paradigm",
        "rearchitect",
    ];
    if strategic_keywords.iter().any(|k| desc_lower.contains(k)) {
        return true;
    }

    // Check for high-risk operations
    let high_risk_keywords = ["delete", "drop", "remove all", "purge", "permanent"];
    if high_risk_keywords.iter().any(|k| desc_lower.contains(k)) && !affected_files.is_empty() {
        return true;
    }

    // Check for multi-file changes that suggest systemic work
    if affected_files.len() > 5 {
        return true;
    }

    // Check for root-level or configuration file changes
    for file in affected_files {
        if file.starts_with("src/main.rs")
            || file.starts_with("Cargo.toml")
            || file.starts_with(".env")
            || file.contains("config")
        {
            return true;
        }
    }

    false
}

/// Represents an approval request for a pending action
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub task_id: Uuid,
    pub request_id: Uuid,
    pub step_id: Option<Uuid>,
    pub action_description: String,
    pub risk_level: swell_core::RiskLevel,
    pub autonomy_level: swell_core::AutonomyLevel,
    pub agent_role: swell_core::AgentRole,
    pub task_type: TaskType,
    pub affected_files: Vec<String>,
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
            agent_role: swell_core::AgentRole::Generator, // Default
            task_type: TaskType::General,
            affected_files: Vec::new(),
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
            agent_role: swell_core::AgentRole::Generator,
            task_type: TaskType::General,
            affected_files: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Builder method to set the agent role
    pub fn with_agent_role(mut self, role: swell_core::AgentRole) -> Self {
        self.agent_role = role;
        self
    }

    /// Builder method to set the task type
    pub fn with_task_type(mut self, task_type: TaskType) -> Self {
        self.task_type = task_type;
        self
    }

    /// Builder method to set affected files
    pub fn with_affected_files(mut self, files: Vec<String>) -> Self {
        self.affected_files = files;
        self
    }

    /// Returns true if this action is classified as strategic
    pub fn is_strategic(&self) -> bool {
        classify_action_strategic(&self.action_description, &self.affected_files, self.task_type)
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
    /// Override matrix for per-agent-type and per-task-type autonomy control
    override_matrix: Arc<RwLock<AutonomyOverrideMatrix>>,
}

impl AutonomyController {
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(AutonomyOverrideMatrix::new())),
        }
    }

    /// Create a new controller with a specific override matrix
    pub fn with_override_matrix(matrix: AutonomyOverrideMatrix) -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(matrix)),
        }
    }

    /// Get a reference to the current override matrix
    pub async fn get_override_matrix(&self) -> AutonomyOverrideMatrix {
        self.override_matrix.read().await.clone()
    }

    /// Update the override matrix
    pub async fn set_override_matrix(&self, matrix: AutonomyOverrideMatrix) {
        let mut current = self.override_matrix.write().await;
        *current = matrix;
    }

    /// Add a specific override for an (agent_role, task_type) combination
    pub async fn add_override(
        &self,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        override_config: AutonomyOverride,
    ) {
        let mut matrix = self.override_matrix.write().await;
        let agent_key = format!("{:?}", agent_role);
        matrix.overrides.insert((agent_key, task_type), override_config);
    }

    /// Check if an approval is needed based on task autonomy level, risk, agent role, and task type.
    /// This method incorporates the override matrix to determine the effective autonomy level
    /// and whether strategic actions require approval.
    pub async fn needs_approval(
        &self,
        _task_id: Uuid,
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        is_strategic_action: bool,
    ) -> bool {
        // Get effective autonomy level from override matrix
        let (effective_level, strategic_requires_approval) = {
            let matrix = self.override_matrix.read().await;
            matrix.get_effective(agent_role, task_type, autonomy_level)
        };

        // L4 FullAuto never needs approval (unless strategic override requires it)
        if effective_level == swell_core::AutonomyLevel::FullAuto {
            // But check if strategic actions require approval
            if strategic_requires_approval && is_strategic_action {
                return true;
            }
            return false;
        }

        // L1 Supervised always needs approval
        if effective_level == swell_core::AutonomyLevel::Supervised {
            return true;
        }

        // L2 Guided only needs approval for plan (handled separately) and strategic actions if configured
        if effective_level == swell_core::AutonomyLevel::Guided {
            // Strategic actions always need approval under Guided
            if is_strategic_action {
                return true;
            }
            return false; // Steps auto-execute after plan approval
        }

        // L3 Autonomous only needs approval for high-risk actions and strategic if configured
        if effective_level == swell_core::AutonomyLevel::Autonomous {
            if is_strategic_action && strategic_requires_approval {
                return true;
            }
            return risk_level == swell_core::RiskLevel::High;
        }

        false
    }

    /// Legacy check for approval (without agent role and task type info)
    /// Uses default values for agent_role and task_type
    pub async fn needs_approval_simple(
        &self,
        task_id: Uuid,
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
    ) -> bool {
        self.needs_approval(
            task_id,
            risk_level,
            autonomy_level,
            swell_core::AgentRole::Generator,
            TaskType::General,
            false, // Assume non-strategic for backward compatibility
        )
        .await
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

    /// Check if a request's action is strategic based on the request's own attributes
    pub async fn is_request_strategic(&self, request_id: Uuid) -> Option<bool> {
        let pending = self.pending_requests.read().await;
        pending.get(&request_id).map(|r| r.is_strategic())
    }

    /// Get the effective autonomy level for a given agent and task type
    /// considering the override matrix
    pub async fn get_effective_level(
        &self,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        base_level: swell_core::AutonomyLevel,
    ) -> AutonomyLevel {
        let matrix = self.override_matrix.read().await;
        matrix.get_effective(agent_role, task_type, base_level).0
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
                    swell_core::AutonomyLevel::FullAuto,
                    swell_core::AgentRole::Generator,
                    TaskType::CodeGeneration,
                    false, // is_strategic
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::FullAuto,
                    swell_core::AgentRole::Generator,
                    TaskType::Documentation,
                    false, // is_strategic
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
                    swell_core::AutonomyLevel::Supervised,
                    swell_core::AgentRole::Generator,
                    TaskType::CodeGeneration,
                    false, // is_strategic
                )
                .await
        );
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Supervised,
                    swell_core::AgentRole::Generator,
                    TaskType::Documentation,
                    false, // is_strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_guided_strategic_needs_approval() {
        let controller = AutonomyController::new();

        // L2 Guided: strategic actions need approval
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Guided,
                    swell_core::AgentRole::Generator,
                    TaskType::CodeGeneration,
                    true, // is_strategic
                )
                .await
        );
        // Non-strategic actions don't need approval
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Guided,
                    swell_core::AgentRole::Generator,
                    TaskType::Documentation,
                    false, // is_strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_guided_steps_dont_need_approval() {
        let controller = AutonomyController::new();

        // L2 Guided: non-strategic steps auto-execute after plan approval
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Guided,
                    swell_core::AgentRole::Generator,
                    TaskType::Testing,
                    false, // not strategic
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Guided,
                    swell_core::AgentRole::Generator,
                    TaskType::Documentation,
                    false, // not strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_autonomous_high_risk_needs_approval() {
        let controller = AutonomyController::new();

        // L3 Autonomous: only high-risk needs approval (non-strategic)
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::High,
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::General,
                    false, // not strategic
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Medium,
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::General,
                    false, // not strategic
                )
                .await
        );
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::General,
                    false, // not strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_autonomous_strategic_with_override() {
        // Test that strategic actions with strategic_requires_approval flag
        // require approval even at Autonomous level
        let matrix = AutonomyOverrideMatrix::new()
            .add_agent_override(
                swell_core::AgentRole::Generator,
                AutonomyOverride::with_strategic_approval(swell_core::AutonomyLevel::Autonomous),
            );
        let controller = AutonomyController::with_override_matrix(matrix);

        // Strategic action at Autonomous with override requires approval
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low, // Low risk but strategic
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::CodeGeneration,
                    true, // is_strategic
                )
                .await
        );

        // Non-strategic action at Autonomous doesn't require approval (low risk)
        assert!(
            !controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low,
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::Documentation,
                    false, // not strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_per_task_type_override() {
        // Test: set code-gen tasks to Supervised even for Autonomous agent
        let matrix = AutonomyOverrideMatrix::new()
            .add_task_override(
                TaskType::CodeGeneration,
                AutonomyOverride::with_level(swell_core::AutonomyLevel::Supervised),
            );
        let controller = AutonomyController::with_override_matrix(matrix);

        // Agent is Autonomous but task type is CodeGeneration which is Supervised
        // So approval should be required
        assert!(
            controller
                .needs_approval(
                    Uuid::new_v4(),
                    swell_core::RiskLevel::Low, // Low risk, but task type override
                    swell_core::AutonomyLevel::Autonomous,
                    swell_core::AgentRole::Generator,
                    TaskType::CodeGeneration,
                    false, // not strategic
                )
                .await
        );
    }

    #[tokio::test]
    async fn test_effective_level_with_override() {
        let matrix = AutonomyOverrideMatrix::new()
            .add_agent_override(
                swell_core::AgentRole::Evaluator,
                AutonomyOverride::with_level(swell_core::AutonomyLevel::Supervised),
            );
        let controller = AutonomyController::with_override_matrix(matrix);

        // Base level is Guided but Evaluator agent is overridden to Supervised
        let effective = controller
            .get_effective_level(
                swell_core::AgentRole::Evaluator,
                TaskType::General,
                swell_core::AutonomyLevel::Guided,
            )
            .await;
        assert_eq!(effective, swell_core::AutonomyLevel::Supervised);
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

    #[tokio::test]
    async fn test_task_type_is_strategic() {
        // Planning tasks are always strategic
        assert!(TaskType::Planning.is_strategic());
        // Refactoring tasks are always strategic
        assert!(TaskType::Refactoring.is_strategic());
        // Documentation is routine
        assert!(!TaskType::Documentation.is_strategic());
        // Testing is routine
        assert!(!TaskType::Testing.is_strategic());
        // Code review is routine
        assert!(!TaskType::CodeReview.is_strategic());
    }

    #[tokio::test]
    async fn test_task_type_is_routine() {
        // Documentation is routine
        assert!(TaskType::Documentation.is_routine());
        // Testing is routine
        assert!(TaskType::Testing.is_routine());
        // Code review is routine
        assert!(TaskType::CodeReview.is_routine());
        // Planning is not routine
        assert!(!TaskType::Planning.is_routine());
        // Refactoring is not routine
        assert!(!TaskType::Refactoring.is_routine());
    }

    #[tokio::test]
    async fn test_override_matrix_priority() {
        let matrix = AutonomyOverrideMatrix::new()
            .add_agent_override(
                swell_core::AgentRole::Generator,
                AutonomyOverride::with_level(swell_core::AutonomyLevel::Supervised),
            )
            .add_task_override(
                TaskType::Documentation,
                AutonomyOverride::with_level(swell_core::AutonomyLevel::FullAuto),
            );

        // Agent override should win over task override for Generator/Documentation
        let (level, _) = matrix.get_effective(
            swell_core::AgentRole::Generator,
            TaskType::Documentation,
            swell_core::AutonomyLevel::Autonomous,
        );
        assert_eq!(level, swell_core::AutonomyLevel::Supervised);

        // Task override should apply for different agent
        let (level, _) = matrix.get_effective(
            swell_core::AgentRole::Evaluator,
            TaskType::Documentation,
            swell_core::AutonomyLevel::Autonomous,
        );
        assert_eq!(level, swell_core::AutonomyLevel::FullAuto);
    }

    #[tokio::test]
    async fn test_approval_request_builder() {
        let request = ApprovalRequest::new(
            Uuid::new_v4(),
            "Implement feature X".to_string(),
            swell_core::RiskLevel::High,
            swell_core::AutonomyLevel::Guided,
        )
        .with_agent_role(swell_core::AgentRole::Generator)
        .with_task_type(TaskType::CodeGeneration)
        .with_affected_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]);

        assert_eq!(request.agent_role, swell_core::AgentRole::Generator);
        assert_eq!(request.task_type, TaskType::CodeGeneration);
        assert_eq!(request.affected_files.len(), 2);
    }

    #[tokio::test]
    async fn test_classify_action_strategic() {
        // Strategic: architecture decision
        assert!(classify_action_strategic(
            "Make architecture decision for new module",
            &["src/main.rs".to_string()],
            TaskType::Planning
        ));

        // Strategic: many files affected
        let files: Vec<String> = vec!["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]
            .into_iter()
            .map(String::from)
            .collect();
        assert!(classify_action_strategic("Refactor core module", &files, TaskType::General));

        // Not strategic: routine doc task
        assert!(!classify_action_strategic(
            "Update README",
            &["README.md".to_string()],
            TaskType::Documentation
        ));

        // Not strategic: short routine task
        assert!(!classify_action_strategic(
            "Add comment",
            &[],
            TaskType::Documentation
        ));
    }
}
