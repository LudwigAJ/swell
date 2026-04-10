use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Task lifecycle states as defined in the orchestrator spec
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Created,
    Enriched,
    Ready,
    Assigned,
    Executing,
    Paused, // Operator-initiated pause
    Validating,
    Accepted,
    Rejected,
    Failed,
    Escalated,
}

/// Autonomy level for task execution, controlling approval requirements
///
/// - L1 (Supervised): Every action requires approval before execution
/// - L2 (Guided): Plan approval required, then auto-execute (default)
/// - L3 (Autonomous): Minimal guidance, only high-risk actions need approval
/// - L4 (Full Auto): Fully autonomous, no approvals needed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AutonomyLevel {
    /// L1-Supervised: every action requires explicit approval
    Supervised,
    /// L2-Guided: plan approval required, then execute auto (default)
    #[default]
    Guided,
    /// L3-Autonomous: minimal guidance, decreasing approval needs
    Autonomous,
    /// L4-Full Auto: fully autonomous operation
    FullAuto,
}

impl AutonomyLevel {
    /// Returns true if the plan needs approval before execution
    pub fn needs_plan_approval(&self) -> bool {
        match self {
            AutonomyLevel::Supervised | AutonomyLevel::Guided => true,
            AutonomyLevel::Autonomous | AutonomyLevel::FullAuto => false,
        }
    }

    /// Returns true if a step with the given risk level needs approval
    pub fn needs_step_approval(&self, risk_level: RiskLevel) -> bool {
        match self {
            AutonomyLevel::Supervised => true, // Every action needs approval
            AutonomyLevel::Guided => false,    // Only plan approval needed, steps auto-execute
            AutonomyLevel::Autonomous => risk_level == RiskLevel::High, // Only high-risk needs approval
            AutonomyLevel::FullAuto => false,                           // No approvals needed
        }
    }

    /// Returns true if validation results need approval
    pub fn needs_validation_approval(&self) -> bool {
        match self {
            AutonomyLevel::Supervised => true,
            AutonomyLevel::Guided | AutonomyLevel::Autonomous => false,
            AutonomyLevel::FullAuto => false,
        }
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskState::Created => write!(f, "CREATED"),
            TaskState::Enriched => write!(f, "ENRICHED"),
            TaskState::Ready => write!(f, "READY"),
            TaskState::Assigned => write!(f, "ASSIGNED"),
            TaskState::Executing => write!(f, "EXECUTING"),
            TaskState::Paused => write!(f, "PAUSED"),
            TaskState::Validating => write!(f, "VALIDATING"),
            TaskState::Accepted => write!(f, "ACCEPTED"),
            TaskState::Rejected => write!(f, "REJECTED"),
            TaskState::Failed => write!(f, "FAILED"),
            TaskState::Escalated => write!(f, "ESCALATED"),
        }
    }
}

/// A unit of work to be executed by the orchestrator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub description: String,
    pub state: TaskState,
    pub source: TaskSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub assigned_agent: Option<AgentId>,
    pub plan: Option<Plan>,
    pub dependencies: Vec<Uuid>,
    pub dependents: Vec<Uuid>,
    pub iteration_count: u32,
    pub token_budget: u64,
    pub tokens_used: u64,
    pub validation_result: Option<ValidationResult>,
    /// Autonomy level controlling approval requirements
    #[serde(default)]
    pub autonomy_level: AutonomyLevel,
    /// Reason for pause (set when state is Paused)
    #[serde(default)]
    pub paused_reason: Option<String>,
    /// Instructions injected by operator mid-task
    #[serde(default)]
    pub injected_instructions: Vec<String>,
    /// Original scope boundaries (for modify_scope)
    #[serde(default)]
    pub original_scope: Option<TaskScope>,
    /// Current scope boundaries
    #[serde(default)]
    pub current_scope: TaskScope,
}

/// Scope defining task boundaries for modification
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskScope {
    /// Files in scope
    pub files: Vec<String>,
    /// Directories in scope
    pub directories: Vec<String>,
    /// Allowed operations
    pub allowed_operations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskSource {
    UserRequest,
    PlanDecomposition,
    FailureDerived {
        original_task_id: Uuid,
        failure_signal: String,
    },
    SpecGap {
        spec_id: Uuid,
    },
}

impl Task {
    pub fn new(description: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            description,
            state: TaskState::Created,
            source: TaskSource::UserRequest,
            created_at: now,
            updated_at: now,
            assigned_agent: None,
            plan: None,
            dependencies: Vec::new(),
            dependents: Vec::new(),
            iteration_count: 0,
            token_budget: 1_000_000, // 1M tokens default
            tokens_used: 0,
            validation_result: None,
            autonomy_level: AutonomyLevel::default(),
            paused_reason: None,
            injected_instructions: Vec::new(),
            original_scope: None,
            current_scope: TaskScope::default(),
        }
    }

    /// Create a new task with a specific autonomy level
    pub fn with_autonomy_level(description: String, autonomy_level: AutonomyLevel) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            description,
            state: TaskState::Created,
            source: TaskSource::UserRequest,
            created_at: now,
            updated_at: now,
            assigned_agent: None,
            plan: None,
            dependencies: Vec::new(),
            dependents: Vec::new(),
            iteration_count: 0,
            token_budget: 1_000_000,
            tokens_used: 0,
            validation_result: None,
            autonomy_level,
            paused_reason: None,
            injected_instructions: Vec::new(),
            original_scope: None,
            current_scope: TaskScope::default(),
        }
    }

    pub fn transition_to(&mut self, new_state: TaskState) {
        tracing::info!(task_id = %self.id, from = %self.state, to = %new_state, "Task state transition");
        self.state = new_state;
        self.updated_at = Utc::now();
    }
}

/// Plan produced by the Planner agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub task_id: Uuid,
    pub steps: Vec<PlanStep>,
    pub total_estimated_tokens: u64,
    pub risk_assessment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: Uuid,
    pub description: String,
    pub affected_files: Vec<String>,
    pub expected_tests: Vec<String>,
    pub risk_level: RiskLevel,
    pub dependencies: Vec<Uuid>,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
    Failed,
}

/// Agent identifiers
pub type AgentId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Generator,
    Evaluator,
    Coder,
    TestWriter,
    Reviewer,
    Refactorer,
    DocWriter,
}

/// Agent definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub role: AgentRole,
    pub model: String,
    pub iteration_budget: u32,
    pub current_task: Option<Uuid>,
}

impl Agent {
    pub fn new(role: AgentRole, model: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            model,
            iteration_budget: 5,
            current_task: None,
        }
    }
}

/// Tool definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub risk_level: ToolRiskLevel,
    pub permission_tier: PermissionTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolRiskLevel {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionTier {
    Auto, // Approved automatically
    Ask,  // Requires user confirmation
    Deny, // Never allowed without explicit override
}

/// Validation result from the validation pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub passed: bool,
    pub lint_passed: bool,
    pub tests_passed: bool,
    pub security_passed: bool,
    pub ai_review_passed: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Memory block types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub id: Uuid,
    pub label: String,
    pub description: String,
    pub content: String,
    pub block_type: MemoryBlockType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryBlockType {
    Project,
    User,
    Task,
    Skill,
    Convention,
}

impl MemoryBlock {
    pub fn new(label: String, block_type: MemoryBlockType, content: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            description: String::new(),
            label,
            content,
            block_type,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Safety and cost tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyState {
    pub cost_guard: CostGuard,
    pub doom_loop_detected: bool,
    pub consecutive_failures: u32,
    pub kill_switch_triggered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostGuard {
    pub budget_limit: u64,
    pub spent: u64,
    pub warning_threshold: f64, // 0.0 to 1.0
    pub hard_stop_threshold: f64,
}

impl CostGuard {
    pub fn new(budget_limit: u64) -> Self {
        Self {
            budget_limit,
            spent: 0,
            warning_threshold: 0.75,
            hard_stop_threshold: 1.0,
        }
    }

    pub fn add_cost(&mut self, tokens: u64) {
        self.spent += tokens;
        tracing::debug!(
            spent = self.spent,
            limit = self.budget_limit,
            "CostGuard updated"
        );
    }

    pub fn is_warning_threshold(&self) -> bool {
        let ratio = self.spent as f64 / self.budget_limit as f64;
        ratio >= self.warning_threshold && ratio < self.hard_stop_threshold
    }

    pub fn is_hard_stop(&self) -> bool {
        let ratio = self.spent as f64 / self.budget_limit as f64;
        ratio >= self.hard_stop_threshold
    }
}

/// CLI <-> Daemon protocol messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum CliCommand {
    TaskCreate {
        description: String,
    },
    TaskApprove {
        task_id: Uuid,
    },
    TaskReject {
        task_id: Uuid,
        reason: String,
    },
    TaskCancel {
        task_id: Uuid,
    },
    TaskList,
    TaskWatch {
        task_id: Uuid,
    },
    /// Pause a task (operator intervention)
    TaskPause {
        task_id: Uuid,
        reason: String,
    },
    /// Resume a paused task (operator intervention)
    TaskResume {
        task_id: Uuid,
    },
    /// Inject instructions into a running task (operator intervention)
    TaskInjectInstruction {
        task_id: Uuid,
        instruction: String,
    },
    /// Modify task scope boundaries (operator intervention)
    TaskModifyScope {
        task_id: Uuid,
        scope: TaskScope,
    },
}

/// A correlation ID used to track related events across the system.
/// Events that are part of the same operation (e.g., a task lifecycle)
/// share the same correlation ID.
pub type CorrelationId = Uuid;

/// CLI <-> Daemon protocol messages
/// Events include a correlation ID to track related events across operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonEvent {
    TaskCreated {
        id: Uuid,
        correlation_id: CorrelationId,
    },
    TaskStateChanged {
        id: Uuid,
        state: TaskState,
        correlation_id: CorrelationId,
    },
    TaskProgress {
        id: Uuid,
        message: String,
        correlation_id: CorrelationId,
    },
    TaskCompleted {
        id: Uuid,
        pr_url: Option<String>,
        correlation_id: CorrelationId,
    },
    TaskFailed {
        id: Uuid,
        error: String,
        correlation_id: CorrelationId,
    },
    Error {
        message: String,
        correlation_id: CorrelationId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_guard_add_cost() {
        let mut guard = CostGuard::new(1_000_000);
        assert_eq!(guard.spent, 0);

        guard.add_cost(100_000);
        assert_eq!(guard.spent, 100_000);

        guard.add_cost(50_000);
        assert_eq!(guard.spent, 150_000);
    }

    #[test]
    fn test_cost_guard_warning_threshold() {
        let mut guard = CostGuard::new(1_000_000);

        // At 0% - no warning
        assert!(!guard.is_warning_threshold());

        // At 74% - no warning
        guard.add_cost(740_000);
        assert_eq!(guard.spent, 740_000);
        assert!(!guard.is_warning_threshold());

        // At 75% - warning threshold triggered
        guard.add_cost(10_000);
        assert_eq!(guard.spent, 750_000);
        assert!(guard.is_warning_threshold());

        // At 99% - still warning
        guard.add_cost(240_000);
        assert_eq!(guard.spent, 990_000);
        assert!(guard.is_warning_threshold());
    }

    #[test]
    fn test_cost_guard_hard_stop() {
        let mut guard = CostGuard::new(1_000_000);

        // At 0% - no hard stop
        assert!(!guard.is_hard_stop());

        // At 99% - no hard stop
        guard.add_cost(990_000);
        assert_eq!(guard.spent, 990_000);
        assert!(!guard.is_hard_stop());

        // At 100% - hard stop triggered
        guard.add_cost(10_000);
        assert_eq!(guard.spent, 1_000_000);
        assert!(guard.is_hard_stop());

        // At 150% - still hard stop
        guard.add_cost(500_000);
        assert_eq!(guard.spent, 1_500_000);
        assert!(guard.is_hard_stop());
    }

    #[test]
    fn test_cost_guard_warning_before_hard_stop() {
        let mut guard = CostGuard::new(1_000_000);

        // At 75%, warning should be true but hard stop should be false
        guard.add_cost(750_000);
        assert_eq!(guard.spent, 750_000);
        assert!(guard.is_warning_threshold());
        assert!(!guard.is_hard_stop());

        // Reset and test at exactly 100%
        let mut guard = CostGuard::new(1_000_000);
        guard.add_cost(1_000_000);
        assert!(!guard.is_warning_threshold());
        assert!(guard.is_hard_stop());
    }
}
