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
    Validating,
    Accepted,
    Rejected,
    Failed,
    Escalated,
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskState::Created => write!(f, "CREATED"),
            TaskState::Enriched => write!(f, "ENRICHED"),
            TaskState::Ready => write!(f, "READY"),
            TaskState::Assigned => write!(f, "ASSIGNED"),
            TaskState::Executing => write!(f, "EXECUTING"),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskSource {
    UserRequest,
    PlanDecomposition,
    FailureDerived { original_task_id: Uuid, failure_signal: String },
    SpecGap { spec_id: Uuid },
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
    Auto,  // Approved automatically
    Ask,   // Requires user confirmation
    Deny,  // Never allowed without explicit override
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
        tracing::debug!(spent = self.spent, limit = self.budget_limit, "CostGuard updated");
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
    TaskCreate { description: String },
    TaskApprove { task_id: Uuid },
    TaskReject { task_id: Uuid, reason: String },
    TaskCancel { task_id: Uuid },
    TaskList,
    TaskWatch { task_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonEvent {
    TaskCreated(Uuid),
    TaskStateChanged { id: Uuid, state: TaskState },
    TaskProgress { id: Uuid, message: String },
    TaskCompleted { id: Uuid, pr_url: Option<String> },
    TaskFailed { id: Uuid, error: String },
    Error { message: String },
}
