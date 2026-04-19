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
use swell_core::ids::TaskId;
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
            TaskType::Planning | TaskType::Refactoring | TaskType::CodeGeneration // Can be strategic if complex
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
        self.overrides
            .insert((agent_key, task_type), override_config);
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
    pub fn add_task_override(
        mut self,
        task_type: TaskType,
        override_config: AutonomyOverride,
    ) -> Self {
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
            return (
                override_config.level,
                override_config.strategic_requires_approval,
            );
        }

        // Priority 2: Agent-specific override (ignores task type)
        if let Some(override_config) = self.agent_defaults.get(&agent_key) {
            return (
                override_config.level,
                override_config.strategic_requires_approval,
            );
        }

        // Priority 3: Task-specific override (ignores agent type)
        if let Some(override_config) = self.task_defaults.get(&task_type) {
            return (
                override_config.level,
                override_config.strategic_requires_approval,
            );
        }

        // Priority 4: Global fallback
        if let Some(override_config) = &self.global_fallback {
            return (
                override_config.level,
                override_config.strategic_requires_approval,
            );
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
    pub task_id: TaskId,
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
        task_id: TaskId,
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
        task_id: TaskId,
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
        classify_action_strategic(
            &self.action_description,
            &self.affected_files,
            self.task_type,
        )
    }
}

/// Result of an approval decision
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
    Pending,
}

/// Confidence threshold configuration for uncertainty pauses.
/// Stores the confidence threshold for each agent type.
#[derive(Debug, Clone, Default)]
pub struct ConfidenceThresholdConfig {
    /// Per-agent-role confidence thresholds (0.0 to 1.0).
    /// If an agent's confidence score drops below this threshold,
    /// the agent pauses and emits a clarification request.
    thresholds: HashMap<String, f64>,
    /// Default threshold for agents without a specific configuration.
    default_threshold: f64,
}

impl ConfidenceThresholdConfig {
    /// Create a new configuration with a default threshold.
    pub fn new(default_threshold: f64) -> Self {
        Self {
            thresholds: HashMap::new(),
            default_threshold,
        }
    }

    /// Set a threshold for a specific agent role.
    pub fn set_threshold(mut self, role: swell_core::AgentRole, threshold: f64) -> Self {
        let key = format!("{:?}", role);
        self.thresholds.insert(key, threshold);
        self
    }

    /// Get the threshold for a specific agent role.
    pub fn get_threshold(&self, role: swell_core::AgentRole) -> f64 {
        let key = format!("{:?}", role);
        self.thresholds
            .get(&key)
            .copied()
            .unwrap_or(self.default_threshold)
    }

    /// Get the default threshold.
    pub fn default_threshold(&self) -> f64 {
        self.default_threshold
    }

    /// Check if a confidence score is below the threshold for a given role.
    pub fn is_below_threshold(&self, role: swell_core::AgentRole, confidence: f64) -> bool {
        confidence < self.get_threshold(role)
    }
}

/// Approval decay configuration for controlling how approval thresholds change
/// as a task run progresses.
///
/// # Decay Behavior
///
/// - Below `decay_start_completion` (default 80%): standard approval rules apply
/// - Above `decay_start_completion`: elevated approval thresholds apply
/// - Failure-derived tasks targeting files outside original plan scope always
///   require explicit approval after `decay_start_completion`
#[derive(Debug, Clone)]
pub struct ApprovalDecayConfig {
    /// Completion percentage (0.0-1.0) at which decay starts to apply.
    /// Below this threshold, standard approval rules apply.
    /// Default: 0.8 (80%)
    pub decay_start_completion: f64,
    /// Multiplier applied to approval threshold after decay starts.
    /// Higher values require more confidence to auto-approve.
    /// Default: 1.5 (50% harder to auto-approve)
    pub threshold_multiplier: f64,
    /// Maximum multiplier cap to prevent over-escalation.
    /// Default: 2.0
    pub max_multiplier: f64,
}

impl Default for ApprovalDecayConfig {
    fn default() -> Self {
        Self {
            decay_start_completion: 0.8,
            threshold_multiplier: 1.5,
            max_multiplier: 2.0,
        }
    }
}

impl ApprovalDecayConfig {
    /// Create a new decay configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the completion threshold at which decay starts.
    pub fn with_decay_start(mut self, completion: f64) -> Self {
        self.decay_start_completion = completion.clamp(0.0, 1.0);
        self
    }

    /// Set the threshold multiplier applied after decay starts.
    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.threshold_multiplier = multiplier.max(1.0);
        self
    }

    /// Set the maximum multiplier cap.
    pub fn with_max_multiplier(mut self, max_mult: f64) -> Self {
        self.max_multiplier = max_mult.max(1.0);
        self
    }

    /// Calculate the effective threshold multiplier based on completion.
    ///
    /// Returns 1.0 if completion is below decay_start_completion.
    /// Returns progressively higher multiplier as completion approaches 1.0.
    pub fn get_effective_multiplier(&self, completion: f64) -> f64 {
        if completion < self.decay_start_completion {
            return 1.0;
        }

        // Linear interpolation between 1.0 and threshold_multiplier
        // as completion goes from decay_start to 1.0
        let progress =
            (completion - self.decay_start_completion) / (1.0 - self.decay_start_completion);
        let progress = progress.clamp(0.0, 1.0);

        let multiplier_range = self.threshold_multiplier - 1.0;
        let effective = 1.0 + (progress * multiplier_range);
        effective.min(self.max_multiplier)
    }
}

/// Manages approval requests and decisions for autonomy levels
pub struct AutonomyController {
    pending_requests: Arc<RwLock<HashMap<Uuid, ApprovalRequest>>>,
    decisions: Arc<RwLock<HashMap<Uuid, ApprovalDecision>>>,
    /// Override matrix for per-agent-type and per-task-type autonomy control
    override_matrix: Arc<RwLock<AutonomyOverrideMatrix>>,
    /// Confidence threshold configuration for uncertainty pauses
    confidence_threshold: Arc<RwLock<ConfidenceThresholdConfig>>,
    /// Approval decay configuration for post-80% completion behavior
    approval_decay: Arc<RwLock<ApprovalDecayConfig>>,
}

impl AutonomyController {
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(AutonomyOverrideMatrix::new())),
            confidence_threshold: Arc::new(RwLock::new(ConfidenceThresholdConfig::new(0.5))),
            approval_decay: Arc::new(RwLock::new(ApprovalDecayConfig::new())),
        }
    }

    /// Create a new controller with a specific override matrix
    pub fn with_override_matrix(matrix: AutonomyOverrideMatrix) -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(matrix)),
            confidence_threshold: Arc::new(RwLock::new(ConfidenceThresholdConfig::new(0.5))),
            approval_decay: Arc::new(RwLock::new(ApprovalDecayConfig::new())),
        }
    }

    /// Create a new controller with a specific override matrix and confidence threshold config
    pub fn with_confidence_config(
        matrix: AutonomyOverrideMatrix,
        threshold_config: ConfidenceThresholdConfig,
    ) -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(matrix)),
            confidence_threshold: Arc::new(RwLock::new(threshold_config)),
            approval_decay: Arc::new(RwLock::new(ApprovalDecayConfig::new())),
        }
    }

    /// Create a new controller with all configuration options
    pub fn with_config(
        matrix: AutonomyOverrideMatrix,
        threshold_config: ConfidenceThresholdConfig,
        decay_config: ApprovalDecayConfig,
    ) -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            override_matrix: Arc::new(RwLock::new(matrix)),
            confidence_threshold: Arc::new(RwLock::new(threshold_config)),
            approval_decay: Arc::new(RwLock::new(decay_config)),
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
        matrix
            .overrides
            .insert((agent_key, task_type), override_config);
    }

    /// Check if an approval is needed based on task autonomy level, risk, agent role, and task type.
    /// This method incorporates the override matrix to determine the effective autonomy level
    /// and whether strategic actions require approval.
    pub async fn needs_approval(
        &self,
        _task_id: TaskId,
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
        task_id: TaskId,
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
    pub async fn get_pending_for_task(&self, task_id: TaskId) -> Vec<ApprovalRequest> {
        let pending = self.pending_requests.read().await;
        pending
            .values()
            .filter(|r| r.task_id == task_id)
            .cloned()
            .collect()
    }

    /// Clear all pending requests (used when task is cancelled or completed)
    pub async fn clear_task_requests(&self, task_id: TaskId) {
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

    // ============================================================================
    // Confidence Threshold Management (for Uncertainty Pauses)
    // ============================================================================

    /// Get the confidence threshold for a specific agent role.
    pub async fn get_confidence_threshold(&self, role: swell_core::AgentRole) -> f64 {
        let config = self.confidence_threshold.read().await;
        config.get_threshold(role)
    }

    /// Get the default confidence threshold.
    pub async fn get_default_confidence_threshold(&self) -> f64 {
        let config = self.confidence_threshold.read().await;
        config.default_threshold()
    }

    /// Set the confidence threshold for a specific agent role.
    /// This allows per-agent-type customization of the uncertainty pause threshold.
    pub async fn set_confidence_threshold(&self, role: swell_core::AgentRole, threshold: f64) {
        let mut config = self.confidence_threshold.write().await;
        let key = format!("{:?}", role);
        config.thresholds.insert(key, threshold);
    }

    /// Set the default confidence threshold.
    pub async fn set_default_confidence_threshold(&self, threshold: f64) {
        let mut config = self.confidence_threshold.write().await;
        config.default_threshold = threshold;
    }

    /// Check if a confidence score is below the threshold for a given agent role.
    /// Returns true if the agent should pause due to low confidence.
    pub async fn is_confidence_below_threshold(
        &self,
        role: swell_core::AgentRole,
        confidence: f64,
    ) -> bool {
        let config = self.confidence_threshold.read().await;
        config.is_below_threshold(role, confidence)
    }

    /// Set the entire confidence threshold configuration.
    pub async fn set_confidence_threshold_config(&self, config: ConfidenceThresholdConfig) {
        let mut current = self.confidence_threshold.write().await;
        *current = config;
    }

    /// Get a clone of the current confidence threshold configuration.
    pub async fn get_confidence_threshold_config(&self) -> ConfidenceThresholdConfig {
        self.confidence_threshold.read().await.clone()
    }

    // ============================================================================
    // Approval Decay Management
    // ============================================================================

    /// Get the current approval decay configuration.
    pub async fn get_approval_decay_config(&self) -> ApprovalDecayConfig {
        self.approval_decay.read().await.clone()
    }

    /// Set the approval decay configuration.
    pub async fn set_approval_decay_config(&self, config: ApprovalDecayConfig) {
        let mut current = self.approval_decay.write().await;
        *current = config;
    }

    /// Update decay configuration using a builder pattern.
    pub async fn update_approval_decay<F>(self: &Arc<Self>, f: F)
    where
        F: FnOnce(&mut ApprovalDecayConfig),
    {
        let mut config = self.approval_decay.write().await;
        f(&mut config);
    }

    /// Calculate the effective approval threshold multiplier based on plan completion.
    ///
    /// Below the decay start threshold (default 80%), returns 1.0 (no change).
    /// Above 80%, returns progressively higher multiplier up to max_multiplier.
    pub async fn get_approval_multiplier(&self, plan_completion: f64) -> f64 {
        let config = self.approval_decay.read().await;
        config.get_effective_multiplier(plan_completion)
    }

    /// Determine if an approval is required based on plan completion and task origin.
    ///
    /// This method extends `needs_approval` with approval decay logic:
    /// - Below 80% completion: standard approval rules apply
    /// - At or above 80% completion: elevated thresholds apply
    /// - Failure-derived tasks targeting files outside original plan scope ALWAYS
    ///   require explicit approval, regardless of completion percentage
    ///
    /// # Arguments
    /// * `plan_completion` - Progress through the plan (0.0 to 1.0)
    /// * `task_origin` - Source of the task (Planned vs FailureDerived)
    /// * `affected_files` - Files the task would modify
    /// * `original_scope` - Files in the original task plan scope
    /// * `risk_level` - Risk level of the action
    /// * `autonomy_level` - Base autonomy level
    /// * `agent_role` - Role of the agent requesting approval
    /// * `task_type` - Type of task
    /// * `is_strategic_action` - Whether the action is classified as strategic
    #[allow(clippy::too_many_arguments)]
    pub async fn needs_approval_with_decay(
        &self,
        plan_completion: f64,
        task_origin: TaskOrigin,
        affected_files: &[String],
        original_scope: &[String],
        risk_level: swell_core::RiskLevel,
        autonomy_level: swell_core::AutonomyLevel,
        agent_role: swell_core::AgentRole,
        task_type: TaskType,
        is_strategic_action: bool,
    ) -> bool {
        // CRITICAL: Failure-derived tasks targeting files outside original plan
        // ALWAYS require explicit approval after 80% completion
        if task_origin == TaskOrigin::FailureDerived
            && plan_completion >= 0.8
            && !is_file_in_scope(affected_files, original_scope)
        {
            // Out-of-scope failure-derived task - always requires approval
            tracing::debug!(
                plan_completion = plan_completion,
                "Failure-derived task outside original scope - explicit approval required"
            );
            return true;
        }

        // Get the base approval requirement
        let base_needs_approval = self
            .needs_approval(
                TaskId::new(),
                risk_level,
                autonomy_level,
                agent_role,
                task_type,
                is_strategic_action,
            )
            .await;

        // If base says no approval needed, check decay
        if !base_needs_approval {
            return false;
        }

        // If base says approval needed, apply decay multiplier
        // to determine if it's truly required or if it can be auto-approved
        let multiplier = self.get_approval_multiplier(plan_completion).await;

        // At multiplier 1.0, standard rules apply
        if multiplier <= 1.0 {
            return base_needs_approval;
        }

        // With decay active, only high-risk or strategic actions require approval
        // (lower-risk actions might get auto-approved at higher multipliers)
        if risk_level == swell_core::RiskLevel::High || is_strategic_action {
            return true;
        }

        // Medium risk at high multipliers might be approved
        if risk_level == swell_core::RiskLevel::Medium {
            // Threshold increases by multiplier, so medium risk at > 1.5 might pass
            return multiplier < 1.5;
        }

        // Low risk is rarely blocked
        false
    }
}

/// Task origin classification for decay decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOrigin {
    /// Task from the original plan (in-scope, expected work)
    Planned,
    /// Task derived from a validation failure (often out-of-scope)
    FailureDerived,
}

/// Check if any of the affected files are within the original scope.
fn is_file_in_scope(affected_files: &[String], original_scope: &[String]) -> bool {
    if original_scope.is_empty() {
        // Empty scope means everything is in scope
        return true;
    }

    for file in affected_files {
        // Check if file is under any scope directory
        if original_scope
            .iter()
            .any(|scope| file.starts_with(scope) || scope.starts_with(file))
        {
            return true;
        }
    }

    false
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
                    TaskId::new(),
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
        let matrix = AutonomyOverrideMatrix::new().add_agent_override(
            swell_core::AgentRole::Generator,
            AutonomyOverride::with_strategic_approval(swell_core::AutonomyLevel::Autonomous),
        );
        let controller = AutonomyController::with_override_matrix(matrix);

        // Strategic action at Autonomous with override requires approval
        assert!(
            controller
                .needs_approval(
                    TaskId::new(),
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
                    TaskId::new(),
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
        let matrix = AutonomyOverrideMatrix::new().add_task_override(
            TaskType::CodeGeneration,
            AutonomyOverride::with_level(swell_core::AutonomyLevel::Supervised),
        );
        let controller = AutonomyController::with_override_matrix(matrix);

        // Agent is Autonomous but task type is CodeGeneration which is Supervised
        // So approval should be required
        assert!(
            controller
                .needs_approval(
                    TaskId::new(),
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
        let matrix = AutonomyOverrideMatrix::new().add_agent_override(
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
        let task_id = TaskId::new();

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
        let task_id = TaskId::new();

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
        let task_id = TaskId::new();

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
        let other_task = TaskId::new();
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
            TaskId::new(),
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
        assert!(classify_action_strategic(
            "Refactor core module",
            &files,
            TaskType::General
        ));

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

    // ============================================================================
    // Confidence Threshold Configuration Tests
    // ============================================================================

    #[test]
    fn test_confidence_threshold_config_default() {
        let config = ConfidenceThresholdConfig::new(0.5);
        assert_eq!(config.default_threshold(), 0.5);
        // Default threshold should be returned for unknown roles
        assert_eq!(config.get_threshold(swell_core::AgentRole::Generator), 0.5);
    }

    #[test]
    fn test_confidence_threshold_config_per_role() {
        let config = ConfidenceThresholdConfig::new(0.5)
            .set_threshold(swell_core::AgentRole::Planner, 0.3)
            .set_threshold(swell_core::AgentRole::Evaluator, 0.7);

        // Custom thresholds
        assert_eq!(config.get_threshold(swell_core::AgentRole::Planner), 0.3);
        assert_eq!(config.get_threshold(swell_core::AgentRole::Evaluator), 0.7);
        // Generator should use default
        assert_eq!(config.get_threshold(swell_core::AgentRole::Generator), 0.5);
    }

    #[test]
    fn test_confidence_threshold_config_is_below_threshold() {
        let config =
            ConfidenceThresholdConfig::new(0.5).set_threshold(swell_core::AgentRole::Coder, 0.3);

        // Generator uses default 0.5
        assert!(!config.is_below_threshold(swell_core::AgentRole::Generator, 0.6));
        assert!(!config.is_below_threshold(swell_core::AgentRole::Generator, 0.5));
        assert!(config.is_below_threshold(swell_core::AgentRole::Generator, 0.4));

        // Coder uses custom 0.3
        assert!(!config.is_below_threshold(swell_core::AgentRole::Coder, 0.4));
        assert!(!config.is_below_threshold(swell_core::AgentRole::Coder, 0.3));
        assert!(config.is_below_threshold(swell_core::AgentRole::Coder, 0.2));
    }

    #[tokio::test]
    async fn test_autonomy_controller_default_confidence_threshold() {
        let controller = AutonomyController::new();

        // Default threshold should be 0.5
        assert_eq!(controller.get_default_confidence_threshold().await, 0.5);

        // All agent roles should initially use the default
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Generator)
                .await,
            0.5
        );
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Planner)
                .await,
            0.5
        );
    }

    #[tokio::test]
    async fn test_autonomy_controller_set_per_role_threshold() {
        let controller = AutonomyController::new();

        // Set custom thresholds
        controller
            .set_confidence_threshold(swell_core::AgentRole::Planner, 0.3)
            .await;
        controller
            .set_confidence_threshold(swell_core::AgentRole::Evaluator, 0.8)
            .await;

        // Verify thresholds
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Planner)
                .await,
            0.3
        );
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Evaluator)
                .await,
            0.8
        );
        // Generator should still use default
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Generator)
                .await,
            0.5
        );
    }

    #[tokio::test]
    async fn test_autonomy_controller_is_confidence_below_threshold() {
        let controller = AutonomyController::new();

        // Set a custom threshold for Coder
        controller
            .set_confidence_threshold(swell_core::AgentRole::Coder, 0.4)
            .await;

        // Test Generator (default 0.5)
        assert!(
            !controller
                .is_confidence_below_threshold(swell_core::AgentRole::Generator, 0.6)
                .await
        );
        assert!(
            !controller
                .is_confidence_below_threshold(swell_core::AgentRole::Generator, 0.5)
                .await
        );
        assert!(
            controller
                .is_confidence_below_threshold(swell_core::AgentRole::Generator, 0.4)
                .await
        );

        // Test Coder (custom 0.4)
        assert!(
            !controller
                .is_confidence_below_threshold(swell_core::AgentRole::Coder, 0.5)
                .await
        );
        assert!(
            controller
                .is_confidence_below_threshold(swell_core::AgentRole::Coder, 0.3)
                .await
        );
    }

    #[tokio::test]
    async fn test_autonomy_controller_confidence_config_lifecycle() {
        let controller = AutonomyController::new();

        // Set individual thresholds
        controller
            .set_confidence_threshold(swell_core::AgentRole::Generator, 0.3)
            .await;

        // Get and verify config
        let config = controller.get_confidence_threshold_config().await;
        assert_eq!(config.get_threshold(swell_core::AgentRole::Generator), 0.3);

        // Create new config with builder
        let new_config = ConfidenceThresholdConfig::new(0.6)
            .set_threshold(swell_core::AgentRole::TestWriter, 0.2);

        // Set new config
        controller.set_confidence_threshold_config(new_config).await;

        // Verify new config
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Generator)
                .await,
            0.6
        );
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::TestWriter)
                .await,
            0.2
        );
    }

    #[tokio::test]
    async fn test_autonomy_controller_with_confidence_config() {
        // Create controller with custom initial config
        let config = ConfidenceThresholdConfig::new(0.7)
            .set_threshold(swell_core::AgentRole::Refactorer, 0.5);

        let matrix = AutonomyOverrideMatrix::new();
        let controller = AutonomyController::with_confidence_config(matrix, config);

        // Verify custom default
        assert_eq!(controller.get_default_confidence_threshold().await, 0.7);

        // Verify custom role threshold
        assert_eq!(
            controller
                .get_confidence_threshold(swell_core::AgentRole::Refactorer)
                .await,
            0.5
        );
    }

    // ============================================================================
    // Approval Decay Tests
    // ============================================================================

    #[test]
    fn test_approval_decay_config_default() {
        let config = ApprovalDecayConfig::new();
        assert_eq!(config.decay_start_completion, 0.8);
        assert_eq!(config.threshold_multiplier, 1.5);
        assert_eq!(config.max_multiplier, 2.0);
    }

    #[test]
    fn test_approval_decay_get_effective_multiplier_below_threshold() {
        let config = ApprovalDecayConfig::new();

        // Below 80% - multiplier should be 1.0
        assert_eq!(config.get_effective_multiplier(0.0), 1.0);
        assert_eq!(config.get_effective_multiplier(0.5), 1.0);
        assert_eq!(config.get_effective_multiplier(0.79), 1.0);
    }

    #[test]
    fn test_approval_decay_get_effective_multiplier_above_threshold() {
        let config = ApprovalDecayConfig::new();

        // At exactly 80% - not yet past, so multiplier should be 1.0
        let mult_80 = config.get_effective_multiplier(0.8);
        assert_eq!(
            mult_80, 1.0,
            "At exactly 80%, multiplier should be 1.0 (not past threshold yet)"
        );

        // Above 80% - decay starts
        let mult_85 = config.get_effective_multiplier(0.85);
        assert!(
            mult_85 > 1.0,
            "Multiplier at 85% should be > 1.0, got {}",
            mult_85
        );

        // At 100% - should reach threshold_multiplier
        let mult_100 = config.get_effective_multiplier(1.0);
        assert_eq!(
            mult_100, 1.5,
            "Multiplier at 100% should be threshold_multiplier (1.5)"
        );
    }

    #[test]
    fn test_approval_decay_max_multiplier_cap() {
        let config = ApprovalDecayConfig::new().with_max_multiplier(1.2);

        // Even at 100%, multiplier should be capped
        assert_eq!(config.get_effective_multiplier(1.0), 1.2);
    }

    #[test]
    fn test_approval_decay_custom_config() {
        let config = ApprovalDecayConfig::new()
            .with_decay_start(0.5)
            .with_multiplier(2.0)
            .with_max_multiplier(3.0);

        assert_eq!(config.decay_start_completion, 0.5);
        assert_eq!(config.threshold_multiplier, 2.0);
        assert_eq!(config.max_multiplier, 3.0);

        // At exactly 50%, not yet past decay start, so multiplier = 1.0
        let mult = config.get_effective_multiplier(0.5);
        assert_eq!(
            mult, 1.0,
            "At exactly decay_start, multiplier should be 1.0"
        );

        // Just above 50%
        let mult_51 = config.get_effective_multiplier(0.51);
        assert!(
            mult_51 > 1.0,
            "Just above decay_start, multiplier should be > 1.0"
        );

        // At 100%, should reach 2.0
        assert_eq!(config.get_effective_multiplier(1.0), 2.0);
    }

    #[tokio::test]
    async fn test_autonomy_controller_default_approval_decay() {
        let controller = AutonomyController::new();

        let config = controller.get_approval_decay_config().await;
        assert_eq!(config.decay_start_completion, 0.8);
        assert_eq!(config.threshold_multiplier, 1.5);
    }

    #[tokio::test]
    async fn test_autonomy_controller_set_approval_decay_config() {
        let controller = AutonomyController::new();

        let new_config = ApprovalDecayConfig::new()
            .with_decay_start(0.7)
            .with_multiplier(2.0);

        controller.set_approval_decay_config(new_config).await;

        let config = controller.get_approval_decay_config().await;
        assert_eq!(config.decay_start_completion, 0.7);
        assert_eq!(config.threshold_multiplier, 2.0);
    }

    #[tokio::test]
    async fn test_get_approval_multiplier_below_80_percent() {
        let controller = AutonomyController::new();

        // Below 80% - multiplier should be 1.0
        assert_eq!(controller.get_approval_multiplier(0.0).await, 1.0);
        assert_eq!(controller.get_approval_multiplier(0.5).await, 1.0);
        assert_eq!(controller.get_approval_multiplier(0.79).await, 1.0);
    }

    #[tokio::test]
    async fn test_get_approval_multiplier_above_80_percent() {
        let controller = AutonomyController::new();

        // At 100% - should reach 1.5
        assert_eq!(controller.get_approval_multiplier(1.0).await, 1.5);
    }

    #[tokio::test]
    async fn test_needs_approval_with_decay_below_80_percent() {
        let controller = AutonomyController::new();

        // Below 80% - standard rules apply, so low risk at Guided should NOT need approval
        let needs_approval = controller
            .needs_approval_with_decay(
                0.5, // plan_completion < 80%
                TaskOrigin::Planned,
                &["src/lib.rs".to_string()],
                &["src/".to_string()],
                swell_core::RiskLevel::Low,
                swell_core::AutonomyLevel::Guided,
                swell_core::AgentRole::Generator,
                TaskType::CodeGeneration,
                false,
            )
            .await;

        assert!(
            !needs_approval,
            "Low risk planned task at 50% completion should not need approval"
        );
    }

    #[tokio::test]
    async fn test_needs_approval_with_decay_failure_derived_out_of_scope() {
        let controller = AutonomyController::new();

        // Failure-derived task targeting files outside original scope at >80% should ALWAYS need approval
        let needs_approval = controller
            .needs_approval_with_decay(
                0.9, // 90% completion - decay active
                TaskOrigin::FailureDerived,
                &["unrelated/file.rs".to_string()], // NOT in original scope
                &["src/".to_string()],              // original scope
                swell_core::RiskLevel::Low,         // even low risk
                swell_core::AutonomyLevel::Autonomous,
                swell_core::AgentRole::Generator,
                TaskType::CodeGeneration,
                false,
            )
            .await;

        assert!(
            needs_approval,
            "Failure-derived out-of-scope task at 90% completion should ALWAYS need approval"
        );
    }

    #[tokio::test]
    async fn test_needs_approval_with_decay_failure_derived_in_scope() {
        let controller = AutonomyController::new();

        // Failure-derived task targeting files WITHIN original scope - standard rules apply
        let needs_approval = controller
            .needs_approval_with_decay(
                0.9, // 90% completion
                TaskOrigin::FailureDerived,
                &["src/lib.rs".to_string()], // IN original scope
                &["src/".to_string()],
                swell_core::RiskLevel::Low,
                swell_core::AutonomyLevel::Autonomous,
                swell_core::AgentRole::Generator,
                TaskType::CodeGeneration,
                false,
            )
            .await;

        assert!(
            !needs_approval,
            "Failure-derived in-scope task at 90% completion should follow standard rules"
        );
    }

    #[tokio::test]
    async fn test_needs_approval_with_decay_high_risk_always_needs_approval() {
        let controller = AutonomyController::new();

        // High risk tasks should always need approval even with decay
        let needs_approval = controller
            .needs_approval_with_decay(
                0.9,
                TaskOrigin::Planned,
                &["src/lib.rs".to_string()],
                &["src/".to_string()],
                swell_core::RiskLevel::High,
                swell_core::AutonomyLevel::Autonomous,
                swell_core::AgentRole::Generator,
                TaskType::CodeGeneration,
                false,
            )
            .await;

        assert!(
            needs_approval,
            "High risk task should need approval even at 90% completion"
        );
    }

    #[test]
    fn test_is_file_in_scope_empty_scope() {
        // Empty scope means everything is in scope
        assert!(is_file_in_scope(&["any/file.rs".to_string()], &[]));
    }

    #[test]
    fn test_is_file_in_scope_file_in_directory() {
        let scope = &["src/".to_string()];
        assert!(is_file_in_scope(&["src/lib.rs".to_string()], scope));
        assert!(is_file_in_scope(&["src/main.rs".to_string()], scope));
        assert!(!is_file_in_scope(&["other/file.rs".to_string()], scope));
    }

    #[test]
    fn test_is_file_in_scope_exact_match() {
        let scope = &["Cargo.toml".to_string()];
        assert!(is_file_in_scope(&["Cargo.toml".to_string()], scope));
        assert!(!is_file_in_scope(&["Cargo.lock".to_string()], scope));
    }

    #[test]
    fn test_task_origin_serde() {
        // Verify TaskOrigin can be serialized and deserialized
        use serde_json;

        let planned = TaskOrigin::Planned;
        let json = serde_json::to_string(&planned).unwrap();
        assert_eq!(json, "\"planned\"");
        let back: TaskOrigin = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TaskOrigin::Planned);

        let failure = TaskOrigin::FailureDerived;
        let json = serde_json::to_string(&failure).unwrap();
        assert_eq!(json, "\"failure_derived\"");
        let back: TaskOrigin = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TaskOrigin::FailureDerived);
    }
}
