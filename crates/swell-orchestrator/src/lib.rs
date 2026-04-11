//! Orchestrator crate - coordinates multi-agent task execution.
//!
//! # Architecture
//!
//! The orchestrator manages:
//! - [`Orchestrator`] - main coordinator
//! - [`TaskStateMachine`] - state transitions
//! - [`TaskGraph`] - dependency tracking and execution ordering
//! - [`AgentPool`] - manages agent instances
//! - [`ExecutionController`] - handles parallel execution
//! - [`PolicyEngine`] - evaluates YAML-defined policies against agent actions

pub mod agents;
pub mod alerts;
pub mod autonomy;
pub mod backlog;
pub mod drift_detector;
pub mod execution;
pub mod feature_leads;
pub mod followup_generator;
pub mod frozen_spec;
pub mod gap_analyzer;
pub mod hard_limits;
pub mod metrics;
pub mod novelty_check;
pub mod policy;
pub mod retry_policy;
pub mod scheduler;
pub mod session_hygiene;
pub mod soft_limits;
pub mod state_machine;
pub mod task_board;
pub mod task_graph;

pub use agents::{
    AgentComment, AgentCommentType, AgentHandle, AgentHandoff, AgentPool, ChangeOperation,
    CodeIssue, CoderAgent, CondensationLevel, CondensationResult, ConfidenceLevel,
    ContextCondensation, ContextItem, ContextItemType, ContextWindow, CoverageMapping, DocChange,
    DocChangeType, DocWriterAgent, EvaluationResult, EvaluatorAgent, FileChange, GeneratorAgent,
    HandoffArtifact, IssueCategory, IssueSeverity, PlannerAgent, ReactLoop, ReactLoopState,
    ReactLoopSummary, ReactPhase, ReactStep, RefactorOpportunity, RefactorPlan, RefactorerAgent,
    RequirementCoverage, ReviewResult, ReviewerAgent, SystemPromptBuilder, SystemPromptConfig,
    TestPattern, TestSpec, TestWriterAgent, DEFAULT_REACT_MAX_ITERATIONS,
};
pub use alerts::{
    create_alert_manager, create_alert_manager_with_config, Alert, AlertCategory, AlertManager,
    AlertManagerConfig, ConsecutiveFailureConfig, CostThresholdConfig, LoopDetectionConfig,
    LoopDetectionState, PolicyViolationConfig, SharedAlertManager,
};
pub use autonomy::{ApprovalDecision, ApprovalRequest, AutonomyController};
pub use backlog::{
    BacklogItem, BacklogSource, BacklogStats, DeduplicationConfig, PriorityScoringConfig,
    WorkBacklog,
};
pub use drift_detector::{DriftDetector, DriftDetectorConfig, DriftReport, StepDrift};
pub use execution::ExecutionController;
pub use feature_leads::{
    FeatureLead, FeatureLeadManager, FeatureLeadSpawner, StepResult, FEATURE_LEAD_STEP_THRESHOLD,
    MAX_ORCHESTRATOR_DEPTH,
};
pub use followup_generator::{
    FollowUpContext, FollowUpGenerator, FollowUpGeneratorConfig, FollowUpOpportunity,
    FollowUpOpportunityType, FollowUpProposal,
};
pub use frozen_spec::{FrozenSpec, FrozenSpecRef};
pub use gap_analyzer::{
    CategoryGapReport, GapAnalysisReport, GapAnalyzer, GapAnalyzerConfig, ImplementationStatus,
    RequirementCategory, RequirementPriority, SpecRequirement,
};
pub use hard_limits::{
    create_hard_limits, create_hard_limits_with_config, HardLimitError, HardLimitWarning,
    HardLimits, HardLimitsCheck, HardLimitsConfig, SharedHardLimits,
};
pub use metrics::{
    create_metrics_collector, create_metrics_collector_with_thresholds, AggregatedMetrics,
    AlertSeverity, AlertThresholds, AlertType, MetricSample, MetricsAlert, MetricsCollector,
    MetricsWindow, OrchestratorMetrics, SharedMetricsCollector,
};
pub use novelty_check::{
    levenshtein_distance, NoveltyCheckResult, NoveltyChecker, NoveltyCheckerConfig, TrackedTask,
};
pub use policy::{
    action, PolicyAction, PolicyCondition, PolicyDecision, PolicyEffect, PolicyEngine, PolicyFile,
    PolicyRule,
};
pub use retry_policy::{
    RetryDecision, RetryPolicy, RetryState, MAX_RETRIES_BEFORE_ESCALATION, MODEL_SWITCH_RETRY_COUNT,
};
pub use scheduler::{
    Scheduler, SchedulerConfig, SchedulerStats, TaskPriority, DEFAULT_MAX_WORKERS, MAX_MAX_WORKERS,
};
pub use soft_limits::{
    create_soft_limits, create_soft_limits_with_config, ProgressTracker, SharedSoftLimits,
    SoftLimitType, SoftLimitWarning, SoftLimits, SoftLimitsConfig,
};
pub use session_hygiene::{
    ProgressEvaluation, ProgressHealth, SessionCheckpoint, SessionHygiene,
    SessionHygieneConfig,
};
pub use state_machine::TaskStateMachine;
pub use task_board::{
    create_task_board, CostBreakdownEntry, CostModel, SharedTaskBoard, TaskBoard, TaskBoardEntry,
    TaskBoardStats,
};
pub use task_graph::TaskGraph;

use std::sync::Arc;
use swell_core::{
    AgentId, AgentRole, Checkpoint, Plan, SwellError, Task, TaskState, ValidationResult,
};
use swell_state::{traits::in_memory::InMemoryCheckpointStore, CheckpointManager};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

/// Maximum concurrent agents
pub const MAX_CONCURRENT_AGENTS: usize = 6;

/// Events emitted by the orchestrator
#[derive(Debug, Clone)]
pub enum OrchestratorEvent {
    TaskCreated(Uuid),
    TaskStateChanged {
        task_id: Uuid,
        from: TaskState,
        to: TaskState,
    },
    AgentStarted {
        agent_id: AgentId,
        task_id: Uuid,
    },
    AgentFinished {
        agent_id: AgentId,
        task_id: Uuid,
    },
    ExecutionProgress {
        task_id: Uuid,
        message: String,
    },
}

/// The main orchestrator that coordinates agents and tasks
pub struct Orchestrator {
    state_machine: Arc<RwLock<TaskStateMachine>>,
    agent_pool: Arc<RwLock<AgentPool>>,
    checkpoint_manager: Arc<CheckpointManager>,
    event_sender: mpsc::UnboundedSender<OrchestratorEvent>,
}

impl Orchestrator {
    /// Create a new orchestrator with default in-memory checkpoint store
    pub fn new() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        let checkpoint_store = Arc::new(InMemoryCheckpointStore::new());
        let checkpoint_manager = Arc::new(CheckpointManager::new(checkpoint_store));

        Self {
            state_machine: Arc::new(RwLock::new(TaskStateMachine::new())),
            agent_pool: Arc::new(RwLock::new(AgentPool::new())),
            checkpoint_manager,
            event_sender: tx,
        }
    }

    /// Create a new orchestrator with a custom checkpoint manager
    pub fn with_checkpoint_manager(checkpoint_manager: Arc<CheckpointManager>) -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();

        Self {
            state_machine: Arc::new(RwLock::new(TaskStateMachine::new())),
            agent_pool: Arc::new(RwLock::new(AgentPool::new())),
            checkpoint_manager,
            event_sender: tx,
        }
    }

    /// Create a new task
    pub async fn create_task(&self, description: String) -> Task {
        let task = {
            let mut sm = self.state_machine.write().await;
            sm.create_task(description)
        };
        let _ = self
            .event_sender
            .send(OrchestratorEvent::TaskCreated(task.id));
        task
    }

    /// Get a task by ID
    pub async fn get_task(&self, id: Uuid) -> Result<Task, SwellError> {
        let sm = self.state_machine.read().await;
        sm.get_task(id)
    }

    /// Register a new agent
    pub async fn register_agent(&self, role: AgentRole, model: String) -> AgentId {
        let mut pool = self.agent_pool.write().await;
        pool.register(role, model)
    }

    /// Get available agent count for a role
    pub async fn available_agents(&self, role: AgentRole) -> usize {
        let pool = self.agent_pool.read().await;
        pool.available_count(role)
    }

    /// Assign a task to an available agent
    pub async fn assign_task(&self, task_id: Uuid, role: AgentRole) -> Result<AgentId, SwellError> {
        let agent_id = {
            let mut pool = self.agent_pool.write().await;
            pool.reserve(task_id, role)?
        };

        {
            let mut sm = self.state_machine.write().await;
            sm.assign_task(task_id, agent_id)?;
        }

        let _ = self
            .event_sender
            .send(OrchestratorEvent::AgentStarted { agent_id, task_id });
        Ok(agent_id)
    }

    /// Release an agent back to the pool
    pub async fn release_agent(&self, agent_id: AgentId, task_id: Uuid) {
        {
            let mut pool = self.agent_pool.write().await;
            pool.release(agent_id)
        };
        let _ = self
            .event_sender
            .send(OrchestratorEvent::AgentFinished { agent_id, task_id });
    }

    /// Get all tasks
    pub async fn get_all_tasks(&self) -> Vec<Task> {
        let sm = self.state_machine.read().await;
        sm.get_all_tasks()
    }

    /// Get tasks by state
    pub async fn get_tasks_by_state(&self, state: TaskState) -> Vec<Task> {
        let sm = self.state_machine.read().await;
        sm.get_tasks_by_state(state)
    }

    /// Set a plan for a task
    pub async fn set_plan(&self, task_id: Uuid, plan: Plan) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.set_plan(task_id, plan)
    }

    /// Transition task through planning -> ready -> executing
    pub async fn start_task(&self, task_id: Uuid) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;

        // Only enrich if task is in Created state (not after retry)
        let task = sm.get_task(task_id)?;
        if task.state == TaskState::Created {
            sm.enrich_task(task_id)?;
        }

        let task = sm.get_task(task_id)?;
        if task.plan.is_none() {
            return Err(SwellError::InvalidStateTransition(
                "Cannot start task without plan".into(),
            ));
        }

        if task.state == TaskState::Enriched {
            sm.ready_task(task_id)?;
        }

        let task = sm.get_task(task_id)?;
        if task.state == TaskState::Ready {
            sm.assign_task(task_id, Uuid::nil())?; // Will be reassigned when agent picks it up
        }

        sm.start_execution(task_id)?;

        Ok(())
    }

    /// Transition to validating state
    pub async fn start_validation(&self, task_id: Uuid) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.start_validation(task_id)
    }

    /// Complete task with validation result
    pub async fn complete_task(
        &self,
        task_id: Uuid,
        result: ValidationResult,
    ) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;

        // Store validation result
        if let Ok(task) = sm.get_task_mut(task_id) {
            task.validation_result = Some(result.clone());
        }

        if result.passed {
            sm.accept_task(task_id)?;
            info!(task_id = %task_id, "Task accepted");
        } else {
            sm.reject_task(task_id)?;
            info!(task_id = %task_id, "Task rejected");

            // Evaluate retry policy for escalation decision
            let retry_policy = RetryPolicy::new();
            if let Ok(task) = sm.get_task(task_id) {
                let decision = retry_policy.evaluate_for_iteration(task.iteration_count);
                if decision == RetryDecision::EscalateToHuman {
                    sm.escalate_task(task_id)?;
                    warn!(task_id = %task_id, iteration_count = %task.iteration_count, "Task escalated to human after retry exhaustion");
                }
            }
        }

        Ok(())
    }

    /// Get the state machine for direct access (use sparingly)
    pub fn state_machine(&self) -> Arc<RwLock<TaskStateMachine>> {
        self.state_machine.clone()
    }

    /// Get the checkpoint manager for direct access (use sparingly)
    pub fn checkpoint_manager(&self) -> Arc<CheckpointManager> {
        self.checkpoint_manager.clone()
    }

    /// Restore a task from its latest checkpoint
    ///
    /// Returns the restored task if a checkpoint exists, or None if no checkpoint found.
    pub async fn restore_task(&self, task_id: Uuid) -> Result<Option<Task>, SwellError> {
        // Restore from checkpoint
        let restored_task = self.checkpoint_manager.restore(task_id).await?;

        if let Some(task) = restored_task {
            // Update the state machine with the restored task
            let mut sm = self.state_machine.write().await;
            let existing_task = sm.get_task_mut(task_id);

            match existing_task {
                Ok(existing) => {
                    // Update existing task with restored state
                    *existing = task.clone();
                    info!(task_id = %task_id, "Task restored from checkpoint");
                }
                Err(_) => {
                    // Task doesn't exist in state machine - this is unusual but we can handle it
                    // by not inserting - the restored task is returned but not stored
                    warn!(task_id = %task_id, "Task restored from checkpoint but not found in state machine");
                }
            }
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// Check if a task has any checkpoints
    pub async fn has_checkpoint(&self, task_id: Uuid) -> Result<bool, SwellError> {
        self.checkpoint_manager.has_checkpoint(task_id).await
    }

    /// Get checkpoint history for a task
    pub async fn get_checkpoint_history(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<Checkpoint>, SwellError> {
        self.checkpoint_manager.list_checkpoints(task_id).await
    }

    // ========================================================================
    // Operator Intervention APIs
    // ========================================================================

    /// Pause a task (operator-initiated)
    pub async fn pause_task(&self, task_id: Uuid, reason: String) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.pause_task(task_id, reason)
    }

    /// Resume a paused task
    pub async fn resume_task(&self, task_id: Uuid) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.resume_task(task_id)
    }

    /// Inject instructions into a task
    pub async fn inject_instruction(
        &self,
        task_id: Uuid,
        instruction: String,
    ) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.inject_instruction(task_id, instruction)
    }

    /// Modify task scope boundaries
    pub async fn modify_scope(
        &self,
        task_id: Uuid,
        new_scope: swell_core::TaskScope,
    ) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.modify_scope(task_id, new_scope)
    }

    /// Restore original scope (revert modify_scope)
    pub async fn restore_original_scope(&self, task_id: Uuid) -> Result<(), SwellError> {
        let mut sm = self.state_machine.write().await;
        sm.restore_original_scope(task_id)
    }

    /// Get injected instructions for a task
    pub async fn get_injected_instructions(
        &self,
        task_id: Uuid,
    ) -> Result<Vec<String>, SwellError> {
        let sm = self.state_machine.read().await;
        let task = sm.get_task(task_id)?;
        Ok(task.injected_instructions.clone())
    }

    /// Get current scope for a task
    pub async fn get_task_scope(&self, task_id: Uuid) -> Result<swell_core::TaskScope, SwellError> {
        let sm = self.state_machine.read().await;
        let task = sm.get_task(task_id)?;
        Ok(task.current_scope.clone())
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Orchestrator Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{Plan, PlanStep, RiskLevel, StepStatus, ValidationResult};

    fn create_test_plan(task_id: Uuid) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            task_id,
            steps: vec![PlanStep {
                id: Uuid::new_v4(),
                description: "Test step".to_string(),
                affected_files: vec!["test.rs".to_string()],
                expected_tests: vec!["test_foo".to_string()],
                risk_level: RiskLevel::Low,
                dependencies: vec![],
                status: StepStatus::Pending,
            }],
            total_estimated_tokens: 1000,
            risk_assessment: "Low risk".to_string(),
        }
    }

    // --- create_task Tests ---

    #[tokio::test]
    async fn test_create_task_returns_task_with_created_state() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test task".to_string()).await;

        assert_eq!(task.state, TaskState::Created);
        assert_eq!(task.description, "Test task");
        assert!(task.plan.is_none());
    }

    #[tokio::test]
    async fn test_create_task_assigns_unique_id() {
        let orchestrator = Orchestrator::new();

        let task1 = orchestrator.create_task("Task 1".to_string()).await;
        let task2 = orchestrator.create_task("Task 2".to_string()).await;

        assert_ne!(task1.id, task2.id);
    }

    // --- get_task Tests ---

    #[tokio::test]
    async fn test_get_task_returns_task() {
        let orchestrator = Orchestrator::new();
        let created = orchestrator.create_task("Test".to_string()).await;

        let retrieved = orchestrator.get_task(created.id).await.unwrap();

        assert_eq!(retrieved.id, created.id);
        assert_eq!(retrieved.description, created.description);
    }

    #[tokio::test]
    async fn test_get_task_returns_error_for_nonexistent() {
        let orchestrator = Orchestrator::new();
        let fake_id = Uuid::new_v4();

        let result = orchestrator.get_task(fake_id).await;

        assert!(result.is_err());
    }

    // --- register_agent Tests ---

    #[tokio::test]
    async fn test_register_agent_returns_agent_id() {
        let orchestrator = Orchestrator::new();

        let agent_id = orchestrator
            .register_agent(AgentRole::Planner, "claude-sonnet".to_string())
            .await;

        assert_ne!(agent_id, Uuid::nil());
    }

    #[tokio::test]
    async fn test_register_multiple_agents() {
        let orchestrator = Orchestrator::new();

        let planner_id = orchestrator
            .register_agent(AgentRole::Planner, "claude-sonnet".to_string())
            .await;
        let generator_id = orchestrator
            .register_agent(AgentRole::Generator, "claude-sonnet".to_string())
            .await;

        assert_ne!(planner_id, generator_id);
    }

    #[tokio::test]
    async fn test_available_agents_returns_count() {
        let orchestrator = Orchestrator::new();

        assert_eq!(orchestrator.available_agents(AgentRole::Planner).await, 0);

        orchestrator
            .register_agent(AgentRole::Planner, "claude-sonnet".to_string())
            .await;

        assert_eq!(orchestrator.available_agents(AgentRole::Planner).await, 1);
    }

    // --- assign_task Tests ---

    #[tokio::test]
    async fn test_assign_task_reserves_agent_and_assigns_to_task() {
        let orchestrator = Orchestrator::new();

        let agent_id = orchestrator
            .register_agent(AgentRole::Generator, "claude-sonnet".to_string())
            .await;
        let task = orchestrator.create_task("Test".to_string()).await;

        // Set plan and transition to Ready first
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task.id).unwrap();
            sm_guard.ready_task(task.id).unwrap();
        }

        let assigned_agent = orchestrator
            .assign_task(task.id, AgentRole::Generator)
            .await
            .unwrap();

        assert_eq!(assigned_agent, agent_id);
        assert_eq!(orchestrator.available_agents(AgentRole::Generator).await, 0);
    }

    #[tokio::test]
    async fn test_assign_task_returns_error_when_no_agent_available() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task.id).unwrap();
            sm_guard.ready_task(task.id).unwrap();
        }

        let result = orchestrator
            .assign_task(task.id, AgentRole::Generator)
            .await;

        assert!(result.is_err());
    }

    // --- release_agent Tests ---

    #[tokio::test]
    async fn test_release_agent_returns_agent_to_pool() {
        let orchestrator = Orchestrator::new();

        let agent_id = orchestrator
            .register_agent(AgentRole::Generator, "claude-sonnet".to_string())
            .await;
        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task.id).unwrap();
            sm_guard.ready_task(task.id).unwrap();
        }

        orchestrator
            .assign_task(task.id, AgentRole::Generator)
            .await
            .unwrap();
        assert_eq!(orchestrator.available_agents(AgentRole::Generator).await, 0);

        orchestrator.release_agent(agent_id, task.id).await;

        assert_eq!(orchestrator.available_agents(AgentRole::Generator).await, 1);
    }

    // --- get_all_tasks and get_tasks_by_state Tests ---

    #[tokio::test]
    async fn test_get_all_tasks_returns_all_tasks() {
        let orchestrator = Orchestrator::new();

        orchestrator.create_task("Task 1".to_string()).await;
        orchestrator.create_task("Task 2".to_string()).await;

        let all = orchestrator.get_all_tasks().await;

        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_get_tasks_by_state_filters_correctly() {
        let orchestrator = Orchestrator::new();

        let task1 = orchestrator.create_task("Task 1".to_string()).await;
        let _task2 = orchestrator.create_task("Task 2".to_string()).await;

        // Transition task1 to Enriched
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.enrich_task(task1.id).unwrap();
        }

        let created_tasks = orchestrator.get_tasks_by_state(TaskState::Created).await;
        let enriched_tasks = orchestrator.get_tasks_by_state(TaskState::Enriched).await;

        assert_eq!(created_tasks.len(), 1);
        assert_eq!(enriched_tasks.len(), 1);
    }

    // --- set_plan Tests ---

    #[tokio::test]
    async fn test_set_plan_attaches_plan_to_task() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);

        orchestrator.set_plan(task.id, plan.clone()).await.unwrap();

        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert!(retrieved.plan.is_some());
        assert_eq!(retrieved.plan.unwrap().id, plan.id);
    }

    // --- start_task Tests ---

    #[tokio::test]
    async fn test_start_task_transitions_through_states() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();

        orchestrator.start_task(task.id).await.unwrap();

        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(retrieved.state, TaskState::Executing);
    }

    #[tokio::test]
    async fn test_start_task_fails_without_plan() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;

        let result = orchestrator.start_task(task.id).await;

        assert!(result.is_err());
    }

    // --- start_validation Tests ---

    #[tokio::test]
    async fn test_start_validation_transitions_to_validating() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        orchestrator.start_task(task.id).await.unwrap();

        orchestrator.start_validation(task.id).await.unwrap();

        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(retrieved.state, TaskState::Validating);
    }

    #[tokio::test]
    async fn test_start_validation_fails_if_not_executing() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;

        let result = orchestrator.start_validation(task.id).await;

        assert!(result.is_err());
    }

    // --- complete_task Tests ---

    #[tokio::test]
    async fn test_complete_task_with_passed_validation_accepts_task() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();

        let result = ValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: vec![],
            warnings: vec![],
        };

        orchestrator.complete_task(task.id, result).await.unwrap();

        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(retrieved.state, TaskState::Accepted);
        assert!(retrieved.validation_result.is_some());
    }

    #[tokio::test]
    async fn test_complete_task_with_failed_validation_rejects_task() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();

        let result = ValidationResult {
            passed: false,
            lint_passed: false,
            tests_passed: false,
            security_passed: true,
            ai_review_passed: true,
            errors: vec!["Test failed".to_string()],
            warnings: vec![],
        };

        orchestrator.complete_task(task.id, result).await.unwrap();

        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(retrieved.state, TaskState::Rejected);
        assert_eq!(retrieved.iteration_count, 1);
    }

    #[tokio::test]
    async fn test_complete_task_escalates_after_4_failures() {
        let orchestrator = Orchestrator::new();

        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();

        let failed_result = ValidationResult {
            passed: false,
            lint_passed: false,
            tests_passed: false,
            security_passed: true,
            ai_review_passed: true,
            errors: vec!["Failed".to_string()],
            warnings: vec![],
        };

        // First failure: Rejected with iteration_count=1
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();
        orchestrator
            .complete_task(task.id, failed_result.clone())
            .await
            .unwrap();
        {
            let sm = orchestrator.state_machine();
            let sm_guard = sm.read().await;
            let task = sm_guard.get_task(task.id).unwrap();
            assert_eq!(task.state, TaskState::Rejected);
            assert_eq!(task.iteration_count, 1);
        }

        // Retry and second failure: Rejected with iteration_count=2
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.retry_task(task.id).unwrap();
        }
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();
        orchestrator
            .complete_task(task.id, failed_result.clone())
            .await
            .unwrap();
        {
            let sm = orchestrator.state_machine();
            let sm_guard = sm.read().await;
            let task = sm_guard.get_task(task.id).unwrap();
            assert_eq!(task.state, TaskState::Rejected);
            assert_eq!(task.iteration_count, 2);
        }

        // Retry and third failure: iteration_count=3, still Rejected (model switch retry)
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.retry_task(task.id).unwrap();
        }
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();
        orchestrator
            .complete_task(task.id, failed_result.clone())
            .await
            .unwrap();
        {
            let sm = orchestrator.state_machine();
            let sm_guard = sm.read().await;
            let task = sm_guard.get_task(task.id).unwrap();
            assert_eq!(task.state, TaskState::Rejected);
            assert_eq!(task.iteration_count, 3);
        }

        // Retry and fourth failure: escalates (iteration_count=4 >= threshold)
        {
            let sm = orchestrator.state_machine();
            let mut sm_guard = sm.write().await;
            sm_guard.retry_task(task.id).unwrap();
        }
        orchestrator.start_task(task.id).await.unwrap();
        orchestrator.start_validation(task.id).await.unwrap();
        orchestrator
            .complete_task(task.id, failed_result)
            .await
            .unwrap();

        // Task should now be Escalated
        let retrieved = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(retrieved.state, TaskState::Escalated);
        assert_eq!(retrieved.iteration_count, 4);
    }

    // --- Full Lifecycle Integration Test ---

    #[tokio::test]
    async fn test_full_task_lifecycle() {
        let orchestrator = Orchestrator::new();

        // 1. Create task
        let task = orchestrator
            .create_task("Implement feature X".to_string())
            .await;
        assert_eq!(task.state, TaskState::Created);

        // 2. Register agents
        let planner_id = orchestrator
            .register_agent(AgentRole::Planner, "claude-sonnet".to_string())
            .await;
        let generator_id = orchestrator
            .register_agent(AgentRole::Generator, "claude-sonnet".to_string())
            .await;

        assert_ne!(planner_id, generator_id);
        assert_eq!(orchestrator.available_agents(AgentRole::Planner).await, 1);
        assert_eq!(orchestrator.available_agents(AgentRole::Generator).await, 1);

        // 3. Set plan
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();

        // 4. Start task (enrich -> ready -> assign -> execute)
        orchestrator.start_task(task.id).await.unwrap();
        let task_after_start = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(task_after_start.state, TaskState::Executing);

        // 5. Start validation
        orchestrator.start_validation(task.id).await.unwrap();
        let task_validating = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(task_validating.state, TaskState::Validating);

        // 6. Complete with success
        let success_result = ValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: vec![],
            warnings: vec![],
        };
        orchestrator
            .complete_task(task.id, success_result)
            .await
            .unwrap();

        let final_task = orchestrator.get_task(task.id).await.unwrap();
        assert_eq!(final_task.state, TaskState::Accepted);
        assert!(final_task.validation_result.is_some());
        assert!(final_task.validation_result.unwrap().passed);

        // Verify all tasks
        let all_tasks = orchestrator.get_all_tasks().await;
        assert_eq!(all_tasks.len(), 1);
    }

    // --- Error Handling Tests ---

    #[tokio::test]
    async fn test_get_nonexistent_task_returns_error() {
        let orchestrator = Orchestrator::new();
        let result = orchestrator.get_task(Uuid::new_v4()).await;

        assert!(matches!(result.unwrap_err(), SwellError::TaskNotFound(_)));
    }

    #[tokio::test]
    async fn test_assign_task_fails_with_invalid_state() {
        let orchestrator = Orchestrator::new();

        // Try to assign a task that hasn't been made ready
        let task = orchestrator.create_task("Test".to_string()).await;
        let plan = create_test_plan(task.id);
        orchestrator.set_plan(task.id, plan).await.unwrap();

        // Skip enrich and ready steps
        let result = orchestrator
            .assign_task(task.id, AgentRole::Generator)
            .await;

        // Should fail because task is not in Ready state
        assert!(result.is_err());
    }
}
