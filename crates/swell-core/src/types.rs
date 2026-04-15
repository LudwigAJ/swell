use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::cost_tracking::ModelCostInfo;

/// Task lifecycle states as defined in the orchestrator spec
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Created,
    Enriched,
    AwaitingApproval, // Waiting for user approval before execution
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
            TaskState::AwaitingApproval => write!(f, "AWAITING_APPROVAL"),
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
    /// State before pause (for resume restoration)
    #[serde(default)]
    pub paused_from_state: Option<TaskState>,
    /// Reason for rejection (set when state is Rejected)
    #[serde(default)]
    pub rejected_reason: Option<String>,
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

/// Specification for task execution policies and validation requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Tests that must pass for task completion (test names or globs)
    #[serde(default)]
    pub acceptance_tests: Vec<String>,
    /// Policy for committing changes
    #[serde(default)]
    pub commit_policy: CommitPolicy,
    /// Policy for escalating issues
    #[serde(default)]
    pub escalation_policy: EscalationPolicy,
}

/// Policy for when and how to commit changes
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum CommitPolicy {
    /// Commit after each step completes
    EveryStep,
    /// Commit only at the end after all validation passes
    #[default]
    AfterValidation,
    /// Never commit automatically
    Never,
    /// Custom commit rules
    Custom {
        /// Minimum number of steps between commits
        min_steps_between_commits: u32,
        /// Whether to require a clean diff before committing
        require_clean_diff: bool,
    },
}

/// Policy for when to escalate issues to a human
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum EscalationPolicy {
    /// Never escalate automatically
    #[default]
    Never,
    /// Escalate after N consecutive failures
    AfterConsecutiveFailures(u32),
    /// Escalate when task budget is exceeded
    OnBudgetExceeded,
    /// Escalate when specific error patterns are detected
    OnErrorPatterns(Vec<String>),
}

impl TaskSpec {
    /// Create a new TaskSpec with validation
    ///
    /// # Errors
    /// Returns an error if:
    /// - `acceptance_tests` contains empty strings
    /// - `CommitPolicy::Custom` has `min_steps_between_commits` set to 0
    /// - `EscalationPolicy::AfterConsecutiveFailures` is set to 0
    /// - `EscalationPolicy::OnErrorPatterns` contains empty patterns
    pub fn new(
        acceptance_tests: Vec<String>,
        commit_policy: CommitPolicy,
        escalation_policy: EscalationPolicy,
    ) -> Result<Self, TaskSpecValidationError> {
        // Validate acceptance_tests
        if acceptance_tests.iter().any(|t| t.trim().is_empty()) {
            return Err(TaskSpecValidationError::EmptyAcceptanceTest);
        }

        // Validate commit_policy
        if let CommitPolicy::Custom {
            min_steps_between_commits,
            ..
        } = &commit_policy
        {
            if *min_steps_between_commits == 0 {
                return Err(TaskSpecValidationError::InvalidCommitPolicy(
                    "min_steps_between_commits cannot be 0".to_string(),
                ));
            }
        }

        // Validate escalation_policy
        match &escalation_policy {
            EscalationPolicy::AfterConsecutiveFailures(n) if *n == 0 => {
                return Err(TaskSpecValidationError::InvalidEscalationPolicy(
                    "consecutive failures threshold cannot be 0".to_string(),
                ));
            }
            EscalationPolicy::OnErrorPatterns(patterns) => {
                if patterns.iter().any(|p| p.trim().is_empty()) {
                    return Err(TaskSpecValidationError::InvalidEscalationPolicy(
                        "error patterns cannot be empty strings".to_string(),
                    ));
                }
            }
            _ => {}
        }

        Ok(Self {
            acceptance_tests,
            commit_policy,
            escalation_policy,
        })
    }

    /// Create a TaskSpec with default/empty policies
    pub fn default_spec() -> Self {
        Self {
            acceptance_tests: Vec::new(),
            commit_policy: CommitPolicy::AfterValidation,
            escalation_policy: EscalationPolicy::Never,
        }
    }
}

/// Errors that can occur during TaskSpec validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSpecValidationError {
    /// An acceptance test name was empty
    EmptyAcceptanceTest,
    /// The commit policy had invalid configuration
    InvalidCommitPolicy(String),
    /// The escalation policy had invalid configuration
    InvalidEscalationPolicy(String),
}

impl std::fmt::Display for TaskSpecValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskSpecValidationError::EmptyAcceptanceTest => {
                write!(f, "acceptance test names cannot be empty")
            }
            TaskSpecValidationError::InvalidCommitPolicy(msg) => {
                write!(f, "invalid commit policy: {}", msg)
            }
            TaskSpecValidationError::InvalidEscalationPolicy(msg) => {
                write!(f, "invalid escalation policy: {}", msg)
            }
        }
    }
}

impl std::error::Error for TaskSpecValidationError {}

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
            paused_from_state: None,
            rejected_reason: None,
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
            paused_from_state: None,
            rejected_reason: None,
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
    Researcher,
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

/// Scope filters for memory queries
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryQueryScope {
    /// Filter by session ID
    pub session_id: Option<Uuid>,
    /// Filter by task ID
    pub task_id: Option<Uuid>,
    /// Filter by agent role
    pub agent_role: Option<String>,
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
    /// Get full task details by ID
    TaskGet {
        task_id: Uuid,
    },
    /// Get daemon health status including active connections, task counts, cost, and MCP health
    DaemonStatus,
    /// Get a configuration value by key
    ConfigGet {
        key: String,
    },
    /// Set a configuration value (writes to settings.local.json)
    ConfigSet {
        key: String,
        value: serde_json::Value,
    },
    /// Query memory with BM25 search and temporal filters
    MemoryQuery {
        /// Keywords to search for
        query: String,
        /// Scope filters for the query
        scope: MemoryQueryScope,
        /// Maximum number of results
        limit: usize,
    },
    /// Query cost data for a specific task or aggregate across all tasks
    CostQuery {
        /// Task ID to query cost for. If None, returns aggregate cost across all tasks.
        task_id: Option<Uuid>,
    },
}

/// A correlation ID used to track related events across the system.
/// Events that are part of the same operation (e.g., a task lifecycle)
/// share the same correlation ID.
pub type CorrelationId = Uuid;

/// Classification of failures for error handling and recovery.
///
/// This enum provides typed error categories enabling granular failure handling,
/// retry decisions, and escalation policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Network connectivity issues (connection refused, timeout, DNS failure)
    NetworkError,
    /// LLM API errors (rate limits, model errors, API key issues)
    LlmError,
    /// Tool execution failures (tool not found, invalid input, execution error)
    ToolError,
    /// Permission denied by policy or access control
    PermissionDenied,
    /// Budget or cost limit exceeded
    BudgetExceeded,
    /// Operation timed out
    Timeout,
    /// Rate limited by external service
    RateLimited,
    /// Invalid state transition or state machine error
    InvalidState,
    /// Parse error in input or response
    ParseError,
    /// Configuration error (missing config, invalid config)
    ConfigError,
    /// Sandbox isolation error
    SandboxError,
    /// Internal error (bugs, unexpected state)
    InternalError,
}

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
        failure_class: Option<FailureClass>,
        correlation_id: CorrelationId,
    },
    Error {
        message: String,
        failure_class: Option<FailureClass>,
        correlation_id: CorrelationId,
    },
    /// A tool invocation started during agent execution
    ToolInvocationStarted {
        id: Uuid,
        tool_name: String,
        arguments: serde_json::Value,
        turn_number: u32,
        correlation_id: CorrelationId,
    },
    /// A tool invocation completed during agent execution
    ToolInvocationCompleted {
        id: Uuid,
        tool_name: String,
        success: bool,
        duration_ms: u64,
        turn_number: u32,
        correlation_id: CorrelationId,
    },
    /// An agent turn started
    AgentTurnStarted {
        id: Uuid,
        agent_role: String,
        turn_number: u32,
        correlation_id: CorrelationId,
    },
    /// An agent turn completed
    AgentTurnCompleted {
        id: Uuid,
        agent_role: String,
        turn_number: u32,
        action_taken: String,
        tools_invoked: Vec<String>,
        duration_ms: u64,
        correlation_id: CorrelationId,
    },
    /// A validation step started
    ValidationStepStarted {
        id: Uuid,
        step_name: String,
        correlation_id: CorrelationId,
    },
    /// A validation step completed
    ValidationStepCompleted {
        id: Uuid,
        step_name: String,
        passed: bool,
        duration_ms: u64,
        correlation_id: CorrelationId,
    },
    /// Full task details returned by TaskGet command
    TaskDetails {
        id: Uuid,
        /// JSON serialized Task object containing all task fields
        task_json: String,
        correlation_id: CorrelationId,
    },
    /// Daemon health status response
    DaemonHealth {
        /// Number of active CLI connections
        active_connections: usize,
        /// Total number of tasks in the system
        total_tasks: usize,
        /// Number of tasks in each state
        tasks_by_state: HashMap<String, usize>,
        /// Total LLM tokens used in this session
        total_tokens: u64,
        /// Last LLM model used
        last_model: String,
        /// MCP server health status (server name -> health)
        mcp_health: HashMap<String, String>,
        /// Daemon uptime in seconds
        uptime_seconds: u64,
        /// Daemon version string
        version: String,
        /// Total budget (tokens) configured for the session
        total_budget: u64,
        /// Total tokens spent so far
        total_spent: u64,
        /// Remaining budget (tokens)
        remaining_budget: u64,
        correlation_id: CorrelationId,
    },
    /// Configuration value response
    ConfigValue {
        key: String,
        value: serde_json::Value,
        source_file: Option<String>,
        correlation_id: CorrelationId,
    },
    /// Memory search results
    MemoryResults {
        /// JSON serialized recall results
        results: String,
        /// Number of results returned
        count: usize,
        correlation_id: CorrelationId,
    },
    /// Cost query results for a task or aggregate
    CostQueryResult {
        /// Task ID queried (None if aggregate)
        task_id: Option<Uuid>,
        /// Total input tokens
        total_input_tokens: u64,
        /// Total output tokens
        total_output_tokens: u64,
        /// Total cost in USD
        total_cost_usd: f64,
        /// Per-model cost breakdown
        model_breakdown: Vec<ModelCostInfo>,
        correlation_id: CorrelationId,
    },
    /// Typed response payloads for daemon query commands.
    ///
    /// This replaces the previous pattern of using `TaskCompleted` with `Uuid::nil()`
    /// to indicate query responses. Each variant carries typed data appropriate to
    /// the query type.
    ///
    /// # Variants
    /// - `TaskList`: Response for `TaskList` command - list of all tasks
    /// - `TaskDetail`: Response for `TaskGet` command - full task details
    /// - `ConfigValue`: Response for `ConfigGet`/`ConfigSet` commands
    /// - `MemoryResults`: Response for `MemoryQuery` command
    /// - `CostData`: Response for `CostQuery` command
    /// - `DaemonHealth`: Response for `DaemonStatus` command
    DataResponse(Box<DataResponse>),
}

/// Typed response payloads for daemon query commands.
///
/// This enum provides structured, typed payloads for all daemon query responses,
/// replacing the previous pattern of misusing `TaskCompleted` with `Uuid::nil()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub enum DataResponse {
    /// Task list response - returned when querying all tasks
    TaskList {
        /// List of all tasks
        tasks: Vec<Task>,
        correlation_id: CorrelationId,
    },
    /// Task detail response - returned when querying a specific task's full details
    TaskDetail {
        /// The full task object with all fields
        task: Task,
        correlation_id: CorrelationId,
    },
    /// Configuration value response - returned by ConfigGet and ConfigSet
    ConfigValue {
        /// Configuration key
        key: String,
        /// Configuration value
        value: serde_json::Value,
        /// Source file where this config was loaded from
        source_file: Option<String>,
        correlation_id: CorrelationId,
    },
    /// Memory query results - returned by MemoryQuery
    MemoryResults {
        /// JSON serialized recall results
        results: String,
        /// Number of results returned
        count: usize,
        correlation_id: CorrelationId,
    },
    /// Cost data response - returned by CostQuery
    CostData {
        /// Task ID queried (None if aggregate across all tasks)
        task_id: Option<Uuid>,
        /// Total input tokens used
        total_input_tokens: u64,
        /// Total output tokens generated
        total_output_tokens: u64,
        /// Total cost in USD
        total_cost_usd: f64,
        /// Per-model cost breakdown
        model_breakdown: Vec<ModelCostInfo>,
        correlation_id: CorrelationId,
    },
    /// Daemon health response - returned by DaemonStatus
    DaemonHealth {
        /// Number of active CLI connections
        active_connections: usize,
        /// Total number of tasks in the system
        total_tasks: usize,
        /// Number of tasks in each state
        tasks_by_state: HashMap<String, usize>,
        /// Total LLM tokens used in this session
        total_tokens: u64,
        /// Last LLM model used
        last_model: String,
        /// MCP server health status (server name -> health)
        mcp_health: HashMap<String, String>,
        /// Daemon uptime in seconds
        uptime_seconds: u64,
        /// Daemon version string
        version: String,
        /// Total budget (tokens) configured for the session
        total_budget: u64,
        /// Total tokens spent so far
        total_spent: u64,
        /// Remaining budget (tokens)
        remaining_budget: u64,
        correlation_id: CorrelationId,
    },
}

// ============================================================================
// LLM Streaming Events
// ============================================================================

/// Events emitted during LLM streaming responses.
///
/// Used by the `chat_streaming` method on `LlmBackend` to provide
/// real-time updates as the model generates content.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum StreamEvent {
    /// A text delta event - partial text content
    TextDelta {
        /// The accumulated text so far
        text: String,
        /// The delta (增量) of text in this event
        delta: String,
    },

    /// A tool use event - the model wants to call a tool
    ToolUse {
        /// The complete tool call with name and arguments
        tool_call: LlmToolCall,
    },

    /// A tool result event - result from executing a tool
    ToolResult {
        /// The tool call ID this result corresponds to
        tool_call_id: String,
        /// The result content
        result: String,
        /// Whether the tool execution was successful
        success: bool,
    },

    /// Usage statistics from the streaming response
    Usage {
        /// Tokens used in the prompt/input
        input_tokens: u64,
        /// Tokens generated in the response/output
        output_tokens: u64,
        /// Tokens written to provider-managed cache (Anthropic)
        cache_creation_input_tokens: Option<u64>,
        /// Tokens read from provider-managed cache (Anthropic)
        cache_read_input_tokens: Option<u64>,
    },

    /// Stream completion event
    MessageStop {
        /// The reason the stream ended (e.g., "end_turn", "max_tokens", "stop_sequence")
        stop_reason: Option<String>,
    },

    /// Error event during streaming
    Error {
        /// Error message
        message: String,
    },
}

/// A tool call request from an LLM (used in streaming context)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
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

    // =============================================================================
    // TaskSpec tests
    // =============================================================================

    #[test]
    fn test_task_spec_default_spec() {
        let spec = TaskSpec::default_spec();
        assert!(spec.acceptance_tests.is_empty());
        assert_eq!(spec.commit_policy, CommitPolicy::AfterValidation);
        assert_eq!(spec.escalation_policy, EscalationPolicy::Never);
    }

    #[test]
    fn test_task_spec_valid_creation() {
        let spec = TaskSpec::new(
            vec!["test_foo".to_string(), "test_bar".to_string()],
            CommitPolicy::EveryStep,
            EscalationPolicy::AfterConsecutiveFailures(3),
        )
        .expect("valid spec should be created");
        assert_eq!(spec.acceptance_tests.len(), 2);
    }

    #[test]
    fn test_task_spec_valid_custom_commit_policy() {
        let spec = TaskSpec::new(
            vec!["test_foo".to_string()],
            CommitPolicy::Custom {
                min_steps_between_commits: 2,
                require_clean_diff: true,
            },
            EscalationPolicy::Never,
        )
        .expect("valid spec should be created");
        match spec.commit_policy {
            CommitPolicy::Custom {
                min_steps_between_commits,
                require_clean_diff,
            } => {
                assert_eq!(min_steps_between_commits, 2);
                assert!(require_clean_diff);
            }
            _ => panic!("expected Custom commit policy"),
        }
    }

    #[test]
    fn test_task_spec_valid_error_patterns() {
        let spec = TaskSpec::new(
            vec![],
            CommitPolicy::Never,
            EscalationPolicy::OnErrorPatterns(vec![
                "error_oom".to_string(),
                "error_timeout".to_string(),
            ]),
        )
        .expect("valid spec should be created");
        match spec.escalation_policy {
            EscalationPolicy::OnErrorPatterns(patterns) => {
                assert_eq!(patterns.len(), 2);
            }
            _ => panic!("expected OnErrorPatterns escalation policy"),
        }
    }

    #[test]
    fn test_task_spec_rejects_empty_acceptance_test() {
        let result = TaskSpec::new(
            vec![
                "test_foo".to_string(),
                "".to_string(),
                "test_bar".to_string(),
            ],
            CommitPolicy::AfterValidation,
            EscalationPolicy::Never,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            TaskSpecValidationError::EmptyAcceptanceTest
        );
    }

    #[test]
    fn test_task_spec_rejects_zero_commit_interval() {
        let result = TaskSpec::new(
            vec!["test_foo".to_string()],
            CommitPolicy::Custom {
                min_steps_between_commits: 0,
                require_clean_diff: false,
            },
            EscalationPolicy::Never,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            TaskSpecValidationError::InvalidCommitPolicy(
                "min_steps_between_commits cannot be 0".to_string()
            )
        );
    }

    #[test]
    fn test_task_spec_rejects_zero_failure_threshold() {
        let result = TaskSpec::new(
            vec![],
            CommitPolicy::AfterValidation,
            EscalationPolicy::AfterConsecutiveFailures(0),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            TaskSpecValidationError::InvalidEscalationPolicy(
                "consecutive failures threshold cannot be 0".to_string()
            )
        );
    }

    #[test]
    fn test_task_spec_rejects_empty_error_pattern() {
        let result = TaskSpec::new(
            vec![],
            CommitPolicy::Never,
            EscalationPolicy::OnErrorPatterns(vec!["error_oom".to_string(), "".to_string()]),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            TaskSpecValidationError::InvalidEscalationPolicy(
                "error patterns cannot be empty strings".to_string()
            )
        );
    }

    #[test]
    fn test_task_spec_serde_roundtrip_every_step() {
        let spec = TaskSpec::new(
            vec!["test_a".to_string(), "test_b".to_string()],
            CommitPolicy::EveryStep,
            EscalationPolicy::AfterConsecutiveFailures(5),
        )
        .expect("valid spec");

        let serialized = serde_json::to_string(&spec).expect("should serialize");
        let deserialized: TaskSpec = serde_json::from_str(&serialized).expect("should deserialize");

        assert_eq!(spec.acceptance_tests, deserialized.acceptance_tests);
        assert_eq!(spec.commit_policy, deserialized.commit_policy);
        assert_eq!(spec.escalation_policy, deserialized.escalation_policy);
    }

    #[test]
    fn test_task_spec_serde_roundtrip_custom_policy() {
        let spec = TaskSpec::new(
            vec!["test_*.rs".to_string()],
            CommitPolicy::Custom {
                min_steps_between_commits: 3,
                require_clean_diff: true,
            },
            EscalationPolicy::OnErrorPatterns(vec!["E001".to_string(), "E002".to_string()]),
        )
        .expect("valid spec");

        let serialized = serde_json::to_string(&spec).expect("should serialize");
        let deserialized: TaskSpec = serde_json::from_str(&serialized).expect("should deserialize");

        assert_eq!(spec.acceptance_tests, deserialized.acceptance_tests);
        match (&spec.commit_policy, &deserialized.commit_policy) {
            (
                CommitPolicy::Custom {
                    min_steps_between_commits: a,
                    require_clean_diff: b,
                },
                CommitPolicy::Custom {
                    min_steps_between_commits: c,
                    require_clean_diff: d,
                },
            ) => {
                assert_eq!(a, c);
                assert_eq!(b, d);
            }
            _ => panic!("expected Custom commit policy in roundtrip"),
        }
        assert_eq!(spec.escalation_policy, deserialized.escalation_policy);
    }

    #[test]
    fn test_task_spec_serde_roundtrip_default_spec() {
        let spec = TaskSpec::default_spec();

        let serialized = serde_json::to_string(&spec).expect("should serialize");
        let deserialized: TaskSpec = serde_json::from_str(&serialized).expect("should deserialize");

        assert_eq!(spec.acceptance_tests, deserialized.acceptance_tests);
        assert_eq!(spec.commit_policy, deserialized.commit_policy);
        assert_eq!(spec.escalation_policy, deserialized.escalation_policy);
    }

    // =========================================================================
    // FailureClass Tests
    // =========================================================================

    #[test]
    fn test_failure_class_count() {
        // Verify we have at least 12 failure classes as required
        let variants = [
            FailureClass::NetworkError,
            FailureClass::LlmError,
            FailureClass::ToolError,
            FailureClass::PermissionDenied,
            FailureClass::BudgetExceeded,
            FailureClass::Timeout,
            FailureClass::RateLimited,
            FailureClass::InvalidState,
            FailureClass::ParseError,
            FailureClass::ConfigError,
            FailureClass::SandboxError,
            FailureClass::InternalError,
        ];
        assert_eq!(
            variants.len(),
            12,
            "FailureClass must have at least 12 variants"
        );
    }

    #[test]
    fn test_failure_class_exhaustive_match() {
        // This test ensures all variants are handled - if a new variant is added
        // without updating this test, it will fail with a non-exhaustive match warning
        let check_class = |fc: FailureClass| match fc {
            FailureClass::NetworkError => "NetworkError",
            FailureClass::LlmError => "LlmError",
            FailureClass::ToolError => "ToolError",
            FailureClass::PermissionDenied => "PermissionDenied",
            FailureClass::BudgetExceeded => "BudgetExceeded",
            FailureClass::Timeout => "Timeout",
            FailureClass::RateLimited => "RateLimited",
            FailureClass::InvalidState => "InvalidState",
            FailureClass::ParseError => "ParseError",
            FailureClass::ConfigError => "ConfigError",
            FailureClass::SandboxError => "SandboxError",
            FailureClass::InternalError => "InternalError",
        };

        for fc in [
            FailureClass::NetworkError,
            FailureClass::LlmError,
            FailureClass::ToolError,
            FailureClass::PermissionDenied,
            FailureClass::BudgetExceeded,
            FailureClass::Timeout,
            FailureClass::RateLimited,
            FailureClass::InvalidState,
            FailureClass::ParseError,
            FailureClass::ConfigError,
            FailureClass::SandboxError,
            FailureClass::InternalError,
        ] {
            let name = check_class(fc);
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn test_failure_class_serde_roundtrip() {
        let classes = [
            FailureClass::NetworkError,
            FailureClass::LlmError,
            FailureClass::ToolError,
            FailureClass::PermissionDenied,
            FailureClass::BudgetExceeded,
            FailureClass::Timeout,
            FailureClass::RateLimited,
            FailureClass::InvalidState,
            FailureClass::ParseError,
            FailureClass::ConfigError,
            FailureClass::SandboxError,
            FailureClass::InternalError,
        ];

        for original in classes {
            let json = serde_json::to_string(&original).expect("should serialize");
            let deserialized: FailureClass =
                serde_json::from_str(&json).expect("should deserialize");
            assert_eq!(
                original, deserialized,
                "Roundtrip failed for {:?}",
                original
            );
        }
    }

    #[test]
    fn test_failure_class_snake_case_serialization() {
        let original = FailureClass::PermissionDenied;
        let json = serde_json::to_string(&original).expect("should serialize");
        assert_eq!(json, "\"permission_denied\"", "Should use snake_case");
    }

    #[test]
    fn test_failure_class_deserialization_from_snake_case() {
        let json = "\"rate_limited\"";
        let deserialized: FailureClass = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(deserialized, FailureClass::RateLimited);
    }

    // =========================================================================
    // DaemonEvent with FailureClass Tests
    // =========================================================================

    #[test]
    fn test_daemon_event_task_failed_with_failure_class() {
        let task_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::TaskFailed {
            id: task_id,
            error: "Connection refused".to_string(),
            failure_class: Some(FailureClass::NetworkError),
            correlation_id,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

        match deserialized {
            DaemonEvent::TaskFailed {
                id,
                error,
                failure_class,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(error, "Connection refused");
                assert_eq!(failure_class, Some(FailureClass::NetworkError));
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected TaskFailed event, got: {:?}", other),
        }
    }

    #[test]
    fn test_daemon_event_task_failed_without_failure_class() {
        let task_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::TaskFailed {
            id: task_id,
            error: "Something went wrong".to_string(),
            failure_class: None,
            correlation_id,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

        match deserialized {
            DaemonEvent::TaskFailed { failure_class, .. } => {
                assert!(failure_class.is_none());
            }
            other => panic!("Expected TaskFailed event, got: {:?}", other),
        }
    }

    #[test]
    fn test_daemon_event_error_with_failure_class() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::Error {
            message: "LLM rate limit exceeded".to_string(),
            failure_class: Some(FailureClass::RateLimited),
            correlation_id,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

        match deserialized {
            DaemonEvent::Error {
                message,
                failure_class,
                correlation_id: cid,
            } => {
                assert_eq!(message, "LLM rate limit exceeded");
                assert_eq!(failure_class, Some(FailureClass::RateLimited));
                assert_eq!(cid, correlation_id);
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[test]
    fn test_daemon_event_error_without_failure_class() {
        let correlation_id = Uuid::new_v4();
        let event = DaemonEvent::Error {
            message: "Generic error".to_string(),
            failure_class: None,
            correlation_id,
        };

        let json = serde_json::to_string(&event).expect("should serialize");
        let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

        match deserialized {
            DaemonEvent::Error { failure_class, .. } => {
                assert!(failure_class.is_none());
            }
            other => panic!("Expected Error event, got: {:?}", other),
        }
    }

    #[test]
    fn test_daemon_event_all_variants_serde() {
        let task_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();

        let events = vec![
            DaemonEvent::TaskCreated {
                id: task_id,
                correlation_id,
            },
            DaemonEvent::TaskStateChanged {
                id: task_id,
                state: TaskState::Executing,
                correlation_id,
            },
            DaemonEvent::TaskProgress {
                id: task_id,
                message: "Working...".to_string(),
                correlation_id,
            },
            DaemonEvent::TaskCompleted {
                id: task_id,
                pr_url: Some("https://github.com/example/pull/1".to_string()),
                correlation_id,
            },
            DaemonEvent::TaskFailed {
                id: task_id,
                error: "Failed".to_string(),
                failure_class: Some(FailureClass::ToolError),
                correlation_id,
            },
            DaemonEvent::Error {
                message: "Error occurred".to_string(),
                failure_class: Some(FailureClass::InternalError),
                correlation_id,
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).expect("should serialize");
            let _deserialized: DaemonEvent =
                serde_json::from_str(&json).expect("should deserialize");
            // Verify the JSON is valid and contains expected structure
            let parsed: serde_json::Value =
                serde_json::from_str(&json).expect("should parse as JSON");
            assert!(
                parsed.get("type").is_some(),
                "Missing 'type' field in JSON for {:?}",
                event
            );
        }
    }

    #[test]
    fn test_correlation_id_links_related_events() {
        // All events in a task lifecycle should share the same correlation_id
        let correlation_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let created = DaemonEvent::TaskCreated {
            id: task_id,
            correlation_id,
        };
        let state_changed = DaemonEvent::TaskStateChanged {
            id: task_id,
            state: TaskState::Executing,
            correlation_id,
        };
        let failed = DaemonEvent::TaskFailed {
            id: task_id,
            error: "Test failure".to_string(),
            failure_class: Some(FailureClass::InternalError),
            correlation_id,
        };

        // Extract correlation IDs and verify they match
        let check_cid = |event: &DaemonEvent| -> Uuid {
            match event {
                DaemonEvent::TaskCreated { correlation_id, .. } => *correlation_id,
                DaemonEvent::TaskStateChanged { correlation_id, .. } => *correlation_id,
                DaemonEvent::TaskFailed { correlation_id, .. } => *correlation_id,
                DaemonEvent::Error { correlation_id, .. } => *correlation_id,
                DaemonEvent::TaskProgress { correlation_id, .. } => *correlation_id,
                DaemonEvent::TaskCompleted { correlation_id, .. } => *correlation_id,
                DaemonEvent::ToolInvocationStarted { correlation_id, .. } => *correlation_id,
                DaemonEvent::ToolInvocationCompleted { correlation_id, .. } => *correlation_id,
                DaemonEvent::AgentTurnStarted { correlation_id, .. } => *correlation_id,
                DaemonEvent::AgentTurnCompleted { correlation_id, .. } => *correlation_id,
                DaemonEvent::ValidationStepStarted { correlation_id, .. } => *correlation_id,
                DaemonEvent::ValidationStepCompleted { correlation_id, .. } => *correlation_id,
                DaemonEvent::TaskDetails { correlation_id, .. } => *correlation_id,
                DaemonEvent::DaemonHealth { correlation_id, .. } => *correlation_id,
                DaemonEvent::ConfigValue { correlation_id, .. } => *correlation_id,
                DaemonEvent::MemoryResults { correlation_id, .. } => *correlation_id,
                DaemonEvent::CostQueryResult { correlation_id, .. } => *correlation_id,
                DaemonEvent::DataResponse(data) => match &**data {
                    DataResponse::TaskList { correlation_id, .. } => *correlation_id,
                    DataResponse::TaskDetail { correlation_id, .. } => *correlation_id,
                    DataResponse::ConfigValue { correlation_id, .. } => *correlation_id,
                    DataResponse::MemoryResults { correlation_id, .. } => *correlation_id,
                    DataResponse::CostData { correlation_id, .. } => *correlation_id,
                    DataResponse::DaemonHealth { correlation_id, .. } => *correlation_id,
                },
            }
        };

        assert_eq!(check_cid(&created), correlation_id);
        assert_eq!(check_cid(&state_changed), correlation_id);
        assert_eq!(check_cid(&failed), correlation_id);
    }

    #[test]
    fn test_different_tasks_have_different_correlation_ids() {
        let correlation_id_1 = Uuid::new_v4();
        let correlation_id_2 = Uuid::new_v4();
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();

        let event_1 = DaemonEvent::TaskFailed {
            id: task_id_1,
            error: "Error 1".to_string(),
            failure_class: Some(FailureClass::NetworkError),
            correlation_id: correlation_id_1,
        };
        let event_2 = DaemonEvent::TaskFailed {
            id: task_id_2,
            error: "Error 2".to_string(),
            failure_class: Some(FailureClass::LlmError),
            correlation_id: correlation_id_2,
        };

        // Correlation IDs should be different
        assert_ne!(correlation_id_1, correlation_id_2);

        // Verify in JSON that correlation IDs are different
        let json_1 = serde_json::to_string(&event_1).expect("should serialize");
        let json_2 = serde_json::to_string(&event_2).expect("should serialize");

        let parsed_1: serde_json::Value = serde_json::from_str(&json_1).expect("should parse");
        let parsed_2: serde_json::Value = serde_json::from_str(&json_2).expect("should parse");

        // The payload should have different correlation IDs
        let payload_1_cid = parsed_1
            .get("payload")
            .and_then(|p| p.get("correlation_id"));
        let payload_2_cid = parsed_2
            .get("payload")
            .and_then(|p| p.get("correlation_id"));

        assert_ne!(payload_1_cid, payload_2_cid);
    }

    #[test]
    fn test_data_response_serde_roundtrip() {
        use std::collections::HashMap;

        let correlation_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        // Test TaskList roundtrip
        let tasks = vec![Task {
            id: Uuid::new_v4(),
            description: "Test task 1".to_string(),
            state: TaskState::Created,
            source: TaskSource::UserRequest,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assigned_agent: None,
            plan: None,
            dependencies: vec![],
            dependents: vec![],
            iteration_count: 0,
            token_budget: 1000,
            tokens_used: 0,
            validation_result: None,
            autonomy_level: AutonomyLevel::Guided,
            paused_reason: None,
            paused_from_state: None,
            rejected_reason: None,
            injected_instructions: vec![],
            original_scope: None,
            current_scope: TaskScope::default(),
        }];
        let task_list = DataResponse::TaskList {
            tasks: tasks.clone(),
            correlation_id,
        };
        let json = serde_json::to_string(&task_list).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::TaskList {
                tasks: parsed_tasks,
                ..
            } => {
                assert_eq!(parsed_tasks.len(), 1);
                assert_eq!(parsed_tasks[0].description, "Test task 1");
            }
            other => panic!("Expected TaskList, got: {:?}", other),
        }

        // Test TaskDetail roundtrip
        let task = Task {
            id: task_id,
            description: "Detail test".to_string(),
            state: TaskState::Executing,
            source: TaskSource::UserRequest,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assigned_agent: None,
            plan: None,
            dependencies: vec![],
            dependents: vec![],
            iteration_count: 1,
            token_budget: 2000,
            tokens_used: 500,
            validation_result: None,
            autonomy_level: AutonomyLevel::Guided,
            paused_reason: None,
            paused_from_state: None,
            rejected_reason: None,
            injected_instructions: vec![],
            original_scope: None,
            current_scope: TaskScope::default(),
        };
        let task_detail = DataResponse::TaskDetail {
            task,
            correlation_id,
        };
        let json = serde_json::to_string(&task_detail).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::TaskDetail {
                task: parsed_task, ..
            } => {
                assert_eq!(parsed_task.id, task_id);
                assert_eq!(parsed_task.description, "Detail test");
            }
            other => panic!("Expected TaskDetail, got: {:?}", other),
        }

        // Test ConfigValue roundtrip
        let config_value = DataResponse::ConfigValue {
            key: "execution.max_iterations".to_string(),
            value: serde_json::json!(100),
            source_file: Some("/path/to/config.json".to_string()),
            correlation_id,
        };
        let json = serde_json::to_string(&config_value).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::ConfigValue {
                key,
                value,
                source_file,
                ..
            } => {
                assert_eq!(key, "execution.max_iterations");
                assert_eq!(value, serde_json::json!(100));
                assert_eq!(source_file, Some("/path/to/config.json".to_string()));
            }
            other => panic!("Expected ConfigValue, got: {:?}", other),
        }

        // Test MemoryResults roundtrip
        let memory_results = DataResponse::MemoryResults {
            results: r#"[{"content": "test memory"}]"#.to_string(),
            count: 1,
            correlation_id,
        };
        let json = serde_json::to_string(&memory_results).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::MemoryResults { results, count, .. } => {
                assert_eq!(count, 1);
                assert!(results.contains("test memory"));
            }
            other => panic!("Expected MemoryResults, got: {:?}", other),
        }

        // Test CostData roundtrip
        let cost_data = DataResponse::CostData {
            task_id: Some(task_id),
            total_input_tokens: 1000,
            total_output_tokens: 2000,
            total_cost_usd: 0.05,
            model_breakdown: vec![ModelCostInfo {
                model: "claude-sonnet".to_string(),
                call_count: 10,
                total_input_tokens: 1000,
                total_output_tokens: 2000,
                total_cost_usd: 0.05,
            }],
            correlation_id,
        };
        let json = serde_json::to_string(&cost_data).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::CostData {
                task_id: parsed_task_id,
                total_input_tokens,
                total_output_tokens,
                total_cost_usd,
                model_breakdown,
                ..
            } => {
                assert_eq!(parsed_task_id, Some(task_id));
                assert_eq!(total_input_tokens, 1000);
                assert_eq!(total_output_tokens, 2000);
                assert_eq!(total_cost_usd, 0.05);
                assert_eq!(model_breakdown.len(), 1);
            }
            other => panic!("Expected CostData, got: {:?}", other),
        }

        // Test DaemonHealth roundtrip
        let mut tasks_by_state = HashMap::new();
        tasks_by_state.insert("Created".to_string(), 5);
        tasks_by_state.insert("Executing".to_string(), 2);

        let daemon_health = DataResponse::DaemonHealth {
            active_connections: 3,
            total_tasks: 7,
            tasks_by_state,
            total_tokens: 50000,
            last_model: "claude-sonnet".to_string(),
            mcp_health: HashMap::new(),
            uptime_seconds: 3600,
            version: "1.0.0".to_string(),
            total_budget: 1_000_000,
            total_spent: 50000,
            remaining_budget: 950_000,
            correlation_id,
        };
        let json = serde_json::to_string(&daemon_health).expect("should serialize");
        let parsed: DataResponse = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DataResponse::DaemonHealth {
                active_connections,
                total_tasks,
                tasks_by_state,
                uptime_seconds,
                version,
                ..
            } => {
                assert_eq!(active_connections, 3);
                assert_eq!(total_tasks, 7);
                assert_eq!(tasks_by_state.get("Created"), Some(&5));
                assert_eq!(uptime_seconds, 3600);
                assert_eq!(version, "1.0.0");
            }
            other => panic!("Expected DaemonHealth, got: {:?}", other),
        }
    }

    #[test]
    fn test_data_response_in_daemon_event_serde() {
        // Verify DataResponse wraps correctly in DaemonEvent
        let correlation_id = Uuid::new_v4();
        let task = Task {
            id: Uuid::new_v4(),
            description: "Test".to_string(),
            state: TaskState::Created,
            source: TaskSource::UserRequest,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assigned_agent: None,
            plan: None,
            dependencies: vec![],
            dependents: vec![],
            iteration_count: 0,
            token_budget: 1000,
            tokens_used: 0,
            validation_result: None,
            autonomy_level: AutonomyLevel::Guided,
            paused_reason: None,
            paused_from_state: None,
            rejected_reason: None,
            injected_instructions: vec![],
            original_scope: None,
            current_scope: TaskScope::default(),
        };

        let event = DaemonEvent::DataResponse(Box::new(DataResponse::TaskDetail {
            task,
            correlation_id,
        }));

        let json = serde_json::to_string(&event).expect("should serialize");
        assert!(json.contains("DataResponse"));
        assert!(json.contains("TaskDetail"));

        let parsed: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");
        match parsed {
            DaemonEvent::DataResponse(data) => match &*data {
                DataResponse::TaskDetail {
                    correlation_id: cid,
                    ..
                } => {
                    assert_eq!(*cid, correlation_id);
                }
                other => panic!("Expected DataResponse::TaskDetail, got: {:?}", other),
            },
            other => panic!("Expected DataResponse::TaskDetail, got: {:?}", other),
        }
    }
}
